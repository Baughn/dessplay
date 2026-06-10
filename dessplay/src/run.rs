//! The headless composition root: everything `main()` does beyond flag
//! parsing. Builds the QUIC connector (TOFU-pinned), loads settings and
//! stored state, spawns the client actors, and runs the event loop
//! until Ctrl-C (graceful Goodbye) or auth failure.
//!
//! Phase 6 grows an interactive sibling with the UI actor; `--seeder`
//! keeps using this one forever. See architecture.md (Composition
//! Root).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use dessplay_core::net::quic::QuicConnector;
use dessplay_core::net::{DEFAULT_PORT, Role};
use dessplay_core::types::UserId;

use crate::actors::network::{NetworkCommand, NetworkEvent};
use crate::actors::sync::SyncCommand;
use crate::client::{ClientConfig, ClientEvent, SyncConfigExtras, spawn_client};
use crate::storage::Storage;

/// Everything `main()` parses from flags. `None` means "fall back to
/// stored settings / environment / defaults".
#[derive(Debug, Default)]
pub struct HeadlessArgs {
    /// Run as a seeder: no settings database, flags/env only, never
    /// gates playback.
    pub seeder: bool,
    /// Server `host[:port]`.
    pub server: Option<String>,
    /// Username.
    pub username: Option<String>,
    /// Room password.
    pub password: Option<String>,
    /// Hex-encoded server certificate fingerprint to pin (mainly for
    /// seeders, which persist nothing).
    pub fingerprint: Option<String>,
    /// Settings database override (interactive only).
    pub db_path: Option<PathBuf>,
}

/// The wall clock in unix millis — the client-side equivalent of the
/// server's shared clock source.
fn system_clock() -> Arc<dyn Fn() -> u64 + Send + Sync> {
    Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    })
}

/// Append the default port unless the address already has one. A
/// `host:port` has exactly one colon; a bracketed IPv6 literal carries
/// its port after `]:`; multiple colons without brackets is a bare
/// IPv6 literal that needs wrapping.
fn with_default_port(server: &str) -> String {
    let has_port = match server.matches(':').count() {
        0 => false,
        1 => true,
        _ => server.starts_with('[') && server.contains("]:"),
    };
    if has_port {
        server.to_string()
    } else if server.contains(':') {
        format!("[{server}]:{DEFAULT_PORT}")
    } else {
        format!("{server}:{DEFAULT_PORT}")
    }
}

/// Run the headless client until Ctrl-C. Errors are human-readable —
/// `main()` just prints them.
pub async fn run_headless(args: HeadlessArgs) -> Result<(), String> {
    // ---- Settings: stored for interactive clients (flags override,
    // never persisted), flags/env only for seeders.
    let storage = if args.seeder {
        None
    } else {
        let path = match &args.db_path {
            Some(path) => path.clone(),
            None => Storage::default_path().ok_or("cannot determine the data directory")?,
        };
        Some(Storage::open(&path).map_err(|e| format!("opening {}: {e}", path.display()))?)
    };
    let settings = match &storage {
        Some(storage) => storage
            .load_settings()
            .map_err(|e| format!("loading settings: {e}"))?,
        None => crate::config::Settings::default(),
    };

    let username = args
        .username
        .or(settings.username)
        .ok_or("no username configured; pass --username")?;
    let password = args
        .password
        .or_else(|| std::env::var("DESSPLAY_PASSWORD").ok())
        .or(settings.password)
        .ok_or("no password configured; pass --password or set DESSPLAY_PASSWORD")?;
    let server = args.server.unwrap_or(settings.server);

    // ---- Resolve and pin.
    let server_addr_str = with_default_port(&server);
    let addr: SocketAddr = tokio::net::lookup_host(&server_addr_str)
        .await
        .map_err(|e| format!("resolving {server_addr_str}: {e}"))?
        .next()
        .ok_or_else(|| format!("{server_addr_str} resolved to no addresses"))?;
    let server_name = server_addr_str
        .rsplit_once(':')
        .map(|(host, _)| host.trim_matches(['[', ']']))
        .unwrap_or(&server_addr_str)
        .to_string();

    let pinned = match &args.fingerprint {
        Some(hex) => Some(decode_hex(hex)?),
        None => match &storage {
            Some(storage) => storage
                .tofu_fingerprint(&server_addr_str)
                .map_err(|e| format!("reading pinned fingerprint: {e}"))?,
            None => None,
        },
    };
    let first_use = pinned.is_none();
    if first_use {
        tracing::warn!("no pinned fingerprint for {server_addr_str}; trusting on first use");
    }
    let connector = Arc::new(
        QuicConnector::new(addr, server_name, pinned)
            .map_err(|e| format!("building QUIC endpoint: {e}"))?,
    );

    // ---- Stored CRDT state, and a second storage handle for the sync
    // actor (SQLite in WAL mode is fine with two connections; the sync
    // actor owns its handle outright).
    let (initial, sync_storage) = match (&args.db_path, args.seeder) {
        (_, true) => (None, None),
        (path, false) => {
            let path = match path {
                Some(path) => path.clone(),
                None => Storage::default_path().ok_or("cannot determine the data directory")?,
            };
            let sync_storage =
                Storage::open(&path).map_err(|e| format!("opening {}: {e}", path.display()))?;
            let initial = sync_storage
                .load_state()
                .map_err(|e| format!("loading stored state: {e}"))?;
            (initial, Some(sync_storage))
        }
    };

    let role = if args.seeder {
        Role::Seeder
    } else {
        Role::Interactive
    };
    let mut handle = spawn_client(
        Arc::clone(&connector),
        ClientConfig {
            username: UserId::new(&username),
            password,
            role,
            session_nonce: rand::random(),
            clock: system_clock(),
            sync: SyncConfigExtras {
                initial,
                storage: sync_storage,
                flush_interval: None,
            },
        },
    );
    tracing::info!("{username} ({role:?}) connecting to {server_addr_str}");

    // ---- Event loop until Ctrl-C.
    let mut pin_pending = first_use && !args.seeder;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
                break;
            }
            event = handle.events.recv() => {
                let Some(event) = event else { break };
                match event {
                    ClientEvent::Network(NetworkEvent::Connected { observed_addr }) => {
                        tracing::info!("connected (we are {observed_addr})");
                        if pin_pending
                            && let Some(storage) = &storage
                            && let Some(fingerprint) = connector.observed_fingerprint()
                        {
                            let now = (system_clock())() as i64;
                            match storage.store_tofu_fingerprint(
                                &server_addr_str,
                                &fingerprint,
                                now,
                            ) {
                                Ok(()) => {
                                    pin_pending = false;
                                    tracing::info!("pinned server fingerprint on first use");
                                }
                                Err(e) => tracing::error!("storing fingerprint: {e}"),
                            }
                        }
                    }
                    ClientEvent::Network(NetworkEvent::AuthFailed) => {
                        return Err("the server rejected the password".into());
                    }
                    ClientEvent::Network(NetworkEvent::Disconnected { reason }) => {
                        tracing::warn!("disconnected ({reason}); retrying");
                    }
                    ClientEvent::Network(NetworkEvent::PeerList(peers)) => {
                        let listed: Vec<String> = peers
                            .iter()
                            .map(|p| format!("{} [{:?}/{:?}]", p.username, p.role, p.presence))
                            .collect();
                        tracing::info!("peers: {}", listed.join(", "));
                    }
                    other => tracing::debug!("{other:?}"),
                }
            }
        }
    }

    // Graceful teardown: Goodbye to the server, flush state to SQLite.
    // Each actor drops its command receiver when it exits, so closed()
    // is the completion signal.
    let _ = handle.network.send(NetworkCommand::Shutdown).await;
    let _ = handle.sync.send(SyncCommand::Shutdown).await;
    handle.network.closed().await;
    handle.sync.closed().await;
    Ok(())
}

fn decode_hex(hex: &str) -> Result<Vec<u8>, String> {
    let hex: String = hex
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ':')
        .collect();
    if !hex.len().is_multiple_of(2) {
        return Err("fingerprint hex has odd length".into());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| format!("bad hex: {e}")))
        .collect()
}

/// Load `./.env` (KEY=VALUE lines; `#` comments) into the environment,
/// without overriding variables that are already set. The project keeps
/// `DESSPLAY_PASSWORD` for the default server there.
pub fn load_dotenv() {
    let Ok(contents) = std::fs::read_to_string(".env") else {
        return;
    };
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let (key, value) = (key.trim(), value.trim().trim_matches('"'));
            if std::env::var_os(key).is_none() {
                // Single-threaded startup: set_var is safe here.
                unsafe { std::env::set_var(key, value) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn default_port_handling() {
        assert_eq!(with_default_port("example.com"), "example.com:9876");
        assert_eq!(with_default_port("example.com:443"), "example.com:443");
        assert_eq!(with_default_port("10.0.0.1"), "10.0.0.1:9876");
        assert_eq!(with_default_port("10.0.0.1:7000"), "10.0.0.1:7000");
        assert_eq!(with_default_port("::1"), "[::1]:9876");
        assert_eq!(with_default_port("[::1]:7000"), "[::1]:7000");
    }

    #[test]
    fn hex_decoding() {
        assert_eq!(decode_hex("0aff").unwrap(), vec![0x0a, 0xff]);
        assert_eq!(decode_hex("0A:FF").unwrap(), vec![0x0a, 0xff]);
        assert!(decode_hex("abc").is_err());
        assert!(decode_hex("zz").is_err());
    }
}

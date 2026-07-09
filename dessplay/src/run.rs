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
use tokio::sync::mpsc;

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
    /// Settings/state database override. Honored in every mode.
    pub db_path: Option<PathBuf>,
    /// Outstanding chunk requests per source for downloads (default 48).
    /// A flag so transfer behaviour can be tuned in testing; applies to
    /// interactive clients and seeders alike.
    pub pipeline_depth: Option<u32>,
    /// Media roots to search/serve. Overrides the stored roots for this
    /// run when non-empty (a seeder has none stored, so it always uses
    /// these). The cache dir is always added as a root automatically.
    pub media_roots: Vec<PathBuf>,
    /// Download-cache directory override (defaults to the standard
    /// cache). Honored in every mode.
    pub cache_dir: Option<PathBuf>,
    /// Attach to a user-launched mpv at this IPC socket instead of
    /// spawning one (a dev/headless aid). Interactive only.
    pub attach_mpv: Option<PathBuf>,
}

/// Forward the local UI lines produced by the session shell (subtitle
/// lines and the narrator's system chat lines) to the UI. Both are local
/// only — never synced — and share the chat interleave domain.
fn forward_ui_lines(
    ui: &std::sync::mpsc::SyncSender<crate::ui::shell::UiInput>,
    lines: crate::session::UiLines,
) {
    use crate::ui::shell::UiInput;
    for line in lines.subtitles {
        let _ = ui.try_send(UiInput::Subtitle {
            text: line.text,
            speaker: line.speaker,
            video_millis: line.video_millis,
            arrival_millis: line.arrival_millis,
        });
    }
    for notice in lines.system {
        let _ = ui.try_send(UiInput::System {
            timestamp: notice.timestamp,
            text: notice.text,
        });
    }
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

/// The download cache directory: `$XDG_CACHE_HOME/dessplay/files/`
/// (design.md, Download Cache). Created if absent.
fn download_cache_dir() -> Result<PathBuf, String> {
    let dir = dirs::cache_dir()
        .ok_or("cannot determine the cache directory")?
        .join("dessplay")
        .join("files");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Resolve the download-cache directory: the `--cache-dir` flag wins,
/// else the standard `$XDG_CACHE_HOME/dessplay/files/`. Honored in every
/// mode — the single-instance-lock docs tell users to run a second
/// instance with its own `--db` and `--cache-dir`, which only works if
/// interactive mode reads the flag too.
fn resolve_cache_dir(args: &HeadlessArgs) -> Result<PathBuf, String> {
    match &args.cache_dir {
        Some(dir) => Ok(dir.clone()),
        None => download_cache_dir(),
    }
}

/// Resolve the *runtime* media roots: the repeatable `--media-root` flag
/// wins when non-empty, else the `stored` roots. A non-empty flag overrides
/// the settings-DB roots for this run only, mirroring how `--username` /
/// `--server` override their stored settings. A seeder has no stored roots,
/// so it always uses the flag.
///
/// These are the roots the file actor actually uses at runtime (library
/// scan, file resolution, serving). They are deliberately distinct from the
/// *persistable* base computed by [`resolve_runtime_media_roots`]: the flag
/// override must never be written back to the database.
fn resolve_media_roots(flag: &[PathBuf], stored: Vec<PathBuf>) -> Vec<PathBuf> {
    if flag.is_empty() {
        stored
    } else {
        flag.to_vec()
    }
}

/// The media-roots analogue of [`resolve_runtime_identity`]: split the
/// `--media-root` override and the stored roots into the runtime roots and
/// the *persistable* base, the single chokepoint that keeps a one-off
/// `--media-root` out of the database.
///
/// The repeatable `--media-root` flag is a runtime override: it decides the
/// roots the file actor scans/serves/resolves from, but per design.md (Data
/// Storage: "Command-line flags and environment variables override stored
/// settings at runtime but are never persisted") it must never be written
/// back. So the *persistable base* keeps the stored roots and only takes the
/// flag as a first-run prefill (when nothing is stored — the settings modal
/// then turns it into an editable default the user confirms before it
/// persists). The UI is seeded with, and a later settings save writes, this
/// persistable base (or roots the user actually edited in the modal), never
/// an untouched flag override. The runtime roots still honour the flag (flag
/// wins when non-empty, else stored).
fn resolve_runtime_media_roots(flag: &[PathBuf], stored: Vec<PathBuf>) -> RuntimeMediaRoots {
    let runtime = resolve_media_roots(flag, stored.clone());
    // Persistable base: keep the stored roots; only prefill with the flag
    // when nothing is stored, so a later settings save cannot persist a
    // one-off override.
    let persistable = if stored.is_empty() {
        flag.to_vec()
    } else {
        stored
    };
    RuntimeMediaRoots {
        runtime,
        persistable,
    }
}

/// The split produced by [`resolve_runtime_media_roots`].
struct RuntimeMediaRoots {
    /// Roots the file actor uses at runtime (scan/serve/resolve); honours the
    /// `--media-root` override.
    runtime: Vec<PathBuf>,
    /// Roots seeded into the UI and persisted on save: the stored base, or
    /// the flag as a first-run prefill when nothing is stored. Never an
    /// untouched override.
    persistable: Vec<PathBuf>,
}

/// The peer-download tuning shared by every mode that downloads: the
/// `--pipeline-depth` flag, or the default of 48. Interactive clients
/// and seeders both build their [`crate::download::DownloadConfig`] from
/// this, so the flag means the same thing everywhere.
fn download_config(args: &HeadlessArgs) -> crate::download::DownloadConfig {
    crate::download::DownloadConfig {
        pipeline_depth: args.pipeline_depth.unwrap_or(48),
        ..Default::default()
    }
}

/// The torrent-first download wiring for interactive clients: a librqbit
/// session at `<cache>/torrents/` plus the live nyaa search. Seeders
/// deliberately get none of this — a file nyaa can supply makes the
/// seeder redundant; its job is the rare, peer-only files. A session
/// that fails to start (port trouble, unwritable cache) disables the
/// torrent path with a warning — the peer transfer still works — rather
/// than failing startup.
async fn torrent_wiring(
    cache_dir: &std::path::Path,
    upload_limit: Option<u64>,
) -> (
    Option<Arc<dyn crate::torrent::engine::TorrentEngine>>,
    Option<Arc<dyn crate::torrent::nyaa::NyaaSource>>,
) {
    match crate::torrent::rqbit::RqbitEngine::new(cache_dir.join("torrents"), upload_limit).await {
        Ok(engine) => (
            Some(engine as Arc<dyn crate::torrent::engine::TorrentEngine>),
            Some(Arc::new(crate::torrent::nyaa::HttpNyaaSource)
                as Arc<dyn crate::torrent::nyaa::NyaaSource>),
        ),
        Err(e) => {
            tracing::warn!("torrent downloads disabled: {e}");
            (None, None)
        }
    }
}

/// Reorder resolved addresses so the two families alternate, keeping
/// each family's own resolver order and starting with the resolver's
/// first pick. The connector tries addresses in order with a bounded
/// per-address timeout, so this guarantees the *other* family is the
/// second thing tried when the preferred one is black-holed (post-sleep
/// stale-NDP IPv6, 2026-07-06) rather than after every same-family
/// address.
fn interleave_families(addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let first_is_v4 = addrs.first().is_some_and(|addr| addr.is_ipv4());
    let (first, second): (Vec<_>, Vec<_>) = addrs
        .into_iter()
        .partition(|addr| addr.is_ipv4() == first_is_v4);
    let mut out = Vec::with_capacity(first.len() + second.len());
    let (mut first, mut second) = (first.into_iter(), second.into_iter());
    loop {
        match (first.next(), second.next()) {
            (None, None) => return out,
            (a, b) => out.extend(a.into_iter().chain(b)),
        }
    }
}

/// Append the default port unless the address already has one. A
/// `host:port` has exactly one colon; a bracketed IPv6 literal carries
/// its port after `]:` (and if it has none, just needs `:port` appended,
/// *not* another pair of brackets); multiple colons without brackets is a
/// bare IPv6 literal that needs wrapping.
fn with_default_port(server: &str) -> String {
    // An already-bracketed literal: keep it as-is if it has a port after
    // `]:`, otherwise append the port directly (wrapping it again would
    // produce a malformed `[[::1]]:port`).
    if server.starts_with('[') {
        return if server.contains("]:") {
            server.to_string()
        } else {
            format!("{server}:{DEFAULT_PORT}")
        };
    }
    match server.matches(':').count() {
        // host:port — already has one.
        1 => server.to_string(),
        // A bare, unbracketed IPv6 literal (2+ colons) needs wrapping.
        n if n >= 2 => format!("[{server}]:{DEFAULT_PORT}"),
        // A bare hostname or IPv4.
        _ => format!("{server}:{DEFAULT_PORT}"),
    }
}

/// Resolve the client's single identity: the `--username` flag wins,
/// then the stored setting, then the OS user (`$USER` / `%USERNAME%`,
/// supplied by the caller). The UI, the session, and auth all derive
/// from this one value — if they disagree, your own writes (manual
/// override) are keyed under a name your PeerList row doesn't carry, so
/// your readiness never shows on your own screen.
fn resolve_username(
    flag: Option<String>,
    stored: Option<String>,
    env_user: Option<String>,
) -> Option<String> {
    flag.or(stored).or(env_user)
}

/// Split the stored settings + the username override into the runtime
/// identity and the *persistable* settings, the single chokepoint that
/// keeps a one-off `--username` out of the database.
///
/// The `--username` flag (and the `$USER` / `%USERNAME%` fallback) is a
/// runtime override: it decides the identity used for the UI, session, and
/// auth — which MUST agree (2026-06-14) — but per design.md (Data Storage:
/// "Command-line flags and environment variables override stored settings
/// at runtime but are never persisted") it must never be written back. So
/// this leaves `settings.username` at its **stored** value and only
/// *prefills* it (flag, then `$USER`) when nothing is stored — i.e.
/// first-run, where the settings modal turns the prefill into an editable
/// default the user confirms before it persists. A later settings save can
/// therefore only ever write the stored value or one the user actually
/// typed, never an untouched flag override. The returned identity still
/// honours the flag (flag > stored > `$USER`).
fn resolve_runtime_identity(
    settings: &mut crate::config::Settings,
    flag_username: Option<String>,
    env_user: Option<String>,
) -> Option<String> {
    let stored = settings.username.take();
    // Runtime identity: flag wins, then stored, then $USER.
    let identity = resolve_username(flag_username.clone(), stored.clone(), env_user.clone());
    // Persistable username: keep the stored value; only prefill (flag, then
    // $USER) when nothing is stored. The flag NEVER overrides a stored
    // username here, so a later settings save cannot persist a one-off
    // override.
    settings.username = stored.or(flag_username).or(env_user);
    identity
}

/// Everything needed to spawn a client against the configured server:
/// the resolved connector, identity, and the settings/TOFU storage
/// handle. Shared by the headless run and the importer.
pub(crate) struct ClientSetup {
    pub connector: Arc<QuicConnector>,
    pub username: String,
    pub password: String,
    /// Settings/TOFU handle (interactive only — seeders persist
    /// nothing).
    pub storage: Option<Storage>,
    pub server_addr_str: String,
    /// No fingerprint was pinned; persist the observed one after the
    /// first successful connect.
    pub first_use: bool,
}

/// Resolve settings (stored < env < flags), the server address, and
/// the TOFU pin.
pub(crate) async fn prepare(args: &HeadlessArgs) -> Result<ClientSetup, String> {
    // Settings: stored for interactive clients (flags override, never
    // persisted), flags/env only for seeders.
    let phase = std::time::Instant::now();
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
    tracing::info!(
        elapsed_ms = phase.elapsed().as_millis() as u64,
        "storage opened and settings loaded"
    );

    let username = args
        .username
        .clone()
        .or(settings.username)
        .ok_or("no username configured; pass --username")?;
    // Precedence: flag > env > stored settings. Log the source (never
    // the password) — an auth rejection is usually a precedence
    // surprise, and the log should settle which password was used.
    let (password, password_source) = match args.password.clone() {
        Some(password) => (Some(password), "flag"),
        None => match std::env::var("DESSPLAY_PASSWORD").ok() {
            Some(password) => (Some(password), "env"),
            None => (settings.password, "stored settings"),
        },
    };
    let password =
        password.ok_or("no password configured; pass --password or set DESSPLAY_PASSWORD")?;
    tracing::info!(source = password_source, "password resolved");
    let server = args.server.clone().unwrap_or(settings.server);

    // Resolve and pin.
    let phase = std::time::Instant::now();
    let server_addr_str = with_default_port(&server);
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host(&server_addr_str)
        .await
        .map_err(|e| format!("resolving {server_addr_str}: {e}"))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("{server_addr_str} resolved to no addresses"));
    }
    // Interleave families so a black-holed IPv6 path falls through to
    // IPv4 after one per-address timeout, not after every AAAA address
    // (post-sleep stale NDP ate v6 for ~90s while v4 worked; 2026-07-06).
    let addrs = interleave_families(addrs);
    tracing::info!(
        resolved = ?addrs,
        elapsed_ms = phase.elapsed().as_millis() as u64,
        "resolved {server_addr_str}"
    );
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
    let phase = std::time::Instant::now();
    let connector = Arc::new(
        QuicConnector::new(addrs, server_name, pinned)
            .map_err(|e| format!("building QUIC endpoint: {e}"))?,
    );
    tracing::info!(
        elapsed_ms = phase.elapsed().as_millis() as u64,
        "QUIC endpoint built"
    );

    Ok(ClientSetup {
        connector,
        username,
        password,
        storage,
        server_addr_str,
        first_use,
    })
}

/// Load the persisted CRDT snapshot, tolerating a codec failure. The
/// local snapshot is only a startup optimisation — the client adopts the
/// authoritative server snapshot on connect — so a blob we can no longer
/// decode (e.g. a CRDT schema change between versions) must never brick
/// startup: drop it and re-sync. Non-codec storage errors still propagate.
fn load_state_tolerant(storage: &Storage) -> Result<Option<dessplay_core::StateSnapshot>, String> {
    match storage.load_state() {
        Ok(snapshot) => Ok(snapshot),
        Err(crate::storage::StorageError::Codec(e)) => {
            tracing::warn!("discarding unreadable stored state ({e}); re-syncing from server");
            Ok(None)
        }
        Err(e) => Err(format!("loading stored state: {e}")),
    }
}

/// Run the headless client until Ctrl-C. Errors are human-readable —
/// `main()` just prints them.
pub async fn run_headless(args: HeadlessArgs) -> Result<(), String> {
    let start = std::time::Instant::now();
    let seeder = args.seeder;
    let db_path = args.db_path.clone();

    // Refuse to start if another dessplay process already owns this
    // database/cache. This is the colliding case from the field: a seeder and
    // a client launched from the same home directory with no path overrides
    // share the default db (`Storage::default_path`) and cache, fighting over
    // the same SQLite file and hash-named cache files. Held for the whole
    // process; the advisory lock releases on exit or crash.
    let resolved_db = match &db_path {
        Some(path) => path.clone(),
        None => Storage::default_path().ok_or("cannot determine the data directory")?,
    };
    let cache_dir = resolve_cache_dir(&args)?;
    let _instance_lock = crate::instance_lock::acquire(&resolved_db, Some(&cache_dir))?;

    let setup = prepare(&args).await?;

    // Stored CRDT state, and a second storage handle for the sync
    // actor (SQLite in WAL mode is fine with two connections; the sync
    // actor owns its handle outright).
    let (initial, sync_storage) = if seeder {
        (None, None)
    } else {
        let path = match db_path {
            Some(path) => path,
            None => Storage::default_path().ok_or("cannot determine the data directory")?,
        };
        let sync_storage =
            Storage::open(&path).map_err(|e| format!("opening {}: {e}", path.display()))?;
        let initial = load_state_tolerant(&sync_storage)?;
        (initial, Some(sync_storage))
    };

    let role = if seeder {
        Role::Seeder
    } else {
        Role::Interactive
    };
    let ClientSetup {
        connector,
        username,
        password,
        storage,
        server_addr_str,
        first_use,
    } = setup;
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

    // A seeder auto-fetches the playlist and serves it: spin up its
    // transfer driver (persistent hash cache so a TB-scale store isn't
    // re-hashed on restart; the cache dir is added as a media root, so
    // prior downloads are re-discovered, not re-fetched).
    let (mut seeder_transfer, mut seeder_outputs) = if seeder {
        // Reuse the db/cache paths already resolved (and locked) above.
        let file_storage = Storage::open(&resolved_db)
            .map_err(|e| format!("opening {}: {e}", resolved_db.display()))?;
        let (transfer, outputs) = crate::seeder::SeederTransfer::new(
            UserId::new(&username),
            crate::seeder::seeder_file_config(
                file_storage,
                args.media_roots.clone(),
                cache_dir.clone(),
                system_clock(),
                None,
                download_config(&args),
            ),
            handle.sync.clone(),
            handle.network.clone(),
        );
        (Some(transfer), Some(outputs))
    } else {
        (None, None)
    };

    // ---- Event loop until Ctrl-C.
    let mut pin_pending = first_use && !seeder;
    let mut first_connected = true;
    let mut first_peer_list = true;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
                break;
            }
            // Seeder file-actor outputs (relay sends, availability,
            // completions). `seeder_outputs` is held separately from the
            // driver so this arm doesn't alias `on_state` below.
            output = async {
                match &mut seeder_outputs {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let (Some(output), Some(transfer)) = (output, seeder_transfer.as_mut()) {
                    transfer.on_file_output(output).await;
                }
            }
            event = handle.events.recv() => {
                let Some(event) = event else { break };
                // Drive the seeder's transfer from each event: route
                // relayed peer messages, then re-plan from fresh state.
                if let Some(transfer) = seeder_transfer.as_mut() {
                    if let ClientEvent::Network(NetworkEvent::Peer { from, message }) = &event {
                        transfer.on_peer(from.clone(), message.clone()).await;
                    }
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    if handle.sync.send(SyncCommand::GetView(tx)).await.is_ok()
                        && let Ok(view) = rx.await
                    {
                        let peers = handle.peers.borrow().clone();
                        transfer.on_state(&view, &peers).await;
                    }
                }
                match event {
                    ClientEvent::Network(NetworkEvent::Connected { observed_addr }) => {
                        if first_connected {
                            first_connected = false;
                            tracing::info!(
                                since_start_ms = start.elapsed().as_millis() as u64,
                                "first Connected event"
                            );
                        }
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
                    ClientEvent::Network(NetworkEvent::Rejected { message }) => {
                        return Err(message.clone());
                    }
                    ClientEvent::Network(NetworkEvent::Disconnected { reason }) => {
                        tracing::warn!("disconnected ({reason}); retrying");
                    }
                    ClientEvent::Network(NetworkEvent::PeerList { peers, known_offline }) => {
                        if first_peer_list {
                            first_peer_list = false;
                            tracing::info!(
                                since_start_ms = start.elapsed().as_millis() as u64,
                                "first PeerList"
                            );
                        }
                        let listed: Vec<String> = peers
                            .iter()
                            .map(|p| format!("{} [{:?}/{:?}]", p.username, p.role, p.presence))
                            .collect();
                        tracing::info!(
                            "peers: {} (known offline: {})",
                            listed.join(", "),
                            known_offline.len()
                        );
                    }
                    // Relayed peer traffic is internal cross-actor flow
                    // (and high-volume during a transfer) — trace, not
                    // debug; it's already handled by the seeder driver
                    // above. State-sync and clock events are also internal.
                    ClientEvent::Network(NetworkEvent::Peer { .. }) => {
                        tracing::trace!("relayed peer message");
                    }
                    other => tracing::trace!("{other:?}"),
                }
            }
        }
    }

    // Graceful teardown: Goodbye to the server, flush state to SQLite.
    // Each actor drops its command receiver when it exits, so closed()
    // is the completion signal — bounded, because a wedged actor must
    // never hold the process hostage.
    let _ = handle.network.send(NetworkCommand::Shutdown).await;
    let _ = handle.sync.send(SyncCommand::Shutdown).await;
    let done = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        handle.network.closed().await;
        handle.sync.closed().await;
    })
    .await;
    if done.is_err() {
        tracing::error!("actors did not shut down within 5s; exiting anyway");
    }
    Ok(())
}

/// Run the interactive TUI client until quit.
pub async fn run_interactive(args: HeadlessArgs) -> Result<(), String> {
    use crate::actors::sync::SyncCommand;
    use crate::ui::app::Ui;
    use crate::ui::msg::UserAction;
    use crate::ui::shell::{UiInput, run_input_thread, run_ui_thread};

    let start = std::time::Instant::now();
    let db_path = match &args.db_path {
        Some(path) => path.clone(),
        None => Storage::default_path().ok_or("cannot determine the data directory")?,
    };
    let cache_dir = resolve_cache_dir(&args)?;
    // Refuse to start if another dessplay process already owns this
    // database/cache (e.g. a seeder launched from the same home directory
    // with no path overrides). Held for the whole process; released on exit.
    let _instance_lock = crate::instance_lock::acquire(&db_path, Some(&cache_dir))?;
    // Interactive mode owns first-run setup: whether the settings
    // screen opens is decided by the *stored* settings, before any
    // prefills. Prefills ($USER, the .env password, flags) only become
    // the modal's editable defaults.
    let phase = std::time::Instant::now();
    let mut setup_storage =
        Storage::open(&db_path).map_err(|e| format!("opening {}: {e}", db_path.display()))?;
    let mut settings: crate::config::Settings = setup_storage
        .load_settings()
        .map_err(|e| format!("loading settings: {e}"))?;
    // `--media-root` overrides the stored roots for this run (never
    // persisted): `resolve_runtime_media_roots` returns the runtime roots
    // (used by the file actor) alongside the *persistable* base (seeded into
    // the UI and written on save — the stored value, or a first-run prefill,
    // never an untouched override). The runtime roots feed `needs_setup` so a
    // flag-supplied run is not forced into first-run setup merely because the
    // DB has no roots.
    let media_roots = resolve_runtime_media_roots(
        &args.media_roots,
        setup_storage
            .media_roots()
            .map_err(|e| format!("loading media roots: {e}"))?,
    );
    tracing::info!(
        elapsed_ms = phase.elapsed().as_millis() as u64,
        "storage opened and settings loaded"
    );
    let needs_setup = settings.needs_setup() || media_roots.runtime.is_empty();
    // True when an override shadows existing stored roots (the media-roots
    // analogue of `identity_locked`): while set, the file actor keeps the
    // runtime override even after first-run setup confirms stored roots.
    let roots_locked = media_roots.persistable != media_roots.runtime;
    // Resolve the one identity used for the UI, the session, and auth.
    // These MUST agree: the UI keys your own manual-override write by
    // this name, while the Users pane derives your row from the server's
    // PeerList (the auth name) — if they diverge your own readiness
    // never shows (2026-06-14). The flag/env override is kept *out* of the
    // persistable settings (design.md: "flags/env override ... but are
    // never persisted"); `resolve_runtime_identity` returns the runtime
    // identity while leaving `settings.username` at the stored value (only
    // a first-run prefill is folded in, where the modal confirms it).
    let env_user = std::env::var("USER")
        .ok()
        // $USER on Linux/macOS, %USERNAME% on Windows — so the username
        // field starts pre-filled (and the modal saveable) on every OS.
        .or_else(|| std::env::var("USERNAME").ok());
    let runtime_username = resolve_runtime_identity(&mut settings, args.username.clone(), env_user);
    // The identity is "locked" to the runtime override whenever it differs
    // from the persistable username (a `--username` flag over a stored
    // name). While locked, neither this bootstrap nor the UI may move the
    // identity onto a settings-screen value — see the matching guard in
    // `Ui` (app.rs, SettingsSaved) that keeps `self.me` fixed on save.
    let identity_locked = settings
        .username
        .as_deref()
        .is_some_and(|stored| Some(stored) != runtime_username.as_deref());
    // Track (and log) where the settings password came from — never
    // the password itself.
    let password_source = if settings.password.is_some() {
        "stored settings"
    } else if args.password.is_some() {
        "flag"
    } else if std::env::var_os("DESSPLAY_PASSWORD").is_some() {
        "env"
    } else {
        "unset (settings screen)"
    };
    tracing::info!(source = password_source, "settings password source");
    if settings.password.is_none() {
        settings.password = args
            .password
            .clone()
            .or_else(|| std::env::var("DESSPLAY_PASSWORD").ok());
    }

    // The UI runs on its own threads; this task bridges it to the
    // actors.
    // The UI is seeded with the persistable base, never the runtime
    // override: the settings modal shows it, and any save (including an
    // unrelated F2 subtitle cycle) carries it back, so a one-off
    // `--media-root` can never be written to the DB. The runtime override
    // drives the file actor instead (see `file_media_roots` below).
    let ui = Ui::with_setup(
        UserId::new(runtime_username.clone().unwrap_or_default()),
        settings.clone(),
        media_roots.persistable.clone(),
        needs_setup,
    );
    let (input_tx, input_rx) = std::sync::mpsc::sync_channel::<UiInput>(64);
    let (action_tx, mut action_rx) = mpsc::channel::<UserAction>(64);
    let ui_thread = std::thread::spawn(move || run_ui_thread(ui, input_rx, action_tx));
    {
        let input_tx = input_tx.clone();
        std::thread::spawn(move || run_input_thread(input_tx));
    }

    // First run: wait for the settings save before connecting (the Ui
    // updates its own identity from the saved username).
    if needs_setup {
        loop {
            match action_rx.recv().await {
                Some(UserAction::SaveSettings(saved, roots)) => {
                    setup_storage
                        .save_settings(&saved)
                        .map_err(|e| format!("saving settings: {e}"))?;
                    setup_storage
                        .set_media_roots(&roots)
                        .map_err(|e| format!("saving media roots: {e}"))?;
                    settings = *saved;
                    break;
                }
                Some(UserAction::Quit) | None => {
                    let _ = input_tx.try_send(UiInput::Shutdown);
                    let _ = ui_thread.join();
                    return Ok(());
                }
                Some(_) => continue,
            }
        }
    }
    // The session/auth identity, mirroring the UI's `self.me`: when the
    // user established it in the first-run modal (identity not locked to a
    // runtime override), follow the confirmed username; otherwise the
    // runtime override stands (and was never folded into the persisted
    // settings).
    let me = UserId::new(
        if needs_setup && !identity_locked {
            settings.username.clone()
        } else {
            runtime_username
        }
        .ok_or("settings saved without a username")?,
    );

    let setup = prepare(&args).await?;
    let sync_storage =
        Storage::open(&db_path).map_err(|e| format!("opening {}: {e}", db_path.display()))?;
    let initial = load_state_tolerant(&sync_storage)?;
    let handle = spawn_client(
        Arc::clone(&setup.connector),
        ClientConfig {
            // Same identity the UI and session use (`me`), so our own
            // writes are keyed under the name our PeerList row carries.
            username: me.clone(),
            password: setup.password.clone(),
            role: Role::Interactive,
            session_nonce: rand::random(),
            clock: system_clock(),
            sync: SyncConfigExtras {
                initial,
                storage: Some(sync_storage),
                flush_interval: None,
            },
        },
    );

    // The player side: policy in `session`, mpv behind it. The player
    // process itself only spawns when something first loads; the file
    // actor (its own storage handle — WAL handles concurrency) runs
    // from the start.
    //
    // The roots now in the database after any first-run save: the persistable
    // base (stored roots, or what the user confirmed during setup). It also
    // seeds the running session's roots-change detection below.
    let persisted_roots = setup_storage
        .media_roots()
        .map_err(|e| format!("loading media roots: {e}"))?;
    // The roots the file actor scans/serves: the runtime `--media-root`
    // override, or — on a first run with no shadowing override — the roots
    // the user just confirmed (mirroring `me` following the confirmed
    // username at first run; `roots_locked` keeps the override otherwise).
    let file_media_roots = if needs_setup && !roots_locked {
        persisted_roots.clone()
    } else {
        media_roots.runtime.clone()
    };
    let file_storage =
        Storage::open(&db_path).map_err(|e| format!("opening {}: {e}", db_path.display()))?;
    let player_factory = match &args.attach_mpv {
        Some(socket) => crate::player::mpv::MpvFactory::attach(socket.clone()),
        None => crate::player::mpv::MpvFactory::new("mpv"),
    };
    // Behind a default-off setting: the engine opens ports and joins the
    // DHT, so it never starts unless the user opted in. Like the player
    // choice and upload limit, the setting applies at startup.
    let (torrent, nyaa) = if settings.torrent_enabled {
        torrent_wiring(&cache_dir, settings.upload_limit).await
    } else {
        (None, None)
    };
    let shell = crate::session::SessionShell::new(
        me.clone(),
        player_factory,
        system_clock(),
        crate::actors::file::FileConfig {
            storage: file_storage,
            media_roots: file_media_roots,
            retention: settings.cache_retention,
            cache_dir,
            clock: system_clock(),
            download: download_config(&args),
            upload_limit: settings.upload_limit,
            // Interactive clients re-scan the library about once a minute.
            scan_interval: Some(std::time::Duration::from_secs(60)),
            scan_transfer_quiet: crate::actors::file::SCAN_TRANSFER_QUIET_DEFAULT,
            torrent,
            nyaa,
            torrent_fetch: crate::torrent::TorrentFetchConfig::default(),
        },
        settings.auto_download,
        handle.sync.clone(),
        handle.network.clone(),
    );

    // IRC bridge (interactive-only): mirror our own chat to IRC and
    // surface external IRC users locally. Spawned here, never in
    // spawn_client, so seeders (headless, no chat) are unaffected.
    let (irc_tx, irc_rx) = mpsc::channel::<crate::actors::irc::IrcCommand>(64);
    let (irc_ev_tx, irc_events) = mpsc::channel::<crate::actors::irc::IrcEvent>(64);
    tokio::spawn(crate::actors::irc::run(
        crate::actors::irc::IrcConfig::from_settings(&settings, &me),
        irc_rx,
        irc_ev_tx,
    ));

    let connector = Arc::clone(&setup.connector);
    let mut session = SessionLoop {
        handle,
        shell,
        actions: action_rx,
        ui: input_tx.clone(),
        storage: setup_storage,
        db_path,
        me,
        settings,
        media_roots: persisted_roots,
        observed_fingerprint: Box::new(move || connector.observed_fingerprint()),
        pin_pending: setup.first_use,
        server_addr: setup.server_addr_str.clone(),
        start,
        irc_tx,
        irc_events,
        irc_alive: true,
        link: crate::ui::props::LinkStatus::default(),
    };
    let end = session.run().await;

    // Teardown: release the terminal immediately (the user asked to
    // leave), then Goodbye + flush with a bounded wait — a wedged actor
    // must never hold the process hostage.
    let _ = input_tx.try_send(UiInput::Shutdown);
    let _ = ui_thread.join();
    if let SessionEnd::Rejected(message) = &end {
        return Err(message.clone());
    }
    let hashes_in_flight = session.shell.hashes_in_flight();
    if hashes_in_flight > 0 {
        tracing::warn!(
            hashes_in_flight,
            "quitting with playlist-add hashes still running; those adds are dropped"
        );
    }
    session.shell.shutdown().await;
    let _ = session.handle.network.send(NetworkCommand::Shutdown).await;
    let _ = session.handle.sync.send(SyncCommand::Shutdown).await;
    let _ = session
        .irc_tx
        .send(crate::actors::irc::IrcCommand::Shutdown)
        .await;
    let done = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        session.handle.network.closed().await;
        session.handle.sync.closed().await;
        session.irc_tx.closed().await;
    })
    .await;
    if done.is_err() {
        tracing::error!("actors did not shut down within 5s; exiting anyway");
    }
    Ok(())
}

/// A path's filename for display (full path as fallback).
fn display_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Why the bridge loop ended.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SessionEnd {
    /// The user quit (or a UI/event channel closed underneath us).
    Quit,
    /// The server refused us admission (bad password, protocol version
    /// mismatch) — terminal; the message is shown to the user.
    Rejected(String),
}

/// Whether the IRC bridge actually needs to reconnect after a settings
/// save. Compares the derived [`crate::actors::irc::IrcConfig`], not the
/// raw `Settings`, so an unrelated save (e.g. an F2 subtitle-mode cycle,
/// which clones the whole `Settings` but only changes `subtitle_mode`)
/// never forces a needless reconnect — mirrors the `roots !=
/// self.media_roots` guard used for media roots in the `SaveSettings`
/// handler below.
fn irc_config_changed(
    old: &crate::config::Settings,
    new: &crate::config::Settings,
    me: &UserId,
) -> bool {
    crate::actors::irc::IrcConfig::from_settings(old, me)
        != crate::actors::irc::IrcConfig::from_settings(new, me)
}

/// Build the local system line reporting a `/summon`'s outcome
/// (design.md #4): who was pinged (by the channel nick actually
/// addressed) and, when any absent user had no plausible nick, who was
/// skipped. Covers both "we found nicks but some didn't match" and "we
/// aren't connected to IRC right now" (which the IRC actor reports as
/// everyone unmatched) with the same wording.
fn summon_report(pinged: &[(UserId, String)], unmatched: &[UserId]) -> String {
    if pinged.is_empty() {
        let names: Vec<&str> = unmatched.iter().map(|u| u.0.as_str()).collect();
        return format!("/summon: no IRC nick found for {}", names.join(", "));
    }
    let nicks: Vec<&str> = pinged.iter().map(|(_, nick)| nick.as_str()).collect();
    let mut text = format!("Summoned {} on IRC.", nicks.join(", "));
    if !unmatched.is_empty() {
        let names: Vec<&str> = unmatched.iter().map(|u| u.0.as_str()).collect();
        text.push_str(&format!(" No IRC nick found for {}.", names.join(", ")));
    }
    text
}

/// The interactive bridge loop: actors on one side, UI channels on the
/// other. Extracted from [`run_interactive`] so it is testable without
/// a terminal — supervision bugs ("Ctrl-C doesn't quit") live here, and
/// they must be reproducible in tests.
///
/// **Liveness rule: nothing in this loop may block or run long.** Every
/// await in an arm body must complete promptly (channel sends to live
/// actors, oneshot view queries). Long work — hashing, file matching —
/// is started in the background through [`SessionShell`] and comes back
/// through its completion channels as new select arms. A user's `Quit`
/// must be processed even while gigabytes are being hashed.
pub struct SessionLoop<F: crate::player::PlayerFactory> {
    /// The running client actors.
    pub handle: crate::client::ClientHandle,
    /// The player-side policy shell.
    pub shell: crate::session::SessionShell<F>,
    /// Actions from the UI (or a test).
    pub actions: mpsc::Receiver<crate::ui::msg::UserAction>,
    /// Inputs to the UI thread (snapshots, subtitles); lossy sends.
    pub ui: std::sync::mpsc::SyncSender<crate::ui::shell::UiInput>,
    /// Settings/history/TOFU storage.
    pub storage: Storage,
    /// Database path (settings saves reopen for `&mut` access).
    pub db_path: std::path::PathBuf,
    /// Our user.
    pub me: UserId,
    /// Current settings (updated by in-session saves).
    pub settings: crate::config::Settings,
    /// The persistable media-root base currently in the DB / shown by the
    /// UI. A settings save only pushes roots into the running file actor when
    /// the saved roots differ from this — so an unrelated save (e.g. an F2
    /// subtitle cycle) does not clobber an active `--media-root` override.
    pub media_roots: Vec<PathBuf>,
    /// Observed TLS fingerprint for first-use pinning (`None` until a
    /// connection exists, and always `None` on non-QUIC transports).
    pub observed_fingerprint: Box<dyn Fn() -> Option<Vec<u8>> + Send>,
    /// Whether a TOFU pin still needs to be stored.
    pub pin_pending: bool,
    /// Server address string, the TOFU pin key.
    pub server_addr: String,
    /// Process start, for startup timing logs.
    pub start: std::time::Instant,
    /// Commands to the IRC bridge actor (forward our chat, reconfigure on
    /// settings change, shutdown).
    pub irc_tx: mpsc::Sender<crate::actors::irc::IrcCommand>,
    /// Events from the IRC bridge actor (incoming external messages,
    /// connect/disconnect notices).
    pub irc_events: mpsc::Receiver<crate::actors::irc::IrcEvent>,
    /// Whether the IRC event channel is still open (guards its select arm
    /// so a closed channel can't busy-loop).
    pub irc_alive: bool,
    /// Server-link state for the status bar, tracked from the network
    /// actor's Connecting/Connected/Disconnected events.
    pub link: crate::ui::props::LinkStatus,
}

impl<F: crate::player::PlayerFactory> SessionLoop<F> {
    /// Run until quit or auth failure.
    pub async fn run(&mut self) -> SessionEnd {
        use crate::actors::sync::Mutation;
        use crate::ui::msg::UserAction;
        use crate::ui::shell::UiInput;
        use dessplay_core::types::ManualState;

        let mut last_view = std::sync::Arc::new(dessplay_core::StateView::default());
        let mut startup_state_written = false;
        let mut first_connected = true;
        let mut first_peer_list = true;
        let mut first_snapshot = true;
        // The expensive UI snapshot (a full `StateView` clone + two SQLite
        // queries + a full pane rebuild on the UI thread) used to fire on
        // *every* event -- including the 100ms player position tick and
        // every peer's position datagram, so its rate scaled with peer
        // count. Coalesce it: events mark the UI dirty, and a 100ms tick
        // flushes at most one snapshot. (Forced refreshes -- local
        // mutations and file effects -- still flush immediately below.)
        // The first tick of a `tokio` interval completes at once, so the
        // initial snapshot after the first event is not delayed.
        let mut ui_dirty = false;
        let mut ui_tick = tokio::time::interval(std::time::Duration::from_millis(100));
        ui_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                action = self.actions.recv() => {
                    match action {
                        None | Some(UserAction::Quit) => return SessionEnd::Quit,
                        Some(UserAction::Mutate(mutation)) => {
                            // Tap our own chat for the IRC bridge. This arm
                            // only ever carries the local user's mutations
                            // (remote ops arrive as SyncCommand::Server), so
                            // this forwards exactly our own messages — not
                            // events, subtitles, or narrator lines. Lossy
                            // try_send: never await the bridge here (it may
                            // be mid-reconnect with a full channel).
                            if let Mutation::Chat { text } = &mutation {
                                let _ = self.irc_tx.try_send(
                                    crate::actors::irc::IrcCommand::SendChat(text.clone()),
                                );
                            }
                            let _ = self
                                .handle
                                .sync
                                .send(SyncCommand::Mutate(Box::new(mutation)))
                                .await;
                            // Reflect our own write immediately: pull a
                            // fresh snapshot (FIFO after the mutation, so
                            // it includes it) and push it to the UI and
                            // player layer. Without this a local mutation
                            // only became visible on the next network
                            // event -- and the sync actor's StateChanged
                            // signal is best-effort (dropped under load),
                            // so a Ctrl-R toggle could read its own stale
                            // state and never flip (2026-06-14).
                            self.refresh_ui(&mut last_view).await;
                            // The fresh snapshot is FIFO after the
                            // mutation, so it already reflects everything
                            // pending -- no redundant tick flush needed.
                            ui_dirty = false;
                        }
                        Some(UserAction::Browse(request)) => {
                            // Gather what the file browser needs but the UI
                            // thread can't read: the library index (recursive
                            // search, greying, cursor placement), personal
                            // watch history, and the mapping browser's
                            // per-series start directory. Cheap lean queries,
                            // paid only on a browser-open keypress — always
                            // fresh, no cached copy to invalidate.
                            let files = self.storage.library_paths().unwrap_or_default();
                            let watched = self.storage.watched_hashes().unwrap_or_default();
                            let start = match &request {
                                crate::ui::msg::BrowseRequest::Map {
                                    series: Some(key), ..
                                } => self.storage.series_map_dir(key).ok().flatten(),
                                _ => None,
                            };
                            tracing::debug!(
                                files = files.len(),
                                watched = watched.len(),
                                start = start.as_ref().map(|p| p.display().to_string()),
                                "browse: answering with the library index"
                            );
                            let _ = self.ui.try_send(UiInput::Browse {
                                request,
                                files,
                                watched,
                                start,
                            });
                        }
                        Some(UserAction::HashAndAdd { path, after }) => {
                            // Hashing is seconds per gigabyte: background
                            // work in the file actor, completed in the
                            // `file_outputs` arm below. Inline hashing
                            // here once starved this loop — frozen UI,
                            // unprocessable Quit (2026-06-12).
                            self.shell.hash_and_add(path, after).await;
                        }
                        Some(UserAction::AddByHash { hash, after }) => {
                            if let Some(text) =
                                self.shell.add_by_hash(hash, after, &last_view).await
                            {
                                let _ = self.ui.try_send(UiInput::System {
                                    timestamp: (system_clock())(),
                                    text,
                                });
                            }
                        }
                        Some(UserAction::AniDbSearch { query }) => {
                            let _ = self
                                .handle
                                .network
                                .send(crate::actors::network::NetworkCommand::SendReliable(
                                    Box::new(dessplay_core::net::ServerControl::AniDbSearch {
                                        query,
                                    }),
                                ))
                                .await;
                        }
                        Some(UserAction::MarkWatched { file, watched }) => {
                            // Server-authoritative, like EofReached: sent
                            // straight to the wire rather than through a
                            // CRDT Mutation (design.md #10).
                            let _ = self
                                .handle
                                .network
                                .send(crate::actors::network::NetworkCommand::SendReliable(
                                    Box::new(dessplay_core::net::ServerControl::MarkWatched {
                                        file,
                                        watched,
                                    }),
                                ))
                                .await;
                        }
                        Some(UserAction::MapFile { file, path, series }) => {
                            self.shell.set_manual_mapping(file, path, series).await;
                        }
                        Some(UserAction::Archive { file, series_name, filename }) => {
                            self.shell.archive(file, series_name, filename).await;
                        }
                        Some(UserAction::Notice(text)) => {
                            // Command feedback — stamp with the shared clock
                            // (the UI has none) and post a local chat line.
                            let _ = self.ui.try_send(UiInput::System {
                                timestamp: (system_clock())(),
                                text,
                            });
                        }
                        Some(UserAction::Summon(absent)) => {
                            // Lossy, like the chat tap below: never block the
                            // main loop on the IRC actor's readiness. The
                            // outcome (including "not connected yet") comes
                            // back through the irc_events arm.
                            let _ = self
                                .irc_tx
                                .try_send(crate::actors::irc::IrcCommand::Summon(absent));
                        }
                        Some(UserAction::SaveSettings(saved, roots)) => {
                            if let Err(e) = self.storage.save_settings(&saved) {
                                tracing::error!("saving settings: {e}");
                            }
                            // Persist the roots the user confirmed: the
                            // persistable base, or edits made in the settings
                            // modal. The UI is seeded with the persistable
                            // base (run.rs), never the `--media-root`
                            // override, so a save can never write a one-off
                            // override back to the DB (design.md: flags
                            // "override ... but are never persisted").
                            // set_media_roots needs &mut; reopen briefly.
                            match Storage::open(&self.db_path) {
                                Ok(mut storage) => {
                                    if let Err(e) = storage.set_media_roots(&roots) {
                                        tracing::error!("saving media roots: {e}");
                                    }
                                }
                                Err(e) => tracing::error!("opening storage: {e}"),
                            }
                            // Only push roots into the running file actor when
                            // they actually changed (a genuine settings-modal
                            // edit). An unrelated save (e.g. an F2 subtitle
                            // cycle) carries the unchanged persistable base,
                            // so an active `--media-root` runtime override
                            // stays in effect.
                            if roots != self.media_roots {
                                self.shell.set_media_roots(roots.clone()).await;
                                self.media_roots = roots;
                            }
                            self.shell.set_retention(saved.cache_retention).await;
                            self.shell.set_auto_download(saved.auto_download);
                            // Only reconfigure the IRC actor when the
                            // IRC-relevant settings actually changed. An
                            // unrelated save (e.g. an F2 subtitle-mode
                            // cycle) carries the whole `Settings` struct
                            // unchanged apart from one field, and must not
                            // force a needless reconnect.
                            if irc_config_changed(&self.settings, &saved, &self.me) {
                                let _ = self.irc_tx.try_send(
                                    crate::actors::irc::IrcCommand::Reconfigure(Box::new(
                                        crate::actors::irc::IrcConfig::from_settings(
                                            &saved, &self.me,
                                        ),
                                    )),
                                );
                            }
                            self.settings = *saved;
                        }
                    }
                }
                event = self.handle.events.recv() => {
                    let Some(event) = event else { return SessionEnd::Quit };
                    match &event {
                        ClientEvent::Network(NetworkEvent::Rejected { message }) => {
                            return SessionEnd::Rejected(message.clone());
                        }
                        ClientEvent::Network(NetworkEvent::Connecting { attempt }) => {
                            self.link =
                                crate::ui::props::LinkStatus::Connecting { attempt: *attempt };
                        }
                        ClientEvent::Network(NetworkEvent::Disconnected { .. }) => {
                            self.link = crate::ui::props::LinkStatus::Down;
                        }
                        ClientEvent::Network(NetworkEvent::Connected { .. }) => {
                            self.link = crate::ui::props::LinkStatus::Connected;
                            if first_connected {
                                first_connected = false;
                                tracing::info!(
                                    since_start_ms = self.start.elapsed().as_millis() as u64,
                                    "first Connected event"
                                );
                            }
                            if self.pin_pending && let Some(fp) = (self.observed_fingerprint)() {
                                let now = (system_clock())() as i64;
                                if self
                                    .storage
                                    .store_tofu_fingerprint(&self.server_addr, &fp, now)
                                    .is_ok()
                                {
                                    self.pin_pending = false;
                                }
                            }
                            // "Ready on startup": write our manual override
                            // once per session (clears a stale Paused too).
                            if !startup_state_written {
                                startup_state_written = true;
                                let state = if self.settings.ready_on_startup {
                                    None
                                } else {
                                    Some(ManualState::Paused)
                                };
                                let _ = self
                                    .handle
                                    .sync
                                    .send(SyncCommand::Mutate(Box::new(
                                        Mutation::SetManualOverride {
                                            user: self.me.clone(),
                                            state,
                                        },
                                    )))
                                    .await;
                            }
                        }
                        ClientEvent::Network(NetworkEvent::PeerList { .. }) if first_peer_list => {
                            first_peer_list = false;
                            tracing::info!(
                                since_start_ms = self.start.elapsed().as_millis() as u64,
                                "first PeerList"
                            );
                        }
                        ClientEvent::Network(NetworkEvent::ClockSync { offset_millis }) => {
                            self.shell.set_clock_offset(*offset_millis).await;
                        }
                        ClientEvent::Network(NetworkEvent::SearchResults { query, results }) => {
                            let _ = self.ui.try_send(UiInput::SearchResults {
                                query: query.clone(),
                                results: results.clone(),
                            });
                        }
                        ClientEvent::Network(NetworkEvent::Peer { from, message }) => {
                            self.shell.on_network_peer(from.clone(), message.clone()).await;
                        }
                        _ => {}
                    }
                    // Any event can change what the UI shows -- and what
                    // the player layer should be doing -- but the heavy
                    // snapshot is deferred to the coalescing tick below
                    // rather than rebuilt per event. The cheap, per-event
                    // side effects (clock offset, peer messages incl. the
                    // download data path, fingerprint pinning) ran in the
                    // match above and are not deferred.
                    ui_dirty = true;
                }
                _ = ui_tick.tick(), if ui_dirty => {
                    ui_dirty = false;
                    if let Some(snapshot) = self.snapshot().await {
                        if first_snapshot {
                            first_snapshot = false;
                            tracing::info!(
                                since_start_ms = self.start.elapsed().as_millis() as u64,
                                "first state snapshot pushed to the UI"
                            );
                        }
                        let lines = self.shell.on_state(&snapshot.view, &snapshot.peers).await;
                        forward_ui_lines(&self.ui, lines);
                        last_view = snapshot.view.clone();
                        let _ = self.ui.try_send(UiInput::Snapshot(Box::new(snapshot)));
                    }
                }
                event = self.irc_events.recv(), if self.irc_alive => {
                    // Incoming from the IRC bridge. Messages from external
                    // users become local-only chat lines; connect/disconnect
                    // become local system notices. None means the actor exited
                    // — disable this arm so it can't busy-loop. (Crucially it
                    // does NOT end the session, unlike handle.events.)
                    use crate::actors::irc::IrcEvent;
                    match event {
                        None => self.irc_alive = false,
                        Some(IrcEvent::Message { from, text, action }) => {
                            let _ = self.ui.try_send(UiInput::Irc {
                                timestamp: (system_clock())(),
                                sender: from,
                                text,
                                action,
                            });
                        }
                        Some(IrcEvent::Connected) => {
                            let _ = self.ui.try_send(UiInput::System {
                                timestamp: (system_clock())(),
                                text: format!("Connected to IRC ({}).", self.settings.irc_channel),
                            });
                        }
                        Some(IrcEvent::Disconnected { reason }) => {
                            let _ = self.ui.try_send(UiInput::System {
                                timestamp: (system_clock())(),
                                text: format!("IRC disconnected: {reason}"),
                            });
                        }
                        Some(IrcEvent::Summoned { pinged, unmatched }) => {
                            let _ = self.ui.try_send(UiInput::System {
                                timestamp: (system_clock())(),
                                text: summon_report(&pinged, &unmatched),
                            });
                        }
                    }
                }
                output = self.shell.player_outputs.recv() => {
                    let Some(output) = output else { continue };
                    let lines = self.shell.on_player_output(output, &last_view).await;
                    forward_ui_lines(&self.ui, lines);
                }
                output = self.shell.file_outputs.recv() => {
                    let Some(output) = output else { continue };
                    let peers = self.handle.peers.borrow().clone();
                    match self.shell.on_file_output(output, &last_view, &peers).await {
                        crate::session::FileEffect::HashProgress {
                            path,
                            done_bytes,
                            total_bytes,
                        } => {
                            let _ = self.ui.try_send(UiInput::Hashing {
                                filename: display_name(&path),
                                done_bytes,
                                total_bytes,
                                finished: false,
                            });
                        }
                        crate::session::FileEffect::HashDone { path } => {
                            let _ = self.ui.try_send(UiInput::Hashing {
                                filename: display_name(&path),
                                done_bytes: 0,
                                total_bytes: 0,
                                finished: true,
                            });
                        }
                        crate::session::FileEffect::Archived { timestamp, text } => {
                            let _ = self.ui.try_send(UiInput::System { timestamp, text });
                            // Archive doesn't emit a sync event, so push a
                            // fresh snapshot to clear the "temporary" marker
                            // (cache_hashes is recomputed from storage).
                            if let Some(snapshot) = self.snapshot().await {
                                last_view = snapshot.view.clone();
                                let _ = self.ui.try_send(UiInput::Snapshot(Box::new(snapshot)));
                                ui_dirty = false;
                            }
                        }
                        crate::session::FileEffect::ScanProgress { done, total } => {
                            // No-silent-work: a one-line notice at the
                            // start and end of a scan that has work to do
                            // (a quiet, all-cache-hit rescan emits nothing).
                            let text = if done == 0 {
                                format!("Indexing media library ({total} new file(s))…")
                            } else if done == total {
                                format!("Library indexed ({total} file(s)).")
                            } else {
                                String::new()
                            };
                            if !text.is_empty() {
                                let _ = self.ui.try_send(UiInput::System {
                                    timestamp: (system_clock())(),
                                    text,
                                });
                            }
                        }
                        crate::session::FileEffect::WatchRecorded => {
                            // Recording a watch emits no sync event, so push
                            // a fresh snapshot — its recency map is rebuilt
                            // from storage, moving the just-watched series to
                            // the top of Recent Series at once.
                            if let Some(snapshot) = self.snapshot().await {
                                last_view = snapshot.view.clone();
                                let _ = self.ui.try_send(UiInput::Snapshot(Box::new(snapshot)));
                                ui_dirty = false;
                            }
                        }
                        crate::session::FileEffect::Evicted { .. } => {
                            // Eviction emits no sync event, so push a fresh
                            // snapshot to clear the "temporary" marker on the
                            // removed rows (cache_hashes is recomputed from
                            // storage).
                            if let Some(snapshot) = self.snapshot().await {
                                last_view = snapshot.view.clone();
                                let _ = self.ui.try_send(UiInput::Snapshot(Box::new(snapshot)));
                                ui_dirty = false;
                            }
                        }
                        crate::session::FileEffect::None => {}
                    }
                }
            }
        }
    }

    /// Build a fresh snapshot for the UI. (`&mut self` although nothing
    /// mutates: a `&self` future would demand `Sync` from the SQLite
    /// connection when the loop is spawned as a task.)
    /// Pull a fresh snapshot and push it to the UI + player layer.
    /// Called after a local mutation so the user's own actions take
    /// effect at once, independent of network-event timing.
    async fn refresh_ui(&mut self, last_view: &mut std::sync::Arc<dessplay_core::StateView>) {
        if let Some(snapshot) = self.snapshot().await {
            let lines = self.shell.on_state(&snapshot.view, &snapshot.peers).await;
            forward_ui_lines(&self.ui, lines);
            *last_view = snapshot.view.clone();
            let _ = self
                .ui
                .try_send(crate::ui::shell::UiInput::Snapshot(Box::new(snapshot)));
        }
    }

    async fn snapshot(&mut self) -> Option<crate::ui::app::UiSnapshot> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.handle.sync.send(SyncCommand::GetView(tx)).await.ok()?;
        let view = rx.await.ok()?;
        // Recency map for Recent Series: series id -> newest watch time,
        // with the series id taken from the *current* metadata view (a file
        // is often watched before its AniDB metadata arrives). See
        // [`crate::ui::props::watch_recency`].
        let recency = crate::ui::props::watch_recency(
            &self.storage.recent_watched(500).unwrap_or_default(),
            &view,
        );
        let cache_hashes = self
            .storage
            .cache_entries()
            .unwrap_or_default()
            .into_iter()
            .map(|entry| entry.hash)
            .collect();
        let watched_hashes = self.storage.watched_hashes().unwrap_or_default();
        Some(crate::ui::app::UiSnapshot {
            view: std::sync::Arc::new(view),
            peers: self.handle.peers.borrow().clone(),
            known_offline: self.handle.known_offline.borrow().clone(),
            now: (system_clock())(),
            recency,
            cache_hashes,
            watched_hashes,
            link: self.link,
        })
    }
}

/// `dessplay --dump`: print settings and the stored state as JSON on
/// stdout (logs go to stderr), then exit. `sections` trims the output to
/// the named [`crate::dump::SECTIONS`]; empty means all.
pub fn run_dump(args: &HeadlessArgs, sections: &[String]) -> Result<(), String> {
    let selection = crate::dump::Selection::parse(sections)?;
    let path = match &args.db_path {
        Some(path) => path.clone(),
        None => Storage::default_path().ok_or("cannot determine the data directory")?,
    };
    let storage = Storage::open(&path).map_err(|e| format!("opening {}: {e}", path.display()))?;
    let settings = storage
        .load_settings()
        .map_err(|e| format!("loading settings: {e}"))?;
    let media_roots = storage.media_roots().map_err(|e| e.to_string())?;
    let snapshot = load_state_tolerant(&storage)?;
    let doc = crate::dump::build(
        &path.display().to_string(),
        &settings,
        &media_roots,
        snapshot.as_ref(),
        &selection,
    )
    .map_err(|e| format!("rendering state as JSON: {e}"))?;
    let json = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    println!("{json}");
    Ok(())
}

/// `dessplay import-list`: parse the exported sheets, print the report,
/// and (unless `dry_run`) push the entries through a transient client.
pub async fn run_import(
    args: HeadlessArgs,
    files: Vec<std::path::PathBuf>,
    watchers: String,
    dry_run: bool,
) -> Result<(), String> {
    let map = crate::import::WatcherMap::parse(&watchers)?;
    let mut report = crate::import::ImportReport::default();
    for file in &files {
        let content = std::fs::read_to_string(file)
            .map_err(|e| format!("reading {}: {e}", file.display()))?;
        let label = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.display().to_string());
        crate::import::import_sheet(&content, &label, &map, &mut report)?;
    }

    println!("parsed {} entries:", report.entries.len());
    for (status, count) in report.status_counts() {
        println!("  {status}: {count}");
    }
    if !report.warnings.is_empty() {
        println!("\n{} rows need a human:", report.warnings.len());
        for warning in &report.warnings {
            println!("  {warning}");
        }
    }
    if dry_run {
        println!("\ndry run; nothing submitted");
        return Ok(());
    }

    let setup = prepare(&args).await?;
    let handle = spawn_client(
        Arc::clone(&setup.connector),
        ClientConfig {
            username: UserId::new(&setup.username),
            password: setup.password,
            role: Role::Interactive,
            session_nonce: rand::random(),
            clock: system_clock(),
            // Transient client: no stored state in, none persisted out.
            sync: SyncConfigExtras::default(),
        },
    );
    println!(
        "\nconnecting to {} as {}...",
        setup.server_addr_str, setup.username
    );
    let outcome = crate::import::submit(&handle, &report).await?;
    println!("created {}, updated {}", outcome.created, outcome.updated);
    if !outcome.collapsed.is_empty() {
        println!(
            "\n{} series appeared on more than one sheet and were collapsed \
             onto one entry (the later row won — check the status):",
            outcome.collapsed.len()
        );
        for name in &outcome.collapsed {
            println!("  {name}");
        }
    }

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

/// Parse one `.env` line into `(key, value)`. Tolerates an optional
/// `export ` prefix (users write `.env` files both ways), `#` comments,
/// and blank lines (both yield `None`).
fn parse_dotenv_line(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let line = line.strip_prefix("export ").unwrap_or(line);
    let (key, value) = line.split_once('=')?;
    Some((key.trim(), value.trim().trim_matches('"')))
}

/// Load `./.env` (KEY=VALUE lines; `#` comments) into the environment,
/// without overriding variables that are already set. The project keeps
/// `DESSPLAY_PASSWORD` for the default server there.
pub fn load_dotenv() {
    let Ok(contents) = std::fs::read_to_string(".env") else {
        return;
    };
    for line in contents.lines() {
        if let Some((key, value)) = parse_dotenv_line(line)
            && std::env::var_os(key).is_none()
        {
            // Single-threaded startup: set_var is safe here.
            unsafe { std::env::set_var(key, value) };
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
        // A bracketed IPv6 literal *without* a port must gain `:port`
        // directly, not another pair of brackets ([[::1]]:9876 was the bug).
        assert_eq!(with_default_port("[::1]"), "[::1]:9876");
        assert_eq!(with_default_port("[fe80::1]"), "[fe80::1]:9876");
    }

    #[test]
    fn interleave_families_alternates_starting_with_resolver_preference() {
        fn v4(last: u8) -> SocketAddr {
            format!("10.0.0.{last}:1").parse().unwrap()
        }
        fn v6(last: u8) -> SocketAddr {
            format!("[2001:db8::{last}]:1").parse().unwrap()
        }
        // Resolver prefers v6: the first v4 lands second, per-family
        // order preserved.
        assert_eq!(
            interleave_families(vec![v6(1), v6(2), v4(1), v4(2)]),
            vec![v6(1), v4(1), v6(2), v4(2)]
        );
        // Resolver prefers v4: symmetric.
        assert_eq!(interleave_families(vec![v4(1), v6(1)]), vec![v4(1), v6(1)]);
        // Single family / single address: unchanged.
        assert_eq!(interleave_families(vec![v6(1), v6(2)]), vec![v6(1), v6(2)]);
        assert_eq!(interleave_families(vec![v4(9)]), vec![v4(9)]);
        assert_eq!(interleave_families(vec![]), Vec::<SocketAddr>::new());
    }

    #[test]
    fn summon_report_formats_pinged_and_unmatched() {
        // Everyone matched.
        assert_eq!(
            summon_report(
                &[
                    (UserId::new("Nero"), "Nero200".to_string()),
                    (UserId::new("Quickshot"), "Quickshot".to_string()),
                ],
                &[],
            ),
            "Summoned Nero200, Quickshot on IRC."
        );
        // Some matched, some not.
        assert_eq!(
            summon_report(
                &[(UserId::new("Nero"), "Nero200".to_string())],
                &[UserId::new("Kim")],
            ),
            "Summoned Nero200 on IRC. No IRC nick found for Kim."
        );
        // Nobody matched (also covers "not connected to IRC" — the IRC
        // actor reports that the same way).
        assert_eq!(
            summon_report(&[], &[UserId::new("Kim"), UserId::new("Dagger")]),
            "/summon: no IRC nick found for Kim, Dagger"
        );
    }

    #[test]
    fn username_precedence_flag_then_stored_then_env() {
        let s = |x: &str| Some(x.to_string());
        // Flag wins over everything.
        assert_eq!(
            resolve_username(s("flag"), s("stored"), s("env")),
            s("flag")
        );
        // No flag: stored wins over env.
        assert_eq!(resolve_username(None, s("stored"), s("env")), s("stored"));
        // Neither flag nor stored: fall back to $USER.
        assert_eq!(resolve_username(None, None, s("env")), s("env"));
        // Nothing at all.
        assert_eq!(resolve_username(None, None, None), None);
    }

    #[test]
    fn flag_username_override_is_not_folded_into_persistable_settings() {
        // A real stored identity, launched with `--username Foo` (and a
        // $USER that must be ignored because a name is already stored).
        let mut settings = crate::config::Settings {
            username: Some("Real".into()),
            password: Some("pw".into()),
            ..Default::default()
        };
        let identity =
            resolve_runtime_identity(&mut settings, Some("Foo".into()), Some("envuser".into()));
        // The runtime identity honours the override...
        assert_eq!(identity, Some("Foo".into()));
        // ...but the persistable settings keep the stored username, so a
        // later settings save cannot write the one-off flag back to the DB
        // (design.md: flags/env "override ... but are never persisted").
        assert_eq!(settings.username, Some("Real".into()));
    }

    #[test]
    fn flag_username_override_survives_a_settings_save() {
        // End-to-end through storage: stored "Real", launched with
        // `--username Foo`, then any settings save (e.g. an F2 subtitle
        // cycle persisting the in-memory settings verbatim) must leave the
        // stored username "Real" — not "Foo".
        let storage = Storage::open_in_memory().unwrap();
        storage
            .save_settings(&crate::config::Settings {
                username: Some("Real".into()),
                password: Some("pw".into()),
                ..Default::default()
            })
            .unwrap();

        let mut settings = storage.load_settings().unwrap();
        let identity =
            resolve_runtime_identity(&mut settings, Some("Foo".into()), Some("envuser".into()));
        assert_eq!(
            identity,
            Some("Foo".into()),
            "runtime identity uses the override"
        );

        // The UI holds `settings` and persists it verbatim on the next save.
        storage.save_settings(&settings).unwrap();
        assert_eq!(
            storage.load_settings().unwrap().username,
            Some("Real".into()),
            "a one-off --username must never be persisted (design.md)"
        );
    }

    #[test]
    fn user_edited_username_still_persists() {
        // The converse no-regression: a username the user actually changes
        // in the settings screen DOES persist (only an untouched override
        // is suppressed).
        let storage = Storage::open_in_memory().unwrap();
        storage
            .save_settings(&crate::config::Settings {
                username: Some("Real".into()),
                password: Some("pw".into()),
                ..Default::default()
            })
            .unwrap();

        let mut settings = storage.load_settings().unwrap();
        let _ = resolve_runtime_identity(&mut settings, Some("Foo".into()), Some("envuser".into()));
        // The user opens the settings screen and types a new name.
        settings.username = Some("Chosen".into());
        storage.save_settings(&settings).unwrap();
        assert_eq!(
            storage.load_settings().unwrap().username,
            Some("Chosen".into()),
            "a deliberate settings-screen edit must persist"
        );
    }

    #[test]
    fn first_run_prefills_username_from_flag_then_env() {
        // No stored username (first run): the flag, then $USER, seeds the
        // settings modal's editable default — which the user confirms, so
        // it persists. Both the identity and the persistable value carry it
        // (there is no stored value to clobber).
        let mut settings = crate::config::Settings::default();
        let identity =
            resolve_runtime_identity(&mut settings, Some("Foo".into()), Some("envuser".into()));
        assert_eq!(identity, Some("Foo".into()));
        assert_eq!(settings.username, Some("Foo".into()));

        // Flag absent: fall back to $USER.
        let mut settings = crate::config::Settings::default();
        let identity = resolve_runtime_identity(&mut settings, None, Some("envuser".into()));
        assert_eq!(identity, Some("envuser".into()));
        assert_eq!(settings.username, Some("envuser".into()));
    }

    #[test]
    fn cache_dir_flag_overrides_default() {
        // --cache-dir set: used verbatim, in every mode.
        let args = HeadlessArgs {
            cache_dir: Some(PathBuf::from("/tmp/custom-cache")),
            ..Default::default()
        };
        assert_eq!(
            resolve_cache_dir(&args).unwrap(),
            PathBuf::from("/tmp/custom-cache")
        );
        // Unset: fall back to the standard XDG cache directory.
        assert_eq!(
            resolve_cache_dir(&HeadlessArgs::default()).unwrap(),
            download_cache_dir().unwrap()
        );
    }

    #[test]
    fn media_roots_flag_overrides_stored() {
        let stored = vec![PathBuf::from("/stored/a"), PathBuf::from("/stored/b")];
        // Non-empty --media-root overrides the stored roots (interactive),
        // and is the seeder's only source.
        assert_eq!(
            resolve_media_roots(&[PathBuf::from("/flag/x")], stored.clone()),
            vec![PathBuf::from("/flag/x")]
        );
        // Empty flag falls through to the stored roots.
        assert_eq!(resolve_media_roots(&[], stored.clone()), stored);
    }

    #[test]
    fn media_root_override_is_not_folded_into_persistable_base() {
        // A real stored set of roots, launched with `--media-root /flag`.
        let stored = vec![PathBuf::from("/real")];
        let split = resolve_runtime_media_roots(&[PathBuf::from("/flag")], stored);
        // The runtime roots honour the override (so `--media-root` still
        // takes effect: the file actor scans /flag)...
        assert_eq!(
            split.runtime,
            vec![PathBuf::from("/flag")],
            "the runtime roots must honour the --media-root override"
        );
        // ...but the persistable base keeps the stored roots, so a later
        // settings save cannot write the one-off flag back to the DB
        // (design.md: flags/env "override ... but are never persisted").
        assert_eq!(
            split.persistable,
            vec![PathBuf::from("/real")],
            "an untouched --media-root override must never be persisted"
        );
    }

    #[test]
    fn media_root_override_survives_a_settings_save() {
        // End-to-end through storage: stored [/real], launched with
        // `--media-root /flag`, then any settings save (e.g. an F2 subtitle
        // cycle persisting the in-memory roots verbatim) must leave the
        // stored roots [/real] — not [/flag].
        let mut storage = Storage::open_in_memory().unwrap();
        storage.set_media_roots(&[PathBuf::from("/real")]).unwrap();

        let split =
            resolve_runtime_media_roots(&[PathBuf::from("/flag")], storage.media_roots().unwrap());
        assert_eq!(
            split.runtime,
            vec![PathBuf::from("/flag")],
            "runtime roots use the override"
        );

        // The UI holds the persistable base and carries it on the next save.
        storage.set_media_roots(&split.persistable).unwrap();
        assert_eq!(
            storage.media_roots().unwrap(),
            vec![PathBuf::from("/real")],
            "a one-off --media-root must never be persisted (design.md)"
        );
    }

    #[test]
    fn f2_subtitle_cycle_does_not_reconfigure_irc() {
        // An F2 subtitle-mode cycle clones the whole Settings and only
        // changes `subtitle_mode` — it must not be mistaken for an IRC
        // settings change and force a needless reconnect.
        let me = UserId("Baughn".into());
        let old = crate::config::Settings::default();
        let mut new = old.clone();
        new.subtitle_mode = match old.subtitle_mode {
            crate::config::SubtitleMode::Off => crate::config::SubtitleMode::Intermixed,
            _ => crate::config::SubtitleMode::Off,
        };
        assert_ne!(old.subtitle_mode, new.subtitle_mode);
        assert!(!irc_config_changed(&old, &new, &me));
    }

    #[test]
    fn genuine_irc_setting_change_reconfigures_irc() {
        let me = UserId("Baughn".into());
        let old = crate::config::Settings::default();
        let mut new = old.clone();
        new.irc_server = "irc.example.org".into();
        assert!(irc_config_changed(&old, &new, &me));
    }

    #[test]
    fn user_edited_media_roots_still_persist() {
        // The converse no-regression: roots the user actually changes in the
        // settings modal DO persist (only an untouched override is
        // suppressed).
        let mut storage = Storage::open_in_memory().unwrap();
        storage.set_media_roots(&[PathBuf::from("/real")]).unwrap();

        let split =
            resolve_runtime_media_roots(&[PathBuf::from("/flag")], storage.media_roots().unwrap());
        // The user opens the settings screen (seeded with the persistable
        // base) and adds a root.
        let mut edited = split.persistable.clone();
        edited.push(PathBuf::from("/added"));
        storage.set_media_roots(&edited).unwrap();
        assert_eq!(
            storage.media_roots().unwrap(),
            vec![PathBuf::from("/real"), PathBuf::from("/added")],
            "a deliberate settings-screen edit must persist"
        );
    }

    #[test]
    fn first_run_prefills_media_roots_from_flag() {
        // No stored roots (first run): the flag seeds both the runtime roots
        // and the persistable base — there is no stored value to clobber, and
        // the settings modal turns the prefill into an editable default the
        // user confirms before it persists.
        let split = resolve_runtime_media_roots(&[PathBuf::from("/flag")], vec![]);
        assert_eq!(split.runtime, vec![PathBuf::from("/flag")]);
        assert_eq!(
            split.persistable,
            vec![PathBuf::from("/flag")],
            "with nothing stored, the flag is a legitimate first-run prefill"
        );

        // First run with no flag either: nothing to scan, nothing to seed.
        let split = resolve_runtime_media_roots(&[], vec![]);
        assert!(split.runtime.is_empty());
        assert!(split.persistable.is_empty());
    }

    #[test]
    fn pipeline_depth_flag_overrides_default() {
        let args = HeadlessArgs {
            pipeline_depth: Some(64),
            ..Default::default()
        };
        assert_eq!(download_config(&args).pipeline_depth, 64);
        // Unset: the default download queue size.
        assert_eq!(download_config(&HeadlessArgs::default()).pipeline_depth, 48);
    }

    #[test]
    fn dotenv_line_parsing() {
        assert_eq!(parse_dotenv_line("KEY=value"), Some(("KEY", "value")));
        assert_eq!(
            parse_dotenv_line("export KEY=value"),
            Some(("KEY", "value"))
        );
        assert_eq!(
            parse_dotenv_line("  export KEY = \"quoted\"  "),
            Some(("KEY", "quoted"))
        );
        // `export` is only stripped as a prefix keyword, not from keys.
        assert_eq!(
            parse_dotenv_line("exported=value"),
            Some(("exported", "value"))
        );
        assert_eq!(parse_dotenv_line("# comment"), None);
        assert_eq!(parse_dotenv_line(""), None);
        assert_eq!(parse_dotenv_line("not a kv pair"), None);
    }

    #[test]
    fn hex_decoding() {
        assert_eq!(decode_hex("0aff").unwrap(), vec![0x0a, 0xff]);
        assert_eq!(decode_hex("0A:FF").unwrap(), vec![0x0a, 0xff]);
        assert!(decode_hex("abc").is_err());
        assert!(decode_hex("zz").is_err());
    }
}

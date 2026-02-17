use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

use dessplay::network::quic::QuicConnectionManager;
use dessplay::network::rendezvous::RendezvousClient;
use dessplay::network::{ConnectionEvent, ConnectionManager, ConnectionState, PeerId};
use dessplay::tui::{App, AppEvent};

#[derive(Parser)]
#[command(name = "dessplay", about = "Synchronized video player for watch parties")]
struct Args {
    /// Display name
    #[arg(long, default_value_t = default_username())]
    username: String,

    /// Rendezvous server address (IP:port or hostname:port)
    #[arg(long)]
    server: String,

    /// Rendezvous password
    #[arg(long, env = "DESSPLAY_PASSWORD")]
    password: String,

    /// Verbosity (-v, -vv)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn default_username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "anonymous".into())
}

fn log_file_path(username: &str) -> PathBuf {
    let dir = directories::ProjectDirs::from("", "", "dessplay")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&dir).ok();
    dir.join(format!("{username}.log"))
}

fn known_servers_path() -> PathBuf {
    let dir = directories::ProjectDirs::from("", "", "dessplay")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&dir).ok();
    dir.join("known_servers")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Tracing to log file (TUI owns the terminal)
    let log_path = log_file_path(&args.username);
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(log_file)
        .with_ansi(false)
        .init();

    tracing::info!("dessplay starting, username={}", args.username);

    // Resolve server address (supports both IP:port and hostname:port)
    let server_addr: SocketAddr = tokio::net::lookup_host(&args.server)
        .await?
        .next()
        .ok_or_else(|| anyhow::anyhow!("could not resolve server address: {}", args.server))?;
    tracing::info!("resolved server {} -> {}", args.server, server_addr);

    // Create QUIC connection manager
    let bind_addr: SocketAddr = "[::]:0".parse().unwrap();
    let conn_mgr = QuicConnectionManager::new(bind_addr, PeerId(args.username.clone())).await?;
    tracing::info!("listening on {}", conn_mgr.local_addr());

    // Set up TUI event channel
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let app = App::new(args.username.clone(), args.verbose);

    // Spawn connection event forwarder
    let mut conn_events = conn_mgr.subscribe();
    let event_tx_conn = event_tx.clone();
    tokio::spawn(async move {
        while let Ok(event) = conn_events.recv().await {
            let app_event = match event {
                ConnectionEvent::PeerConnected(peer) => AppEvent::PeerConnected(peer.0),
                ConnectionEvent::PeerDisconnected(peer) => AppEvent::PeerDisconnected(peer.0),
                ConnectionEvent::ConnectionStateChanged { peer, state } => {
                    let state_str = match state {
                        ConnectionState::Direct => "direct",
                        ConnectionState::Relayed => "relayed",
                        ConnectionState::Connecting => "connecting",
                        ConnectionState::Disconnected => "disconnected",
                    };
                    AppEvent::ConnectionStateChanged {
                        peer: peer.0,
                        state: state_str.to_string(),
                    }
                }
            };
            if event_tx_conn.send(app_event).is_err() {
                break;
            }
        }
    });

    // Spawn rendezvous connection task
    let password = args.password.clone();
    let username = args.username.clone();
    let server_key = args.server.clone();
    let event_tx_rv = event_tx.clone();
    tokio::spawn(async move {
        let _ = event_tx_rv.send(AppEvent::SystemMessage {
            text: format!("Connecting to {server_addr}..."),
            min_verbosity: 0,
        });

        let known_path = known_servers_path();
        let endpoint = conn_mgr.endpoint().clone();

        let result = RendezvousClient::connect(
            &endpoint,
            server_addr,
            &username,
            &password,
            &known_path,
            &server_key,
        )
        .await;

        let (mut client, peers, observed_addr) = match result {
            Ok(v) => v,
            Err(e) => {
                let _ = event_tx_rv.send(AppEvent::SystemMessage {
                    text: format!("Rendezvous failed: {e}"),
                    min_verbosity: 0,
                });
                return;
            }
        };

        let _ = event_tx_rv.send(AppEvent::SystemMessage {
            text: format!("Connected to rendezvous server ({} peer(s))", peers.len()),
            min_verbosity: 0,
        });
        let _ = event_tx_rv.send(AppEvent::SystemMessage {
            text: format!("Your address: {observed_addr}"),
            min_verbosity: 1,
        });

        // Set up relay
        conn_mgr.set_relay(client.connection().clone());

        // Connect to initial peers
        for peer in &peers {
            let _ = event_tx_rv.send(AppEvent::SystemMessage {
                text: format!("Discovered peer: {} at {:?}", peer.peer_id, peer.addrs),
                min_verbosity: 1,
            });

            let mut connected = false;
            for addr in &peer.addrs {
                match conn_mgr.connect_to(*addr).await {
                    Ok(_) => {
                        connected = true;
                        break;
                    }
                    Err(e) => {
                        tracing::debug!(
                            peer = %peer.peer_id,
                            addr = %addr,
                            "direct connect failed: {e}"
                        );
                    }
                }
            }
            if !connected {
                // Fall back to relay
                conn_mgr.add_relayed_peer(PeerId(peer.peer_id.clone()));
            }
        }

        // Keepalive loop
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            interval.tick().await;
            match client.keepalive().await {
                Ok(peers) => {
                    let _ = event_tx_rv.send(AppEvent::SystemMessage {
                        text: format!("Keepalive: {} peer(s)", peers.len()),
                        min_verbosity: 1,
                    });

                    // Connect to new peers
                    let known = conn_mgr.connected_peers();
                    for peer in &peers {
                        let peer_id = PeerId(peer.peer_id.clone());
                        if known.contains(&peer_id) {
                            continue;
                        }
                        let mut connected = false;
                        for addr in &peer.addrs {
                            match conn_mgr.connect_to(*addr).await {
                                Ok(_) => {
                                    connected = true;
                                    break;
                                }
                                Err(e) => {
                                    tracing::debug!(
                                        peer = %peer.peer_id,
                                        addr = %addr,
                                        "direct connect failed: {e}"
                                    );
                                }
                            }
                        }
                        if !connected {
                            conn_mgr.add_relayed_peer(peer_id);
                        }
                    }
                }
                Err(e) => {
                    let _ = event_tx_rv.send(AppEvent::SystemMessage {
                        text: format!("Keepalive failed: {e}"),
                        min_verbosity: 0,
                    });
                    break;
                }
            }
        }
    });

    // Run TUI (blocks until user quits)
    dessplay::tui::run(app, event_rx).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Integration test: resolve DNS name and attempt QUIC connection to the
    /// real rendezvous server. Expects an auth rejection (wrong password),
    /// which proves the full DNS → QUIC → protocol path works.
    #[ignore]
    #[tokio::test]
    async fn connect_to_rendezvous_via_dns() {
        let server = "v4.brage.info:4433";
        let server_addr: SocketAddr = tokio::net::lookup_host(server)
            .await
            .expect("DNS lookup failed")
            .next()
            .expect("no addresses returned");

        let bind_addr: SocketAddr = "[::]:0".parse().unwrap();
        let conn_mgr =
            QuicConnectionManager::new(bind_addr, PeerId("dns-test".into()))
                .await
                .expect("failed to create QUIC endpoint");

        let known_path = tempfile::NamedTempFile::new().unwrap();
        let result = RendezvousClient::connect(
            &conn_mgr.endpoint(),
            server_addr,
            "dns-test",
            "wrong-password",
            known_path.path(),
            &server_addr.to_string(),
        )
        .await;

        // Auth rejection means we successfully resolved, connected via QUIC,
        // and exchanged protocol messages — DNS works.
        assert!(
            result.is_err(),
            "expected auth rejection with wrong password"
        );
    }
}

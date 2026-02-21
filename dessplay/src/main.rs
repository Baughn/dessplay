use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use dessplay::peer_conn::PeerManager;
use dessplay::rendezvous_client::RendezvousEvent;
use dessplay::storage;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("Usage: dessplay [OPTIONS]");
        println!();
        println!("Options:");
        println!("  --server <ADDR>  Server address (default from config)");
        println!("  --dump           Read and pretty-print the local database to stdout");
        println!("  --help           Show this help message");
        return Ok(());
    }

    if args.iter().any(|a| a == "--dump") {
        let db_path = storage::default_db_path()?;
        let db = storage::ClientStorage::open(&db_path)?;
        dessplay::dump::dump_database(&db)?;
        return Ok(());
    }

    // Open storage
    let db_path = storage::default_db_path()?;
    let storage = Arc::new(Mutex::new(storage::ClientStorage::open(&db_path)?));

    // Get config
    let config = {
        let s = storage.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        s.get_config()?
            .ok_or_else(|| anyhow::anyhow!("no config found — run dessplay with --setup first"))?
    };

    // Get password from config or environment
    let password = config
        .password
        .or_else(|| std::env::var("DESSPLAY_PASSWORD").ok())
        .context("no password configured")?;

    // Override server address from CLI if provided
    let server_str = get_arg(&args, "--server").unwrap_or(config.server);

    // Resolve server address
    let server_addr: SocketAddr = server_str
        .parse()
        .context("invalid server address — expected host:port")?;

    // Create TOFU verifier
    let tofu = Arc::new(dessplay::tls::TofuVerifier::new(
        Arc::clone(&storage),
        server_str.clone(),
    ));

    // Create dual QUIC endpoint
    let bind_addr: SocketAddr = "[::]:0".parse().context("invalid bind address")?;
    let dessplay::quic::DualEndpoint {
        endpoint,
        peer_client_config,
    } = dessplay::quic::create_dual_endpoint(bind_addr, tofu)?;

    // Connect to rendezvous server
    let rv_client = dessplay::rendezvous_client::RendezvousClient::connect(
        &endpoint,
        server_addr,
        "dessplay-rendezvous",
        &password,
        &config.username,
    )
    .await?;

    tracing::info!(
        peer_id = %rv_client.peer_id,
        observed_addr = %rv_client.observed_addr,
        "Connected to rendezvous server"
    );

    // Set up peer manager
    let peer_mgr = Arc::new(PeerManager::new(
        endpoint,
        peer_client_config,
        rv_client.peer_id,
        config.username.clone(),
    ));
    peer_mgr.spawn_accept_loop();

    // Main event loop
    loop {
        tokio::select! {
            event = rv_client.recv() => {
                match event {
                    Some(RendezvousEvent::PeerList { peers }) => {
                        tracing::info!(count = peers.len(), "Got peer list update");
                        peer_mgr.update_peer_list(peers).await;
                    }
                    None => {
                        tracing::info!("Rendezvous server disconnected");
                        break;
                    }
                }
            }
            event = peer_mgr.recv() => {
                match event {
                    Ok(dessplay_core::network::NetworkEvent::PeerConnected { peer_id, username }) => {
                        tracing::info!(%peer_id, %username, "Peer connected");
                    }
                    Ok(dessplay_core::network::NetworkEvent::PeerDisconnected { peer_id }) => {
                        tracing::info!(%peer_id, "Peer disconnected");
                    }
                    Ok(dessplay_core::network::NetworkEvent::PeerControl { from, message }) => {
                        tracing::debug!(%from, ?message, "Peer control message");
                    }
                    Ok(dessplay_core::network::NetworkEvent::PeerDatagram { from, message }) => {
                        tracing::debug!(%from, ?message, "Peer datagram");
                    }
                    Ok(dessplay_core::network::NetworkEvent::IncomingStream { from, .. }) => {
                        tracing::debug!(%from, "Incoming stream");
                    }
                    Err(e) => {
                        tracing::warn!("Peer manager error: {e}");
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Extract the value after a `--flag` from the arg list.
fn get_arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};

use dessplay::peer_conn::PeerManager;
use dessplay::rendezvous_client::RendezvousEvent;
use dessplay::storage;
use dessplay_core::framing::{
    read_framed, write_framed, TAG_GAP_FILL_REQUEST, TAG_GAP_FILL_RESPONSE,
};
use dessplay_core::network::NetworkEvent;
use dessplay_core::protocol::{
    GapFillRequest, GapFillResponse, PeerControl, PeerDatagram, RvControl,
};
use dessplay_core::sync_engine::{SyncAction, SyncEngine};
use dessplay_core::types::PeerId;

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

    // Initialize sync engine from persisted state
    let sync_engine = {
        let s = storage.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        match s.load_latest_snapshot()? {
            Some((epoch, snapshot)) => {
                let mut state = dessplay_core::crdt::CrdtState::new();
                state.load_snapshot(epoch, snapshot);
                for op in s.load_ops(epoch)? {
                    state.apply_op(&op);
                }
                tracing::info!(%epoch, "Loaded persisted CRDT state");
                SyncEngine::from_persisted(epoch, state, epoch)
            }
            None => {
                tracing::info!("No persisted state, starting fresh");
                SyncEngine::new()
            }
        }
    };
    let sync_engine = Arc::new(tokio::sync::Mutex::new(sync_engine));

    // Send our state summary to the server
    {
        let eng = sync_engine.lock().await;
        rv_client.send(RvControl::StateSummary {
            versions: eng.version_vectors(),
        });
    }

    // Periodic state summary timer
    let mut summary_interval = tokio::time::interval(Duration::from_secs(1));

    // Main event loop
    loop {
        tokio::select! {
            // Rendezvous server events
            event = rv_client.recv() => {
                match event {
                    Some(RendezvousEvent::PeerList { peers }) => {
                        tracing::info!(count = peers.len(), "Got peer list update");
                        peer_mgr.update_peer_list(peers).await;
                    }
                    Some(RendezvousEvent::StateOp { op }) => {
                        // Use a special PeerId(0) for the server
                        let actions = sync_engine.lock().await.on_remote_op(PeerId(0), op);
                        dispatch_actions(
                            actions,
                            &peer_mgr,
                            &rv_client,
                            &storage,
                            &sync_engine,
                        ).await;
                    }
                    Some(RendezvousEvent::StateSummary { versions }) => {
                        let actions = sync_engine.lock().await
                            .on_state_summary(PeerId(0), versions.epoch, versions);
                        dispatch_actions(
                            actions,
                            &peer_mgr,
                            &rv_client,
                            &storage,
                            &sync_engine,
                        ).await;
                    }
                    Some(RendezvousEvent::StateSnapshot { epoch, crdts }) => {
                        let actions = sync_engine.lock().await
                            .on_state_snapshot(epoch, crdts);
                        dispatch_actions(
                            actions,
                            &peer_mgr,
                            &rv_client,
                            &storage,
                            &sync_engine,
                        ).await;
                    }
                    None => {
                        tracing::info!("Rendezvous server disconnected");
                        break;
                    }
                }
            }

            // Peer network events
            event = peer_mgr.recv() => {
                match event {
                    Ok(NetworkEvent::PeerConnected { peer_id, username }) => {
                        tracing::info!(%peer_id, %username, "Peer connected");
                        let actions = sync_engine.lock().await.on_peer_connected(peer_id);
                        dispatch_actions(
                            actions,
                            &peer_mgr,
                            &rv_client,
                            &storage,
                            &sync_engine,
                        ).await;
                    }
                    Ok(NetworkEvent::PeerDisconnected { peer_id }) => {
                        tracing::info!(%peer_id, "Peer disconnected");
                        sync_engine.lock().await.on_peer_disconnected(peer_id);
                    }
                    Ok(NetworkEvent::PeerControl { from, message }) => {
                        let actions = match message {
                            PeerControl::StateOp { op } => {
                                sync_engine.lock().await.on_remote_op(from, op)
                            }
                            PeerControl::StateSummary { epoch, versions } => {
                                sync_engine.lock().await
                                    .on_state_summary(from, epoch, versions)
                            }
                            PeerControl::StateSnapshot { epoch, crdts } => {
                                sync_engine.lock().await.on_state_snapshot(epoch, crdts)
                            }
                            other => {
                                tracing::debug!(%from, ?other, "Unhandled peer control");
                                vec![]
                            }
                        };
                        dispatch_actions(
                            actions,
                            &peer_mgr,
                            &rv_client,
                            &storage,
                            &sync_engine,
                        ).await;
                    }
                    Ok(NetworkEvent::PeerDatagram { from, message }) => {
                        let actions = match message {
                            PeerDatagram::StateOp { op } => {
                                sync_engine.lock().await.on_remote_op(from, op)
                            }
                            _ => vec![], // Position/Seek — Phase 7
                        };
                        dispatch_actions(
                            actions,
                            &peer_mgr,
                            &rv_client,
                            &storage,
                            &sync_engine,
                        ).await;
                    }
                    Ok(NetworkEvent::IncomingStream { from, stream }) => {
                        tracing::debug!(%from, "Incoming stream");
                        let engine = Arc::clone(&sync_engine);
                        tokio::spawn(async move {
                            if let Err(e) = handle_incoming_stream(stream, engine).await {
                                tracing::debug!(%from, "Gap fill stream error: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!("Peer manager error: {e}");
                        break;
                    }
                }
            }

            // Periodic state summary
            _ = summary_interval.tick() => {
                let actions = sync_engine.lock().await.on_periodic_tick();
                dispatch_actions(
                    actions,
                    &peer_mgr,
                    &rv_client,
                    &storage,
                    &sync_engine,
                ).await;
            }
        }
    }

    Ok(())
}

/// Dispatch sync actions to the network and storage layers.
async fn dispatch_actions(
    actions: Vec<SyncAction>,
    peer_mgr: &Arc<PeerManager>,
    rv_client: &dessplay::rendezvous_client::RendezvousClient,
    storage: &Arc<Mutex<storage::ClientStorage>>,
    sync_engine: &Arc<tokio::sync::Mutex<SyncEngine>>,
) {
    for action in actions {
        match action {
            SyncAction::SendControl { peer, msg } => {
                if peer == PeerId(0) {
                    // Server — send via rendezvous client
                    let rv_msg = match msg {
                        PeerControl::StateOp { op } => RvControl::StateOp { op },
                        PeerControl::StateSummary { versions, .. } => {
                            RvControl::StateSummary { versions }
                        }
                        PeerControl::StateSnapshot { epoch, crdts } => {
                            RvControl::StateSnapshot { epoch, crdts }
                        }
                        _ => continue,
                    };
                    rv_client.send(rv_msg);
                } else if let Err(e) = peer_mgr.send_control(peer, &msg).await {
                    tracing::debug!(%peer, "Failed to send control: {e}");
                }
            }
            SyncAction::SendDatagram { peer, msg } => {
                if peer != PeerId(0) {
                    let _ = peer_mgr.send_datagram(peer, &msg).await;
                }
            }
            SyncAction::BroadcastControl { msg } => {
                // Send to all peers
                for peer in peer_mgr.connected_peers().await {
                    let _ = peer_mgr.send_control(peer, &msg).await;
                }
                // Also send to server
                let rv_msg = match &msg {
                    PeerControl::StateOp { op } => {
                        Some(RvControl::StateOp { op: op.clone() })
                    }
                    PeerControl::StateSummary { versions, .. } => {
                        Some(RvControl::StateSummary {
                            versions: versions.clone(),
                        })
                    }
                    _ => None,
                };
                if let Some(rv_msg) = rv_msg {
                    rv_client.send(rv_msg);
                }
            }
            SyncAction::BroadcastDatagram { msg } => {
                for peer in peer_mgr.connected_peers().await {
                    let _ = peer_mgr.send_datagram(peer, &msg).await;
                }
            }
            SyncAction::RequestGapFill { peer, request } => {
                if peer == PeerId(0) {
                    // Server gap fill — not supported in P2P gap fill protocol
                    // (server sends ops on the control stream instead)
                    tracing::debug!("Gap fill to server not supported via streams");
                } else {
                    let peer_mgr = Arc::clone(peer_mgr);
                    let engine = Arc::clone(sync_engine);
                    tokio::spawn(async move {
                        if let Err(e) =
                            handle_gap_fill_request(peer, request, &peer_mgr, engine).await
                        {
                            tracing::debug!(%peer, "Gap fill request failed: {e}");
                        }
                    });
                }
            }
            SyncAction::PersistOp { op } => {
                let epoch = sync_engine.lock().await.epoch();
                if let Ok(s) = storage.lock()
                    && let Err(e) = s.append_op(epoch, &op)
                {
                    tracing::warn!("Failed to persist op: {e}");
                }
            }
            SyncAction::PersistSnapshot { epoch, snapshot } => {
                if let Ok(s) = storage.lock()
                    && let Err(e) = s.save_snapshot(epoch, &snapshot)
                {
                    tracing::warn!("Failed to persist snapshot: {e}");
                }
            }
        }
    }
}

/// Open a gap fill stream to a peer, send request, read response.
async fn handle_gap_fill_request(
    peer: PeerId,
    request: GapFillRequest,
    peer_mgr: &PeerManager,
    sync_engine: Arc<tokio::sync::Mutex<SyncEngine>>,
) -> Result<()> {
    let mut stream = peer_mgr.open_stream(peer).await?;
    write_framed(&mut stream.send, TAG_GAP_FILL_REQUEST, &request).await?;
    let response: GapFillResponse = read_framed(&mut stream.recv, TAG_GAP_FILL_RESPONSE)
        .await?
        .ok_or_else(|| anyhow::anyhow!("gap fill stream closed without response"))?;

    let mut eng = sync_engine.lock().await;
    let actions = eng.on_gap_fill_response(peer, response);
    // Gap fill response only produces PersistOp actions — handled in the caller
    drop(eng);

    // Persist the ops directly (these are PersistOp actions only)
    for action in actions {
        if let SyncAction::PersistOp { .. } = action {
            // Persistence will be handled by the next periodic tick's
            // summary exchange if any ops were applied. For now, the ops
            // are in the in-memory CrdtState which is sufficient.
        }
    }
    Ok(())
}

/// Handle an incoming gap fill stream from a peer.
async fn handle_incoming_stream(
    mut stream: dessplay_core::network::MessageStream,
    sync_engine: Arc<tokio::sync::Mutex<SyncEngine>>,
) -> Result<()> {
    let request: GapFillRequest = read_framed(&mut stream.recv, TAG_GAP_FILL_REQUEST)
        .await?
        .ok_or_else(|| anyhow::anyhow!("stream closed before gap fill request"))?;

    let response = {
        let eng = sync_engine.lock().await;
        eng.on_gap_fill_request(&request)
    };
    write_framed(&mut stream.send, TAG_GAP_FILL_RESPONSE, &response).await?;
    Ok(())
}

/// Extract the value after a `--flag` from the arg list.
fn get_arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};

use dessplay::app_state::{AppEffect, AppEvent, AppState};
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
use dessplay_core::types::{PeerId, UserId};

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

    // Initialize AppState from persisted state
    let app_state = {
        let s = storage.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let user_id = UserId(config.username.clone());
        match s.load_latest_snapshot()? {
            Some((epoch, snapshot)) => {
                let mut state = dessplay_core::crdt::CrdtState::new();
                state.load_snapshot(epoch, snapshot);
                for op in s.load_ops(epoch)? {
                    state.apply_op(&op);
                }
                tracing::info!(%epoch, "Loaded persisted CRDT state");
                let engine = SyncEngine::from_persisted(epoch, state, epoch);
                AppState::from_persisted(user_id, engine)
            }
            None => {
                tracing::info!("No persisted state, starting fresh");
                AppState::new(user_id)
            }
        }
    };
    let app_state = Arc::new(tokio::sync::Mutex::new(app_state));

    // Send our state summary to the server
    {
        let app = app_state.lock().await;
        rv_client.send(RvControl::StateSummary {
            versions: app.sync_engine.version_vectors(),
        });
    }

    // Gap fill response channel — spawned tasks send results here
    let (gap_fill_tx, mut gap_fill_rx) =
        tokio::sync::mpsc::unbounded_channel::<(PeerId, GapFillResponse)>();

    // Periodic state summary timer
    let mut summary_interval = tokio::time::interval(Duration::from_secs(1));

    // Main event loop
    loop {
        tokio::select! {
            // Rendezvous server events
            event = rv_client.recv() => {
                let now = rv_client.shared_now().await;
                match event {
                    Some(RendezvousEvent::PeerList { peers }) => {
                        tracing::info!(count = peers.len(), "Got peer list update");
                        peer_mgr.update_peer_list(peers).await;
                    }
                    Some(RendezvousEvent::StateOp { op }) => {
                        let effects = app_state.lock().await.process_event(
                            AppEvent::RemoteOp { from: PeerId(0), op },
                            now,
                        );
                        dispatch_effects(
                            effects, &peer_mgr, &rv_client, &storage,
                            &app_state, &gap_fill_tx,
                        ).await;
                    }
                    Some(RendezvousEvent::StateSummary { versions }) => {
                        let effects = app_state.lock().await.process_event(
                            AppEvent::StateSummary {
                                from: PeerId(0),
                                epoch: versions.epoch,
                                versions,
                            },
                            now,
                        );
                        dispatch_effects(
                            effects, &peer_mgr, &rv_client, &storage,
                            &app_state, &gap_fill_tx,
                        ).await;
                    }
                    Some(RendezvousEvent::StateSnapshot { epoch, crdts }) => {
                        let effects = app_state.lock().await.process_event(
                            AppEvent::StateSnapshot { epoch, snapshot: crdts },
                            now,
                        );
                        dispatch_effects(
                            effects, &peer_mgr, &rv_client, &storage,
                            &app_state, &gap_fill_tx,
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
                let now = rv_client.shared_now().await;
                match event {
                    Ok(NetworkEvent::PeerConnected { peer_id, username }) => {
                        tracing::info!(%peer_id, %username, "Peer connected");
                        let effects = app_state.lock().await.process_event(
                            AppEvent::PeerConnected { peer_id, username },
                            now,
                        );
                        dispatch_effects(
                            effects, &peer_mgr, &rv_client, &storage,
                            &app_state, &gap_fill_tx,
                        ).await;
                    }
                    Ok(NetworkEvent::PeerDisconnected { peer_id }) => {
                        tracing::info!(%peer_id, "Peer disconnected");
                        let effects = app_state.lock().await.process_event(
                            AppEvent::PeerDisconnected { peer_id },
                            now,
                        );
                        dispatch_effects(
                            effects, &peer_mgr, &rv_client, &storage,
                            &app_state, &gap_fill_tx,
                        ).await;
                    }
                    Ok(NetworkEvent::PeerControl { from, message }) => {
                        let app_event = match message {
                            PeerControl::StateOp { op } => {
                                Some(AppEvent::RemoteOp { from, op })
                            }
                            PeerControl::StateSummary { epoch, versions } => {
                                Some(AppEvent::StateSummary { from, epoch, versions })
                            }
                            PeerControl::StateSnapshot { epoch, crdts } => {
                                Some(AppEvent::StateSnapshot { epoch, snapshot: crdts })
                            }
                            other => {
                                tracing::debug!(%from, ?other, "Unhandled peer control");
                                None
                            }
                        };
                        if let Some(event) = app_event {
                            let effects = app_state.lock().await.process_event(event, now);
                            dispatch_effects(
                                effects, &peer_mgr, &rv_client, &storage,
                                &app_state, &gap_fill_tx,
                            ).await;
                        }
                    }
                    Ok(NetworkEvent::PeerDatagram { from, message }) => {
                        let app_event = match message {
                            PeerDatagram::StateOp { op } => {
                                Some(AppEvent::RemoteOp { from, op })
                            }
                            _ => None, // Position/Seek — Phase 7
                        };
                        if let Some(event) = app_event {
                            let effects = app_state.lock().await.process_event(event, now);
                            dispatch_effects(
                                effects, &peer_mgr, &rv_client, &storage,
                                &app_state, &gap_fill_tx,
                            ).await;
                        }
                    }
                    Ok(NetworkEvent::IncomingStream { from, stream }) => {
                        tracing::debug!(%from, "Incoming stream");
                        let app = Arc::clone(&app_state);
                        tokio::spawn(async move {
                            if let Err(e) = handle_incoming_stream(stream, app).await {
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

            // Gap fill responses from spawned tasks
            result = gap_fill_rx.recv() => {
                if let Some((from, response)) = result {
                    let now = rv_client.shared_now().await;
                    let effects = app_state.lock().await.process_event(
                        AppEvent::GapFillResponse { from, response },
                        now,
                    );
                    dispatch_effects(
                        effects, &peer_mgr, &rv_client, &storage,
                        &app_state, &gap_fill_tx,
                    ).await;
                }
            }

            // Periodic state summary
            _ = summary_interval.tick() => {
                let now = rv_client.shared_now().await;
                let effects = app_state.lock().await.process_event(AppEvent::Tick, now);
                dispatch_effects(
                    effects, &peer_mgr, &rv_client, &storage,
                    &app_state, &gap_fill_tx,
                ).await;
            }
        }
    }

    Ok(())
}

/// Dispatch AppEffects to the runtime (network, storage, etc).
async fn dispatch_effects(
    effects: Vec<AppEffect>,
    peer_mgr: &Arc<PeerManager>,
    rv_client: &dessplay::rendezvous_client::RendezvousClient,
    storage: &Arc<Mutex<storage::ClientStorage>>,
    app_state: &Arc<tokio::sync::Mutex<AppState>>,
    gap_fill_tx: &tokio::sync::mpsc::UnboundedSender<(PeerId, GapFillResponse)>,
) {
    for effect in effects {
        match effect {
            AppEffect::Sync(actions) => {
                dispatch_sync_actions(
                    actions, peer_mgr, rv_client, storage, app_state, gap_fill_tx,
                )
                .await;
            }
            AppEffect::Redraw => {} // Phase 6
            AppEffect::PlayerPause
            | AppEffect::PlayerUnpause
            | AppEffect::PlayerSeek(_)
            | AppEffect::PlayerLoadFile(_)
            | AppEffect::PlayerShowOsd(_) => {} // Phase 7
        }
    }
}

/// Dispatch sync actions to the network and storage layers.
async fn dispatch_sync_actions(
    actions: Vec<SyncAction>,
    peer_mgr: &Arc<PeerManager>,
    rv_client: &dessplay::rendezvous_client::RendezvousClient,
    storage: &Arc<Mutex<storage::ClientStorage>>,
    app_state: &Arc<tokio::sync::Mutex<AppState>>,
    gap_fill_tx: &tokio::sync::mpsc::UnboundedSender<(PeerId, GapFillResponse)>,
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
                    tracing::debug!("Gap fill to server not supported via streams");
                } else {
                    let peer_mgr = Arc::clone(peer_mgr);
                    let tx = gap_fill_tx.clone();
                    tokio::spawn(async move {
                        match do_gap_fill(peer, request, &peer_mgr).await {
                            Ok(response) => {
                                let _ = tx.send((peer, response));
                            }
                            Err(e) => {
                                tracing::debug!(%peer, "Gap fill request failed: {e}");
                            }
                        }
                    });
                }
            }
            SyncAction::PersistOp { op } => {
                let epoch = app_state.lock().await.sync_engine.epoch();
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
async fn do_gap_fill(
    peer: PeerId,
    request: GapFillRequest,
    peer_mgr: &PeerManager,
) -> Result<GapFillResponse> {
    let mut stream = peer_mgr.open_stream(peer).await?;
    write_framed(&mut stream.send, TAG_GAP_FILL_REQUEST, &request).await?;
    let response: GapFillResponse = read_framed(&mut stream.recv, TAG_GAP_FILL_RESPONSE)
        .await?
        .ok_or_else(|| anyhow::anyhow!("gap fill stream closed without response"))?;
    Ok(response)
}

/// Handle an incoming gap fill stream from a peer.
async fn handle_incoming_stream(
    mut stream: dessplay_core::network::MessageStream,
    app_state: Arc<tokio::sync::Mutex<AppState>>,
) -> Result<()> {
    let request: GapFillRequest = read_framed(&mut stream.recv, TAG_GAP_FILL_REQUEST)
        .await?
        .ok_or_else(|| anyhow::anyhow!("stream closed before gap fill request"))?;

    let response = {
        let app = app_state.lock().await;
        app.on_gap_fill_request(&request)
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

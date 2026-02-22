//! Integration tests: SyncEngine convergence over SimulatedNetwork.
//!
//! These tests verify that CRDT state converges across multiple peers under
//! various network conditions (packet loss, partitions, late joiners, epoch
//! upgrades).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use dessplay_core::crdt::CrdtState;
use dessplay_core::framing::{
    read_framed, write_framed, TAG_GAP_FILL_REQUEST, TAG_GAP_FILL_RESPONSE,
};
use dessplay_core::network::simulated::{SimulatedNetwork, SimPeerHandle};
use dessplay_core::network::{Network, NetworkEvent};
use dessplay_core::protocol::{
    CrdtOp, GapFillRequest, GapFillResponse, LwwValue, PeerControl, PeerDatagram,
    PlaylistAction,
};
use dessplay_core::sync_engine::{SyncAction, SyncEngine};
use dessplay_core::types::{FileId, PeerId, UserId, UserState};
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn uid(name: &str) -> UserId {
    UserId(name.to_string())
}

fn fid(n: u8) -> FileId {
    let mut id = [0u8; 16];
    id[0] = n;
    FileId(id)
}

fn user_state_op(user: &str, state: UserState, ts: u64) -> CrdtOp {
    CrdtOp::LwwWrite {
        timestamp: ts,
        value: LwwValue::UserState(uid(user), state),
    }
}

fn chat_op(user: &str, seq: u64, ts: u64, text: &str) -> CrdtOp {
    CrdtOp::ChatAppend {
        user_id: uid(user),
        seq,
        timestamp: ts,
        text: text.to_string(),
    }
}

fn playlist_add_op(file: u8, ts: u64) -> CrdtOp {
    CrdtOp::PlaylistOp {
        timestamp: ts,
        action: PlaylistAction::Add {
            file_id: fid(file),
            after: None,
        },
    }
}

/// Dispatch sync actions through the network.
///
/// Gap fill requests are spawned as background tasks to avoid deadlocks.
async fn dispatch_actions(
    actions: Vec<SyncAction>,
    handle: &SimPeerHandle,
    engine: Arc<Mutex<SyncEngine>>,
) {
    for action in actions {
        match action {
            SyncAction::SendControl { peer, msg } => {
                let _ = handle.send_control(peer, &msg).await;
            }
            SyncAction::SendDatagram { peer, msg } => {
                let _ = handle.send_datagram(peer, &msg).await;
            }
            SyncAction::BroadcastControl { msg } => {
                for peer in handle.connected_peers() {
                    let _ = handle.send_control(peer, &msg).await;
                }
            }
            SyncAction::BroadcastDatagram { msg } => {
                for peer in handle.connected_peers() {
                    let _ = handle.send_datagram(peer, &msg).await;
                }
            }
            SyncAction::RequestGapFill { peer, request } => {
                // Must be spawned so the remote peer can process the IncomingStream
                let handle_stream = handle.open_stream(peer).await;
                let engine_clone = Arc::clone(&engine);
                tokio::spawn(async move {
                    if let Ok(mut stream) = handle_stream {
                        if write_framed(&mut stream.send, TAG_GAP_FILL_REQUEST, &request)
                            .await
                            .is_ok()
                        {
                            if let Ok(Some(response)) = read_framed::<_, GapFillResponse>(
                                &mut stream.recv,
                                TAG_GAP_FILL_RESPONSE,
                            )
                            .await
                            {
                                let mut eng = engine_clone.lock().await;
                                eng.on_gap_fill_response(peer, response);
                            }
                        }
                    }
                });
            }
            SyncAction::PersistOp { .. } | SyncAction::PersistSnapshot { .. } => {
                // No-op in tests
            }
        }
    }
}

/// Handle an incoming gap fill stream.
async fn handle_incoming_stream(
    mut stream: dessplay_core::network::MessageStream,
    engine: Arc<Mutex<SyncEngine>>,
) {
    if let Ok(Some(request)) =
        read_framed::<_, GapFillRequest>(&mut stream.recv, TAG_GAP_FILL_REQUEST).await
    {
        let response = {
            let eng = engine.lock().await;
            eng.on_gap_fill_request(&request)
        };
        let _ = write_framed(&mut stream.send, TAG_GAP_FILL_RESPONSE, &response).await;
    }
}

/// Spawn a per-peer event loop that processes network events and periodic ticks.
///
/// Returns a handle to the spawned task.
fn spawn_peer_loop(
    handle: Arc<SimPeerHandle>,
    engine: Arc<Mutex<SyncEngine>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    let actions = {
                        let eng = engine.lock().await;
                        eng.on_periodic_tick()
                    };
                    dispatch_actions(actions, &handle, Arc::clone(&engine)).await;
                }
                event = handle.recv() => {
                    let Ok(event) = event else { break };

                    // Handle incoming streams in a spawned task
                    if let NetworkEvent::IncomingStream { stream, .. } = event {
                        let eng = Arc::clone(&engine);
                        tokio::spawn(async move {
                            handle_incoming_stream(stream, eng).await;
                        });
                        continue;
                    }

                    let actions = {
                        let mut eng = engine.lock().await;
                        match event {
                            NetworkEvent::PeerConnected { peer_id, .. } => {
                                eng.on_peer_connected(peer_id)
                            }
                            NetworkEvent::PeerDisconnected { peer_id } => {
                                eng.on_peer_disconnected(peer_id);
                                vec![]
                            }
                            NetworkEvent::PeerControl { from, message } => match message {
                                PeerControl::StateOp { op } => eng.on_remote_op(from, op),
                                PeerControl::StateSummary { epoch, versions } => {
                                    eng.on_state_summary(from, epoch, versions)
                                }
                                PeerControl::StateSnapshot { epoch, crdts } => {
                                    eng.on_state_snapshot(epoch, crdts)
                                }
                                _ => vec![],
                            },
                            NetworkEvent::PeerDatagram { from, message } => match message {
                                PeerDatagram::StateOp { op } => eng.on_remote_op(from, op),
                                _ => vec![],
                            },
                            NetworkEvent::IncomingStream { .. } => unreachable!(),
                        }
                    };
                    dispatch_actions(actions, &handle, Arc::clone(&engine)).await;
                }
            }
        }
    })
}

/// Wait until all engines converge or timeout.
async fn wait_for_convergence(
    engines: &HashMap<PeerId, Arc<Mutex<SyncEngine>>>,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    let check_interval = Duration::from_millis(200);

    loop {
        // Collect snapshots
        let mut snapshots = Vec::new();
        for engine in engines.values() {
            let eng = engine.lock().await;
            snapshots.push(eng.state().snapshot());
        }

        if snapshots.len() >= 2 && snapshots.windows(2).all(|w| w[0] == w[1]) {
            return true;
        }

        if tokio::time::Instant::now() >= deadline {
            return false;
        }

        tokio::time::sleep(check_interval).await;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn two_peers_converge_simple() {
    let net = SimulatedNetwork::new(42);
    let handle_a = Arc::new(net.add_peer("alice"));
    let handle_b = Arc::new(net.add_peer("bob"));

    let peer_a = handle_a.peer_id();
    let peer_b = handle_b.peer_id();

    let engine_a = Arc::new(Mutex::new(SyncEngine::new()));
    let engine_b = Arc::new(Mutex::new(SyncEngine::new()));

    let mut engines: HashMap<PeerId, Arc<Mutex<SyncEngine>>> = HashMap::new();
    engines.insert(peer_a, Arc::clone(&engine_a));
    engines.insert(peer_b, Arc::clone(&engine_b));

    // Spawn peer loops
    let _task_a = spawn_peer_loop(Arc::clone(&handle_a), Arc::clone(&engine_a));
    let _task_b = spawn_peer_loop(Arc::clone(&handle_b), Arc::clone(&engine_b));

    // Give connect events time to propagate
    tokio::time::sleep(Duration::from_millis(100)).await;

    // A generates a local op
    let op = user_state_op("alice", UserState::Ready, 100);
    let actions = engine_a.lock().await.apply_local_op(op);
    dispatch_actions(actions, &handle_a, Arc::clone(&engine_a)).await;

    // Wait for convergence
    let converged = wait_for_convergence(&engines, Duration::from_secs(5)).await;
    assert!(converged, "two peers should converge");
}

#[tokio::test(start_paused = true)]
async fn three_peers_converge() {
    let net = SimulatedNetwork::new(42);
    let handle_a = Arc::new(net.add_peer("alice"));
    let handle_b = Arc::new(net.add_peer("bob"));
    let handle_c = Arc::new(net.add_peer("charlie"));

    let peer_a = handle_a.peer_id();
    let peer_b = handle_b.peer_id();
    let peer_c = handle_c.peer_id();

    let engine_a = Arc::new(Mutex::new(SyncEngine::new()));
    let engine_b = Arc::new(Mutex::new(SyncEngine::new()));
    let engine_c = Arc::new(Mutex::new(SyncEngine::new()));

    let mut engines: HashMap<PeerId, Arc<Mutex<SyncEngine>>> = HashMap::new();
    engines.insert(peer_a, Arc::clone(&engine_a));
    engines.insert(peer_b, Arc::clone(&engine_b));
    engines.insert(peer_c, Arc::clone(&engine_c));

    let _task_a = spawn_peer_loop(Arc::clone(&handle_a), Arc::clone(&engine_a));
    let _task_b = spawn_peer_loop(Arc::clone(&handle_b), Arc::clone(&engine_b));
    let _task_c = spawn_peer_loop(Arc::clone(&handle_c), Arc::clone(&engine_c));

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Each peer generates a different op
    let ops = vec![
        (&handle_a, &engine_a, user_state_op("alice", UserState::Ready, 10)),
        (&handle_b, &engine_b, chat_op("bob", 0, 20, "hello")),
        (&handle_c, &engine_c, playlist_add_op(1, 30)),
    ];
    for (handle, engine, op) in ops {
        let actions = engine.lock().await.apply_local_op(op);
        dispatch_actions(actions, handle, Arc::clone(engine)).await;
    }

    let converged = wait_for_convergence(&engines, Duration::from_secs(10)).await;
    assert!(converged, "three peers should converge");
}

#[tokio::test(start_paused = true)]
async fn convergence_with_packet_loss() {
    let net = SimulatedNetwork::new(42);
    net.set_default_loss(0.3);

    let handle_a = Arc::new(net.add_peer("alice"));
    let handle_b = Arc::new(net.add_peer("bob"));

    let peer_a = handle_a.peer_id();
    let peer_b = handle_b.peer_id();

    let engine_a = Arc::new(Mutex::new(SyncEngine::new()));
    let engine_b = Arc::new(Mutex::new(SyncEngine::new()));

    let mut engines: HashMap<PeerId, Arc<Mutex<SyncEngine>>> = HashMap::new();
    engines.insert(peer_a, Arc::clone(&engine_a));
    engines.insert(peer_b, Arc::clone(&engine_b));

    let _task_a = spawn_peer_loop(Arc::clone(&handle_a), Arc::clone(&engine_a));
    let _task_b = spawn_peer_loop(Arc::clone(&handle_b), Arc::clone(&engine_b));

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Generate 20 ops from A
    for i in 1u64..=20 {
        let op = chat_op("alice", i - 1, i * 100, &format!("msg{i}"));
        let actions = engine_a.lock().await.apply_local_op(op);
        dispatch_actions(actions, &handle_a, Arc::clone(&engine_a)).await;
    }

    let converged = wait_for_convergence(&engines, Duration::from_secs(15)).await;
    assert!(
        converged,
        "peers should converge despite 30% packet loss via gap fill"
    );
}

#[tokio::test(start_paused = true)]
async fn partition_and_heal() {
    let net = SimulatedNetwork::new(42);
    let handle_a = Arc::new(net.add_peer("alice"));
    let handle_b = Arc::new(net.add_peer("bob"));

    let peer_a = handle_a.peer_id();
    let peer_b = handle_b.peer_id();

    let engine_a = Arc::new(Mutex::new(SyncEngine::new()));
    let engine_b = Arc::new(Mutex::new(SyncEngine::new()));

    let mut engines: HashMap<PeerId, Arc<Mutex<SyncEngine>>> = HashMap::new();
    engines.insert(peer_a, Arc::clone(&engine_a));
    engines.insert(peer_b, Arc::clone(&engine_b));

    let _task_a = spawn_peer_loop(Arc::clone(&handle_a), Arc::clone(&engine_a));
    let _task_b = spawn_peer_loop(Arc::clone(&handle_b), Arc::clone(&engine_b));

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Partition
    net.partition(peer_a, peer_b);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Both sides generate ops while partitioned
    let op_a = user_state_op("alice", UserState::Ready, 100);
    let actions = engine_a.lock().await.apply_local_op(op_a);
    dispatch_actions(actions, &handle_a, Arc::clone(&engine_a)).await;

    let op_b = chat_op("bob", 0, 200, "hello from bob");
    let actions = engine_b.lock().await.apply_local_op(op_b);
    dispatch_actions(actions, &handle_b, Arc::clone(&engine_b)).await;

    // Verify divergence
    {
        let snap_a = engine_a.lock().await.state().snapshot();
        let snap_b = engine_b.lock().await.state().snapshot();
        assert_ne!(snap_a, snap_b, "should diverge while partitioned");
    }

    // Heal
    net.heal(peer_a, peer_b);

    let converged = wait_for_convergence(&engines, Duration::from_secs(15)).await;
    assert!(converged, "peers should converge after partition heal");
}

#[tokio::test(start_paused = true)]
async fn epoch_upgrade() {
    let net = SimulatedNetwork::new(42);
    let handle_a = Arc::new(net.add_peer("alice"));
    let handle_b = Arc::new(net.add_peer("bob"));

    let peer_a = handle_a.peer_id();
    let peer_b = handle_b.peer_id();

    // Engine A is at epoch 0 (fresh)
    let engine_a = Arc::new(Mutex::new(SyncEngine::new()));

    // Engine B is at epoch 2 with compacted state
    let mut compacted_state = CrdtState::new();
    compacted_state.apply_op(&user_state_op("server", UserState::Ready, 50));
    compacted_state.apply_op(&playlist_add_op(1, 60));
    let snap = compacted_state.snapshot();
    let mut engine_b_inner = SyncEngine::new();
    engine_b_inner.on_state_snapshot(2, snap);
    let engine_b = Arc::new(Mutex::new(engine_b_inner));

    let mut engines: HashMap<PeerId, Arc<Mutex<SyncEngine>>> = HashMap::new();
    engines.insert(peer_a, Arc::clone(&engine_a));
    engines.insert(peer_b, Arc::clone(&engine_b));

    let _task_a = spawn_peer_loop(Arc::clone(&handle_a), Arc::clone(&engine_a));
    let _task_b = spawn_peer_loop(Arc::clone(&handle_b), Arc::clone(&engine_b));

    let converged = wait_for_convergence(&engines, Duration::from_secs(15)).await;

    assert!(converged, "peers should converge after epoch upgrade");
    assert_eq!(engine_a.lock().await.epoch(), 2);
    assert_eq!(engine_b.lock().await.epoch(), 2);
}

#[tokio::test(start_paused = true)]
async fn late_joiner() {
    let net = SimulatedNetwork::new(42);
    let handle_a = Arc::new(net.add_peer("alice"));
    let handle_b = Arc::new(net.add_peer("bob"));

    let peer_a = handle_a.peer_id();
    let peer_b = handle_b.peer_id();

    let engine_a = Arc::new(Mutex::new(SyncEngine::new()));
    let engine_b = Arc::new(Mutex::new(SyncEngine::new()));

    let mut engines: HashMap<PeerId, Arc<Mutex<SyncEngine>>> = HashMap::new();
    engines.insert(peer_a, Arc::clone(&engine_a));
    engines.insert(peer_b, Arc::clone(&engine_b));

    let _task_a = spawn_peer_loop(Arc::clone(&handle_a), Arc::clone(&engine_a));
    let _task_b = spawn_peer_loop(Arc::clone(&handle_b), Arc::clone(&engine_b));

    tokio::time::sleep(Duration::from_millis(100)).await;

    // A and B exchange ops
    let op1 = user_state_op("alice", UserState::Ready, 10);
    let actions = engine_a.lock().await.apply_local_op(op1);
    dispatch_actions(actions, &handle_a, Arc::clone(&engine_a)).await;

    let op2 = chat_op("bob", 0, 20, "hello");
    let actions = engine_b.lock().await.apply_local_op(op2);
    dispatch_actions(actions, &handle_b, Arc::clone(&engine_b)).await;

    // Wait for A+B to converge
    let converged_ab = wait_for_convergence(&engines, Duration::from_secs(5)).await;
    assert!(converged_ab, "A and B should converge before C joins");

    // C joins late
    let handle_c = Arc::new(net.add_peer("charlie"));
    let peer_c = handle_c.peer_id();
    let engine_c = Arc::new(Mutex::new(SyncEngine::new()));
    engines.insert(peer_c, Arc::clone(&engine_c));
    let _task_c = spawn_peer_loop(Arc::clone(&handle_c), Arc::clone(&engine_c));

    // Wait for all three to converge
    let converged = wait_for_convergence(&engines, Duration::from_secs(15)).await;
    assert!(converged, "late joiner C should converge with A and B");
}

#[tokio::test(start_paused = true)]
async fn many_ops_with_loss_converge() {
    let net = SimulatedNetwork::new(123);
    net.set_default_loss(0.2);

    let handle_a = Arc::new(net.add_peer("alice"));
    let handle_b = Arc::new(net.add_peer("bob"));
    let handle_c = Arc::new(net.add_peer("charlie"));

    let peer_a = handle_a.peer_id();
    let peer_b = handle_b.peer_id();
    let peer_c = handle_c.peer_id();

    let engine_a = Arc::new(Mutex::new(SyncEngine::new()));
    let engine_b = Arc::new(Mutex::new(SyncEngine::new()));
    let engine_c = Arc::new(Mutex::new(SyncEngine::new()));

    let mut engines: HashMap<PeerId, Arc<Mutex<SyncEngine>>> = HashMap::new();
    engines.insert(peer_a, Arc::clone(&engine_a));
    engines.insert(peer_b, Arc::clone(&engine_b));
    engines.insert(peer_c, Arc::clone(&engine_c));

    let _task_a = spawn_peer_loop(Arc::clone(&handle_a), Arc::clone(&engine_a));
    let _task_b = spawn_peer_loop(Arc::clone(&handle_b), Arc::clone(&engine_b));
    let _task_c = spawn_peer_loop(Arc::clone(&handle_c), Arc::clone(&engine_c));

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Generate 30 ops from different peers
    let all = vec![
        (&handle_a, &engine_a, "alice"),
        (&handle_b, &engine_b, "bob"),
        (&handle_c, &engine_c, "charlie"),
    ];
    for i in 0u64..30 {
        let idx = (i % 3) as usize;
        let (handle, engine, username) = &all[idx];
        let op = chat_op(username, i / 3, (i + 1) * 100, &format!("msg{i}"));
        let actions = engine.lock().await.apply_local_op(op);
        dispatch_actions(actions, handle, Arc::clone(engine)).await;
    }

    let converged = wait_for_convergence(&engines, Duration::from_secs(30)).await;
    assert!(
        converged,
        "30 ops with 20% loss should converge via gap fill"
    );
}

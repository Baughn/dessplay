mod common;

use std::sync::Arc;

use dessplay::network::clock::ClockSyncService;
use dessplay::network::sync::{LocalEvent, SyncEngine};
use dessplay::network::PeerId;
use dessplay::state::types::*;
use dessplay::state::SharedState;
use tokio::time::Duration;

use common::convergence::assert_converged;
use common::simulated_network::{LinkConfig, SimulatedNetwork};

fn peer(name: &str) -> PeerId {
    PeerId(name.to_string())
}

/// Parameterized test runner: sets up 3 peers, runs a workload under
/// various network conditions, then checks convergence.
async fn run_scenario(
    seed: u64,
    setup: impl FnOnce(&SimulatedNetwork),
    mid_scenario: Option<Box<dyn FnOnce(&SimulatedNetwork) + Send>>,
) {
    let network = SimulatedNetwork::new(seed);
    let names = ["alice", "bob", "charlie"];
    let mut engines: Vec<Arc<SyncEngine>> = Vec::new();

    for name in &names {
        let peer_id = peer(name);
        let conn = Arc::new(network.add_peer(peer_id.clone()));
        let clock_svc = ClockSyncService::new(conn, Duration::from_millis(500));
        clock_svc.start();

        let (shared_state, _rx) = SharedState::new();
        let shared_state = Arc::new(shared_state);
        let engine = SyncEngine::new(clock_svc, peer_id, shared_state);
        engine.start();
        engines.push(engine);
    }

    setup(&network);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Run workload: each peer does some operations
    let tx_a = engines[0].local_event_sender();
    let tx_b = engines[1].local_event_sender();
    let tx_c = engines[2].local_event_sender();

    tx_a.send(LocalEvent::ChatSent { text: "hi from alice".to_string() }).unwrap();
    tx_b.send(LocalEvent::ChatSent { text: "hi from bob".to_string() }).unwrap();
    tx_c.send(LocalEvent::ChatSent { text: "hi from charlie".to_string() }).unwrap();

    tx_a.send(LocalEvent::PlaylistAction(PlaylistAction::Add {
        id: ItemId { user: peer("alice"), seq: 1 },
        filename: "ep01.mkv".to_string(),
        after: None,
    })).unwrap();

    tx_b.send(LocalEvent::PlaylistAction(PlaylistAction::Add {
        id: ItemId { user: peer("bob"), seq: 1 },
        filename: "ep02.mkv".to_string(),
        after: None,
    })).unwrap();

    tx_a.send(LocalEvent::UserStateChanged(UserState::Ready)).unwrap();
    tx_b.send(LocalEvent::UserStateChanged(UserState::Ready)).unwrap();
    tx_c.send(LocalEvent::UserStateChanged(UserState::Ready)).unwrap();

    tx_a.send(LocalEvent::PositionUpdated { position: 30.0 }).unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;

    // Mid-scenario action (e.g., heal a partition)
    if let Some(action) = mid_scenario {
        action(&network);
    }

    // Wait for convergence
    tokio::time::sleep(Duration::from_secs(8)).await;

    assert_converged(&engines, 1.0);
}

// ── Scenario 1: Clean baseline ──────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn scenario_clean_baseline() {
    run_scenario(42, |_| {}, None).await;
}

// ── Scenario 2: Moderate loss ───────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn scenario_moderate_loss() {
    run_scenario(
        42,
        |network| {
            let config = LinkConfig {
                latency_ms: 20,
                loss_rate: 0.05,
                ..Default::default()
            };
            for a in &["alice", "bob", "charlie"] {
                for b in &["alice", "bob", "charlie"] {
                    if a != b {
                        network.set_link_symmetric(&peer(a), &peer(b), config.clone());
                    }
                }
            }
        },
        None,
    )
    .await;
}

// ── Scenario 3: Heavy loss ──────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn scenario_heavy_loss() {
    run_scenario(
        42,
        |network| {
            let config = LinkConfig {
                latency_ms: 20,
                loss_rate: 0.30,
                ..Default::default()
            };
            for a in &["alice", "bob", "charlie"] {
                for b in &["alice", "bob", "charlie"] {
                    if a != b {
                        network.set_link_symmetric(&peer(a), &peer(b), config.clone());
                    }
                }
            }
        },
        None,
    )
    .await;
}

// ── Scenario 4: High latency ───────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn scenario_high_latency() {
    run_scenario(
        42,
        |network| {
            let config = LinkConfig {
                latency_ms: 500,
                ..Default::default()
            };
            for a in &["alice", "bob", "charlie"] {
                for b in &["alice", "bob", "charlie"] {
                    if a != b {
                        network.set_link_symmetric(&peer(a), &peer(b), config.clone());
                    }
                }
            }
        },
        None,
    )
    .await;
}

// ── Scenario 5: Partition A-B (both reach C only) ───────────────────────

#[tokio::test(start_paused = true)]
async fn scenario_partition_a_b() {
    run_scenario(
        42,
        |network| {
            network.partition(&peer("alice"), &peer("bob"));
            network.partition(&peer("bob"), &peer("alice"));
        },
        None,
    )
    .await;
}

// ── Scenario 6: Full partition then heal ────────────────────────────────

#[tokio::test(start_paused = true)]
async fn scenario_full_partition_then_heal() {
    run_scenario(
        42,
        |network| {
            // Full partition
            for a in &["alice", "bob", "charlie"] {
                for b in &["alice", "bob", "charlie"] {
                    if a != b {
                        network.partition(&peer(a), &peer(b));
                    }
                }
            }
        },
        Some(Box::new(|network| {
            // Heal after 2 seconds
            for a in &["alice", "bob", "charlie"] {
                for b in &["alice", "bob", "charlie"] {
                    if a != b {
                        network.heal(&peer(a), &peer(b));
                    }
                }
            }
        })),
    )
    .await;
}

// ── Scenario 7: Asymmetric loss ─────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn scenario_asymmetric_loss() {
    run_scenario(
        42,
        |network| {
            // A→B: 20% loss, B→A: 0% loss
            network.set_link(
                &peer("alice"),
                &peer("bob"),
                LinkConfig {
                    latency_ms: 20,
                    loss_rate: 0.20,
                    ..Default::default()
                },
            );
            network.set_link(
                &peer("bob"),
                &peer("alice"),
                LinkConfig {
                    latency_ms: 20,
                    ..Default::default()
                },
            );
        },
        None,
    )
    .await;
}

// ── Scenario 8: High jitter ────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn scenario_high_jitter() {
    run_scenario(
        42,
        |network| {
            let config = LinkConfig {
                latency_ms: 50,
                jitter_ms: 45,
                ..Default::default()
            };
            for a in &["alice", "bob", "charlie"] {
                for b in &["alice", "bob", "charlie"] {
                    if a != b {
                        network.set_link_symmetric(&peer(a), &peer(b), config.clone());
                    }
                }
            }
        },
        None,
    )
    .await;
}

// ── Scenario 9: Mid-session join ────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn scenario_mid_session_join() {
    let network = SimulatedNetwork::new(42);

    // Start with alice and bob
    let mut engines: Vec<Arc<SyncEngine>> = Vec::new();
    for name in &["alice", "bob"] {
        let peer_id = peer(name);
        let conn = Arc::new(network.add_peer(peer_id.clone()));
        let clock_svc = ClockSyncService::new(conn, Duration::from_millis(500));
        clock_svc.start();
        let (shared_state, _rx) = SharedState::new();
        let shared_state = Arc::new(shared_state);
        let engine = SyncEngine::new(clock_svc, peer_id, shared_state);
        engine.start();
        engines.push(engine);
    }

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Alice adds content
    let tx = engines[0].local_event_sender();
    tx.send(LocalEvent::ChatSent { text: "before charlie".to_string() }).unwrap();
    tx.send(LocalEvent::PlaylistAction(PlaylistAction::Add {
        id: ItemId { user: peer("alice"), seq: 1 },
        filename: "ep01.mkv".to_string(),
        after: None,
    })).unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;

    // Charlie joins mid-session
    let charlie_conn = Arc::new(network.add_peer(peer("charlie")));
    let charlie_clock = ClockSyncService::new(charlie_conn, Duration::from_millis(500));
    charlie_clock.start();
    let (charlie_state, _rx) = SharedState::new();
    let charlie_state = Arc::new(charlie_state);
    let charlie_engine = SyncEngine::new(charlie_clock, peer("charlie"), charlie_state);
    charlie_engine.start();
    engines.push(charlie_engine);

    // Wait for convergence
    tokio::time::sleep(Duration::from_secs(5)).await;

    assert_converged(&engines, 1.0);
}

// ── Scenario 10: Leave and rejoin ───────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn scenario_leave_and_rejoin() {
    let network = SimulatedNetwork::new(42);
    let mut engines: Vec<Arc<SyncEngine>> = Vec::new();

    for name in &["alice", "bob", "charlie"] {
        let peer_id = peer(name);
        let conn = Arc::new(network.add_peer(peer_id.clone()));
        let clock_svc = ClockSyncService::new(conn, Duration::from_millis(500));
        clock_svc.start();
        let (shared_state, _rx) = SharedState::new();
        let shared_state = Arc::new(shared_state);
        let engine = SyncEngine::new(clock_svc, peer_id, shared_state);
        engine.start();
        engines.push(engine);
    }

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Initial state
    let tx = engines[0].local_event_sender();
    tx.send(LocalEvent::ChatSent { text: "hello".to_string() }).unwrap();

    tokio::time::sleep(Duration::from_secs(1)).await;

    // Charlie disconnects
    network.partition(&peer("charlie"), &peer("alice"));
    network.partition(&peer("charlie"), &peer("bob"));
    network.partition(&peer("alice"), &peer("charlie"));
    network.partition(&peer("bob"), &peer("charlie"));

    tokio::time::sleep(Duration::from_secs(1)).await;

    // More activity while charlie is gone
    tx.send(LocalEvent::ChatSent { text: "while charlie away".to_string() }).unwrap();

    tokio::time::sleep(Duration::from_secs(1)).await;

    // Charlie reconnects
    network.heal(&peer("charlie"), &peer("alice"));
    network.heal(&peer("charlie"), &peer("bob"));
    network.heal(&peer("alice"), &peer("charlie"));
    network.heal(&peer("bob"), &peer("charlie"));

    tokio::time::sleep(Duration::from_secs(5)).await;

    assert_converged(&engines, 1.0);
}

// ── Scenario 11: Packet reordering ──────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn scenario_packet_reordering() {
    run_scenario(
        42,
        |network| {
            let config = LinkConfig {
                latency_ms: 20,
                reorder_rate: 0.30,
                ..Default::default()
            };
            for a in &["alice", "bob", "charlie"] {
                for b in &["alice", "bob", "charlie"] {
                    if a != b {
                        network.set_link_symmetric(&peer(a), &peer(b), config.clone());
                    }
                }
            }
        },
        None,
    )
    .await;
}

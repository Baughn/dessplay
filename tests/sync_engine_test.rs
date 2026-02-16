mod common;

use std::sync::Arc;

use dessplay::network::clock::{ClockSyncService, SharedTimestamp};
use dessplay::network::sync::{AppendLog, LocalEvent, LwwRegister, SyncEngine};
use dessplay::network::PeerId;
use dessplay::state::types::*;
use dessplay::state::SharedState;
use tokio::time::Duration;

use common::simulated_network::{LinkConfig, SimulatedNetwork};

fn peer(name: &str) -> PeerId {
    PeerId(name.to_string())
}

fn ts(us: i64) -> SharedTimestamp {
    SharedTimestamp(us)
}

/// Helper: set up N peers on a SimulatedNetwork with SyncEngines.
async fn setup_peers(
    names: &[&str],
    seed: u64,
    link_config: Option<LinkConfig>,
) -> (Vec<Arc<SyncEngine>>, SimulatedNetwork) {
    let network = SimulatedNetwork::new(seed);
    let mut engines = Vec::new();

    for name in names {
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

    if let Some(config) = link_config {
        for i in 0..names.len() {
            for j in 0..names.len() {
                if i != j {
                    network.set_link_symmetric(&peer(names[i]), &peer(names[j]), config.clone());
                }
            }
        }
    }

    (engines, network)
}

// ── LWW Register tests ─────────────────────────────────────────────────

#[test]
fn lww_newer_timestamp_wins() {
    let mut reg = LwwRegister::new(UserState::Ready, ts(100));
    assert!(reg.merge(UserState::Paused, ts(200)));
    assert_eq!(reg.value, UserState::Paused);
    assert_eq!(reg.timestamp, ts(200));
}

#[test]
fn lww_older_timestamp_ignored() {
    let mut reg = LwwRegister::new(UserState::Paused, ts(200));
    assert!(!reg.merge(UserState::Ready, ts(100)));
    assert_eq!(reg.value, UserState::Paused);
}

// ── Append Log tests ────────────────────────────────────────────────────

#[test]
fn append_log_gap_detection() {
    let mut log = AppendLog::new();
    log.append_local(&peer("alice"), vec![1], ts(100));
    log.append_local(&peer("alice"), vec![2], ts(200));

    let mut remote = std::collections::HashMap::new();
    remote.insert(peer("alice"), 5u64);
    remote.insert(peer("bob"), 3u64);

    let gaps = log.find_gaps(&remote);
    assert_eq!(gaps.len(), 2);
}

// ── Gossip forwarding: A→B→C ────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn gossip_forwarding_a_to_b_to_c() {
    // Create 3 peers, partition A from C so they can only communicate via B
    let (engines, network) = setup_peers(
        &["alice", "bob", "charlie"],
        42,
        Some(LinkConfig {
            latency_ms: 5,
            ..Default::default()
        }),
    ).await;

    // Partition A ↔ C (both directions)
    network.partition(&peer("alice"), &peer("charlie"));
    network.partition(&peer("charlie"), &peer("alice"));

    // Give engines time to initialize
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Alice changes her user state
    let tx = engines[0].local_event_sender();
    tx.send(LocalEvent::UserStateChanged(UserState::Paused)).unwrap();

    // Wait for gossip to propagate A→B→C
    // Burst mode sends at 100ms, plus forwarding delay
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Charlie should see Alice's pause state via B's forwarding
    let charlie_view = engines[2].shared_state().view();
    assert_eq!(
        charlie_view.user_states.get(&peer("alice")),
        Some(&UserState::Paused),
        "charlie should see alice's paused state via forwarding through bob"
    );
}

// ── Append log eager push ───────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn append_log_eager_push() {
    let (engines, _network) = setup_peers(
        &["alice", "bob"],
        42,
        Some(LinkConfig {
            latency_ms: 5,
            ..Default::default()
        }),
    ).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Alice sends a chat message
    let tx = engines[0].local_event_sender();
    tx.send(LocalEvent::ChatSent {
        text: "hello!".to_string(),
    })
    .unwrap();

    // Wait for eager push
    tokio::time::sleep(Duration::from_millis(200)).await;

    let bob_view = engines[1].shared_state().view();
    assert_eq!(bob_view.chat_messages.len(), 1);
    assert_eq!(bob_view.chat_messages[0].text, "hello!");
    assert_eq!(bob_view.chat_messages[0].sender, peer("alice"));
}

// ── Gap fill: late joiner ───────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn gap_fill_late_joiner() {
    let network = SimulatedNetwork::new(42);

    // Start with alice and bob
    let alice_conn = Arc::new(network.add_peer(peer("alice")));
    let alice_clock = ClockSyncService::new(alice_conn, Duration::from_millis(500));
    alice_clock.start();
    let (alice_state, _) = SharedState::new();
    let alice_state = Arc::new(alice_state);
    let alice_engine = SyncEngine::new(alice_clock, peer("alice"), alice_state);
    alice_engine.start();

    let bob_conn = Arc::new(network.add_peer(peer("bob")));
    let bob_clock = ClockSyncService::new(bob_conn, Duration::from_millis(500));
    bob_clock.start();
    let (bob_state, _) = SharedState::new();
    let bob_state = Arc::new(bob_state);
    let bob_engine = SyncEngine::new(bob_clock, peer("bob"), bob_state);
    bob_engine.start();

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Alice sends some chat messages
    let tx = alice_engine.local_event_sender();
    for i in 1..=5 {
        tx.send(LocalEvent::ChatSent {
            text: format!("msg-{i}"),
        })
        .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Wait for bob to receive them
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Now charlie joins late
    let charlie_conn = Arc::new(network.add_peer(peer("charlie")));
    let charlie_clock = ClockSyncService::new(charlie_conn, Duration::from_millis(500));
    charlie_clock.start();
    let (charlie_state, _) = SharedState::new();
    let charlie_state = Arc::new(charlie_state);
    let charlie_engine = SyncEngine::new(charlie_clock, peer("charlie"), charlie_state);
    charlie_engine.start();

    // Wait for snapshot + gap fill
    tokio::time::sleep(Duration::from_secs(5)).await;

    let charlie_view = charlie_engine.shared_state().view();
    assert_eq!(
        charlie_view.chat_messages.len(),
        5,
        "late joiner should receive all 5 chat messages via gap fill, got {}",
        charlie_view.chat_messages.len()
    );
}

// ── Gap fill: origin disconnected ───────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn gap_fill_origin_disconnected() {
    let network = SimulatedNetwork::new(42);

    // Start with alice and bob
    let alice_conn = Arc::new(network.add_peer(peer("alice")));
    let alice_clock = ClockSyncService::new(alice_conn, Duration::from_millis(500));
    alice_clock.start();
    let (alice_state, _) = SharedState::new();
    let alice_state = Arc::new(alice_state);
    let alice_engine = SyncEngine::new(alice_clock, peer("alice"), alice_state);
    alice_engine.start();

    let bob_conn = Arc::new(network.add_peer(peer("bob")));
    let bob_clock = ClockSyncService::new(bob_conn, Duration::from_millis(500));
    bob_clock.start();
    let (bob_state, _) = SharedState::new();
    let bob_state = Arc::new(bob_state);
    let bob_engine = SyncEngine::new(bob_clock, peer("bob"), bob_state);
    bob_engine.start();

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Alice sends messages
    let tx = alice_engine.local_event_sender();
    for i in 1..=3 {
        tx.send(LocalEvent::ChatSent {
            text: format!("msg-{i}"),
        })
        .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Wait for bob to receive them
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Partition alice from everyone
    network.partition(&peer("alice"), &peer("bob"));
    network.partition(&peer("bob"), &peer("alice"));

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Charlie joins — alice is disconnected, bob should fill the gap
    let charlie_conn = Arc::new(network.add_peer(peer("charlie")));
    let charlie_clock = ClockSyncService::new(charlie_conn, Duration::from_millis(500));
    charlie_clock.start();
    let (charlie_state, _) = SharedState::new();
    let charlie_state = Arc::new(charlie_state);
    let charlie_engine = SyncEngine::new(charlie_clock, peer("charlie"), charlie_state);
    charlie_engine.start();

    // Wait for snapshot + gap fill from bob
    tokio::time::sleep(Duration::from_secs(5)).await;

    let charlie_view = charlie_engine.shared_state().view();
    assert_eq!(
        charlie_view.chat_messages.len(),
        3,
        "charlie should receive alice's 3 messages via bob's gap fill, got {}",
        charlie_view.chat_messages.len()
    );
}

// ── State vector triggers gap fill ──────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn state_vector_triggers_gap_fill() {
    let (engines, network) = setup_peers(
        &["alice", "bob"],
        42,
        Some(LinkConfig {
            latency_ms: 5,
            ..Default::default()
        }),
    ).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Temporarily partition to create a gap
    network.partition(&peer("alice"), &peer("bob"));
    network.partition(&peer("bob"), &peer("alice"));

    let tx = engines[0].local_event_sender();
    for i in 1..=3 {
        tx.send(LocalEvent::ChatSent {
            text: format!("gap-msg-{i}"),
        })
        .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Heal the partition — snapshot will reveal gaps, triggering gap fill
    network.heal(&peer("alice"), &peer("bob"));
    network.heal(&peer("bob"), &peer("alice"));

    tokio::time::sleep(Duration::from_secs(5)).await;

    let bob_view = engines[1].shared_state().view();
    assert_eq!(
        bob_view.chat_messages.len(),
        3,
        "bob should receive all 3 messages after partition heals, got {}",
        bob_view.chat_messages.len()
    );
}

// ── Broadcast rate tests ────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn broadcast_rate_while_playing() {
    let (engines, _network) = setup_peers(
        &["alice", "bob"],
        42,
        Some(LinkConfig {
            latency_ms: 1,
            ..Default::default()
        }),
    ).await;

    // Both peers ready → playing
    let tx_a = engines[0].local_event_sender();
    let tx_b = engines[1].local_event_sender();
    tx_a.send(LocalEvent::UserStateChanged(UserState::Ready)).unwrap();
    tx_b.send(LocalEvent::UserStateChanged(UserState::Ready)).unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;

    // While playing, position should be synced frequently
    tx_a.send(LocalEvent::PositionUpdated { position: 10.0 }).unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let bob_view = engines[1].shared_state().view();
    assert!(
        (bob_view.position - 10.0).abs() < 1.0,
        "position should sync within 500ms while playing, got {}",
        bob_view.position,
    );
}

#[tokio::test(start_paused = true)]
async fn broadcast_rate_while_paused() {
    let (engines, _network) = setup_peers(
        &["alice", "bob"],
        42,
        Some(LinkConfig {
            latency_ms: 1,
            ..Default::default()
        }),
    ).await;

    // One peer paused → all paused
    let tx_a = engines[0].local_event_sender();
    tx_a.send(LocalEvent::UserStateChanged(UserState::Paused)).unwrap();

    tokio::time::sleep(Duration::from_secs(3)).await;

    // Position update during paused state
    tx_a.send(LocalEvent::PositionUpdated { position: 50.0 }).unwrap();

    // After 2 seconds (enough for >1 paused-rate broadcast), should be synced
    tokio::time::sleep(Duration::from_secs(2)).await;

    let bob_view = engines[1].shared_state().view();
    assert!(
        (bob_view.position - 50.0).abs() < 1.0,
        "position should sync within 2s while paused, got {}",
        bob_view.position,
    );
}

// ── Burst mode ──────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn burst_mode_on_state_change() {
    let (engines, _network) = setup_peers(
        &["alice", "bob"],
        42,
        Some(LinkConfig {
            latency_ms: 1,
            ..Default::default()
        }),
    ).await;

    // Start paused
    let tx_a = engines[0].local_event_sender();
    tx_a.send(LocalEvent::UserStateChanged(UserState::Paused)).unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Now change state — should trigger burst mode (100ms broadcasts)
    tx_a.send(LocalEvent::UserStateChanged(UserState::Ready)).unwrap();

    // Should propagate quickly even though we were in paused broadcast rate
    tokio::time::sleep(Duration::from_millis(500)).await;

    let bob_view = engines[1].shared_state().view();
    assert_eq!(
        bob_view.user_states.get(&peer("alice")),
        Some(&UserState::Ready),
        "state change should propagate quickly via burst mode"
    );
}

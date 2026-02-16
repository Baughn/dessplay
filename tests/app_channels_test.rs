mod common;

use std::sync::Arc;

use dessplay::network::clock::ClockSyncService;
use dessplay::network::sync::{LocalEvent, SyncEngine};
use dessplay::network::PeerId;
use dessplay::state::types::*;
use dessplay::state::SharedState;
use tokio::time::Duration;

use common::simulated_network::{LinkConfig, SimulatedNetwork};

fn peer(name: &str) -> PeerId {
    PeerId(name.to_string())
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

fn default_link() -> Option<LinkConfig> {
    Some(LinkConfig {
        latency_ms: 5,
        ..Default::default()
    })
}

// ── Derived Pause Tests ─────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn derived_pause_all_ready_is_playing() {
    let (engines, _network) = setup_peers(&["alice", "bob"], 42, default_link()).await;

    let tx_a = engines[0].local_event_sender();
    let tx_b = engines[1].local_event_sender();
    tx_a.send(LocalEvent::UserStateChanged(UserState::Ready)).unwrap();
    tx_b.send(LocalEvent::UserStateChanged(UserState::Ready)).unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;

    let view_a = engines[0].shared_state().view();
    let view_b = engines[1].shared_state().view();
    assert!(view_a.is_playing, "all ready → should be playing (alice's view)");
    assert!(view_b.is_playing, "all ready → should be playing (bob's view)");
}

#[tokio::test(start_paused = true)]
async fn derived_pause_one_paused_stops_all() {
    let (engines, _network) = setup_peers(&["alice", "bob"], 42, default_link()).await;

    let tx_a = engines[0].local_event_sender();
    let tx_b = engines[1].local_event_sender();
    tx_a.send(LocalEvent::UserStateChanged(UserState::Ready)).unwrap();
    tx_b.send(LocalEvent::UserStateChanged(UserState::Paused)).unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;

    let view_a = engines[0].shared_state().view();
    assert!(!view_a.is_playing, "one paused → should not be playing");
}

#[tokio::test(start_paused = true)]
async fn derived_pause_not_watching_excluded() {
    let (engines, _network) = setup_peers(&["alice", "bob", "charlie"], 42, default_link()).await;

    let tx_a = engines[0].local_event_sender();
    let tx_b = engines[1].local_event_sender();
    let tx_c = engines[2].local_event_sender();
    tx_a.send(LocalEvent::UserStateChanged(UserState::Ready)).unwrap();
    tx_b.send(LocalEvent::UserStateChanged(UserState::Ready)).unwrap();
    tx_c.send(LocalEvent::UserStateChanged(UserState::NotWatching)).unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;

    let view = engines[0].shared_state().view();
    assert!(view.is_playing, "NotWatching should be excluded from pause check");
}

// ── User State LWW ──────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn user_state_lww_merge_on_reconnection() {
    let (engines, network) = setup_peers(&["alice", "bob"], 42, default_link()).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Partition
    network.partition(&peer("alice"), &peer("bob"));
    network.partition(&peer("bob"), &peer("alice"));

    // Alice changes state while partitioned
    let tx = engines[0].local_event_sender();
    tx.send(LocalEvent::UserStateChanged(UserState::Paused)).unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Heal partition
    network.heal(&peer("alice"), &peer("bob"));
    network.heal(&peer("bob"), &peer("alice"));

    tokio::time::sleep(Duration::from_secs(3)).await;

    let bob_view = engines[1].shared_state().view();
    assert_eq!(
        bob_view.user_states.get(&peer("alice")),
        Some(&UserState::Paused),
        "alice's paused state should merge after reconnection"
    );
}

// ── Playlist Tests ──────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn playlist_concurrent_adds_both_survive() {
    let (engines, _network) = setup_peers(&["alice", "bob"], 42, default_link()).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Both add items concurrently
    let tx_a = engines[0].local_event_sender();
    let tx_b = engines[1].local_event_sender();

    tx_a.send(LocalEvent::PlaylistAction(PlaylistAction::Add {
        id: ItemId { user: peer("alice"), seq: 1 },
        filename: "alice-ep01.mkv".to_string(),
        after: None,
    })).unwrap();

    tx_b.send(LocalEvent::PlaylistAction(PlaylistAction::Add {
        id: ItemId { user: peer("bob"), seq: 1 },
        filename: "bob-ep01.mkv".to_string(),
        after: None,
    })).unwrap();

    tokio::time::sleep(Duration::from_secs(3)).await;

    let view_a = engines[0].shared_state().view();
    let view_b = engines[1].shared_state().view();

    assert_eq!(view_a.playlist.len(), 2, "both adds should survive on alice");
    assert_eq!(view_b.playlist.len(), 2, "both adds should survive on bob");

    // Both should see the same playlist (deterministic ordering via tiebreaker)
    let filenames_a: Vec<&str> = view_a.playlist.iter().map(|i| i.filename.as_str()).collect();
    let filenames_b: Vec<&str> = view_b.playlist.iter().map(|i| i.filename.as_str()).collect();
    assert_eq!(filenames_a, filenames_b, "playlists should be identical across peers");
}

#[tokio::test(start_paused = true)]
async fn playlist_remove_idempotent() {
    let (engines, _network) = setup_peers(&["alice", "bob"], 42, default_link()).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    let tx = engines[0].local_event_sender();
    let item_id = ItemId { user: peer("alice"), seq: 1 };

    tx.send(LocalEvent::PlaylistAction(PlaylistAction::Add {
        id: item_id.clone(),
        filename: "ep01.mkv".to_string(),
        after: None,
    })).unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Remove twice
    tx.send(LocalEvent::PlaylistAction(PlaylistAction::Remove {
        id: item_id.clone(),
    })).unwrap();
    tx.send(LocalEvent::PlaylistAction(PlaylistAction::Remove {
        id: item_id.clone(),
    })).unwrap();

    tokio::time::sleep(Duration::from_secs(3)).await;

    let view = engines[1].shared_state().view();
    assert!(view.playlist.is_empty(), "double remove should result in empty playlist");
}

#[tokio::test(start_paused = true)]
async fn playlist_move_with_stable_ids() {
    let (engines, _network) = setup_peers(&["alice", "bob"], 42, default_link()).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    let tx = engines[0].local_event_sender();

    // Add 3 items
    for i in 1..=3 {
        let after = if i == 1 {
            None
        } else {
            Some(ItemId { user: peer("alice"), seq: i - 1 })
        };
        tx.send(LocalEvent::PlaylistAction(PlaylistAction::Add {
            id: ItemId { user: peer("alice"), seq: i },
            filename: format!("ep{i:02}.mkv"),
            after,
        })).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Move ep03 to beginning
    tx.send(LocalEvent::PlaylistAction(PlaylistAction::Move {
        id: ItemId { user: peer("alice"), seq: 3 },
        after: None,
    })).unwrap();

    tokio::time::sleep(Duration::from_secs(3)).await;

    let view = engines[1].shared_state().view();
    assert_eq!(view.playlist.len(), 3);
    assert_eq!(view.playlist[0].filename, "ep03.mkv", "ep03 should be first after move");
    assert_eq!(view.playlist[1].filename, "ep01.mkv");
    assert_eq!(view.playlist[2].filename, "ep02.mkv");
}

// ── Chat Tests ──────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn chat_sorted_by_timestamp() {
    let (engines, _network) = setup_peers(&["alice", "bob"], 42, default_link()).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    let tx_a = engines[0].local_event_sender();
    let tx_b = engines[1].local_event_sender();

    tx_a.send(LocalEvent::ChatSent { text: "alice first".to_string() }).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    tx_b.send(LocalEvent::ChatSent { text: "bob second".to_string() }).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    tx_a.send(LocalEvent::ChatSent { text: "alice third".to_string() }).unwrap();

    tokio::time::sleep(Duration::from_secs(3)).await;

    let view = engines[0].shared_state().view();
    assert_eq!(view.chat_messages.len(), 3);
    assert_eq!(view.chat_messages[0].text, "alice first");
    assert_eq!(view.chat_messages[1].text, "bob second");
    assert_eq!(view.chat_messages[2].text, "alice third");
}

#[tokio::test(start_paused = true)]
async fn chat_gap_fill_recovers_missed_messages() {
    let (engines, network) = setup_peers(&["alice", "bob"], 42, default_link()).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Partition bob
    network.partition(&peer("alice"), &peer("bob"));
    network.partition(&peer("bob"), &peer("alice"));

    // Alice sends messages while partitioned
    let tx = engines[0].local_event_sender();
    for i in 1..=5 {
        tx.send(LocalEvent::ChatSent { text: format!("msg-{i}") }).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Heal partition
    network.heal(&peer("alice"), &peer("bob"));
    network.heal(&peer("bob"), &peer("alice"));

    // Wait for gap fill
    tokio::time::sleep(Duration::from_secs(5)).await;

    let bob_view = engines[1].shared_state().view();
    assert_eq!(
        bob_view.chat_messages.len(),
        5,
        "bob should recover all 5 messages via gap fill, got {}",
        bob_view.chat_messages.len()
    );
}

// ── Late Join ───────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn late_join_full_state() {
    let network = SimulatedNetwork::new(42);

    // Start with alice
    let alice_conn = Arc::new(network.add_peer(peer("alice")));
    let alice_clock = ClockSyncService::new(alice_conn, Duration::from_millis(500));
    alice_clock.start();
    let (alice_state, _) = SharedState::new();
    let alice_state = Arc::new(alice_state);
    let alice_engine = SyncEngine::new(alice_clock, peer("alice"), alice_state);
    alice_engine.start();

    tokio::time::sleep(Duration::from_millis(100)).await;

    let tx = alice_engine.local_event_sender();

    // Set up various state (sleeps between events ensure unique timestamps under paused time)
    tx.send(LocalEvent::UserStateChanged(UserState::Ready)).unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    tx.send(LocalEvent::PlaylistAction(PlaylistAction::Add {
        id: ItemId { user: peer("alice"), seq: 1 },
        filename: "ep01.mkv".to_string(),
        after: None,
    })).unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    tx.send(LocalEvent::ChatSent { text: "hello world".to_string() }).unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    tx.send(LocalEvent::FileChanged {
        file_id: Some(ItemId { user: peer("alice"), seq: 1 }),
    }).unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    tx.send(LocalEvent::PositionUpdated { position: 42.5 }).unwrap();

    tokio::time::sleep(Duration::from_secs(1)).await;

    // Bob joins late
    let bob_conn = Arc::new(network.add_peer(peer("bob")));
    let bob_clock = ClockSyncService::new(bob_conn, Duration::from_millis(500));
    bob_clock.start();
    let (bob_state, _) = SharedState::new();
    let bob_state = Arc::new(bob_state);
    let bob_engine = SyncEngine::new(bob_clock, peer("bob"), bob_state);
    bob_engine.start();

    // Wait for full state transfer
    tokio::time::sleep(Duration::from_secs(5)).await;

    let bob_view = bob_engine.shared_state().view();

    assert_eq!(
        bob_view.user_states.get(&peer("alice")),
        Some(&UserState::Ready),
        "bob should see alice's user state"
    );
    assert_eq!(bob_view.playlist.len(), 1, "bob should have alice's playlist item");
    assert_eq!(bob_view.chat_messages.len(), 1, "bob should have alice's chat message");
    assert_eq!(
        bob_view.current_file,
        Some(ItemId { user: peer("alice"), seq: 1 }),
        "bob should see alice's current file"
    );
}

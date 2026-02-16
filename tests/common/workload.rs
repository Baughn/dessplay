use std::sync::Arc;

use rand::Rng;
use rand::rngs::StdRng;
use tokio::time::Duration;

use dessplay::network::PeerId;
use dessplay::network::sync::{LocalEvent, SyncEngine};
use dessplay::state::types::*;

/// Run a random workload on a sync engine for the given duration.
///
/// Generates random chat messages, playlist actions, user state changes,
/// and position updates at random intervals.
pub async fn run_random_workload(engine: &Arc<SyncEngine>, rng: &mut StdRng, duration: Duration) {
    let peer_id = engine.shared_state().view().peers[0].clone(); // our peer
    let tx = engine.local_event_sender();
    let start = tokio::time::Instant::now();
    let mut playlist_seq = 0u64;

    while tokio::time::Instant::now() - start < duration {
        let action: u8 = rng.random_range(0..5);
        match action {
            0 => {
                // Chat message
                let msg_num: u32 = rng.random_range(1..1000);
                let _ = tx.send(LocalEvent::ChatSent {
                    text: format!("msg-{msg_num}"),
                });
            }
            1 => {
                // Playlist add
                playlist_seq += 1;
                let _ = tx.send(LocalEvent::PlaylistAction(PlaylistAction::Add {
                    id: ItemId {
                        user: peer_id.clone(),
                        seq: playlist_seq,
                    },
                    filename: format!("file-{playlist_seq}.mkv"),
                    after: None,
                }));
            }
            2 => {
                // User state change
                let states = [UserState::Ready, UserState::Paused, UserState::NotWatching];
                let state = states[rng.random_range(0..states.len())];
                let _ = tx.send(LocalEvent::UserStateChanged(state));
            }
            3 => {
                // Position update
                let pos: f64 = rng.random_range(0.0..3600.0);
                let _ = tx.send(LocalEvent::PositionUpdated { position: pos });
            }
            4 => {
                // File state change
                let _ = tx.send(LocalEvent::FileStateChanged(FileState::Ready));
            }
            _ => unreachable!(),
        }

        // Random delay between actions: 10-200ms
        let delay_ms: u64 = rng.random_range(10..200);
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
}

/// Create a standard 3-peer test setup with sync engines.
///
/// Returns (engines, network) where engines are [alice, bob, charlie].
pub async fn setup_three_peers(
    seed: u64,
) -> (
    Vec<Arc<SyncEngine>>,
    super::simulated_network::SimulatedNetwork,
) {
    use dessplay::network::clock::ClockSyncService;
    use super::simulated_network::SimulatedNetwork;

    let network = SimulatedNetwork::new(seed);

    let names = ["alice", "bob", "charlie"];
    let mut engines = Vec::new();

    for name in &names {
        let peer_id = PeerId(name.to_string());
        let conn = Arc::new(network.add_peer(peer_id.clone()));
        let clock_svc = ClockSyncService::new(conn, Duration::from_millis(500));
        clock_svc.start();

        let (shared_state, _rx) = dessplay::state::SharedState::new();
        let shared_state = Arc::new(shared_state);
        let engine = SyncEngine::new(clock_svc, peer_id, shared_state);
        engine.start();
        engines.push(engine);
    }

    (engines, network)
}

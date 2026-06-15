//! Phase 5: server compaction — view preservation, epoch bumps,
//! snapshot adoption by connected clients, stale-epoch reconnects, and
//! post-compaction convergence (the session-actor clock collapse).

mod common;

use std::time::Duration;

use common::*;
use dessplay::actors::sync::Mutation;
use dessplay_core::types::{Epoch, PlaybackIntent};
use dessplay_rendezvous::server::{CompactionSchedule, ServerConfig};

fn compacting_config(period: Duration, chat_keep: usize) -> ServerConfig {
    let mut config = ServerConfig::new(PASSWORD);
    config.compaction = CompactionSchedule::Every(period);
    config.chat_keep = chat_keep;
    config
}

/// Connected clients adopt the compaction snapshot: epoch bumps, the
/// resolved view is preserved (minus the trimmed chat and the cleared
/// lookup set), tombstones are gone, and — critically — clients can
/// keep editing afterwards and still converge (their session-actor
/// clocks were collapsed into the server actor by the rebuild).
#[tokio::test(start_paused = true)]
async fn compaction_preserves_view_and_clients_keep_working() {
    let harness = Harness::with_config(0x5EED, compacting_config(Duration::from_secs(300), 3));
    let kim = harness.client("kim", 1);
    let baughn = harness.client("baughn", 2);

    // Build up state: three entries, one removed (tombstone), five chat
    // messages (keep = 3), a lookup request, intent Playing.
    for i in 1..=3 {
        mutate(&kim, Mutation::PushPlaylist { new: entry(i) }).await;
    }
    mutate(&kim, Mutation::RemovePlaylist { hash: hash(2) }).await;
    for i in 0..5 {
        mutate(
            &baughn,
            Mutation::Chat {
                text: format!("m{i}"),
            },
        )
        .await;
    }
    mutate(
        &baughn,
        Mutation::RequestLookup {
            info: dessplay_core::types::FileHashInfo {
                hash: hash(1),
                size: 1_000_000,
                filename: "ep1.mkv".into(),
                mtime: None,
                series_hint: None,
            },
        },
    )
    .await;
    mutate(
        &kim,
        Mutation::SetNowPlaying {
            file: Some(hash(1)),
        },
    )
    .await;
    mutate(
        &kim,
        Mutation::SetPlaybackIntent {
            intent: PlaybackIntent::Playing,
        },
    )
    .await;

    let before = eventually_views(&[&kim, &baughn], Duration::from_secs(30), |views| {
        views
            .iter()
            .all(|v| v.playlist.len() == 2 && v.chat.len() == 5 && !v.lookup_requests.is_empty())
            && views[0] == views[1]
    })
    .await;

    // Cross the compaction tick (300s period).
    tokio::time::sleep(Duration::from_secs(310)).await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |_| true).await;
    assert_eq!(epoch_of(&kim).await, Epoch(2));
    assert_eq!(epoch_of(&baughn).await, Epoch(2));

    let after = eventually_views(&[&kim, &baughn], Duration::from_secs(30), |views| {
        views[0] == views[1]
    })
    .await;
    // Preserved: playlist content & order (positions rebalanced, so
    // compare hashes), registers, the lot.
    let hashes =
        |v: &dessplay_core::StateView| v.playlist.iter().map(|e| e.hash).collect::<Vec<_>>();
    assert_eq!(hashes(&after[0]), hashes(&before[0]));
    assert_eq!(after[0].now_playing, before[0].now_playing);
    assert_eq!(after[0].playback_intent, PlaybackIntent::Playing);
    assert_eq!(after[0].watched, before[0].watched);
    // Trimmed/cleared:
    let texts: Vec<&str> = after[0].chat.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(texts, ["m2", "m3", "m4"], "chat should keep the tail");
    assert!(after[0].lookup_requests.is_empty());

    // Post-compaction edits from both clients still flow and converge.
    mutate(&kim, Mutation::PushPlaylist { new: entry(4) }).await;
    mutate(
        &baughn,
        Mutation::Chat {
            text: "post-compaction".into(),
        },
    )
    .await;
    eventually_views(&[&kim, &baughn], Duration::from_secs(30), |views| {
        views.iter().all(|v| {
            v.playlist.len() == 3 && v.chat.last().is_some_and(|m| m.text == "post-compaction")
        }) && views[0] == views[1]
    })
    .await;
}

/// A client that misses a compaction reconnects with a stale epoch,
/// gets a snapshot, re-applies its offline edits on top, and pushes
/// them up — nothing is lost, everyone converges on the new epoch.
#[tokio::test(start_paused = true)]
async fn stale_epoch_reconnect_recovers_offline_edits() {
    let harness = Harness::with_config(0x5EED, compacting_config(Duration::from_secs(300), 100));
    let kim = harness.client("kim", 1);
    let baughn = harness.client("baughn", 2);

    mutate(
        &kim,
        Mutation::Chat {
            text: "before".into(),
        },
    )
    .await;
    eventually_views(&[&kim, &baughn], Duration::from_secs(30), |views| {
        views.iter().all(|v| v.chat.len() == 1)
    })
    .await;

    // Kim drops off and edits offline.
    harness.isolate("kim");
    tokio::time::sleep(Duration::from_millis(500)).await;
    mutate(
        &kim,
        Mutation::Chat {
            text: "offline".into(),
        },
    )
    .await;
    mutate(&kim, Mutation::PushPlaylist { new: entry(9) }).await;

    // The server compacts while kim is away (baughn adopts live).
    tokio::time::sleep(Duration::from_secs(310)).await;
    eventually(&[&baughn], Duration::from_secs(30), |_| true).await;
    assert_eq!(epoch_of(&baughn).await, Epoch(2));

    // Kim returns with epoch 1: snapshot, replay, upward merge.
    harness.heal("kim");
    eventually(&[&kim, &baughn], Duration::from_secs(60), |snaps| {
        snaps
            .iter()
            .all(|s| s.view.chat.iter().any(|m| m.text == "offline") && s.view.playlist.len() == 1)
            && snaps[0].view == snaps[1].view
    })
    .await;
    assert_eq!(epoch_of(&kim).await, Epoch(2));
}

/// Chaos with compaction in the mix: random mutations, link damage and
/// connection kills while the server compacts every 20 simulated
/// seconds. In-flight ops may legitimately die at a compaction edge
/// (the epoch guard drops them, by design — the daily noon schedule
/// makes the window irrelevant in production), so the assertion is the
/// milestone property: after quiescing, every view is identical.
#[tokio::test(start_paused = true)]
async fn chaos_with_compaction_converges() {
    use dessplay_core::net::sim::{EndpointId, LinkConfig};
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    for seed in [3u64, 99, 2024] {
        let mut rng = StdRng::seed_from_u64(seed);
        let harness = Harness::with_config(seed, compacting_config(Duration::from_secs(20), 100));
        let names = ["kim", "baughn", "dagger"];
        let clients: Vec<_> = names
            .iter()
            .enumerate()
            .map(|(i, name)| harness.client(name, i as u128 + 1))
            .collect();
        let server_id = harness.server_id.clone();

        for round in 0..40u32 {
            let who = rng.random_range(0..clients.len());
            match rng.random_range(0..10u32) {
                0..=4 => {
                    mutate(
                        &clients[who],
                        Mutation::Chat {
                            text: format!("s{seed} r{round}"),
                        },
                    )
                    .await;
                }
                5..=6 => {
                    mutate(
                        &clients[who],
                        Mutation::PushPlaylist {
                            new: entry(rng.random_range(0..30)),
                        },
                    )
                    .await;
                }
                7 => {
                    mutate(
                        &clients[who],
                        Mutation::SetPlaybackPosition {
                            position_millis: round as u64 * 100,
                        },
                    )
                    .await;
                }
                8 => {
                    harness.net.set_link(
                        &EndpointId::new(names[who]),
                        &server_id,
                        LinkConfig {
                            latency: Duration::from_millis(rng.random_range(0..80)),
                            datagram_loss: rng.random_range(0.0..0.8),
                            datagram_jitter: Duration::from_millis(rng.random_range(0..60)),
                            ..LinkConfig::default()
                        },
                    );
                }
                _ => {
                    harness
                        .net
                        .disconnect(&EndpointId::new(names[who]), &server_id);
                }
            }
            tokio::time::sleep(Duration::from_millis(rng.random_range(10..400))).await;
        }

        // Quiesce across at least one compaction: identical views,
        // identical epochs, and proof a compaction actually ran.
        let refs: Vec<&dessplay::client::ClientHandle> = clients.iter().collect();
        eventually(&refs, Duration::from_secs(180), |snaps| {
            snaps.iter().all(|s| s.epoch >= Epoch(2))
                && snaps
                    .windows(2)
                    .all(|w| w[0].view == w[1].view && w[0].epoch == w[1].epoch)
        })
        .await;
    }
}

//! Phase 4 milestone tests: full sync clients (network + sync actors)
//! against the real server over the simulated transport. Paused time
//! throughout — simulated minutes are free.

mod common;

use std::time::Duration;

use common::*;
use dessplay::actors::sync::Mutation;
use dessplay_core::net::sim::{EndpointId, LinkConfig};
use dessplay_core::test_support;
use dessplay_core::types::UserId;

/// The Phase 4 milestone: multiple clients modify CRDTs through the
/// server and converge to identical state.
#[tokio::test(start_paused = true)]
async fn clients_converge_through_the_server() {
    let harness = Harness::new(0x5EED);
    let kim = harness.client("kim", 1);
    let baughn = harness.client("baughn", 2);

    mutate(&kim, Mutation::Chat { text: "hi".into() }).await;
    mutate(&kim, Mutation::PushPlaylist { new: entry(1) }).await;
    mutate(&baughn, Mutation::Chat { text: "yo".into() }).await;
    mutate(&baughn, Mutation::PushPlaylist { new: entry(2) }).await;
    mutate(
        &baughn,
        Mutation::SetNowPlaying {
            file: Some(hash(1)),
        },
    )
    .await;

    let views = eventually_views(&[&kim, &baughn], Duration::from_secs(30), |views| {
        views
            .iter()
            .all(|v| v.chat.len() == 2 && v.playlist.len() == 2 && v.now_playing == Some(hash(1)))
            && views[0] == views[1]
    })
    .await;
    // Chat ordering is identical, not merely same-length.
    assert_eq!(views[0].chat, views[1].chat);
}

/// Heavy datagram loss and jitter: the reliable path carries everything.
#[tokio::test(start_paused = true)]
async fn convergence_survives_datagram_loss() {
    let harness = Harness::new(0x5EED);
    for name in ["kim", "baughn", "dagger"] {
        harness.net.set_link(
            &EndpointId::new(name),
            &harness.server_id,
            LinkConfig {
                latency: Duration::from_millis(30),
                datagram_loss: 0.4,
                datagram_jitter: Duration::from_millis(50),
                ..LinkConfig::default()
            },
        );
    }
    let kim = harness.client("kim", 1);
    let baughn = harness.client("baughn", 2);
    let dagger = harness.client("dagger", 3);
    let clients = [&kim, &baughn, &dagger];

    for (i, c) in clients.iter().enumerate() {
        for round in 0..5u8 {
            mutate(
                c,
                Mutation::Chat {
                    text: format!("c{i} r{round}"),
                },
            )
            .await;
            mutate(
                c,
                Mutation::PushPlaylist {
                    new: entry(i as u8 * 5 + round),
                },
            )
            .await;
        }
    }

    eventually_views(&clients, Duration::from_secs(60), |views| {
        views
            .iter()
            .all(|v| v.chat.len() == 15 && v.playlist.len() == 15)
            && views.windows(2).all(|w| w[0] == w[1])
    })
    .await;
}

/// Total datagram loss: playback positions still propagate via the 1s
/// reliable tick.
#[tokio::test(start_paused = true)]
async fn positions_survive_total_datagram_loss() {
    let harness = Harness::new(0x5EED);
    harness.net.set_default_link(LinkConfig {
        datagram_loss: 1.0,
        ..LinkConfig::default()
    });
    let kim = harness.client("kim", 1);
    let baughn = harness.client("baughn", 2);

    // 3 simulated seconds of 100ms position updates from kim.
    for i in 0..30u64 {
        mutate(
            &kim,
            Mutation::SetPlaybackPosition {
                position_millis: i * 100,
            },
        )
        .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    eventually_views(&[&baughn], Duration::from_secs(10), |views| {
        views[0]
            .playback_position
            .get(&UserId::new("kim"))
            .is_some_and(|p| p.position_millis >= 2_000)
    })
    .await;
}

/// Kill the connection mid-session: edits made while down are buffered,
/// the client reconnects (same epoch -> StateMerge), replays its buffer,
/// and everyone converges.
#[tokio::test(start_paused = true)]
async fn reconnect_merges_and_replays_offline_edits() {
    let harness = Harness::new(0x5EED);
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

    // Cut kim's connection. The network actor notices and retries on a
    // 2s backoff; give the Disconnected event a moment to land, then
    // edit while down.
    harness
        .net
        .disconnect(&EndpointId::new("kim"), &harness.server_id);
    tokio::time::sleep(Duration::from_millis(500)).await;
    mutate(
        &kim,
        Mutation::Chat {
            text: "offline-1".into(),
        },
    )
    .await;
    mutate(&kim, Mutation::PushPlaylist { new: entry(9) }).await;
    // Meanwhile baughn keeps editing.
    mutate(
        &baughn,
        Mutation::Chat {
            text: "while-away".into(),
        },
    )
    .await;

    // Reconnect happens automatically; everything converges.
    eventually_views(&[&kim, &baughn], Duration::from_secs(60), |views| {
        views
            .iter()
            .all(|v| v.chat.len() == 3 && v.playlist.len() == 1)
            && views[0] == views[1]
    })
    .await;
}

/// No spurious divergence alarms over a long quiet stretch with hashes
/// flowing (and none while actively editing either).
#[tokio::test(start_paused = true)]
async fn no_false_divergence_alarms() {
    let harness = Harness::new(0x5EED);
    let mut kim = harness.client("kim", 1);

    mutate(
        &kim,
        Mutation::Chat {
            text: "hello".into(),
        },
    )
    .await;
    // 3 simulated minutes: at least 6 StateHash rounds.
    tokio::time::sleep(Duration::from_secs(180)).await;

    let mut diverged = false;
    while let Ok(event) = kim.events.try_recv() {
        if matches!(
            event,
            dessplay::client::ClientEvent::Sync(dessplay::actors::sync::SyncEvent::Diverged)
        ) {
            diverged = true;
        }
    }
    assert!(!diverged, "spurious divergence alarm");
    // Sanity: state intact.
    assert_eq!(view_of(&kim).await.chat.len(), 1);
}

/// Randomized chaos: scripted mutations from three clients while the
/// network degrades and connections die, then quiesce — all views equal.
/// (Compaction chaos lives in tests/compaction.rs.)
#[tokio::test(start_paused = true)]
async fn chaos_converges_after_quiesce() {
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    for seed in [1u64, 7, 42, 1337] {
        let mut rng = StdRng::seed_from_u64(seed);
        let harness = Harness::new(seed);
        let names = ["kim", "baughn", "dagger"];
        let clients: Vec<_> = names
            .iter()
            .enumerate()
            .map(|(i, name)| harness.client(name, i as u128 + 1))
            .collect();

        let mut sent_chats = 0usize;
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
                    sent_chats += 1;
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
                        &harness.server_id,
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
                        .disconnect(&EndpointId::new(names[who]), &harness.server_id);
                }
            }
            tokio::time::sleep(Duration::from_millis(rng.random_range(10..400))).await;
        }

        // Quiesce: ample time for reconnects (2s backoff) and replay.
        let refs: Vec<&dessplay::client::ClientHandle> = clients.iter().collect();
        eventually_views(&refs, Duration::from_secs(120), |views| {
            views.iter().all(|v| v.chat.len() == sent_chats)
                && views.windows(2).all(|w| w[0] == w[1])
        })
        .await;

        // Use test_support's hash domain sanity: nothing panicked, all
        // views identical — the milestone property under chaos.
        let _ = test_support::file(0);
    }
}

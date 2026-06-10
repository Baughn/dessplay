//! Phase 4 milestone tests: full sync clients (network + sync actors)
//! against the real server over the simulated transport. Paused time
//! throughout — simulated minutes are free.

use std::sync::Arc;
use std::time::Duration;

use dessplay::actors::sync::{Mutation, SyncCommand};
use dessplay::client::{ClientConfig, ClientHandle, SyncConfigExtras, spawn_client};
use dessplay_core::net::Role;
use dessplay_core::net::sim::{EndpointId, LinkConfig, SimNetwork};
use dessplay_core::playlist::NewPlaylistEntry;
use dessplay_core::types::{Ed2kHash, UserId};
use dessplay_core::{StateView, test_support};
use dessplay_rendezvous::server::{self, ServerConfig};
use tokio::sync::oneshot;

const PASSWORD: &str = "hunter2";

fn sim_clock(skew_millis: i64) -> Arc<dyn Fn() -> u64 + Send + Sync> {
    let origin = tokio::time::Instant::now();
    Arc::new(move || {
        let elapsed = tokio::time::Instant::now().duration_since(origin);
        (1_700_000_000_000_i64 + elapsed.as_millis() as i64 + skew_millis) as u64
    })
}

fn setup() -> (SimNetwork, EndpointId) {
    let net = SimNetwork::new(0x5EED);
    let server_id = EndpointId::new("server");
    let listener = net.listener(&server_id);
    tokio::spawn(server::run(
        listener,
        ServerConfig::new(PASSWORD),
        sim_clock(0),
        None,
    ));
    (net, server_id)
}

fn client(net: &SimNetwork, server_id: &EndpointId, name: &str, nonce: u128) -> ClientHandle {
    let connector = Arc::new(net.connector(&EndpointId::new(name), server_id));
    spawn_client(
        connector,
        ClientConfig {
            username: UserId::new(name),
            password: PASSWORD.into(),
            role: Role::Interactive,
            session_nonce: nonce,
            clock: sim_clock(0),
            sync: SyncConfigExtras::default(),
        },
    )
}

async fn view_of(handle: &ClientHandle) -> StateView {
    let (tx, rx) = oneshot::channel();
    handle.sync.send(SyncCommand::GetView(tx)).await.unwrap();
    rx.await.unwrap()
}

async fn mutate(handle: &ClientHandle, mutation: Mutation) {
    handle
        .sync
        .send(SyncCommand::Mutate(Box::new(mutation)))
        .await
        .unwrap();
}

/// Wait (in simulated time) until `pred` holds over all client views.
async fn eventually<F: FnMut(&[StateView]) -> bool>(
    clients: &[&ClientHandle],
    budget: Duration,
    mut pred: F,
) -> Vec<StateView> {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let mut views = Vec::new();
        for client in clients {
            views.push(view_of(client).await);
        }
        if pred(&views) {
            return views;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("condition not reached; final views: {views:#?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn entry(i: u8) -> NewPlaylistEntry {
    NewPlaylistEntry {
        hash: Ed2kHash([i; 16]),
        added_by: UserId::new("whoever"),
        filename: format!("ep{i}.mkv"),
        size_bytes: 1_000_000,
        duration_millis: Some(1_440_000),
    }
}

/// The Phase 4 milestone: multiple clients modify CRDTs through the
/// server and converge to identical state.
#[tokio::test(start_paused = true)]
async fn clients_converge_through_the_server() {
    let (net, server_id) = setup();
    let kim = client(&net, &server_id, "kim", 1);
    let baughn = client(&net, &server_id, "baughn", 2);

    mutate(&kim, Mutation::Chat { text: "hi".into() }).await;
    mutate(&kim, Mutation::PushPlaylist { new: entry(1) }).await;
    mutate(&baughn, Mutation::Chat { text: "yo".into() }).await;
    mutate(&baughn, Mutation::PushPlaylist { new: entry(2) }).await;
    mutate(
        &baughn,
        Mutation::SetNowPlaying {
            file: Some(Ed2kHash([1; 16])),
        },
    )
    .await;

    let views = eventually(&[&kim, &baughn], Duration::from_secs(30), |views| {
        views.iter().all(|v| {
            v.chat.len() == 2 && v.playlist.len() == 2 && v.now_playing == Some(Ed2kHash([1; 16]))
        }) && views[0] == views[1]
    })
    .await;
    // Chat ordering is identical, not merely same-length.
    assert_eq!(views[0].chat, views[1].chat);
}

/// Heavy datagram loss and jitter: the reliable path carries everything.
#[tokio::test(start_paused = true)]
async fn convergence_survives_datagram_loss() {
    let (net, server_id) = setup();
    for name in ["kim", "baughn", "dagger"] {
        net.set_link(
            &EndpointId::new(name),
            &server_id,
            LinkConfig {
                latency: Duration::from_millis(30),
                datagram_loss: 0.4,
                datagram_jitter: Duration::from_millis(50),
                ..LinkConfig::default()
            },
        );
    }
    let kim = client(&net, &server_id, "kim", 1);
    let baughn = client(&net, &server_id, "baughn", 2);
    let dagger = client(&net, &server_id, "dagger", 3);
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

    eventually(&clients, Duration::from_secs(60), |views| {
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
    let (net, server_id) = setup();
    net.set_default_link(LinkConfig {
        datagram_loss: 1.0,
        ..LinkConfig::default()
    });
    let kim = client(&net, &server_id, "kim", 1);
    let baughn = client(&net, &server_id, "baughn", 2);

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

    eventually(&[&baughn], Duration::from_secs(10), |views| {
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
    let (net, server_id) = setup();
    let kim_id = EndpointId::new("kim");
    let kim = client(&net, &server_id, "kim", 1);
    let baughn = client(&net, &server_id, "baughn", 2);

    mutate(
        &kim,
        Mutation::Chat {
            text: "before".into(),
        },
    )
    .await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |views| {
        views.iter().all(|v| v.chat.len() == 1)
    })
    .await;

    // Cut kim's connection. The network actor notices and retries on a
    // 2s backoff; give the Disconnected event a moment to land, then
    // edit while down.
    net.disconnect(&kim_id, &server_id);
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
    eventually(&[&kim, &baughn], Duration::from_secs(60), |views| {
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
    let (net, server_id) = setup();
    let kim = client(&net, &server_id, "kim", 1);
    let mut kim = kim;

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
#[tokio::test(start_paused = true)]
async fn chaos_converges_after_quiesce() {
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    for seed in [1u64, 7, 42, 1337] {
        let mut rng = StdRng::seed_from_u64(seed);
        let net = SimNetwork::new(seed);
        let server_id = EndpointId::new("server");
        let listener = net.listener(&server_id);
        tokio::spawn(server::run(
            listener,
            ServerConfig::new(PASSWORD),
            sim_clock(0),
            None,
        ));

        let names = ["kim", "baughn", "dagger"];
        let clients: Vec<ClientHandle> = names
            .iter()
            .enumerate()
            .map(|(i, name)| client(&net, &server_id, name, i as u128 + 1))
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
                    let name = EndpointId::new(names[who]);
                    net.set_link(
                        &name,
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
                    net.disconnect(&EndpointId::new(names[who]), &server_id);
                }
            }
            tokio::time::sleep(Duration::from_millis(rng.random_range(10..400))).await;
        }

        // Quiesce: ample time for reconnects (2s backoff) and replay.
        let refs: Vec<&ClientHandle> = clients.iter().collect();
        eventually(&refs, Duration::from_secs(120), |views| {
            views.iter().all(|v| v.chat.len() == sent_chats)
                && views.windows(2).all(|w| w[0] == w[1])
        })
        .await;

        // Use test_support's hash domain sanity: nothing panicked, all
        // views identical — the milestone property under chaos.
        let _ = test_support::file(0);
    }
}

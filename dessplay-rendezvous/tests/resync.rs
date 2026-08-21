//! `/resync`: the deliberate remedy for persistent divergence
//! (docs/sync-state.md, Manual Reset). Since the 2026-08-21 review it
//! is clear-and-re-exec — the client tears down, clears its sync
//! database, and restarts as a fresh process (fresh session `ActorId`,
//! empty replica). This test models the restart half: the fresh
//! incarnation adopts the server's copy through the connect handshake,
//! local-only garbage a divergence bug deposited dies with the old
//! process, local files re-announce availability, and post-reset
//! writes still propagate.

mod common;

use std::time::Duration;

use common::*;
use dessplay::actors::sync::{Mutation, SyncCommand};
use dessplay_core::net::ServerControl;
use dessplay_core::types::{FileAvailability, SharedTimestamp, UserId};
use dessplay_core::{ChatMessage, CrdtState, StateSnapshot};

const BUDGET: Duration = Duration::from_secs(20);

/// Alice holds a file locally and everyone sees her Ready announcement.
/// A divergence bug plants garbage in her replica alone; she runs
/// `/resync`, which restarts her client with a cleared sync database.
/// The fresh incarnation (new nonce — a fresh session `ActorId`, the
/// re-exec's defining property) adopts the server's state: the garbage
/// is gone, the shared state re-converges, her availability re-derives
/// from her local file, and a post-reset write still propagates.
#[tokio::test(start_paused = true)]
async fn resync_restart_reconverges_and_reannounces_availability() {
    let harness = Harness::new(741);
    let alice = harness.player_client("alice", 1);
    let bob = harness.player_client("bob", 2);

    // Alice holds the episode locally and queues it.
    let file = media_file(1);
    alice.install(&file);
    mutate(
        &alice,
        Mutation::PushPlaylist {
            new: file_entry(&file, "alice"),
        },
    )
    .await;

    // Everyone sees the entry and alice's Ready announcement (her file
    // actor scanned the root, verified the hash, and announced).
    let hash = file.hash;
    eventually_views(&[&alice, &bob], BUDGET, |views| {
        views.iter().all(|v| {
            v.playlist.len() == 1
                && v.file_availability.get(&(UserId::new("alice"), hash))
                    == Some(&FileAvailability::Ready)
        })
    })
    .await;

    // A divergence bug's signature: state that exists ONLY in alice's
    // replica. Delivered as a stray same-epoch inbound merge — the
    // server never held it, so nothing but a fresh adoption can remove
    // it (an additive union would keep it forever).
    let mut garbage = CrdtState::new();
    garbage.append_chat(ChatMessage {
        timestamp: SharedTimestamp(1),
        sender: UserId::new("gremlin"),
        text: "local-only garbage".into(),
    });
    let epoch = epoch_of(&alice).await;
    alice
        .sync
        .send(SyncCommand::Server {
            msg: Box::new(ServerControl::StateMerge(StateSnapshot {
                epoch,
                state: garbage,
            })),
            via_datagram: false,
        })
        .await
        .unwrap();
    eventually_views(&[&alice], BUDGET, |views| {
        views[0].chat.iter().any(|m| m.text == "local-only garbage")
    })
    .await;

    // The deliberate act: /resync — tear down and restart with a
    // cleared sync database. The player clients here run stateless, so
    // a fresh incarnation with a new nonce IS the post-clear restart.
    quit(&alice).await;
    let alice = harness.player_client("alice", 3);
    alice.install(&file);

    // Re-convergence through the connect handshake: the garbage died
    // with the old process (and was never seen by bob), the playlist is
    // back, and alice's availability for her local file re-announces.
    eventually_views(&[&alice, &bob], BUDGET, |views| {
        views.iter().all(|v| {
            v.playlist.len() == 1
                && !v.chat.iter().any(|m| m.text == "local-only garbage")
                && v.file_availability.get(&(UserId::new("alice"), hash))
                    == Some(&FileAvailability::Ready)
        })
    })
    .await;

    // Liveness after the reset: a post-reset write from the fresh
    // incarnation reaches everyone.
    mutate(
        &alice,
        Mutation::Chat {
            text: "post-reset hello".into(),
        },
    )
    .await;
    eventually_views(&[&alice, &bob], BUDGET, |views| {
        views
            .iter()
            .all(|v| v.chat.iter().any(|m| m.text == "post-reset hello"))
    })
    .await;
}

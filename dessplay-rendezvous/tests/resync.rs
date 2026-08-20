//! Mid-session `/resync` (`SyncCommand::ResetState`): the deliberate
//! remedy for persistent divergence (docs/sync-state.md, Divergence
//! Alarm). The client discards its replica — including local-only
//! garbage a divergence bug deposited — re-adopts the server's copy
//! through the curative snapshot, keeps working afterwards, and its
//! local files re-announce availability.

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
/// `/resync`. The garbage vanishes without ever reaching anyone else,
/// the shared state re-converges, her availability survives, and a
/// post-reset write still propagates (and wins its LWW stamps — the
/// Lamport floor survived the reset).
#[tokio::test(start_paused = true)]
async fn mid_session_resync_reconverges_and_reannounces_availability() {
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
    // server never held it, so nothing but a snapshot adoption can
    // remove it (an additive union would keep it forever).
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

    // The deliberate act: /resync.
    alice.sync.send(SyncCommand::ResetState).await.unwrap();

    // Re-convergence: the garbage is gone from alice (and was never
    // seen by bob), the playlist is back, and alice's availability for
    // her local file is re-announced.
    eventually_views(&[&alice, &bob], BUDGET, |views| {
        views.iter().all(|v| {
            v.playlist.len() == 1
                && !v.chat.iter().any(|m| m.text == "local-only garbage")
                && v.file_availability.get(&(UserId::new("alice"), hash))
                    == Some(&FileAvailability::Ready)
        })
    })
    .await;

    // Liveness after the reset: a post-reset write from alice reaches
    // everyone.
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

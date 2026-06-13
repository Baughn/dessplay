//! Phase 9B-4: seeder auto-fetch. A headless seeder pulls every playlist
//! entry from whoever has it, becoming the durable seed — even with no
//! now-playing file and no player.

mod common;

use std::time::Duration;

use common::*;
use dessplay::actors::sync::Mutation;
use dessplay_core::types::{FileAvailability, UserId};

const BUDGET: Duration = Duration::from_secs(30);

#[tokio::test(start_paused = true)]
async fn a_seeder_auto_fetches_playlist_entries() {
    let harness = Harness::new(708);
    let kim = harness.player_client("kim", 1); // has the file
    let nas = harness.seeder_client("nas", 2); // has nothing yet
    let file = media_file(1);
    kim.install(&file);

    // Kim adds the file (no now-playing needed — the seeder fetches the
    // whole playlist, not just what's playing).
    mutate(
        &kim,
        Mutation::PushPlaylist {
            new: file_entry(&file, "kim"),
        },
    )
    .await;

    // The seeder resolves it missing, downloads it from kim through the
    // relay, and ends up Ready — now a source for everyone else.
    eventually(&[&nas], BUDGET, |snaps| {
        snaps.iter().all(|s| {
            s.view
                .file_availability
                .get(&(UserId::new("nas"), file.hash))
                == Some(&FileAvailability::Ready)
        })
    })
    .await;
}

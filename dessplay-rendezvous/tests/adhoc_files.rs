//! Phase 31, end to end: a file dragged into the terminal from outside
//! every media root plays for everyone — the hash-add registers a
//! servable copy in the same session, and the out-of-root registration
//! is durable, so the adder still serves it after a restart.
//!
//! Real time (`multi_thread`), like the other [`LoopRig`] tests: hashing
//! and the transfer path live on the blocking pool, which paused time
//! cannot drive.

mod common;

use std::time::Duration;

use common::*;
use dessplay::ui::msg::UserAction;
use dessplay_core::types::{Ed2kHash, FileAvailability, UserId};

/// Poll `rig`'s own synced view until `who` advertises `file` Ready.
async fn eventually_ready(rig: &LoopRig, who: &str, file: Ed2kHash, budget: Duration) {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let view = rig.view().await;
        if view.file_availability.get(&(UserId::new(who), file)) == Some(&FileAvailability::Ready) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{who} never became Ready for {file}; availability: {:?}",
            view.file_availability
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn dragged_in_out_of_root_file_serves_the_group_and_survives_restart() {
    let harness = Harness::new(3101);
    let dir_a = tempfile::tempdir().expect("tempdir");
    let dir_b = tempfile::tempdir().expect("tempdir");
    let dir_c = tempfile::tempdir().expect("tempdir");
    // The dragged file lives outside every media root (loop rigs have
    // none configured at all).
    let dragged = tempfile::tempdir().expect("tempdir");
    let file = media_file(1);
    let path = dragged.path().join(&file.filename);
    std::fs::write(&path, &file.contents).expect("writing dragged file");

    let rig_a = loop_rig(&harness, "kim", 1, dir_a.path());
    let rig_b = loop_rig(&harness, "nero", 1, dir_b.path());

    // A "drags the file in": the paste branch emits exactly this action.
    rig_a
        .actions
        .send(UserAction::HashAndAdd {
            path: path.clone(),
            after: None,
        })
        .await
        .expect("loop gone");

    // B downloads it from A and flips Ready — the same-session serve
    // that used to wedge on the silent Ready-but-unservable state.
    eventually_ready(&rig_b, "nero", file.hash, Duration::from_secs(30)).await;

    // B leaves, A restarts: the only copy left in the group is A's
    // out-of-root one, so a fresh downloader exercises the durable
    // registration.
    rig_b.quit().await;
    rig_a.quit().await;
    let _rig_a2 = loop_rig(&harness, "kim", 2, dir_a.path());
    let rig_c = loop_rig(&harness, "baughn", 1, dir_c.path());
    eventually_ready(&rig_c, "baughn", file.hash, Duration::from_secs(30)).await;
}

//! Supervision tests for the interactive bridge loop (`SessionLoop`),
//! extracted from `run_interactive` precisely so these flows are
//! testable: "Ctrl-C must quit" has regressed repeatedly, each time
//! because some arm of the loop blocked.
//!
//! The reproducible stand-in for "hashing a 1.4GB file" is a FIFO:
//! opening a named pipe for reading blocks until a writer appears, so a
//! `HashAndAdd` pointed at one hangs for as long as the test wants —
//! exactly the shape of the 2026-06-12 bug, where inline hashing
//! starved the loop, froze the UI, and left a queued Quit unread.
//!
//! Two care points baked into the structure:
//! - **Real time, not paused**: the hang under test lives on the
//!   blocking pool, which simulated time cannot touch.
//! - **The FIFO must be released before any assertion can panic**:
//!   tokio's runtime shutdown waits for blocking-pool tasks, so a
//!   still-blocked `File::open` would hang the whole test binary on the
//!   way out — pass or fail.

mod common;

use std::path::Path;
use std::time::Duration;

use common::*;
use dessplay::run::SessionEnd;
use dessplay::ui::msg::UserAction;
use dessplay::ui::shell::UiInput;

fn mkfifo(path: &Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("running mkfifo");
    assert!(status.success(), "mkfifo failed: {status:?}");
}

/// Unblock whoever is stuck opening `fifo` for reading: connect the
/// write side and immediately close it (the reader sees EOF).
fn release_fifo(fifo: &Path) {
    let _ = std::fs::OpenOptions::new().write(true).open(fifo);
}

/// A stalled renderer must neither lose one-shot replies nor prevent Quit.
/// Ordered actions and joining the session provide the barriers; no sleeps
/// or assumptions about the relative scheduling of the actors are needed.
#[tokio::test(flavor = "multi_thread")]
async fn browser_replies_survive_a_stalled_ui_without_blocking_quit() {
    let harness = Harness::new(803);
    let dir = tempfile::tempdir().expect("tempdir");
    let mut rig = loop_rig(&harness, "kim", 1, dir.path());
    let requests: Vec<_> = (0..128)
        .map(|index| dessplay::ui::msg::BrowseRequest::Add {
            after: Some(hash(index)),
        })
        .collect();
    for request in &requests {
        rig.actions
            .send(UserAction::Browse(request.clone()))
            .await
            .expect("loop gone");
    }
    rig.actions.send(UserAction::Quit).await.expect("loop gone");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), &mut rig.task)
            .await
            .expect("UI delivery blocked Quit")
            .expect("loop panicked"),
        SessionEnd::Quit,
    );
    let received: Vec<_> = rig
        .ui_rx
        .try_iter()
        .filter_map(|input| match input {
            UiInput::Browse { request, .. } => Some(request),
            _ => None,
        })
        .collect();
    assert_eq!(received, requests, "a required browser reply was lost");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_ui_ends_the_session_without_another_action() {
    let harness = Harness::new(804);
    let dir = tempfile::tempdir().expect("tempdir");
    let rig = loop_rig(&harness, "kim", 1, dir.path());
    drop(rig.ui_rx);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), rig.task)
            .await
            .expect("session ignored UI closure")
            .expect("loop panicked"),
        SessionEnd::Quit,
    );
}

/// The Ctrl-C regression: a quit must be processed even while a
/// playlist-add hash is stuck (or merely slow). Pointing the hash at a
/// FIFO makes "stuck" reproducible.
#[tokio::test(flavor = "multi_thread")]
async fn quit_is_processed_while_a_hash_is_stuck() {
    let harness = Harness::new(801);
    let dir = tempfile::tempdir().expect("tempdir");
    let mut rig = loop_rig(&harness, "kim", 1, dir.path());

    let fifo = dir.path().join("never.mkv");
    mkfifo(&fifo);
    rig.actions
        .send(UserAction::HashAndAdd {
            path: fifo.clone(),
            after: None,
        })
        .await
        .expect("loop gone");
    // Let the loop pick the add up before the quit arrives.
    tokio::time::sleep(Duration::from_millis(300)).await;
    rig.actions.send(UserAction::Quit).await.expect("loop gone");

    let end = tokio::time::timeout(Duration::from_secs(5), &mut rig.task).await;
    // Unstick the hasher *before* asserting, so the binary can exit
    // even when the assertion fails.
    release_fifo(&fifo);
    let end = end
        .expect("Ctrl-C regression: the bridge loop did not exit while a hash was in flight")
        .expect("loop task panicked");
    assert_eq!(end, SessionEnd::Quit);
}

/// The frozen-playlist regression: while one hash is stuck, other adds
/// must still land in the synced state (the loop must not serialize
/// behind hashing).
#[tokio::test(flavor = "multi_thread")]
async fn adds_keep_flowing_while_a_hash_is_stuck() {
    let harness = Harness::new(802);
    let dir = tempfile::tempdir().expect("tempdir");
    let mut rig = loop_rig(&harness, "kim", 1, dir.path());

    let fifo = dir.path().join("never.mkv");
    mkfifo(&fifo);
    let real = dir.path().join("real.mkv");
    std::fs::write(&real, b"a real episode").expect("writing test file");

    rig.actions
        .send(UserAction::HashAndAdd {
            path: fifo.clone(),
            after: None,
        })
        .await
        .expect("loop gone");
    tokio::time::sleep(Duration::from_millis(300)).await;
    rig.actions
        .send(UserAction::HashAndAdd {
            path: real,
            after: None,
        })
        .await
        .expect("loop gone");

    // The real file must reach the playlist despite the stuck hash.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut landed = false;
    while tokio::time::Instant::now() < deadline {
        let (tx, rx) = tokio::sync::oneshot::channel();
        rig.sync
            .send(dessplay::actors::sync::SyncCommand::GetView(tx))
            .await
            .expect("sync actor gone");
        let view = rx.await.expect("sync actor gone");
        if view
            .playlist
            .iter()
            .any(|entry| entry.state.filename == "real.mkv")
        {
            landed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    release_fifo(&fifo);
    assert!(
        landed,
        "the add never landed; the loop is starved by the stuck hash"
    );

    // The no-silent-work rule, end to end: the loop must have pushed
    // hashing progress for the real file to the UI, including its
    // completion.
    let mut saw_progress = false;
    let mut saw_finished = false;
    while let Ok(input) = rig.ui_rx.try_recv() {
        if let UiInput::Hashing {
            filename, finished, ..
        } = input
            && filename == "real.mkv"
        {
            saw_progress = true;
            saw_finished |= finished;
        }
    }
    assert!(saw_progress, "no hashing progress reached the UI");
    assert!(saw_finished, "the hashing row was never cleared");

    rig.actions.send(UserAction::Quit).await.expect("loop gone");
    let end = tokio::time::timeout(Duration::from_secs(5), &mut rig.task)
        .await
        .expect("loop did not exit on quit")
        .expect("loop task panicked");
    assert_eq!(end, SessionEnd::Quit);
}

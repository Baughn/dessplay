//! Real-mpv integration: spawns an actual mpv process and drives it
//! over JSON IPC. Gated behind `--features mpv-tests` (needs the `mpv`
//! binary); the test video is encoded on the fly by mpv itself from a
//! lavfi synthetic source, so no media files are committed and no
//! ffmpeg is required.
//!
//! One end-to-end journey by design — the logic lives in the
//! MockPlayer suites; this proves the production IPC layer speaks
//! actual mpv.

#![cfg(feature = "mpv-tests")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::time::Duration;

use dessplay::player::mpv::MpvPlayer;
use dessplay::player::{Player, PlayerEvent};

const BUDGET: Duration = Duration::from_secs(15);

async fn expect_event<T>(
    player: &MpvPlayer,
    budget: Duration,
    mut pred: impl FnMut(&PlayerEvent) -> Option<T>,
) -> T {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let event = tokio::time::timeout_at(deadline, player.recv())
            .await
            .expect("event budget exhausted")
            .expect("player gone");
        if let Some(out) = pred(&event) {
            return out;
        }
    }
}

/// Encode a 4-second test pattern with mpv's own encoder. A real
/// container file (unlike playing `av://lavfi:` directly) reports its
/// duration up front and seeks faithfully.
async fn encode_test_video(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("testsrc.mkv");
    let status = tokio::process::Command::new("mpv")
        .arg("av://lavfi:testsrc=duration=4:rate=25")
        .arg(format!("--o={}", path.display()))
        .arg("--of=matroska")
        .arg("--no-terminal")
        .status()
        .await
        .expect("running mpv encoder");
    assert!(status.success(), "mpv encode failed: {status:?}");
    path
}

#[tokio::test]
async fn full_journey_against_real_mpv() {
    let dir = tempfile::tempdir().unwrap();
    let video = encode_test_video(dir.path()).await;
    let player = MpvPlayer::launch(
        "mpv",
        dir.path().join("ipc.sock"),
        &[
            "--vo=null".into(),
            "--ao=null".into(),
            "--force-window=no".into(),
        ],
    )
    .await
    .expect("launching mpv");

    player.load(&video, None).await.unwrap();
    // Event order is load-bearing: the observed `path` echo must precede
    // file-loaded and duration — it is what opens the actor's attribution
    // gate (design.md, Events from Player), and the MockPlayer acks in
    // this order on the strength of it. Verify real mpv actually does.
    expect_event(&player, BUDGET, |e| match e {
        PlayerEvent::PathChanged { path } => {
            assert_eq!(
                Path::new(path),
                video,
                "path echo names a different file than the load"
            );
            Some(())
        }
        PlayerEvent::Loaded | PlayerEvent::DurationKnown { .. } => {
            panic!("{e:?} arrived before the loaded file's path echo")
        }
        _ => None,
    })
    .await;
    expect_event(&player, BUDGET, |e| {
        matches!(e, PlayerEvent::Loaded).then_some(())
    })
    .await;
    let duration = expect_event(&player, BUDGET, |e| match e {
        PlayerEvent::DurationKnown { duration_millis } => Some(*duration_millis),
        _ => None,
    })
    .await;
    assert!(
        (3_500..=4_500).contains(&duration),
        "expected ~4s, got {duration}ms"
    );

    // Unpause: the echo comes back as a PauseChanged, and positions
    // start advancing.
    player.set_pause(false).await.unwrap();
    expect_event(&player, BUDGET, |e| {
        matches!(e, PlayerEvent::PauseChanged(false)).then_some(())
    })
    .await;
    expect_event(&player, BUDGET, |e| match e {
        PlayerEvent::Position { position_millis } if *position_millis > 200 => Some(()),
        _ => None,
    })
    .await;

    // Slew must be accepted (no observable event; just not an error).
    player.set_speed(1.02).await.unwrap();
    player.set_speed(1.0).await.unwrap();

    // Seek near the end; the landed position comes from the post-seek
    // query.
    player.seek(3_500).await.unwrap();
    let landed = expect_event(&player, BUDGET, |e| match e {
        PlayerEvent::Seeked { position_millis } => Some(*position_millis),
        _ => None,
    })
    .await;
    assert!(
        (3_300..=3_800).contains(&landed),
        "seek landed at {landed}ms, expected ~3500"
    );

    // Let it run out: EOF (keep-open holds the file; mpv's mechanical
    // pause must NOT surface as a user pause).
    expect_event(&player, BUDGET, |e| {
        assert!(
            !matches!(e, PlayerEvent::PauseChanged(true)),
            "keep-open's EOF pause leaked as a user pause"
        );
        matches!(e, PlayerEvent::Eof).then_some(())
    })
    .await;

    // Overlay round trip: set and remove must both be accepted (mpv
    // rejects malformed osd-overlay arguments with a command error).
    player
        .set_osd_overlay(1, Some("{\\an7\\fs26}dessplay test"))
        .await
        .unwrap();
    player.set_osd_overlay(1, None).await.unwrap();

    // Clean shutdown.
    player.shutdown().await;
    expect_event(&player, BUDGET, |e| match e {
        PlayerEvent::Exited { clean } => Some(*clean),
        _ => None,
    })
    .await;
}

/// Attach mode (`--attach-mpv`): dessplay connects to an mpv the user
/// launched and drives it, but must leave that process running on
/// shutdown — it isn't ours to kill. We spawn mpv ourselves here, standing
/// in for the user's tmux pane.
#[tokio::test]
async fn attach_to_external_mpv_and_leave_it_running() {
    let dir = tempfile::tempdir().unwrap();
    let video = encode_test_video(dir.path()).await;
    let socket = dir.path().join("external.sock");

    // The "user's" mpv — launched with the flags attach mode depends on
    // (idle + keep-open), exactly as the --attach-mpv help instructs.
    let mut external = tokio::process::Command::new("mpv")
        .arg("--idle=yes")
        .arg("--keep-open=yes")
        .arg("--vo=null")
        .arg("--ao=null")
        .arg("--no-terminal")
        .arg(format!("--input-ipc-server={}", socket.display()))
        .spawn()
        .expect("spawning external mpv");

    let player = MpvPlayer::attach(socket.clone())
        .await
        .expect("attaching to mpv");

    // We can drive the attached instance: load, then a pause echo.
    player.load(&video, None).await.unwrap();
    expect_event(&player, BUDGET, |e| {
        matches!(e, PlayerEvent::Loaded).then_some(())
    })
    .await;
    player.set_pause(false).await.unwrap();
    expect_event(&player, BUDGET, |e| {
        matches!(e, PlayerEvent::PauseChanged(false)).then_some(())
    })
    .await;
    player.seek(1_000).await.unwrap();
    expect_event(&player, BUDGET, |e| {
        matches!(e, PlayerEvent::Seeked { .. }).then_some(())
    })
    .await;

    // Detaching reports Exited (so the actor's relaunch path runs) but must
    // NOT quit the user's mpv.
    player.shutdown().await;
    expect_event(&player, BUDGET, |e| match e {
        PlayerEvent::Exited { .. } => Some(()),
        _ => None,
    })
    .await;
    assert!(
        external.try_wait().unwrap().is_none(),
        "attach shutdown killed the external mpv"
    );

    // We spawned it, so we clean it up.
    external.kill().await.unwrap();
}

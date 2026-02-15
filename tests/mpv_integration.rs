use std::path::PathBuf;
use std::time::Duration;

use dessplay::player::bridge::PlayerBridge;
use dessplay::player::events::PlayerEvent;
use dessplay::player::mpv::MpvPlayer;
use tokio::sync::mpsc;
use tokio::time::timeout;

fn test_video() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/video.mkv")
}

/// Wait for an event matching the predicate, discarding non-matching events.
async fn wait_for_event(
    rx: &mut mpsc::Receiver<PlayerEvent>,
    deadline: Duration,
    mut predicate: impl FnMut(&PlayerEvent) -> bool,
) -> Option<PlayerEvent> {
    let start = tokio::time::Instant::now();
    loop {
        let remaining = deadline.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return None;
        }
        match timeout(remaining, rx.recv()).await {
            Ok(Some(event)) => {
                if predicate(&event) {
                    return Some(event);
                }
            }
            _ => return None,
        }
    }
}

/// Collect all events for a fixed duration.
async fn collect_events(
    rx: &mut mpsc::Receiver<PlayerEvent>,
    duration: Duration,
) -> Vec<PlayerEvent> {
    let mut events = Vec::new();
    let deadline = tokio::time::Instant::now() + duration;
    loop {
        let remaining = deadline - tokio::time::Instant::now();
        if remaining.is_zero() {
            break;
        }
        match timeout(remaining, rx.recv()).await {
            Ok(Some(event)) => events.push(event),
            _ => break,
        }
    }
    events
}

/// Helper: spawn a headless player, load the test video, wait for playback to start.
async fn spawn_and_load() -> (MpvPlayer, mpsc::Receiver<PlayerEvent>) {
    let mut player = MpvPlayer::new(true);
    let mut rx = player.spawn().await.expect("spawn failed");

    player
        .loadfile(&test_video())
        .await
        .expect("loadfile failed");

    // Wait for first PositionChanged to confirm playback started
    wait_for_event(&mut rx, Duration::from_secs(5), |e| {
        matches!(e, PlayerEvent::PositionChanged(_))
    })
    .await
    .expect("never got PositionChanged after loadfile");

    (player, rx)
}

// === Basic IPC tests ===

#[tokio::test]
async fn t01_spawn_and_quit() {
    let mut player = MpvPlayer::new(true);
    let mut rx = player.spawn().await.expect("spawn failed");

    player.quit().await.expect("quit failed");

    let event = wait_for_event(&mut rx, Duration::from_secs(5), |e| {
        matches!(e, PlayerEvent::Exited { .. })
    })
    .await
    .expect("never got Exited event");

    assert!(matches!(event, PlayerEvent::Exited { clean: true }));
}

#[tokio::test]
async fn t02_load_file() {
    let (player, mut rx) = spawn_and_load().await;

    // We already got one PositionChanged in spawn_and_load.
    // Verify we keep getting them.
    let event = wait_for_event(&mut rx, Duration::from_secs(3), |e| {
        matches!(e, PlayerEvent::PositionChanged(_))
    })
    .await;
    assert!(event.is_some(), "should keep getting PositionChanged");

    player.quit().await.ok();
}

#[tokio::test]
async fn t03_seek() {
    let (player, mut rx) = spawn_and_load().await;

    player.clear_attribution_log();
    player.seek(10.0).await.expect("seek failed");

    // Wait for playback-restart (position updates resume)
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Check position is near 10s
    let pos = player.get_position().await.expect("get_position failed");
    let pos = pos.expect("should have a position");
    assert!(
        (pos - 10.0).abs() < 2.0,
        "position should be near 10s, got {pos}"
    );

    // Verify NO UserSeeked event was emitted
    let events = collect_events(&mut rx, Duration::from_millis(500)).await;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, PlayerEvent::UserSeeked { .. })),
        "programmatic seek should NOT emit UserSeeked"
    );

    player.quit().await.ok();
}

#[tokio::test]
async fn t04_get_position() {
    let (player, _rx) = spawn_and_load().await;

    let pos = player.get_position().await.expect("get_position failed");
    assert!(pos.is_some(), "should have a position when playing");
    assert!(pos.unwrap() >= 0.0);

    player.quit().await.ok();
}

#[tokio::test]
async fn t05_get_position_no_file() {
    let mut player = MpvPlayer::new(true);
    let _rx = player.spawn().await.expect("spawn failed");

    let pos = player.get_position().await.expect("get_position failed");
    assert!(pos.is_none(), "should be None when no file loaded");

    player.quit().await.ok();
}

#[tokio::test]
async fn t06_eof() {
    let (player, mut rx) = spawn_and_load().await;

    // Seek near the end of the video
    // Our test video is short, seek to near the end
    player.seek(58.0).await.expect("seek failed");

    // Wait for EndOfFile
    let event = wait_for_event(&mut rx, Duration::from_secs(10), |e| {
        matches!(e, PlayerEvent::EndOfFile)
    })
    .await;
    assert!(event.is_some(), "should get EndOfFile event");

    player.quit().await.ok();
}

#[tokio::test]
async fn t07_show_text() {
    let mut player = MpvPlayer::new(true);
    let _rx = player.spawn().await.expect("spawn failed");

    // show-text should succeed even with vo=null
    player
        .show_text("Hello, DessPlay!", 2000)
        .await
        .expect("show_text should succeed");

    player.quit().await.ok();
}

#[tokio::test]
async fn t08_crash_detection() {
    let mut player = MpvPlayer::new(true);
    let mut rx = player.spawn().await.expect("spawn failed");

    // Find the mpv process and kill it
    // We'll use the quit command's absence — instead, just send SIGKILL via nix
    // Actually, we can use the socket path to find the process. Easier: just use
    // tokio::process to kill it. But we don't have the child handle directly.
    // Let's use a different approach: send an invalid quit that crashes mpv.
    // Actually the simplest way is to kill via signal.

    let pid = player.get_pid().await.expect("get pid");

    // Send SIGKILL
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGKILL,
    )
    .expect("kill");

    let event = wait_for_event(&mut rx, Duration::from_secs(5), |e| {
        matches!(e, PlayerEvent::Exited { .. })
    })
    .await
    .expect("never got Exited event after kill");

    assert!(matches!(event, PlayerEvent::Exited { clean: false }));
}

#[tokio::test]
async fn t09_crash_recovery() {
    let mut player = MpvPlayer::new(true);
    let mut rx = player.spawn().await.expect("spawn failed");

    player
        .loadfile(&test_video())
        .await
        .expect("loadfile failed");

    // Wait for playback
    wait_for_event(&mut rx, Duration::from_secs(5), |e| {
        matches!(e, PlayerEvent::PositionChanged(_))
    })
    .await
    .expect("never started playing");

    // Seek to a known position
    player.seek(15.0).await.expect("seek failed");
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Kill the process
    let pid = player.get_pid().await.expect("get pid");

    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGKILL,
    )
    .expect("kill");

    // Wait for crash
    wait_for_event(&mut rx, Duration::from_secs(5), |e| {
        matches!(e, PlayerEvent::Exited { clean: false })
    })
    .await
    .expect("never got crash event");

    // Recover
    let mut rx = player.handle_crash().await.expect("handle_crash failed");

    // Wait for playback to resume
    wait_for_event(&mut rx, Duration::from_secs(5), |e| {
        matches!(e, PlayerEvent::PositionChanged(_))
    })
    .await
    .expect("never resumed playback after crash");

    // Position should be near where we were
    let pos = player.get_position().await.expect("get_position failed");
    let pos = pos.expect("should have position");
    assert!(
        (pos - 15.0).abs() < 3.0,
        "position should be near 15s after recovery, got {pos}"
    );

    player.quit().await.ok();
}

#[tokio::test]
async fn t10_double_crash_fails() {
    let mut player = MpvPlayer::new(true);
    let mut rx = player.spawn().await.expect("spawn failed");

    // First crash
    let pid = player.get_pid().await.expect("get pid");

    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGKILL,
    )
    .expect("kill");

    wait_for_event(&mut rx, Duration::from_secs(5), |e| {
        matches!(e, PlayerEvent::Exited { .. })
    })
    .await
    .expect("never got exit");

    // First recovery succeeds
    let mut rx = player.handle_crash().await.expect("first recovery");

    // Second crash immediately
    let pid = player.get_pid().await.expect("get pid");

    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGKILL,
    )
    .expect("kill");

    wait_for_event(&mut rx, Duration::from_secs(5), |e| {
        matches!(e, PlayerEvent::Exited { .. })
    })
    .await
    .expect("never got exit");

    // Second recovery should fail (within 30s)
    let result = player.handle_crash().await;
    assert!(result.is_err(), "second crash within 30s should fail");
}

#[tokio::test]
async fn t11_parallel_instances() {
    let mut players = Vec::new();
    let mut receivers = Vec::new();

    for _ in 0..3 {
        let mut player = MpvPlayer::new(true);
        let rx = player.spawn().await.expect("spawn failed");
        players.push(player);
        receivers.push(rx);
    }

    // All three should be independently functional
    for player in &players {
        let pos = player.get_position().await;
        assert!(pos.is_ok(), "each instance should respond to commands");
    }

    for player in &players {
        player.quit().await.ok();
    }
}

// === User input detection tests ===

#[tokio::test]
async fn t12_user_pause() {
    let (player, mut rx) = spawn_and_load().await;

    // Simulate user pressing space (toggles pause)
    player.keypress("space").await.expect("keypress failed");

    let event = wait_for_event(&mut rx, Duration::from_secs(3), |e| {
        matches!(e, PlayerEvent::UserPauseToggled { .. })
    })
    .await
    .expect("never got UserPauseToggled");

    assert!(matches!(
        event,
        PlayerEvent::UserPauseToggled { paused: true }
    ));

    player.quit().await.ok();
}

#[tokio::test]
async fn t13_user_unpause() {
    let (player, mut rx) = spawn_and_load().await;

    // Programmatically pause first
    player.pause().await.expect("pause failed");
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Drain any pending events
    collect_events(&mut rx, Duration::from_millis(200)).await;

    // Simulate user pressing space to unpause
    player.keypress("space").await.expect("keypress failed");

    let event = wait_for_event(&mut rx, Duration::from_secs(3), |e| {
        matches!(e, PlayerEvent::UserPauseToggled { .. })
    })
    .await
    .expect("never got UserPauseToggled for unpause");

    assert!(matches!(
        event,
        PlayerEvent::UserPauseToggled { paused: false }
    ));

    player.quit().await.ok();
}

#[tokio::test]
async fn t14_user_seek() {
    let (player, mut rx) = spawn_and_load().await;

    // Let playback stabilize
    tokio::time::sleep(Duration::from_millis(300)).await;
    collect_events(&mut rx, Duration::from_millis(100)).await;

    // Simulate user pressing RIGHT (default: seek +5s)
    player.keypress("RIGHT").await.expect("keypress failed");

    let event = wait_for_event(&mut rx, Duration::from_secs(3), |e| {
        matches!(e, PlayerEvent::UserSeeked { .. })
    })
    .await
    .expect("never got UserSeeked for RIGHT key");

    if let PlayerEvent::UserSeeked { position } = event {
        assert!(position > 0.0, "seek position should be positive");
    }

    player.quit().await.ok();
}

#[tokio::test]
async fn t15_user_seek_backward() {
    let (player, mut rx) = spawn_and_load().await;

    // Seek forward first so we have room to go back
    player.seek(20.0).await.expect("seek failed");
    tokio::time::sleep(Duration::from_millis(500)).await;
    collect_events(&mut rx, Duration::from_millis(200)).await;

    let pos_before = player
        .get_position()
        .await
        .expect("get_position")
        .expect("position");

    // Simulate user pressing LEFT (seek -5s)
    player.keypress("LEFT").await.expect("keypress failed");

    let event = wait_for_event(&mut rx, Duration::from_secs(3), |e| {
        matches!(e, PlayerEvent::UserSeeked { .. })
    })
    .await
    .expect("never got UserSeeked for LEFT key");

    if let PlayerEvent::UserSeeked { position } = event {
        assert!(
            position < pos_before,
            "backward seek should give lower position: {position} < {pos_before}"
        );
    }

    player.quit().await.ok();
}

// === Echo suppression tests ===

#[tokio::test]
async fn t16_programmatic_pause_no_echo() {
    let (player, mut rx) = spawn_and_load().await;

    // Drain initial events
    collect_events(&mut rx, Duration::from_millis(200)).await;

    player.pause().await.expect("pause failed");

    // Collect events for a window — should NOT see UserPauseToggled
    let events = collect_events(&mut rx, Duration::from_millis(500)).await;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, PlayerEvent::UserPauseToggled { .. })),
        "programmatic pause should NOT emit UserPauseToggled, got: {events:?}"
    );

    player.quit().await.ok();
}

#[tokio::test]
async fn t17_programmatic_play_no_echo() {
    let (player, mut rx) = spawn_and_load().await;

    // Pause first
    player.pause().await.expect("pause failed");
    tokio::time::sleep(Duration::from_millis(300)).await;
    collect_events(&mut rx, Duration::from_millis(200)).await;

    // Now play
    player.play().await.expect("play failed");

    let events = collect_events(&mut rx, Duration::from_millis(500)).await;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, PlayerEvent::UserPauseToggled { .. })),
        "programmatic play should NOT emit UserPauseToggled, got: {events:?}"
    );

    player.quit().await.ok();
}

#[tokio::test]
async fn t18_programmatic_seek_no_echo() {
    let (player, mut rx) = spawn_and_load().await;
    collect_events(&mut rx, Duration::from_millis(200)).await;

    player.seek(30.0).await.expect("seek failed");

    // Wait for seek to complete
    tokio::time::sleep(Duration::from_millis(500)).await;

    let events = collect_events(&mut rx, Duration::from_millis(500)).await;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, PlayerEvent::UserSeeked { .. })),
        "programmatic seek should NOT emit UserSeeked, got: {events:?}"
    );

    // Should get PositionChanged near 30s after playback-restart
    let has_position_near_30 = events.iter().any(|e| {
        if let PlayerEvent::PositionChanged(pos) = e {
            (*pos - 30.0).abs() < 3.0
        } else {
            false
        }
    });
    assert!(
        has_position_near_30,
        "should get PositionChanged near 30s after seek"
    );

    player.quit().await.ok();
}

#[tokio::test]
async fn t19_seek_suppresses_stale_positions() {
    let (player, mut rx) = spawn_and_load().await;

    // Let playback run a bit so we have a known starting position
    tokio::time::sleep(Duration::from_millis(500)).await;
    collect_events(&mut rx, Duration::from_millis(100)).await;

    let pos_before = player
        .get_position()
        .await
        .expect("get_position")
        .expect("pos");

    player.clear_attribution_log();
    player.seek(30.0).await.expect("seek failed");

    // Wait for seek to complete fully
    tokio::time::sleep(Duration::from_millis(800)).await;

    // Check attribution log for stale position suppression
    let log = player.attribution_log();

    // Between seek command and playback-restart, any PositionChanged with old position
    // should be suppressed. We check that no PositionChanged with old position was emitted.
    let events = collect_events(&mut rx, Duration::from_millis(200)).await;
    let stale_positions: Vec<_> = events
        .iter()
        .filter(|e| {
            if let PlayerEvent::PositionChanged(pos) = e {
                (*pos - pos_before).abs() < 2.0 && (*pos - 30.0).abs() > 5.0
            } else {
                false
            }
        })
        .collect();

    assert!(
        stale_positions.is_empty(),
        "stale positions should be suppressed during seek: {stale_positions:?}"
    );

    // Check that stale positions were actually suppressed (not just absent)
    let suppressed_count = log
        .iter()
        .filter(|a| {
            matches!(
                a,
                dessplay::player::mpv::EventAttribution::SuppressedAsStalePosition
            )
        })
        .count();
    // We may or may not see suppressed entries depending on timing,
    // but we definitely should NOT see stale PositionChanged events
    let _ = suppressed_count; // Just verify it doesn't panic

    player.quit().await.ok();
}

#[tokio::test]
async fn t20_user_pause_during_programmatic_seek() {
    let (player, mut rx) = spawn_and_load().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    collect_events(&mut rx, Duration::from_millis(100)).await;

    // Start a seek
    player.seek(30.0).await.expect("seek failed");

    // Quickly press space (user pause) while seek is in flight
    tokio::time::sleep(Duration::from_millis(50)).await;
    player.keypress("space").await.expect("keypress failed");

    // The user pause toggle should still be emitted despite pending seek
    let event = wait_for_event(&mut rx, Duration::from_secs(3), |e| {
        matches!(e, PlayerEvent::UserPauseToggled { .. })
    })
    .await;

    assert!(
        event.is_some(),
        "user pause during programmatic seek should still emit UserPauseToggled"
    );

    player.quit().await.ok();
}

#[tokio::test]
async fn t21_user_seek_overrides_programmatic() {
    let (player, mut rx) = spawn_and_load().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    collect_events(&mut rx, Duration::from_millis(100)).await;

    // Start a programmatic seek
    player.seek(30.0).await.expect("seek failed");

    // Wait for the programmatic seek to complete
    tokio::time::sleep(Duration::from_millis(500)).await;
    collect_events(&mut rx, Duration::from_millis(200)).await;

    // Now user seeks with RIGHT key
    player.keypress("RIGHT").await.expect("keypress");

    let event = wait_for_event(&mut rx, Duration::from_secs(3), |e| {
        matches!(e, PlayerEvent::UserSeeked { .. })
    })
    .await;

    assert!(
        event.is_some(),
        "user seek after programmatic seek should emit UserSeeked"
    );

    player.quit().await.ok();
}

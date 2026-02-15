use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use dessplay::player::bridge::PlayerBridge;
use dessplay::player::events::PlayerEvent;
use dessplay::player::mpv::{EventAttribution, MpvPlayer};
use rand::Rng;
use rand::SeedableRng;
use tokio::sync::mpsc;
use tokio::time::timeout;

fn test_video() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/video.mkv")
}

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

/// Record of an action taken by the network or user actor.
#[derive(Debug, Clone)]
enum ActorAction {
    NetworkPause,
    NetworkPlay,
    NetworkSeek,
    UserPauseToggle,
    UserSeekForward,
    UserSeekBackward,
}

/// Run the fuzz test with a given seed for the specified duration.
async fn fuzz_run(seed: u64, run_duration: Duration) {
    let mut player = MpvPlayer::new(true);
    let mut rx = player.spawn().await.expect("spawn failed");

    player
        .loadfile(&test_video())
        .await
        .expect("loadfile failed");

    // Wait for playback to start
    wait_for_event(&mut rx, Duration::from_secs(5), |e| {
        matches!(e, PlayerEvent::PositionChanged(_))
    })
    .await
    .expect("never started playing");

    player.clear_attribution_log();

    // Wrap in Arc for sharing across tasks.
    // MpvPlayer's command methods all take &self, so this works.
    let player = Arc::new(player);
    let actions_log = Arc::new(std::sync::Mutex::new(Vec::<ActorAction>::new()));

    let deadline = tokio::time::Instant::now() + run_duration;

    // Network actor: programmatic commands with random delays
    let network_task = {
        let player = Arc::clone(&player);
        let log = Arc::clone(&actions_log);
        tokio::spawn(async move {
            let mut rng = rand::rngs::SmallRng::seed_from_u64(seed);
            while tokio::time::Instant::now() < deadline {
                let delay_ms = rng.random_range(10..=200);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;

                if tokio::time::Instant::now() >= deadline {
                    break;
                }

                let action: u32 = rng.random_range(0..3);
                match action {
                    0 => {
                        log.lock().unwrap().push(ActorAction::NetworkPause);
                        let _ = player.pause().await;
                    }
                    1 => {
                        log.lock().unwrap().push(ActorAction::NetworkPlay);
                        let _ = player.play().await;
                    }
                    _ => {
                        let pos = rng.random_range(1.0..55.0);
                        log.lock().unwrap().push(ActorAction::NetworkSeek);
                        let _ = player.seek(pos).await;
                    }
                }
            }
        })
    };

    // User actor: simulated keypresses with random delays
    let user_task = {
        let player = Arc::clone(&player);
        let log = Arc::clone(&actions_log);
        tokio::spawn(async move {
            let mut rng = rand::rngs::SmallRng::seed_from_u64(seed.wrapping_add(1));
            while tokio::time::Instant::now() < deadline {
                let delay_ms = rng.random_range(50..=500);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;

                if tokio::time::Instant::now() >= deadline {
                    break;
                }

                let action: u32 = rng.random_range(0..3);
                match action {
                    0 => {
                        log.lock().unwrap().push(ActorAction::UserPauseToggle);
                        let _ = player.keypress("space").await;
                    }
                    1 => {
                        log.lock().unwrap().push(ActorAction::UserSeekForward);
                        let _ = player.keypress("RIGHT").await;
                    }
                    _ => {
                        log.lock().unwrap().push(ActorAction::UserSeekBackward);
                        let _ = player.keypress("LEFT").await;
                    }
                }
            }
        })
    };

    // Event drainer — collect all events until deadline + settling time
    let drain_deadline = deadline + Duration::from_secs(1);
    let drain_task = tokio::spawn(async move {
        let mut received_events = Vec::new();
        loop {
            let remaining = drain_deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match timeout(remaining, rx.recv()).await {
                Ok(Some(event)) => received_events.push(event),
                _ => break,
            }
        }
        received_events
    });

    let _ = network_task.await;
    let _ = user_task.await;

    // Give a moment for events to settle
    tokio::time::sleep(Duration::from_millis(500)).await;

    let _received_events = drain_task.await.unwrap_or_default();
    let attribution_log = player.attribution_log();
    let actions = actions_log.lock().unwrap().clone();

    // === Invariant checks ===

    // Invariant 1: Every suppressed event has a matching pending command
    let suppress_pause_count = attribution_log
        .iter()
        .filter(|a| matches!(a, EventAttribution::SuppressedByPendingPause))
        .count();
    let network_pause_play_count = actions
        .iter()
        .filter(|a| matches!(a, ActorAction::NetworkPause | ActorAction::NetworkPlay))
        .count();
    assert!(
        suppress_pause_count <= network_pause_play_count,
        "seed {seed}: more pause suppressions ({suppress_pause_count}) than \
         network pause/play commands ({network_pause_play_count})"
    );

    let suppress_seek_count = attribution_log
        .iter()
        .filter(|a| matches!(a, EventAttribution::SuppressedByPendingSeek))
        .count();
    let network_seek_count = actions
        .iter()
        .filter(|a| matches!(a, ActorAction::NetworkSeek))
        .count();
    assert!(
        suppress_seek_count <= network_seek_count,
        "seed {seed}: more seek suppressions ({suppress_seek_count}) than \
         network seek commands ({network_seek_count})"
    );

    // Invariant 2: No event is both suppressed and emitted (structural — verified by enum).

    // Invariant 3: Attribution log is not empty (fuzz run generated events).
    assert!(
        !attribution_log.is_empty(),
        "seed {seed}: attribution log should not be empty"
    );

    eprintln!(
        "Fuzz seed {seed}: {actions_len} actions, {attr_len} attributions, \
         {suppress_pause_count} pause suppressed, {suppress_seek_count} seek suppressed",
        actions_len = actions.len(),
        attr_len = attribution_log.len(),
    );

    player.quit().await.ok();
}

#[tokio::test]
#[ignore]
async fn fuzz_echo_suppression_seed_1() {
    fuzz_run(1, Duration::from_secs(5)).await;
}

#[tokio::test]
#[ignore]
async fn fuzz_echo_suppression_seed_2() {
    fuzz_run(2, Duration::from_secs(5)).await;
}

#[tokio::test]
#[ignore]
async fn fuzz_echo_suppression_seed_3() {
    fuzz_run(3, Duration::from_secs(5)).await;
}

#[tokio::test]
#[ignore]
async fn fuzz_echo_suppression_seed_42() {
    fuzz_run(42, Duration::from_secs(5)).await;
}

#[tokio::test]
#[ignore]
async fn fuzz_echo_suppression_seed_1337() {
    fuzz_run(1337, Duration::from_secs(5)).await;
}

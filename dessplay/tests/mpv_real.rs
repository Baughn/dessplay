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
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

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
        PlayerEvent::DurationKnown { duration_millis } => Some(duration_millis.get()),
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

/// The marker value the test `[dessplay]` profile sets `sub-pos` to
/// (default 100) — an innocuous global option we can read back to ask
/// "is the profile in effect right now?".
const PROFILE_SUB_POS: f64 = 73.0;

/// A second IPC client on the same socket (mpv serves several at once;
/// attach mode depends on that), used to read properties back without
/// going through — or disturbing — the production layer under test.
struct Probe {
    write: tokio::net::unix::OwnedWriteHalf,
    lines: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    next_id: u64,
}

impl Probe {
    async fn connect(socket: &Path) -> Probe {
        let stream = UnixStream::connect(socket).await.expect("probe connect");
        let (read, write) = stream.into_split();
        Probe {
            write,
            lines: BufReader::new(read).lines(),
            next_id: 0,
        }
    }

    /// One `get_property` round trip. mpv broadcasts events to every
    /// IPC client; skip everything that isn't our reply.
    async fn get(&mut self, name: &str) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let mut line = json!({"command": ["get_property", name], "request_id": id}).to_string();
        line.push('\n');
        self.write
            .write_all(line.as_bytes())
            .await
            .expect("probe write");
        let deadline = tokio::time::Instant::now() + BUDGET;
        loop {
            let line = tokio::time::timeout_at(deadline, self.lines.next_line())
                .await
                .expect("probe reply budget exhausted")
                .expect("probe read")
                .expect("probe socket closed");
            let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if msg.get("request_id").and_then(Value::as_u64) == Some(id) {
                return msg.get("data").cloned().unwrap_or(Value::Null);
            }
        }
    }
}

/// Assert the profile's marker option is in effect. Polls up to the
/// budget: right after a (re)launch, setup's `apply-profile` write
/// races the probe's read (separate IPC connections have no ordering).
async fn expect_profile_marker(probe: &mut Probe, context: &str) {
    let deadline = tokio::time::Instant::now() + BUDGET;
    let mut last = Value::Null;
    while tokio::time::Instant::now() < deadline {
        last = probe.get("sub-pos").await;
        if last.as_f64() == Some(PROFILE_SUB_POS) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("{context}: sub-pos is {last}, expected {PROFILE_SUB_POS} from the [dessplay] profile");
}

/// Load a file and wait until mpv confirms it opened.
async fn load_and_settle(player: &MpvPlayer, path: &Path, title: Option<&str>) {
    player.load(path, title).await.unwrap();
    expect_event(player, BUDGET, |e| {
        matches!(e, PlayerEvent::Loaded).then_some(())
    })
    .await;
}

/// Opportunistically empty the event channel (bounded: while unpaused,
/// position updates never go quiet) so it cannot back up mid-sequence.
async fn drain_events(player: &MpvPlayer) {
    for _ in 0..64 {
        if tokio::time::timeout(Duration::from_millis(50), player.recv())
            .await
            .is_err()
        {
            return;
        }
    }
}

/// The `[dessplay]` mpv.conf profile is applied once per IPC connection
/// (`apply-profile` in setup) and must then *hold* across everything a
/// session does to the player: the profile has no per-file
/// re-application, so nothing — placeholder loads, placeholder→real
/// swaps, later episode swaps, pause churn, seeks, speed slew, crash
/// relaunches — may reset it. Field report (2026-09-06): a user's mpv
/// script saw no dessplay profile on the playing episode after a
/// placeholder had loaded first.
///
/// The opening is pinned to the reported scenario (idle → placeholder →
/// real file); a seeded arbitrary action tail then hunts the rest of
/// the class. mpv runs against a config dir whose only content is a
/// `[dessplay]` profile setting a marker option, and a second IPC
/// client reads the marker back after every step. Reproduce a failure
/// from the seed and step in the panic message.
#[tokio::test]
async fn dessplay_profile_holds_across_arbitrary_player_sequences() {
    let dir = tempfile::tempdir().unwrap();
    let video = encode_test_video(dir.path()).await;
    // The real placeholder artwork path: the same renderer the session
    // uses when a user is missing the file.
    let placeholder = dir.path().join("placeholder.png");
    dessplay::placeholder::render_to(
        &placeholder,
        &["Test Episode.mkv".into(), "You don't have this file".into()],
    )
    .unwrap();

    let conf_dir = dir.path().join("mpv-home");
    std::fs::create_dir(&conf_dir).unwrap();
    std::fs::write(
        conf_dir.join("mpv.conf"),
        format!("[dessplay]\nsub-pos={PROFILE_SUB_POS}\n"),
    )
    .unwrap();

    let socket = dir.path().join("profile.sock");
    let extra_args: Vec<String> = vec![
        "--vo=null".into(),
        "--ao=null".into(),
        "--force-window=no".into(),
        format!("--config-dir={}", conf_dir.display()),
    ];
    let launch = || async {
        MpvPlayer::launch("mpv", socket.clone(), &extra_args)
            .await
            .expect("launching mpv")
    };

    for seed in [1u64, 2] {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut player = launch().await;
        let mut probe = Probe::connect(&socket).await;
        // Applied while idle, before anything has loaded at all.
        expect_profile_marker(&mut probe, &format!("seed {seed}: idle after launch")).await;

        // The reported scenario, pinned: the placeholder loads first,
        // then the real episode swaps in over it.
        load_and_settle(&player, &placeholder, Some("Test Episode.mkv")).await;
        expect_profile_marker(&mut probe, &format!("seed {seed}: placeholder loaded")).await;
        load_and_settle(&player, &video, None).await;
        expect_profile_marker(
            &mut probe,
            &format!("seed {seed}: episode swapped over the placeholder"),
        )
        .await;

        for step in 0..10 {
            let context = format!("seed {seed} step {step}");
            match rng.random_range(0..8u8) {
                0 => load_and_settle(&player, &placeholder, Some("placeholder")).await,
                1 | 2 => {
                    let title = rng.random_bool(0.5).then_some("Test Episode.mkv");
                    load_and_settle(&player, &video, title).await;
                }
                3 | 4 => player.set_pause(rng.random_bool(0.5)).await.unwrap(),
                5 => player.seek(rng.random_range(0..4_000)).await.unwrap(),
                6 => player
                    .set_speed(rng.random_range(0.95..=1.05))
                    .await
                    .unwrap(),
                // The crash-relaunch path: a fresh mpv connection, whose
                // setup must re-apply the profile.
                _ => {
                    player.shutdown().await;
                    expect_event(&player, BUDGET, |e| {
                        matches!(e, PlayerEvent::Exited { .. }).then_some(())
                    })
                    .await;
                    player = launch().await;
                    probe = Probe::connect(&socket).await;
                }
            }
            drain_events(&player).await;
            expect_profile_marker(&mut probe, &context).await;
        }

        // Let this mpv die fully before the next seed reuses the socket.
        player.shutdown().await;
        expect_event(&player, BUDGET, |e| {
            matches!(e, PlayerEvent::Exited { .. }).then_some(())
        })
        .await;
    }
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
        .arg("--keep-open=always")
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

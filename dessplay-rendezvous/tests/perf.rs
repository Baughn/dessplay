//! Real-time performance *regression* tests for the interactive client.
//!
//! Background: the TUI lags during playback (~113% CPU) and downloads
//! (~200% CPU). The traced cause is that the session loop rebuilds and
//! re-pushes a full UI snapshot on *every* event — and each
//! `apply_snapshot` rebuilds the whole franchise grouping
//! (`franchise::franchises`, the profiled hot path) from scratch on the
//! UI thread. Position updates (local player ticks + every peer's
//! position datagram) fire that path many times per second.
//!
//! These two tests reproduce that load against the *real* `SessionLoop`
//! and the *real* UI loop (run headless via a ratatui `TestBackend`) and
//! assert the symptoms are gone:
//!   1. the UI thread stays responsive (<50ms to service new input);
//!   2. steady-state playback uses <10% of one CPU core.
//!
//! They are deliberately wall-clock, multi-threaded tests (paused sim
//! time would make "blocks 50ms" and "CPU %" meaningless), so the hard
//! thresholds are asserted only in **release** builds — `cargo test
//! --release`. A debug run executes the whole pipeline and just prints
//! the measured numbers, so it exercises the wiring without flaking on
//! unoptimized code. They are expected to FAIL on the unfixed code.
//!
//! Note on the fix: the UI-latency symptom is cured by merely throttling
//! how often snapshots reach the UI thread, but the CPU symptom is not —
//! `franchises()` is genuinely expensive (its per-component file scan
//! makes it grow ~quadratically with the library), so recomputing it even
//! a few times a second still burns a large fraction of a core. Getting
//! playback CPU under 10% needs the franchise grouping to stop being
//! rebuilt on position-only updates at all (memoize / dirty-flag), not
//! just rate-limited.
//!
//! The "playback load" is modelled by injecting `SetPlaybackPosition`
//! mutations at a high rate: each one drives the exact
//! `StateChanged -> snapshot -> apply_snapshot -> franchises()` chain a
//! real position tick does, and the rate stands in for "several users
//! playing at once" (which the bug needs to manifest). A large seeded
//! AniDB metadata library gives `franchises()` realistic work.

mod common;

use std::path::PathBuf;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use dessplay::actors::sync::{Mutation, SyncCommand};
use dessplay::config::Settings;
use dessplay::player::mock::MockFactory;
use dessplay::run::{SessionEnd, SessionLoop};
use dessplay::session::SessionShell;
use dessplay::storage::Storage;
use dessplay::ui::app::Ui;
use dessplay::ui::msg::UserAction;
use dessplay::ui::shell::{UiInput, run_ui_loop};
use dessplay_core::types::{AniDbMetadata, AniDbSeriesId, Ed2kHash, MetadataSource, UserId};
use tokio::sync::mpsc;
use tuirealm::ratatui::layout::Size;
use tuirealm::terminal::TestTerminalAdapter;

// Every seeded entry is its own AniDB series, so `franchises()` — whose
// per-component file collection scans the whole metadata map — builds this
// many components and rescans them all on every `apply_snapshot`. The two
// tests measure different symptoms of rebuilding it per event, so they use
// different (library, rate) operating points:
//
//   * CPU test — a realistic event rate (≈ a few users' position updates),
//     where the per-event *rebuild* dominates total CPU. The library is
//     sized so rebuilding-every-event clearly exceeds 10% of a core while
//     rebuilding-at-a-throttled-rate stays well under it (the per-event
//     plumbing cost at this rate is small).
//
//   * Latency test — a high event rate, needed so the *UI thread itself*
//     can't keep pace and input backs up behind a queue of full redraws.
//     One rebuild still fits under the 50ms budget, so a fix that stops
//     rebuilding on every snapshot drains the queue promptly.

/// CPU test: library size and a realistic ~100 updates/s flood. Only the
/// Linux-gated `playback_cpu_under_10_percent` reads these (CPU sampling is
/// implemented via /proc), so gate them too or they're dead code elsewhere.
#[cfg(target_os = "linux")]
const SEED_CPU: u32 = 4000;
#[cfg(target_os = "linux")]
const FLOOD_CPU: Duration = Duration::from_millis(10);

/// Latency test: smaller library, but a high ~1000 updates/s flood that
/// saturates the UI thread so the probe observes the backlog.
const SEED_LATENCY: u32 = 2500;
const FLOOD_LATENCY: Duration = Duration::from_millis(1);

/// A real `SessionLoop` + the real (headless) UI loop, wired exactly as
/// `run_interactive` wires them, minus the terminal. Holds the levers a
/// perf test needs: inject load (`sync`), inject UI latency probes
/// (`ui_probe`, a clone of the loop's own UI sender so probes land in the
/// same flooded channel), and add download churn (`actions` + `media_root`).
struct PerfRig {
    actions: mpsc::Sender<UserAction>,
    sync: mpsc::Sender<SyncCommand>,
    ui_probe: SyncSender<UiInput>,
    media_root: PathBuf,
    _loop: tokio::task::JoinHandle<SessionEnd>,
    _ui_thread: std::thread::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

/// A unique 16-byte ed2k key for seed entry `i`.
fn meta_hash(i: u32) -> Ed2kHash {
    let mut bytes = [0u8; 16];
    bytes[..4].copy_from_slice(&i.to_le_bytes());
    Ed2kHash(bytes)
}

async fn view_via(sync: &mpsc::Sender<SyncCommand>) -> dessplay_core::StateView {
    let (tx, rx) = tokio::sync::oneshot::channel();
    sync.send(SyncCommand::GetView(tx))
        .await
        .expect("sync gone");
    rx.await.expect("sync gone")
}

async fn perf_rig(harness: &Harness, name: &str, nonce: u128, series_count: u32) -> PerfRig {
    let mut handle = harness.client(name, nonce);
    // Keep the client offline for the whole test. Seeding and the position
    // flood are then purely local: no per-op server round-trip, no echoes
    // flooding back, so seeding a big library is fast and deterministic.
    // The bug under test is entirely local anyway — each local mutation
    // drives the same `changed() -> StateChanged -> snapshot ->
    // apply_snapshot -> franchises()` chain a real position tick does; the
    // server/peers are only the *source* of that event rate, which we
    // substitute by injecting locally.
    harness.isolate(name);
    let dir = tempfile::tempdir().expect("tempdir");
    let media_root = dir.path().join("media");
    std::fs::create_dir_all(&media_root).expect("media dir");
    let cache_dir = dir.path().join("cache");
    std::fs::create_dir_all(&cache_dir).expect("cache dir");

    let shell = SessionShell::new(
        UserId::new(name),
        MockFactory::new([]),
        sim_clock(0),
        dessplay::actors::file::FileConfig {
            storage: Storage::open_in_memory().expect("in-memory storage"),
            media_roots: vec![media_root.clone()],
            retention: dessplay::config::CacheRetention::default(),
            cache_dir,
            clock: sim_clock(0),
            download: dessplay::download::DownloadConfig::default(),
            upload_limit: None,
            scan_interval: None,
        },
        handle.sync.clone(),
        handle.network.clone(),
    );

    let sync = handle.sync.clone();
    let (action_tx, action_rx) = mpsc::channel(64);
    let (ui_tx, ui_rx) = std::sync::mpsc::sync_channel::<UiInput>(64);
    let ui_probe = ui_tx.clone();

    // Seed the metadata library locally (we're isolated, so these apply to
    // local state and buffer offline — no network traffic). Seed before the
    // loop exists so no per-seed snapshot storm runs through the sync actor.
    // Drain `handle.events` as we go just to keep the bounded event channel
    // from filling with any stray connection-state events.
    for i in 0..series_count {
        sync.send(SyncCommand::Mutate(Box::new(Mutation::SetAniDbMetadata {
            hash: meta_hash(i),
            metadata: Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: format!("Series {i}"),
                series_id: Some(AniDbSeriesId(i + 1)),
                episode_number: Some("1".into()),
            }),
        })))
        .await
        .expect("sync gone");
        while handle.events.try_recv().is_ok() {}
    }
    let want = series_count as usize;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        while handle.events.try_recv().is_ok() {}
        if view_via(&sync).await.anidb_metadata.len() >= want {
            break;
        }
        assert!(Instant::now() < deadline, "metadata seed never applied");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Now spawn the session loop; it owns `handle` and drains its events
    // from here on.
    let mut session = SessionLoop {
        handle,
        shell,
        actions: action_rx,
        ui: ui_tx,
        storage: Storage::open_in_memory().expect("in-memory storage"),
        db_path: dir.path().join("session.db"),
        me: UserId::new(name),
        settings: Settings::default(),
        observed_fingerprint: Box::new(|| None),
        pin_pending: false,
        server_addr: "sim".into(),
        start: Instant::now(),
    };
    let loop_task = tokio::spawn(async move { session.run().await });

    // The real UI loop, headless. Each input drives `apply_snapshot` +
    // `draw` exactly as production does, so `refresh_series` /
    // `franchises()` runs for real on the UI thread.
    let settings = Settings {
        username: Some(name.into()),
        password: Some("hunter2".into()),
        ..Settings::default()
    };
    let ui = Ui::new(UserId::new(name), settings, vec![media_root.clone()]);
    let ui_actions = action_tx.clone();
    let ui_thread = std::thread::spawn(move || {
        let mut adapter = TestTerminalAdapter::new(Size::new(120, 40)).expect("test adapter");
        run_ui_loop(ui, ui_rx, ui_actions, &mut adapter);
    });

    PerfRig {
        actions: action_tx,
        sync,
        ui_probe,
        media_root,
        _loop: loop_task,
        _ui_thread: ui_thread,
        _dir: dir,
    }
}

/// Inject `SetPlaybackPosition` mutations every `interval` until the rig
/// drops. Each one fires the per-event snapshot rebuild; the rate stands
/// in for several users' position updates arriving at once.
fn start_position_flood(
    sync: mpsc::Sender<SyncCommand>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut pos: u64 = 0;
        loop {
            pos += 1000;
            if sync
                .send(SyncCommand::Mutate(Box::new(
                    Mutation::SetPlaybackPosition {
                        position_millis: pos,
                    },
                )))
                .await
                .is_err()
            {
                break;
            }
            tokio::time::sleep(interval).await;
        }
    })
}

/// Total process CPU seconds (all threads) from `/proc/self/stat`:
/// utime + stime, in clock ticks divided by `_SC_CLK_TCK` (100 on Linux).
#[cfg(target_os = "linux")]
fn process_cpu_seconds() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").expect("read /proc/self/stat");
    // Field 2 (comm) is parenthesised and may contain spaces/parens, so
    // index past the LAST ')' — every numeric field follows it.
    let rest = &stat[stat.rfind(')').expect("comm close paren") + 1..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // fields[0] is field 3 (state); utime is field 14, stime field 15.
    let utime: u64 = fields[11].parse().expect("utime");
    let stime: u64 = fields[12].parse().expect("stime");
    (utime + stime) as f64 / 100.0
}

/// How long a single probe will wait before giving up and reporting the
/// cap. Far above the 50ms budget, so a capped result still registers as a
/// failure — it just keeps an unoptimized (debug) run, where one franchise
/// rebuild can take seconds, from blocking tens of seconds per probe.
const PROBE_CAP: Duration = Duration::from_secs(2);

/// Send one latency probe into the UI channel and measure how long until
/// the UI loop stamps it. The time spent waiting for channel space (the
/// snapshot backlog draining ahead of the probe) is exactly the lag we
/// want to catch, so it counts. Bounded by [`PROBE_CAP`] so a saturated
/// debug build can't block the test for tens of seconds.
fn probe_latency(ui_probe: &SyncSender<UiInput>) -> Option<Duration> {
    use std::sync::mpsc::TrySendError;
    let cell = Arc::new(Mutex::new(None));
    let t0 = Instant::now();
    // Enqueue (the channel may be full while the UI thread is behind).
    let mut pending = Some(UiInput::Probe(cell.clone()));
    while let Some(probe) = pending.take() {
        match ui_probe.try_send(probe) {
            Ok(()) => {}
            Err(TrySendError::Full(probe)) => {
                if t0.elapsed() >= PROBE_CAP {
                    return Some(PROBE_CAP); // never even got a slot in time
                }
                pending = Some(probe);
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(TrySendError::Disconnected(_)) => return None,
        }
    }
    // Enqueued: wait for the UI loop to handle it.
    loop {
        if let Some(handled) = *cell.lock().unwrap() {
            return Some(handled.saturating_duration_since(t0));
        }
        if t0.elapsed() >= PROBE_CAP {
            return Some(PROBE_CAP);
        }
        std::thread::sleep(Duration::from_micros(200));
    }
}

/// Steady-state playback must not peg a core. The unfixed loop rebuilds
/// the franchise grouping on every position update, pinning ~1+ cores.
#[tokio::test(flavor = "multi_thread")]
#[cfg(target_os = "linux")]
async fn playback_cpu_under_10_percent() {
    let harness = Harness::new(0xC0FFEE);
    let rig = perf_rig(&harness, "kim", 1, SEED_CPU).await;

    // Several users' worth of position updates.
    let _flood = start_position_flood(rig.sync.clone(), FLOOD_CPU);

    // Warm up (player/UI cold start, first franchise build), then sample
    // process CPU over a fixed wall-clock window.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let cpu_before = process_cpu_seconds();
    let wall = Instant::now();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let cpu_used = process_cpu_seconds() - cpu_before;
    let frac = cpu_used / wall.elapsed().as_secs_f64();

    eprintln!("playback CPU = {:.1}% of one core", frac * 100.0);
    if !cfg!(debug_assertions) {
        assert!(
            frac < 0.10,
            "playback used {:.1}% of a core (want <10%)",
            frac * 100.0
        );
    }
}

/// The UI thread must stay responsive during an active session. Under the
/// snapshot flood the bounded UI channel saturates and new input queues
/// behind a backlog of full redraws — the lag the user reported.
#[tokio::test(flavor = "multi_thread")]
async fn ui_responsive_during_playback_and_download() {
    let harness = Harness::new(0xBADCAB);
    let rig = perf_rig(&harness, "kim", 1, SEED_LATENCY).await;

    let _flood = start_position_flood(rig.sync.clone(), FLOOD_LATENCY);

    // Download leg: a couple of real files hashed + added, so the
    // hashing-progress and file-output UI traffic piles onto the flood.
    for i in 0..2u8 {
        let path = rig.media_root.join(format!("dl{i}.mkv"));
        std::fs::write(&path, vec![i; 8 * 1024 * 1024]).expect("write file");
        rig.actions
            .send(UserAction::HashAndAdd { path, after: None })
            .await
            .expect("loop gone");
    }

    // Warm up, then probe responsiveness on a dedicated thread (blocking
    // sends must not sit on a tokio worker).
    tokio::time::sleep(Duration::from_secs(1)).await;
    let ui_probe = rig.ui_probe.clone();
    let worst = tokio::task::spawn_blocking(move || {
        let mut worst = Duration::ZERO;
        let until = Instant::now() + Duration::from_secs(3);
        while Instant::now() < until {
            if let Some(latency) = probe_latency(&ui_probe) {
                worst = worst.max(latency);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        worst
    })
    .await
    .expect("probe thread panicked");

    eprintln!("worst UI input latency = {worst:?}");
    if !cfg!(debug_assertions) {
        assert!(
            worst < Duration::from_millis(50),
            "UI thread blocked for {worst:?} (want <50ms)"
        );
    }
}

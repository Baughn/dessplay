//! Audible artifacts from drift slew, measured on real mpv output.
//!
//! Drift correction changes mpv's `speed` in small steps (±2%,
//! pitch-corrected). A *sustained* corrected speed is transparent, but a
//! splice in the audio stream around a speed change (a dropped or
//! repeated stretch, a cut mid-waveform, a dip to silence) is a
//! broadband click. The test plays a chord of pure tones through mpv
//! while stepping speed the way the drift controller does, captures the
//! output with `--ao=pcm`, and measures, frame by frame, how much energy
//! lands *outside* the tones' spectral bins. A clean corrected stream
//! keeps the energy in the chord's bins. A click, dropout, or pitch warble
//! does not.
//!
//! The detector itself is checked below without mpv (always runs). The
//! mpv journey needs `--features mpv-tests`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::f64::consts::PI;

use proptest::prelude::*;

const RATE: u32 = 48_000;
/// Analysis frame length. Tones sit exactly on bin centres, so a Hann
/// window confines each one to its bin ±1.
const FRAME: usize = 2048;
const HOP: usize = FRAME / 4;
/// Chord bins (bin width 23.4375Hz at 48kHz / 2048): harmonics 1, 3, 5
/// and 9 of 187.5Hz, so the chord repeats every [`PERIOD`] samples.
/// That period must be well inside scaletempo2's ±20ms similarity
/// search, so a *sustained* stretch splices whole periods and is
/// sample-exact. A chord with a longer period (an incommensurate one
/// repeats only every frame, 43ms) puts WSOLA's inherent ~-29dB
/// steady-state distortion into every stretched frame, and that would
/// swamp the transitions this test is about.
const TONE_BINS: [usize; 4] = [8, 24, 40, 72];
/// The chord's period in samples: `FRAME` / gcd of the bins.
const PERIOD: usize = FRAME / 8;
const TONE_AMPLITUDE: f64 = 0.2;
/// Bins either side of each tone still counted as the tone (Hann main
/// lobe is ±2 bins).
const TONE_GUARD: usize = 2;
/// The worst frame may put at most this much of its energy outside the
/// chord's bins. Clean output sits near -141dB and the clicks this
/// guards against near -10dB. A 1-sample jump, the smallest splice, lands
/// around -31dB. See the calibration note in the journey test.
const MAX_SPLATTER_DB: f64 = -60.0;

fn chord(seconds: f64, phases: &[f64; 4]) -> Vec<f32> {
    let n = (seconds * f64::from(RATE)) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64;
            TONE_BINS
                .iter()
                .zip(phases)
                .map(|(&bin, &phase)| {
                    TONE_AMPLITUDE * (2.0 * PI * bin as f64 * t / FRAME as f64 + phase).sin()
                })
                .sum::<f64>() as f32
        })
        .collect()
}

/// One analysed frame: its start (in samples) and the out-of-chord
/// share of its energy, in dB.
#[derive(Debug, Clone, Copy)]
struct Splatter {
    #[cfg_attr(not(feature = "mpv-tests"), allow(dead_code))]
    at: usize,
    db: f64,
}

/// Per-frame out-of-chord energy share over the non-silent span of
/// `samples`. Leading/trailing silence and one frame at each edge are
/// skipped: the onset and end of the stream are steps by construction.
fn splatter(samples: &[f32]) -> Vec<Splatter> {
    let loud = |s: &&f32| s.abs() > 1e-3;
    let Some(first) = samples.iter().position(|s| loud(&s)) else {
        return Vec::new();
    };
    let last = samples.len() - samples.iter().rev().position(|s| loud(&s)).unwrap();
    let window: Vec<f64> = (0..FRAME)
        .map(|n| 0.5 - 0.5 * (2.0 * PI * n as f64 / FRAME as f64).cos())
        .collect();
    let mut out = Vec::new();
    let mut at = first + FRAME;
    while at + 2 * FRAME <= last {
        let x: Vec<f64> = samples[at..at + FRAME]
            .iter()
            .zip(&window)
            .map(|(&s, &w)| f64::from(s) * w)
            .collect();
        // Parseval: the full two-sided spectrum's energy is N·Σx².
        let total = FRAME as f64 * x.iter().map(|v| v * v).sum::<f64>();
        // The chord's bins, each counted twice (the mirrored negative
        // frequency carries the same energy).
        let chord: f64 = TONE_BINS
            .iter()
            .flat_map(|&b| b - TONE_GUARD..=b + TONE_GUARD)
            .map(|k| {
                let (re, im) = x.iter().enumerate().fold((0.0, 0.0), |(re, im), (n, v)| {
                    let a = 2.0 * PI * (k * n) as f64 / FRAME as f64;
                    (re + v * a.cos(), im - v * a.sin())
                });
                2.0 * (re * re + im * im)
            })
            .sum();
        let db = 10.0 * ((total - chord).max(total * 1e-15) / total).log10();
        out.push(Splatter { at, db });
        at += HOP;
    }
    out
}

fn worst(frames: &[Splatter]) -> Splatter {
    *frames
        .iter()
        .max_by(|a, b| a.db.total_cmp(&b.db))
        .expect("no analysable frames")
}

fn phases() -> impl Strategy<Value = [f64; 4]> {
    [0.0..2.0 * PI, 0.0..2.0 * PI, 0.0..2.0 * PI, 0.0..2.0 * PI]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(
        dessplay_core::test_support::proptest_cases(16)
    ))]

    /// An untouched chord is clean in every frame, whatever its phases.
    #[test]
    fn clean_chord_has_no_splatter(phases in phases()) {
        let w = worst(&splatter(&chord(0.5, &phases)));
        prop_assert!(w.db < MAX_SPLATTER_DB - 40.0, "clean chord splattered: {w:?}");
    }

    /// The artifact class drift slew can produce, anywhere in the
    /// stream: a skipped stretch (content jump), a repeated stretch, or
    /// a dropout to silence. Each must trip the detector. The shortest
    /// case, a 1-sample jump, is the hardest to catch. Jumping a whole
    /// number of periods is seamless by construction (it's what WSOLA
    /// does on purpose), so those lengths are excluded from jumps but
    /// not from dropouts.
    #[test]
    fn splices_are_detected(
        phases in phases(),
        at in 12_000usize..36_000,
        len in (1usize..960).prop_filter("whole-period jump", |l| l % PERIOD != 0),
        kind in 0u8..3,
    ) {
        let clean = chord(1.0, &phases);
        let spliced: Vec<f32> = match kind {
            0 => [&clean[..at], &clean[at + len..]].concat(),
            1 => [&clean[..at], &clean[at - len..]].concat(),
            _ => [&clean[..at], &vec![0.0; len], &clean[at + len..]].concat(),
        };
        let w = worst(&splatter(&spliced));
        prop_assert!(
            w.db > MAX_SPLATTER_DB,
            "kind {kind} splice of {len} samples at {at} went unnoticed: {w:?}"
        );
    }
}

/// Mono 32-bit float WAV.
#[cfg(feature = "mpv-tests")]
fn write_wav(path: &std::path::Path, samples: &[f32]) {
    let data_len = (samples.len() * 4) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&RATE.to_le_bytes());
    b.extend_from_slice(&(RATE * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}

/// Read back mpv's `--ao=pcm` output (forced to mono float): the first
/// channel of the `data` chunk, to end of file. The header's sizes are
/// only patched on a clean close, so they are not trusted.
#[cfg(feature = "mpv-tests")]
fn read_wav(path: &std::path::Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap();
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let mut i = 12;
    let mut channels = 1usize;
    loop {
        let id = &bytes[i..i + 4];
        let len = u32::from_le_bytes(bytes[i + 4..i + 8].try_into().unwrap()) as usize;
        if id == b"fmt " {
            let tag = u16_at(i + 8);
            // 0xFFFE: WAVE_FORMAT_EXTENSIBLE; the subformat GUID's first
            // two bytes carry the real tag.
            let tag = if tag == 0xFFFE {
                u16_at(i + 8 + 24)
            } else {
                tag
            };
            assert_eq!(tag, 3, "expected float output");
            assert_eq!(u16_at(i + 8 + 14), 32, "expected 32-bit float");
            channels = usize::from(u16_at(i + 10));
            assert_eq!(
                u32::from_le_bytes(bytes[i + 12..i + 16].try_into().unwrap()),
                RATE
            );
        } else if id == b"data" {
            return bytes[i + 8..]
                .chunks_exact(4 * channels)
                .map(|c| f32::from_le_bytes(c[..4].try_into().unwrap()))
                .collect();
        }
        i += 8 + len + (len & 1);
    }
}

/// Speed schedule, as (ms of playback since unpause, speed): the drift
/// controller's shapes. Engaging from 1.0, a proportional taper in
/// quantized steps, the release back to exactly 1.0, the other
/// direction, and a short 1.0 → 0.98 → 1.0 blip. Timed by wall clock,
/// not by reported position: `arealtime` paces the stream's input at
/// real time, while time-pos under an untimed AO lurches (it ran 800ms
/// of position in 170ms of wall clock in the first draft of this test).
#[cfg(feature = "mpv-tests")]
const SCHEDULE: [(u64, f64); 10] = [
    (1_000, 0.98),
    (2_000, 0.988),
    (2_400, 0.996),
    (2_800, 1.0),
    (3_200, 1.02),
    (3_600, 1.012),
    (4_000, 1.0),
    (4_400, 0.98),
    (4_600, 1.0),
    (5_000, 1.0),
];

/// How much longer than its input [`SCHEDULE`] makes the output, in
/// seconds (slow segments dominate: about +18ms).
#[cfg(feature = "mpv-tests")]
fn schedule_stretch() -> f64 {
    SCHEDULE
        .windows(2)
        .map(|w| {
            let media = (w[1].0 - w[0].0) as f64 / 1000.0;
            media / w[0].1 - media
        })
        .sum()
}

/// The production player, stepping speed through [`SCHEDULE`] over a
/// pure chord, must not splatter energy outside the chord's bins at any
/// point, whether at a transition or in between.
///
/// Playback goes through [`MpvPlayer::launch`], so setup (the
/// `[dessplay]` profile, the audio filter chain) is exactly what a
/// session gets, and speed goes through `Player::set_speed` like the
/// drift actor's. `--ao=pcm` is untimed (it writes as fast as mpv
/// decodes), so a user `lavfi=[arealtime]` filter paces the stream to
/// real time. Otherwise the file would finish before the first IPC
/// speed command landed. Where exactly each change lands varies by a few
/// ms between runs; the detector doesn't care where a splice is.
///
/// The chord repeats every [`PERIOD`] samples, so WSOLA's steady state
/// splices it sample-exactly. The test judges splices around speed
/// changes, not WSOLA's inherent artifacts on real audio. Because that
/// makes a stretched chord spectrally identical to an unstretched one,
/// the output's length is also checked against the schedule's stretch,
/// so a player that ignored speed can't pass (an early draft did exactly
/// that, unnoticed).
///
/// Calibration (2026-09-24, mpv 0.41.0; results identical across runs):
/// with mpv's automatic pitch correction, the releases back to 1.0 put
/// up to -9.8dB outside the chord (engages and mid-correction steps were
/// clean), and the drains dropped about 14ms of audio. With the
/// resident filter, the worst frame is -141.6dB, float rounding. Mixed
/// with real audio, a -60dB splice would be buried.
#[cfg(feature = "mpv-tests")]
#[tokio::test]
async fn speed_transitions_do_not_click() {
    use std::time::Duration;

    use dessplay::player::mpv::MpvPlayer;
    use dessplay::player::{Player, PlayerEvent};

    const BUDGET: Duration = Duration::from_secs(15);

    async fn expect_event<T>(
        player: &MpvPlayer,
        mut pred: impl FnMut(&PlayerEvent) -> Option<T>,
    ) -> T {
        let deadline = tokio::time::Instant::now() + BUDGET;
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

    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("chord.wav");
    write_wav(&input, &chord(5.5, &[0.0, 1.0, 2.0, 3.0]));
    let output = dir.path().join("out.wav");
    // An empty config dir: the user's mpv.conf must not colour the audio.
    let conf_dir = dir.path().join("mpv-home");
    std::fs::create_dir(&conf_dir).unwrap();

    let player = MpvPlayer::launch(
        "mpv",
        dir.path().join("audio.sock"),
        &[
            "--vo=null".into(),
            "--force-window=no".into(),
            format!("--config-dir={}", conf_dir.display()),
            "--ao=pcm".into(),
            format!("--ao-pcm-file={}", output.display()),
            "--audio-format=float".into(),
            "--af=lavfi=[arealtime]".into(),
        ],
    )
    .await
    .expect("launching mpv");

    player.load(&input, None).await.unwrap();
    expect_event(&player, |e| matches!(e, PlayerEvent::Loaded).then_some(())).await;
    player.set_pause(false).await.unwrap();
    let start = tokio::time::Instant::now();
    for (at, speed) in SCHEDULE {
        tokio::time::sleep_until(start + Duration::from_millis(at)).await;
        player.set_speed(speed).await.unwrap();
    }
    expect_event(&player, |e| matches!(e, PlayerEvent::Eof).then_some(())).await;
    // A clean quit, so ao_pcm flushes everything it has written.
    player.shutdown().await;
    expect_event(&player, |e| {
        matches!(e, PlayerEvent::Exited { .. }).then_some(())
    })
    .await;
    let samples = read_wav(&output);
    let loud: Vec<usize> = samples
        .iter()
        .enumerate()
        .filter(|(_, s)| s.abs() > 1e-3)
        .map(|(i, _)| i)
        .collect();
    let span = (loud[loud.len() - 1] - loud[0]) as f64 / f64::from(RATE);
    let stretch = span - 5.5;
    let expected = schedule_stretch();
    let frames = splatter(&samples);
    assert!(
        frames.len() > 300,
        "only {} frames analysed; did the output get cut short?",
        frames.len()
    );
    let mut sorted: Vec<f64> = frames.iter().map(|f| f.db).collect();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    let w = worst(&frames);
    let mut by_db = frames.clone();
    by_db.sort_by(|a, b| b.db.total_cmp(&a.db));
    for f in by_db.iter().take(5) {
        eprintln!("  {:.3}s {:.1}dB", f.at as f64 / f64::from(RATE), f.db);
    }
    eprintln!(
        "stretch: {:.1}ms (expected {:.1}ms)",
        stretch * 1000.0,
        expected * 1000.0
    );
    eprintln!(
        "splatter: median {median:.1}dB, worst {:.1}dB at {:.3}s",
        w.db,
        w.at as f64 / f64::from(RATE)
    );
    assert!(
        w.db < MAX_SPLATTER_DB,
        "audible artifact: {:.1}dB of a frame's energy outside the chord at {:.3}s of output \
         (median frame {median:.1}dB, threshold {MAX_SPLATTER_DB}dB)",
        w.db,
        w.at as f64 / f64::from(RATE)
    );
    // A lower bound only: arealtime paces in bursts, so where each
    // change lands in the stream wobbles by tens of ms. Dropped audio
    // (the pre-fix drain truncates at every release) or ignored speed
    // commands both land well under it.
    assert!(
        stretch > expected / 2.0,
        "output is {:.1}ms longer than the input, expected about {:.1}ms from the schedule \
         (was speed applied? was audio dropped?)",
        stretch * 1000.0,
        expected * 1000.0
    );
}

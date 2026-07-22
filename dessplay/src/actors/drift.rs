//! Drift-correction speed controller: turns a stream of measured drift
//! deltas (position-reference target minus our estimated position) into
//! player speed commands and hard seeks.
//!
//! Pure logic, no clocks, no I/O — the player actor feeds it one delta
//! per authority sample and applies whatever it returns, which keeps the
//! control behavior testable sample-by-sample (docs/testing-strategy.md).
//!
//! The shape of the controller is dictated by what is *audible*.
//! Measured against real mpv (2026-07-22): a **sustained** ±2%
//! pitch-corrected slew is spectrally indistinguishable from baseline,
//! but every speed **transition** is a broadband click, and a short
//! 1.0 → 0.98 → 1.0 blip puts artifacts within 10dB of the signal. So
//! the controller's job is to spend as few transitions as possible:
//!
//! - **Hysteresis**: engage above [`DRIFT_ENGAGE_MILLIS`], but keep
//!   correcting until the drift is under [`DRIFT_RELEASE_MILLIS`]. The
//!   original bang-bang controller engaged and released at the same
//!   threshold, which parked the client at the edge of its own deadband
//!   — every wobble re-crossed it, firing hundreds of full-amplitude
//!   blips per hour (the 2026-07-22 regression).
//! - **Debounce**: engaging (and hard-seeking) takes [`ENGAGE_RUN`]
//!   consecutive out-of-band samples, so one noisy sample never costs a
//!   transition (samples arrive at ~10Hz; the added latency is ~200ms).
//! - **Proportional slew**: the speed is `1 + delta/τ`, clamped to
//!   ±[`SLEW_RATE`] — full correction speed far out, tapering off as the
//!   gap closes, so convergence glides into 1.0 instead of stepping.
//!   Mid-correction updates are quantized ([`SLEW_QUANTUM`]) *and*
//!   rate-limited ([`SLEW_RECOMMAND_SAMPLES`]), so however noisy the
//!   deltas, the taper spends a few sub-1% steps per correction instead
//!   of chasing every sample.

/// Drift below this (while idle) is ignored; sustained drift at or above
/// it engages a correction.
pub const DRIFT_ENGAGE_MILLIS: u64 = 150;
/// An engaged correction runs until the drift falls below this — well
/// clear of the engage threshold, so the corrected client parks near
/// zero rather than on the trigger.
pub const DRIFT_RELEASE_MILLIS: u64 = 25;
/// Sustained drift beyond this is hard-seeked instead of slewed.
pub const DRIFT_HARD_SEEK_MILLIS: u64 = 3_000;
/// Maximum slew, as a speed delta (±2%; pitch-corrected and, when
/// sustained, measurably inaudible).
pub const SLEW_RATE: f64 = 0.02;
/// Proportional gain: aim to close the gap over roughly this horizon.
/// The slew hits ±[`SLEW_RATE`] at `SLEW_RATE * SLEW_TAU_MILLIS` = 200ms
/// of drift and tapers below it.
const SLEW_TAU_MILLIS: f64 = 10_000.0;
/// Minimum speed change worth re-commanding mpv for. Set above the
/// target-speed wobble that realistic sample noise induces (tens of ms
/// of jitter → a few thousandths of speed), so noise alone almost never
/// re-commands — only genuine gap decay does, a few steps per
/// correction, each a quarter of the old controller's 2% blip.
const SLEW_QUANTUM: f64 = 0.008;
/// Minimum samples between mid-correction speed updates (~1s at the
/// playing sample cadence). Sample noise can exceed what [`SLEW_QUANTUM`]
/// filters (the 2026-07-21 logs show consecutive deltas 100ms apart);
/// this bounds the transition rate structurally no matter how noisy the
/// deltas get. Releases are exempt — snapping back to 1.0 is the point
/// of converging.
const SLEW_RECOMMAND_SAMPLES: u32 = 10;
/// Consecutive out-of-band samples needed to engage or hard-seek.
const ENGAGE_RUN: u32 = 3;

/// What the player actor should do with the latest drift sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DriftAction {
    /// In band (or debouncing): leave the player alone.
    None,
    /// Command this playback speed.
    SetSpeed(f64),
    /// Seek to the target; the controller has reset itself.
    HardSeek,
}

/// Sample-by-sample drift controller. One per player actor.
#[derive(Clone, Debug, Default)]
pub struct DriftController {
    /// The currently commanded slew; `None` while idle (speed 1.0).
    slew: Option<f64>,
    /// Consecutive idle samples at or beyond the engage threshold.
    engage_run: u32,
    /// Consecutive samples beyond the hard-seek threshold.
    seek_run: u32,
    /// Samples since the last speed command (saturating).
    since_command: u32,
}

impl DriftController {
    /// A fresh, idle controller.
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget any correction in progress — the caller is resetting the
    /// player speed out-of-band (file load, player death, ReleaseSlew).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Feed one measured drift delta (target minus current, positive =
    /// we are behind) and get the action to apply.
    pub fn observe(&mut self, delta_millis: i64) -> DriftAction {
        let magnitude = delta_millis.unsigned_abs();
        self.since_command = self.since_command.saturating_add(1);

        if magnitude > DRIFT_HARD_SEEK_MILLIS {
            // Debounced like engagement: a hard seek is the most jarring
            // correction of all, so one bogus sample must never fire it.
            self.seek_run += 1;
            if self.seek_run >= ENGAGE_RUN {
                self.reset();
                return DriftAction::HardSeek;
            }
            return DriftAction::None;
        }
        self.seek_run = 0;

        if self.slew.is_none() {
            // Idle: only sustained out-of-band drift engages.
            if magnitude < DRIFT_ENGAGE_MILLIS {
                self.engage_run = 0;
                return DriftAction::None;
            }
            self.engage_run += 1;
            if self.engage_run < ENGAGE_RUN {
                return DriftAction::None;
            }
        }
        self.engage_run = 0;

        // Correcting (or engaging just now).
        if magnitude < DRIFT_RELEASE_MILLIS {
            self.slew = None;
            self.since_command = 0;
            return DriftAction::SetSpeed(1.0);
        }
        let target = 1.0 + (delta_millis as f64 / SLEW_TAU_MILLIS).clamp(-SLEW_RATE, SLEW_RATE);
        if let Some(current) = self.slew {
            // Mid-correction update: quantized and rate-limited.
            if (target - current).abs() < SLEW_QUANTUM
                || self.since_command < SLEW_RECOMMAND_SAMPLES
            {
                return DriftAction::None;
            }
        }
        self.slew = Some(target);
        self.since_command = 0;
        DriftAction::SetSpeed(target)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// Feed `n` identical deltas; return the non-`None` actions.
    fn feed(ctl: &mut DriftController, delta: i64, n: usize) -> Vec<DriftAction> {
        (0..n)
            .map(|_| ctl.observe(delta))
            .filter(|a| *a != DriftAction::None)
            .collect()
    }

    /// Deltas that never reach the engage threshold do nothing, however
    /// long they persist.
    #[test]
    fn sub_engage_drift_is_ignored() {
        let mut ctl = DriftController::new();
        for _ in 0..100 {
            assert_eq!(
                ctl.observe(DRIFT_ENGAGE_MILLIS as i64 - 1),
                DriftAction::None
            );
        }
    }

    /// A single out-of-band sample — noise — never costs a transition;
    /// only a sustained run engages.
    #[test]
    fn engagement_is_debounced() {
        let mut ctl = DriftController::new();
        assert_eq!(ctl.observe(300), DriftAction::None);
        assert_eq!(ctl.observe(300), DriftAction::None);
        // The run is broken: back to in-band.
        assert_eq!(ctl.observe(50), DriftAction::None);
        assert_eq!(ctl.observe(300), DriftAction::None);
        assert_eq!(ctl.observe(300), DriftAction::None);
        // Third consecutive sample: engage, at the proportional speed.
        assert_eq!(ctl.observe(300), DriftAction::SetSpeed(1.02));
    }

    /// The slew is proportional below 200ms of drift and clamped at ±2%
    /// beyond it, in both directions.
    #[test]
    fn slew_is_proportional_and_clamped() {
        let mut ctl = DriftController::new();
        assert_eq!(feed(&mut ctl, 160, 3), vec![DriftAction::SetSpeed(1.016)]);
        let mut ctl = DriftController::new();
        assert_eq!(feed(&mut ctl, 2_500, 3), vec![DriftAction::SetSpeed(1.02)]);

        let mut ctl = DriftController::new();
        assert_eq!(
            feed(&mut ctl, -160, 3),
            vec![DriftAction::SetSpeed(1.0 - 0.016)]
        );
        let mut ctl = DriftController::new();
        assert_eq!(
            feed(&mut ctl, -2_500, 3),
            vec![DriftAction::SetSpeed(1.0 - SLEW_RATE)]
        );
    }

    /// Hysteresis: once engaged, the correction keeps running below the
    /// engage threshold and releases only under the release threshold —
    /// parking the client near zero, not on the trigger (the 2026-07-22
    /// limit cycle).
    #[test]
    fn correction_runs_to_release_not_to_the_engage_edge() {
        let mut ctl = DriftController::new();
        assert_eq!(feed(&mut ctl, 200, 3), vec![DriftAction::SetSpeed(1.02)]);
        // Still correcting well below the engage threshold, tapering
        // (one rate-limited update).
        assert_eq!(feed(&mut ctl, 100, 10), vec![DriftAction::SetSpeed(1.01)]);
        // A smaller change than the quantum: no re-command.
        assert_eq!(feed(&mut ctl, 50, 10), vec![]);
        // Under the release threshold: back to exactly 1.0, immediately
        // (releases are exempt from the rate limit).
        assert_eq!(
            ctl.observe(DRIFT_RELEASE_MILLIS as i64 - 1),
            DriftAction::SetSpeed(1.0)
        );
        // And now idle again: the same small drift does nothing.
        assert_eq!(feed(&mut ctl, 100, 10), vec![]);
    }

    /// Near-identical consecutive deltas don't re-command mpv.
    #[test]
    fn speed_updates_are_quantized() {
        let mut ctl = DriftController::new();
        assert_eq!(feed(&mut ctl, 200, 3), vec![DriftAction::SetSpeed(1.02)]);
        assert_eq!(feed(&mut ctl, 210, 10), vec![]);
        assert_eq!(feed(&mut ctl, 195, 10), vec![]);
        // A real change re-commands (once).
        assert_eq!(feed(&mut ctl, 100, 10), vec![DriftAction::SetSpeed(1.01)]);
    }

    /// Even a quantum-crossing change waits out the re-command interval:
    /// however noisy the deltas, mid-correction speed updates are capped
    /// at one per [`SLEW_RECOMMAND_SAMPLES`].
    #[test]
    fn mid_correction_updates_are_rate_limited() {
        let mut ctl = DriftController::new();
        assert_eq!(feed(&mut ctl, 2_000, 3), vec![DriftAction::SetSpeed(1.02)]);
        // A genuine large change right after the engage: held until the
        // interval elapses, then applied once.
        assert_eq!(
            feed(&mut ctl, 100, SLEW_RECOMMAND_SAMPLES as usize - 1),
            vec![]
        );
        assert_eq!(ctl.observe(100), DriftAction::SetSpeed(1.01));
    }

    /// A hard seek takes the same sustained-run evidence as engagement,
    /// and resets the controller.
    #[test]
    fn hard_seek_is_debounced_and_resets() {
        let mut ctl = DriftController::new();
        assert_eq!(ctl.observe(10_000), DriftAction::None);
        // An in-band sample breaks the run.
        assert_eq!(ctl.observe(10), DriftAction::None);
        assert_eq!(ctl.observe(10_000), DriftAction::None);
        assert_eq!(ctl.observe(10_000), DriftAction::None);
        assert_eq!(ctl.observe(10_000), DriftAction::HardSeek);
        // Reset: small drift right after the seek does nothing.
        assert_eq!(ctl.observe(100), DriftAction::None);
    }

    proptest::proptest! {
        /// Whatever the input, the commanded speed never leaves the
        /// ±SLEW_RATE band.
        #[test]
        fn commanded_speed_stays_in_the_slew_band(
            deltas in proptest::collection::vec(-5_000i64..5_000, 1..200),
        ) {
            let mut ctl = DriftController::new();
            for delta in deltas {
                if let DriftAction::SetSpeed(speed) = ctl.observe(delta) {
                    proptest::prop_assert!(
                        (1.0 - SLEW_RATE..=1.0 + SLEW_RATE).contains(&speed),
                        "speed {speed} outside the slew band"
                    );
                }
            }
        }

        /// The closed-loop limit-cycle property (the 2026-07-22
        /// regression, pure and seedable): a leader with a bounded
        /// clock-rate mismatch and bounded sample noise must not make
        /// the controller flap. Ten simulated minutes at 10Hz; the
        /// transition budget is a handful of correction episodes' worth,
        /// where the old bang-bang controller burned hundreds.
        #[test]
        fn bounded_noise_and_rate_mismatch_do_not_flap(
            seed in proptest::prelude::any::<u64>(),
            // Leader clock-rate mismatch, ±1ms of drift per second.
            mismatch_ppm in -1_000i64..1_000,
            // Initial gap anywhere in the slew band.
            initial_gap in -2_000i64..2_000,
        ) {
            use rand::{Rng, SeedableRng, rngs::StdRng};
            let mut rng = StdRng::seed_from_u64(seed);
            let mut ctl = DriftController::new();
            // True gap between leader and us, in microseconds.
            let mut gap_us = (initial_gap * 1_000) as f64;
            let mut speed = 1.0f64;
            let mut transitions = 0u32;
            for _ in 0..6_000 {
                // 100ms tick: the leader advances at its mismatched
                // rate (100ms × ppm/1e6 = ppm/10 µs); we advance at the
                // commanded speed.
                gap_us += mismatch_ppm as f64 * 0.1;
                gap_us -= (speed - 1.0) * 100_000.0;
                let noise = rng.random_range(-40..=40);
                let measured = (gap_us / 1_000.0) as i64 + noise;
                match ctl.observe(measured) {
                    DriftAction::SetSpeed(s) => {
                        speed = s;
                        transitions += 1;
                    }
                    DriftAction::HardSeek => {
                        // Unreachable under these bounds, but keep the
                        // plant honest if thresholds ever change.
                        gap_us = 0.0;
                        speed = 1.0;
                        transitions += 1;
                    }
                    DriftAction::None => {}
                }
            }
            // Budget: the initial correction (~a ramp of quantized
            // steps) plus one episode per ~85s of worst-case 1ms/s
            // drift, each another ramp. The old controller produced
            // 300+ under the same plant.
            proptest::prop_assert!(
                transitions <= 100,
                "{transitions} speed transitions in 10 simulated minutes"
            );
            // And the correction must actually work: the true gap ends
            // inside (or near) the engage band, not growing without
            // bound.
            proptest::prop_assert!(
                gap_us.abs() / 1_000.0 <= (DRIFT_ENGAGE_MILLIS + 100) as f64,
                "final gap {}ms — the controller is not converging",
                gap_us / 1_000.0
            );
        }
    }
}

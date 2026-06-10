//! NTP-style time synchronization state.
//!
//! Pure logic: the caller feeds in `(t1, t2, t3, t4)` exchanges and asks
//! for the current offset. No clocks are read here — the network actor
//! supplies timestamps, which keeps this deterministic under test.
//!
//! Per docs/network-design.md: rolling window, offsets averaged over
//! samples whose RTT is at most twice the window median (cheap probes on
//! a congested link would otherwise drag the offset around). Precision
//! target is <50ms — enough for slew-band drift correction.

use std::collections::VecDeque;

use crate::types::SharedTimestamp;

/// One completed exchange.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeSyncSample {
    /// Round-trip time in milliseconds, network legs only.
    pub rtt: u64,
    /// Estimated server-minus-local clock offset in milliseconds.
    pub offset: i64,
}

/// Compute one sample from an exchange. Returns `None` for nonsensical
/// timestamps (t4 < t1, or server turnaround exceeding the round trip),
/// which can happen if a clock steps mid-exchange.
pub fn sample_from_exchange(t1: u64, t2: u64, t3: u64, t4: u64) -> Option<TimeSyncSample> {
    let round = (t4 as i128) - (t1 as i128);
    let turnaround = (t3 as i128) - (t2 as i128);
    if round < 0 || turnaround < 0 || turnaround > round {
        return None;
    }
    let rtt = u64::try_from(round - turnaround).ok()?;
    let offset = ((t2 as i128 - t1 as i128) + (t3 as i128 - t4 as i128)) / 2;
    Some(TimeSyncSample {
        rtt,
        offset: i64::try_from(offset).ok()?,
    })
}

/// Rolling time-sync state.
#[derive(Clone, Debug, Default)]
pub struct TimeSync {
    samples: VecDeque<TimeSyncSample>,
}

/// How many recent samples the estimate uses.
const WINDOW: usize = 16;

impl TimeSync {
    /// Empty state; [`TimeSync::offset`] is `None` until a sample lands.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one completed exchange. Nonsensical exchanges are ignored.
    pub fn add_exchange(&mut self, t1: u64, t2: u64, t3: u64, t4: u64) {
        if let Some(sample) = sample_from_exchange(t1, t2, t3, t4) {
            self.samples.push_back(sample);
            while self.samples.len() > WINDOW {
                self.samples.pop_front();
            }
        }
    }

    /// Number of samples currently in the window.
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// Median RTT over the window.
    pub fn median_rtt(&self) -> Option<u64> {
        if self.samples.is_empty() {
            return None;
        }
        let mut rtts: Vec<u64> = self.samples.iter().map(|s| s.rtt).collect();
        rtts.sort_unstable();
        rtts.get(rtts.len() / 2).copied()
    }

    /// The current offset estimate: the mean offset of all window
    /// samples whose RTT is at most twice the median RTT.
    pub fn offset(&self) -> Option<i64> {
        let median = self.median_rtt()?;
        let cutoff = median.saturating_mul(2);
        let kept: Vec<i64> = self
            .samples
            .iter()
            .filter(|s| s.rtt <= cutoff)
            .map(|s| s.offset)
            .collect();
        if kept.is_empty() {
            return None;
        }
        let sum: i128 = kept.iter().map(|&o| o as i128).sum();
        i64::try_from(sum / kept.len() as i128).ok()
    }

    /// Convert a local clock reading to the shared clock. `None` until
    /// the first sample.
    pub fn shared_now(&self, local_millis: u64) -> Option<SharedTimestamp> {
        let offset = self.offset()?;
        Some(SharedTimestamp(local_millis.saturating_add_signed(offset)))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn symmetric_path_recovers_exact_offset() {
        // Server clock is 5000ms ahead; 40ms each way; 3ms turnaround.
        let mut sync = TimeSync::new();
        let t1 = 1_000;
        let t2 = t1 + 40 + 5_000;
        let t3 = t2 + 3;
        let t4 = t1 + 40 + 3 + 40;
        sync.add_exchange(t1, t2, t3, t4);
        assert_eq!(sync.offset(), Some(5_000));
        assert_eq!(sync.median_rtt(), Some(80));
        assert_eq!(sync.shared_now(2_000), Some(SharedTimestamp(7_000)));
    }

    #[test]
    fn negative_offset_works() {
        // Server clock is 1000ms behind; 10ms each way.
        let mut sync = TimeSync::new();
        let t1: u64 = 10_000;
        let t2 = t1 + 10 - 1_000;
        let t3 = t2;
        let t4 = t1 + 20;
        sync.add_exchange(t1, t2, t3, t4);
        assert_eq!(sync.offset(), Some(-1_000));
    }

    #[test]
    fn asymmetry_bounds_error_by_half_rtt() {
        // 5ms out, 95ms back, zero true offset: estimate must be within
        // RTT/2 of zero.
        let mut sync = TimeSync::new();
        let t1 = 1_000;
        let t2 = t1 + 5;
        let t3 = t2;
        let t4 = t1 + 100;
        sync.add_exchange(t1, t2, t3, t4);
        let offset = sync.offset().unwrap();
        assert!(offset.abs() <= 50, "offset {offset} exceeds rtt/2");
    }

    #[test]
    fn congested_outliers_are_discarded() {
        let mut sync = TimeSync::new();
        // Ten clean samples: 20ms RTT, true offset 100.
        for i in 0..10u64 {
            let t1 = i * 1_000;
            let t2 = t1 + 10 + 100;
            let t3 = t2 + 1;
            let t4 = t1 + 21;
            sync.add_exchange(t1, t2, t3, t4);
        }
        // Two congested samples: 400ms RTT with wildly asymmetric paths
        // suggesting offset ~ -90.
        for i in 10..12u64 {
            let t1 = i * 1_000;
            let t2 = t1 + 390 + 100 - 190; // late outbound leg
            let t3 = t2 + 1;
            let t4 = t1 + 401;
            sync.add_exchange(t1, t2, t3, t4);
        }
        let offset = sync.offset().unwrap();
        assert_eq!(offset, 100, "outliers leaked into the estimate");
    }

    #[test]
    fn window_rolls() {
        let mut sync = TimeSync::new();
        for i in 0..40u64 {
            let t1 = i * 1_000;
            let t2 = t1 + 10 + i; // offset drifts upward
            let t3 = t2;
            let t4 = t1 + 20;
            sync.add_exchange(t1, t2, t3, t4);
        }
        assert_eq!(sync.sample_count(), 16);
        // Estimate reflects recent samples (offsets 24..40), not early ones.
        let offset = sync.offset().unwrap();
        assert!((24..=40).contains(&offset), "stale window: {offset}");
    }

    #[test]
    fn nonsense_exchanges_are_ignored() {
        let mut sync = TimeSync::new();
        sync.add_exchange(100, 150, 160, 200); // valid
        sync.add_exchange(100, 200, 300, 50); // t4 < t1
        sync.add_exchange(100, 200, 500, 110); // turnaround > round trip
        assert_eq!(sync.sample_count(), 1);
    }
}

//! NTP-style time synchronization.
//!
//! Pure logic — no I/O. Computes clock offset from timestamp quadruples
//! (`t1`, `t2`, `t3`, `t4`) using the standard NTP algorithm with outlier
//! rejection.

use std::collections::VecDeque;

use crate::types::SharedTimestamp;

/// Maximum number of samples in the rolling buffer.
const MAX_SAMPLES: usize = 16;

/// A single time sync measurement.
#[derive(Debug, Clone, Copy)]
struct TimeSample {
    offset_ms: i64,
    rtt_ms: i64,
}

/// Computes and maintains a rolling clock offset estimate.
///
/// Feed it timestamp quadruples from NTP-style exchanges:
/// - `t1`: client send time (local clock)
/// - `t2`: server receive time (server clock)
/// - `t3`: server send time (server clock)
/// - `t4`: client receive time (local clock)
///
/// The offset is: `local_time + offset = server_time`.
#[derive(Debug, Clone)]
pub struct TimeSyncState {
    samples: VecDeque<TimeSample>,
    offset_ms: i64,
}

impl Default for TimeSyncState {
    fn default() -> Self {
        Self::new()
    }
}

impl TimeSyncState {
    pub fn new() -> Self {
        Self {
            samples: VecDeque::new(),
            offset_ms: 0,
        }
    }

    /// Process a time sync response and update the offset estimate.
    ///
    /// All timestamps are in milliseconds (same unit as `SharedTimestamp`).
    /// Invalid samples (negative RTT) are silently discarded.
    pub fn process_response(&mut self, t1: u64, t2: u64, t3: u64, t4: u64) {
        let t1 = t1 as i64;
        let t2 = t2 as i64;
        let t3 = t3 as i64;
        let t4 = t4 as i64;

        let rtt_ms = (t4.saturating_sub(t1)).saturating_sub(t3.saturating_sub(t2));
        if rtt_ms < 0 {
            // Invalid: receive time before send time
            return;
        }

        let offset_ms = ((t2.saturating_sub(t1)).saturating_add(t3.saturating_sub(t4))) / 2;

        self.samples.push_back(TimeSample { offset_ms, rtt_ms });
        if self.samples.len() > MAX_SAMPLES {
            self.samples.pop_front();
        }

        self.recompute_offset();
    }

    /// Current estimated offset in milliseconds.
    ///
    /// `local_time + offset_ms = server_time`
    pub fn offset_ms(&self) -> i64 {
        self.offset_ms
    }

    /// Current shared timestamp (local system time adjusted by the offset).
    ///
    /// Returns 0 if system time is before the Unix epoch (shouldn't happen in practice).
    pub fn shared_now(&self) -> SharedTimestamp {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let adjusted = now_ms + self.offset_ms;
        if adjusted <= 0 { 1 } else { adjusted as u64 }
    }

    /// Number of samples currently in the buffer.
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// Recompute offset using outlier rejection.
    ///
    /// Algorithm: discard samples with RTT > 2x median RTT, average the rest.
    fn recompute_offset(&mut self) {
        if self.samples.is_empty() {
            return;
        }

        // Compute median RTT
        let mut rtts: Vec<i64> = self.samples.iter().map(|s| s.rtt_ms).collect();
        rtts.sort();
        let median_rtt = rtts[rtts.len() / 2];

        // Filter: keep samples with RTT <= 2 * median (or all if median is 0)
        let threshold = if median_rtt == 0 { i64::MAX } else { median_rtt.saturating_mul(2) };
        let valid: Vec<i64> = self
            .samples
            .iter()
            .filter(|s| s.rtt_ms <= threshold)
            .map(|s| s.offset_ms)
            .collect();

        if valid.is_empty() {
            // All samples were outliers — keep the last raw offset
            if let Some(last) = self.samples.back() {
                self.offset_ms = last.offset_ms;
            }
            return;
        }

        // Average remaining offsets
        let sum: i64 = valid.iter().fold(0i64, |a, &b| a.saturating_add(b));
        self.offset_ms = sum / valid.len() as i64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_state() {
        let state = TimeSyncState::new();
        assert_eq!(state.offset_ms(), 0);
        assert_eq!(state.sample_count(), 0);
    }

    #[test]
    fn known_offset_zero_rtt() {
        // Server is 100ms ahead, zero network latency
        // t1=1000, t2=1100, t3=1100, t4=1000
        let mut state = TimeSyncState::new();
        state.process_response(1000, 1100, 1100, 1000);
        assert_eq!(state.offset_ms(), 100);
    }

    #[test]
    fn known_offset_symmetric_rtt() {
        // Server is 50ms ahead, 20ms each way
        // t1=1000, server receives at 1070 (1000+50+20), server sends at 1070,
        // client receives at 1040 (1000+40)
        // offset = ((1070-1000) + (1070-1040)) / 2 = (70 + 30) / 2 = 50
        let mut state = TimeSyncState::new();
        state.process_response(1000, 1070, 1070, 1040);
        assert_eq!(state.offset_ms(), 50);
    }

    #[test]
    fn negative_offset() {
        // Client is ahead of server by 200ms
        // t1=1200, t2=1000, t3=1000, t4=1200
        // offset = ((1000-1200) + (1000-1200)) / 2 = (-200 + -200) / 2 = -200
        let mut state = TimeSyncState::new();
        state.process_response(1200, 1000, 1000, 1200);
        assert_eq!(state.offset_ms(), -200);
    }

    #[test]
    fn negative_rtt_discarded() {
        // t4 < t1 (receive before send — impossible)
        let mut state = TimeSyncState::new();
        state.process_response(1000, 1050, 1050, 900);
        assert_eq!(state.sample_count(), 0);
        assert_eq!(state.offset_ms(), 0);
    }

    #[test]
    fn rolling_average() {
        let mut state = TimeSyncState::new();
        // Three samples with offset 100, RTT 10 each
        state.process_response(1000, 1105, 1105, 1010);
        state.process_response(2000, 2105, 2105, 2010);
        state.process_response(3000, 3105, 3105, 3010);
        assert_eq!(state.sample_count(), 3);
        assert_eq!(state.offset_ms(), 100);
    }

    #[test]
    fn outlier_rejection() {
        let mut state = TimeSyncState::new();
        // Good samples: offset ~100, RTT 10
        for i in 0..5 {
            let base = (i * 1000 + 1000) as u64;
            state.process_response(base, base + 105, base + 105, base + 10);
        }
        // Bad sample: same offset but huge RTT (500ms)
        state.process_response(6000, 6350, 6350, 6500);
        // The outlier should be rejected, offset stays ~100
        assert_eq!(state.offset_ms(), 100);
    }

    #[test]
    fn buffer_overflow_drops_oldest() {
        let mut state = TimeSyncState::new();
        // Fill buffer beyond MAX_SAMPLES
        for i in 0..20u64 {
            state.process_response(i * 1000, i * 1000 + 50, i * 1000 + 50, i * 1000);
        }
        assert_eq!(state.sample_count(), MAX_SAMPLES);
    }

    #[test]
    fn single_sample() {
        let mut state = TimeSyncState::new();
        state.process_response(0, 500, 500, 0);
        assert_eq!(state.offset_ms(), 500);
        assert_eq!(state.sample_count(), 1);
    }

    #[test]
    fn outlier_rejected_with_majority_good() {
        let mut state = TimeSyncState::new();
        // Three good samples: RTT = 10, offset = 100
        state.process_response(1000, 1105, 1105, 1010);
        state.process_response(2000, 2105, 2105, 2010);
        state.process_response(3000, 3105, 3105, 3010);
        // One outlier: RTT = 1000 (way more than 2x median of 10), offset = 200
        state.process_response(4000, 4700, 4700, 5000);
        // median RTT = 10 (index 2 of [10,10,10,1000]). Threshold = 20.
        // Outlier rejected, offset stays 100.
        assert_eq!(state.offset_ms(), 100);
    }

    #[test]
    fn zero_rtt_samples() {
        // RTT is 0, which means threshold would be 0 — we use i64::MAX instead
        let mut state = TimeSyncState::new();
        state.process_response(1000, 1100, 1100, 1000);
        state.process_response(2000, 2100, 2100, 2000);
        assert_eq!(state.offset_ms(), 100);
    }

    #[test]
    fn shared_now_returns_nonzero() {
        let state = TimeSyncState::new();
        // With zero offset, shared_now should return current system time
        let now = state.shared_now();
        assert!(now > 0);
    }

    #[test]
    fn convergence_with_jittery_samples() {
        let mut state = TimeSyncState::new();
        // True offset is 100ms. Samples have varying RTT (jitter).
        let samples = [
            (1000u64, 1110u64, 1110u64, 1020u64), // offset=100, rtt=20
            (2000, 2108, 2108, 2016),               // offset=100, rtt=16
            (3000, 3112, 3112, 3024),               // offset=100, rtt=24
            (4000, 4106, 4106, 4012),               // offset=100, rtt=12
            (5000, 5114, 5114, 5028),               // offset=100, rtt=28
        ];
        for (t1, t2, t3, t4) in &samples {
            state.process_response(*t1, *t2, *t3, *t4);
        }
        assert_eq!(state.offset_ms(), 100);
    }

    #[test]
    fn extreme_rtt_no_overflow() {
        // Regression: median_rtt * 2 overflows when RTT > i64::MAX / 2.
        // Found by fuzz target `time_sync_convergence`.
        // t1=0, t2=0, t3=0, t4=i64::MAX → RTT = i64::MAX → median * 2 overflows.
        let mut state = TimeSyncState::new();
        state.process_response(0, 0, 0, i64::MAX as u64);
        // Should not panic; offset value is not important, just robustness.
        let _ = state.offset_ms();
    }

    #[test]
    fn large_timestamps() {
        // Typical real-world timestamps (milliseconds since epoch ~2025)
        let mut state = TimeSyncState::new();
        let base: u64 = 1_740_000_000_000; // ~Feb 2025
        state.process_response(base, base + 50, base + 51, base + 100);
        // offset = ((50) + (51-100)) / 2 = (50 + -49) / 2 = 0
        assert_eq!(state.offset_ms(), 0);
    }
}

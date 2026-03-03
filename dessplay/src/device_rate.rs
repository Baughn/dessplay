//! Per-device hash rate tracking for ETA estimation during background indexing.

use std::collections::HashMap;
use std::time::Duration;

/// A single measurement of bytes hashed over a duration.
#[derive(Debug, Clone)]
pub struct RateSample {
    pub bytes: u64,
    pub duration: Duration,
}

/// Tracks hashing throughput per storage device (by `dev_id` from stat).
///
/// Maintains a sliding window of recent samples per device and remaining
/// bytes to hash, enabling per-device rate estimation and overall ETA
/// calculation.
pub struct DeviceRateTracker {
    /// dev_id -> last N samples (capped at `WINDOW_SIZE`).
    rates: HashMap<u64, Vec<RateSample>>,
    /// dev_id -> bytes remaining to hash on that device.
    remaining_bytes: HashMap<u64, u64>,
    /// Most recent sample (any device), for "current rate" display.
    last_sample: Option<RateSample>,
}

const WINDOW_SIZE: usize = 5;

impl Default for DeviceRateTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceRateTracker {
    pub fn new() -> Self {
        Self {
            rates: HashMap::new(),
            remaining_bytes: HashMap::new(),
            last_sample: None,
        }
    }

    /// Seed the tracker with persisted rates from SQLite.
    ///
    /// Creates synthetic 1-second samples so cold-start ETAs are reasonable.
    pub fn load_persisted(&mut self, rates: &[(u64, f64)]) {
        for &(dev_id, rate_bps) in rates {
            if rate_bps > 0.0 {
                self.rates.entry(dev_id).or_default().push(RateSample {
                    bytes: rate_bps as u64,
                    duration: Duration::from_secs(1),
                });
            }
        }
    }

    /// Register bytes that are queued to be hashed on a device.
    pub fn add_pending(&mut self, dev_id: u64, bytes: u64) {
        *self.remaining_bytes.entry(dev_id).or_default() += bytes;
    }

    /// Record a completed hash measurement and decrement remaining bytes.
    pub fn record_sample(&mut self, dev_id: u64, sample: RateSample) {
        // Decrement remaining
        if let Some(rem) = self.remaining_bytes.get_mut(&dev_id) {
            *rem = rem.saturating_sub(sample.bytes);
        }

        self.last_sample = Some(sample.clone());

        let window = self.rates.entry(dev_id).or_default();
        if window.len() >= WINDOW_SIZE {
            window.remove(0);
        }
        window.push(sample);
    }

    /// Average bytes/sec for a specific device.
    pub fn device_rate_bps(&self, dev_id: &u64) -> Option<f64> {
        let samples = self.rates.get(dev_id)?;
        if samples.is_empty() {
            return None;
        }
        let total_bytes: u64 = samples.iter().map(|s| s.bytes).sum();
        let total_secs: f64 = samples
            .iter()
            .map(|s| s.duration.as_secs_f64())
            .sum();
        if total_secs <= 0.0 {
            return None;
        }
        Some(total_bytes as f64 / total_secs)
    }

    /// Rate from the most recent sample — used as the displayed "current" throughput.
    pub fn current_rate_bps(&self) -> Option<f64> {
        let sample = self.last_sample.as_ref()?;
        let secs = sample.duration.as_secs_f64();
        if secs <= 0.0 {
            return None;
        }
        Some(sample.bytes as f64 / secs)
    }

    /// Estimated time remaining across all devices.
    ///
    /// Since the worker is serial, per-device ETAs are additive:
    /// `sum(remaining[dev] / rate[dev])`.
    pub fn eta(&self) -> Option<Duration> {
        let mut total_secs = 0.0f64;
        let mut any_remaining = false;

        for (&dev_id, &remaining) in &self.remaining_bytes {
            if remaining == 0 {
                continue;
            }
            any_remaining = true;
            if let Some(rate) = self.device_rate_bps(&dev_id) {
                if rate > 0.0 {
                    total_secs += remaining as f64 / rate;
                }
            }
            // If no rate data for a device with remaining bytes, we can't
            // compute a reliable ETA — return None.
            else {
                return None;
            }
        }

        if !any_remaining {
            return Some(Duration::ZERO);
        }

        Some(Duration::from_secs_f64(total_secs))
    }

    /// Snapshot of per-device average rates for SQLite persistence.
    pub fn rates_for_persistence(&self) -> Vec<(u64, f64)> {
        self.rates
            .keys()
            .filter_map(|&dev_id| {
                self.device_rate_bps(&dev_id)
                    .map(|rate| (dev_id, rate))
            })
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn empty_tracker_no_rate_no_eta() {
        let tracker = DeviceRateTracker::new();
        assert!(tracker.current_rate_bps().is_none());
        assert!(tracker.eta().is_some()); // no remaining = zero ETA
        assert_eq!(tracker.eta().unwrap(), Duration::ZERO);
    }

    #[test]
    fn single_sample_rate_matches() {
        let mut tracker = DeviceRateTracker::new();
        tracker.add_pending(1, 1_000_000);
        tracker.record_sample(1, RateSample {
            bytes: 500_000,
            duration: Duration::from_secs(1),
        });
        let rate = tracker.device_rate_bps(&1).unwrap();
        assert!((rate - 500_000.0).abs() < 0.01);

        let current = tracker.current_rate_bps().unwrap();
        assert!((current - 500_000.0).abs() < 0.01);
    }

    #[test]
    fn window_eviction_at_six_samples() {
        let mut tracker = DeviceRateTracker::new();
        tracker.add_pending(1, 10_000_000);

        // Add 6 samples — first should be evicted
        for i in 1..=6u64 {
            tracker.record_sample(1, RateSample {
                bytes: i * 100,
                duration: Duration::from_secs(1),
            });
        }

        let samples = &tracker.rates[&1];
        assert_eq!(samples.len(), WINDOW_SIZE);
        // First sample (bytes=100) should be gone, smallest should be 200
        assert_eq!(samples[0].bytes, 200);
    }

    #[test]
    fn multi_device_eta_sum_of_per_device_times() {
        let mut tracker = DeviceRateTracker::new();

        // Device 1: 1000 bytes remaining, 100 B/s -> 10s
        tracker.add_pending(1, 1000);
        tracker.record_sample(1, RateSample {
            bytes: 0, // doesn't change remaining since it's already tracked
            duration: Duration::from_secs(1),
        });
        // Need a real sample for rate
        tracker.rates.insert(1, vec![RateSample {
            bytes: 100,
            duration: Duration::from_secs(1),
        }]);

        // Device 2: 2000 bytes remaining, 200 B/s -> 10s
        tracker.add_pending(2, 2000);
        tracker.rates.insert(2, vec![RateSample {
            bytes: 200,
            duration: Duration::from_secs(1),
        }]);

        // Total ETA should be 10 + 10 = 20s
        let eta = tracker.eta().unwrap();
        assert!((eta.as_secs_f64() - 20.0).abs() < 0.01);
    }

    #[test]
    fn load_persisted_contributes_to_rate() {
        let mut tracker = DeviceRateTracker::new();
        tracker.load_persisted(&[(1, 500_000.0), (2, 1_000_000.0)]);

        let rate1 = tracker.device_rate_bps(&1).unwrap();
        assert!((rate1 - 500_000.0).abs() < 0.01);

        let rate2 = tracker.device_rate_bps(&2).unwrap();
        assert!((rate2 - 1_000_000.0).abs() < 0.01);
    }

    #[test]
    fn add_pending_and_record_decrements_remaining() {
        let mut tracker = DeviceRateTracker::new();
        tracker.add_pending(1, 1000);
        assert_eq!(tracker.remaining_bytes[&1], 1000);

        tracker.record_sample(1, RateSample {
            bytes: 300,
            duration: Duration::from_millis(100),
        });
        assert_eq!(tracker.remaining_bytes[&1], 700);
    }

    #[test]
    fn zero_duration_sample_no_panic() {
        let mut tracker = DeviceRateTracker::new();
        tracker.add_pending(1, 1000);
        tracker.record_sample(1, RateSample {
            bytes: 500,
            duration: Duration::ZERO,
        });

        // Rate should be None (division by zero avoided)
        assert!(tracker.device_rate_bps(&1).is_none());
        assert!(tracker.current_rate_bps().is_none());
        // ETA should be None (has remaining but no valid rate)
        assert!(tracker.eta().is_none());
    }

    #[test]
    fn rates_for_persistence_snapshot() {
        let mut tracker = DeviceRateTracker::new();
        tracker.record_sample(1, RateSample {
            bytes: 1000,
            duration: Duration::from_secs(1),
        });
        tracker.record_sample(2, RateSample {
            bytes: 2000,
            duration: Duration::from_secs(1),
        });

        let persisted = tracker.rates_for_persistence();
        assert_eq!(persisted.len(), 2);
        let dev1_rate = persisted.iter().find(|(d, _)| *d == 1).unwrap().1;
        assert!((dev1_rate - 1000.0).abs() < 0.01);
    }

    #[test]
    fn load_persisted_skips_zero_rate() {
        let mut tracker = DeviceRateTracker::new();
        tracker.load_persisted(&[(1, 0.0), (2, -100.0), (3, 500.0)]);

        assert!(tracker.device_rate_bps(&1).is_none());
        assert!(tracker.device_rate_bps(&2).is_none());
        assert!(tracker.device_rate_bps(&3).is_some());
    }
}

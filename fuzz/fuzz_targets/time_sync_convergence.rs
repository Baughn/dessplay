#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use dessplay_core::time_sync::TimeSyncState;

/// Feed arbitrary NTP timestamp quadruples to TimeSyncState.
/// Tests that the offset computation never panics, even with extreme values.
#[derive(Arbitrary, Debug)]
struct TimeSyncFuzzInput {
    samples: Vec<(u64, u64, u64, u64)>,
}

fuzz_target!(|input: TimeSyncFuzzInput| {
    let mut state = TimeSyncState::new();

    for (t1, t2, t3, t4) in &input.samples {
        state.process_response(*t1, *t2, *t3, *t4);

        // offset_ms should never panic
        let _ = state.offset_ms();

        // sample_count should be bounded
        assert!(state.sample_count() <= 16);
    }

    // shared_now should never return 0 if we have valid samples
    // (valid = server timestamps between t1 and t4)
    let has_valid = input.samples.iter().any(|(t1, t2, t3, t4)| {
        let rtt = (*t4 as i64).wrapping_sub(*t1 as i64)
            .wrapping_sub((*t3 as i64).wrapping_sub(*t2 as i64));
        rtt >= 0
    });
    if has_valid && state.sample_count() > 0 {
        // shared_now uses system time, so it should be > 0 in any real scenario
        let now = state.shared_now();
        assert!(now > 0, "shared_now returned 0 with valid samples");
    }
});

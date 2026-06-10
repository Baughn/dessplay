//! Any reachable state must survive a postcard snapshot round-trip
//! byte-exactly in behavior: equal state, equal view.

#![no_main]

use dessplay_core::test_support::{ScriptStep, run_script};
use dessplay_core::types::Epoch;
use dessplay_core::{StateSnapshot, wire};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (Vec<ScriptStep>, u64)| {
    let (steps, epoch) = input;
    let (state, _) = run_script(&steps);
    let snapshot = StateSnapshot {
        epoch: Epoch(epoch),
        state,
    };
    let bytes = wire::encode(&snapshot).expect("encode failed");
    let decoded: StateSnapshot = wire::decode(&bytes).expect("decode failed");
    assert_eq!(decoded, snapshot);
    assert_eq!(decoded.state.view(), snapshot.state.view());
});

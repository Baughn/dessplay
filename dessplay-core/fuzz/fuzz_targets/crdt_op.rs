//! Op replay must never panic, and the resulting state must always be
//! viewable and serializable.

#![no_main]

use dessplay_core::test_support::{ScriptStep, run_script};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|steps: Vec<ScriptStep>| {
    let (state, ops) = run_script(&steps);
    let _ = state.view();
    let _ = dessplay_core::wire::encode(&state);

    // Re-applying every op (duplicate delivery) must not panic either.
    let mut state = state;
    for (_, op) in ops {
        state.apply(op);
    }
    let _ = state.view();
});

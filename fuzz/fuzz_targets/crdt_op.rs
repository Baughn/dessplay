#![no_main]

use dessplay_core::crdt::CrdtState;
use dessplay_core::protocol::CrdtOp;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|ops: Vec<CrdtOp>| {
    let mut state = CrdtState::new();
    for op in &ops {
        state.apply_op(op);
    }
    // Must not panic
    let _ = state.snapshot();
    let _ = state.version_vectors();
});

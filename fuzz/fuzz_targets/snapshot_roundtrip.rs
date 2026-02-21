#![no_main]

use dessplay_core::crdt::CrdtState;
use dessplay_core::protocol::CrdtOp;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|ops: Vec<CrdtOp>| {
    // Build state from ops
    let mut state = CrdtState::new();
    for op in &ops {
        state.apply_op(op);
    }

    // Snapshot and reload into a fresh state
    let snap = state.snapshot();
    let mut restored = CrdtState::new();
    restored.load_snapshot(state.epoch(), snap);

    // Must produce identical snapshots
    assert_eq!(state.snapshot(), restored.snapshot());

    // Version vectors must also match
    assert_eq!(state.version_vectors(), restored.version_vectors());
});

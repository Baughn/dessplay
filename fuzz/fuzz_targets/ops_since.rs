#![no_main]

use arbitrary::Arbitrary;
use dessplay_core::crdt::CrdtState;
use dessplay_core::protocol::CrdtOp;
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct OpsSinceInput {
    /// Ops that both peers have seen.
    base_ops: Vec<CrdtOp>,
    /// Additional ops that only the "ahead" peer has seen.
    new_ops: Vec<CrdtOp>,
}

fuzz_target!(|input: OpsSinceInput| {
    // Build the "behind" peer's state from base ops
    let mut behind = CrdtState::new();
    for op in &input.base_ops {
        behind.apply_op(op);
    }

    // Build the "ahead" peer's state from base + new ops
    let mut ahead = behind.clone();
    for op in &input.new_ops {
        ahead.apply_op(op);
    }

    // Ask the ahead peer for ops the behind peer is missing
    let behind_vv = behind.version_vectors();
    let catch_up_ops = ahead.ops_since(&behind_vv);

    // Apply catch-up ops to the behind peer
    for op in &catch_up_ops {
        behind.apply_op(op);
    }

    // After catch-up, snapshots must match
    assert_eq!(ahead.snapshot(), behind.snapshot());

    // Version vectors must also agree so both peers consider themselves in sync
    assert_eq!(ahead.version_vectors(), behind.version_vectors());
});

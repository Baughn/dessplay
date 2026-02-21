#![no_main]

use arbitrary::Arbitrary;
use dessplay_core::crdt::CrdtState;
use dessplay_core::protocol::CrdtOp;
use dessplay_core::types::UserId;
use libfuzzer_sys::fuzz_target;

/// Constrained chat input: 2 users, seq numbers 0-15. The small seq space
/// makes non-contiguous sequences inevitable, exposing gap-fill bugs where
/// max-seq version tracking misses lower-numbered entries.
#[derive(Arbitrary, Debug)]
struct Input {
    /// Base messages both peers have: (user_idx, seq, timestamp, text_byte)
    base: Vec<(u8, u8, u16, u8)>,
    /// Additional messages only the "ahead" peer has
    new: Vec<(u8, u8, u16, u8)>,
}

fn make_uid(idx: u8) -> UserId {
    UserId(format!("user{}", idx % 2))
}

fuzz_target!(|input: Input| {
    // Build "behind" peer from base ops
    let mut behind = CrdtState::new();
    for &(ui, seq, ts, text) in &input.base {
        behind.apply_op(&CrdtOp::ChatAppend {
            user_id: make_uid(ui),
            seq: u64::from(seq % 16),
            timestamp: u64::from(ts),
            text: format!("b{text}"),
        });
    }

    // Build "ahead" peer: everything behind has + new ops
    let mut ahead = behind.clone();
    for &(ui, seq, ts, text) in &input.new {
        ahead.apply_op(&CrdtOp::ChatAppend {
            user_id: make_uid(ui),
            seq: u64::from(seq % 16),
            timestamp: u64::from(ts),
            text: format!("n{text}"),
        });
    }

    // Gap fill: behind asks ahead for missing ops
    let behind_vv = behind.version_vectors();
    let catch_up = ahead.ops_since(&behind_vv);
    for op in &catch_up {
        behind.apply_op(op);
    }

    // After catch-up, snapshots must match
    assert_eq!(ahead.snapshot(), behind.snapshot());
});

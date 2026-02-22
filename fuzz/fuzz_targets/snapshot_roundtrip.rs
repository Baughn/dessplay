#![no_main]

use arbitrary::Arbitrary;
use dessplay_core::crdt::CrdtState;
use dessplay_core::protocol::CrdtOp;
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Input {
    /// Ops applied before the first snapshot/restore cycle.
    initial_ops: Vec<CrdtOp>,
    /// Ops applied after restore, then a second snapshot/restore cycle.
    /// Timestamps are offset to be above initial_ops (compaction is an epoch
    /// boundary; real post-compaction ops always have higher timestamps).
    post_restore_ops: Vec<CrdtOp>,
}

/// Find the maximum timestamp across all ops.
fn max_timestamp(ops: &[CrdtOp]) -> u64 {
    ops.iter()
        .map(|op| match op {
            CrdtOp::LwwWrite { timestamp, .. } => *timestamp,
            CrdtOp::PlaylistOp { timestamp, .. } => *timestamp,
            CrdtOp::ChatAppend { timestamp, .. } => *timestamp,
        })
        .max()
        .unwrap_or(0)
}

/// Shift all timestamps in ops by a fixed offset.
fn offset_timestamps(ops: &mut [CrdtOp], offset: u64) {
    for op in ops {
        let ts = match op {
            CrdtOp::LwwWrite { timestamp, .. } => timestamp,
            CrdtOp::PlaylistOp { timestamp, .. } => timestamp,
            CrdtOp::ChatAppend { timestamp, .. } => timestamp,
        };
        // wrapping_add is fine — we only call this when we've verified
        // the offset won't cause collisions with initial timestamps.
        *ts = ts.wrapping_add(offset);
    }
}

fuzz_target!(|input: Input| {
    // Build state from initial ops
    let mut state = CrdtState::new();
    for op in &input.initial_ops {
        state.apply_op(op);
    }

    // Snapshot and reload into a fresh state
    let snap = state.snapshot();
    let mut restored = CrdtState::new();
    restored.load_snapshot(state.epoch(), snap);

    // Must produce identical snapshots and version vectors
    assert_eq!(state.snapshot(), restored.snapshot());
    assert_eq!(state.version_vectors(), restored.version_vectors());

    // Apply post-restore ops only when timestamps won't collide with
    // initial ops. In the real protocol, compaction is an epoch boundary
    // and subsequent timestamps are always strictly higher.
    let base_ts = max_timestamp(&input.initial_ops);
    let post_max = max_timestamp(&input.post_restore_ops);
    if !input.post_restore_ops.is_empty()
        && base_ts < u64::MAX / 2
        && post_max < u64::MAX / 2
    {
        let mut post_ops = input.post_restore_ops;
        offset_timestamps(&mut post_ops, base_ts + 1);

        // Apply same ops to both original and restored state
        for op in &post_ops {
            state.apply_op(op);
            restored.apply_op(op);
        }

        // Both paths (original + ops vs restored + ops) must agree
        assert_eq!(state.snapshot(), restored.snapshot());
        assert_eq!(state.version_vectors(), restored.version_vectors());

        // Second roundtrip: snapshot the post-ops state and restore again
        let snap2 = restored.snapshot();
        let mut restored2 = CrdtState::new();
        restored2.load_snapshot(restored.epoch(), snap2);

        assert_eq!(restored.snapshot(), restored2.snapshot());
        assert_eq!(restored.version_vectors(), restored2.version_vectors());
    }
});

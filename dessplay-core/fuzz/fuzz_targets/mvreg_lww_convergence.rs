//! Lww resolution over concurrent register writes: every delivery order
//! resolves to max((timestamp, value)).

#![no_main]

use dessplay_core::test_support::file;
use dessplay_core::types::{ActorId, SharedTimestamp};
use dessplay_core::{CrdtState, Lww};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (Vec<(u8, u8)>, u8)| {
    let (writes, rotation) = input;
    if writes.is_empty() || writes.len() > 8 {
        return;
    }

    // Each write on its own replica: all writes pairwise concurrent.
    let mut ops = Vec::new();
    let mut replicas = Vec::new();
    for (i, (ts, value)) in writes.iter().enumerate() {
        let mut replica = CrdtState::new();
        let op = replica.set_now_playing(
            ActorId(i as u128 + 1),
            SharedTimestamp(*ts as u64),
            Some(file(*value)),
        );
        ops.push(op);
        replicas.push(replica);
    }

    let expected = writes
        .iter()
        .map(|(ts, value)| Lww::new(SharedTimestamp(*ts as u64), Some(file(*value))))
        .max()
        .and_then(|lww| lww.value);

    // Op delivery in a rotated order.
    let rotation = rotation as usize % ops.len();
    let mut by_ops = CrdtState::new();
    for op in ops.iter().cycle().skip(rotation).take(ops.len()) {
        by_ops.apply(op.clone());
    }
    assert_eq!(by_ops.view().now_playing, expected);

    // State merge in reverse order.
    let mut by_merge = CrdtState::new();
    for replica in replicas.iter().rev() {
        by_merge.merge(replica.clone());
    }
    assert_eq!(by_merge.view().now_playing, expected);
});

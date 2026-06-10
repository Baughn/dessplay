//! Lww resolution properties: among causally concurrent writes, the
//! highest timestamp wins, with value tiebreaking on equal timestamps —
//! regardless of merge or apply order.

use dessplay_core::types::{ActorId, Ed2kHash, SharedTimestamp};
use dessplay_core::{CrdtState, Lww};
use proptest::collection::vec;
use proptest::prelude::*;

fn hash(i: u8) -> Ed2kHash {
    Ed2kHash([i; 16])
}

proptest! {
    /// N replicas each write the register concurrently; everyone
    /// converges on max((timestamp, value)).
    #[test]
    fn concurrent_writes_resolve_to_max(
        writes in vec((0u64..8, 0u8..8), 1..6),
        merge_order in vec(any::<u8>(), 8),
    ) {
        // Each write happens on its own replica (distinct actors), making
        // all writes pairwise concurrent.
        let mut replicas: Vec<CrdtState> = Vec::new();
        let mut ops = Vec::new();
        for (i, (ts, value)) in writes.iter().enumerate() {
            let mut replica = CrdtState::new();
            let op = replica.set_now_playing(
                ActorId(i as u128 + 1),
                SharedTimestamp(*ts),
                Some(hash(*value)),
            );
            ops.push(op);
            replicas.push(replica);
        }

        let expected = writes
            .iter()
            .map(|(ts, value)| Lww::new(SharedTimestamp(*ts), Some(hash(*value))))
            .max()
            .and_then(|lww| lww.value);

        // Convergence via op broadcast, in a rotated order per case.
        let rotation = merge_order.first().copied().unwrap_or(0) as usize % ops.len();
        let mut observer = CrdtState::new();
        for op in ops.iter().cycle().skip(rotation).take(ops.len()) {
            observer.apply(op.clone());
        }
        prop_assert_eq!(observer.view().now_playing, expected);

        // Convergence via state merge, folding in a different order.
        let mut by_merge = CrdtState::new();
        for replica in replicas.iter().rev() {
            by_merge.merge(replica.clone());
        }
        prop_assert_eq!(by_merge.view().now_playing, expected);
        prop_assert_eq!(by_merge.view(), observer.view());
    }
}

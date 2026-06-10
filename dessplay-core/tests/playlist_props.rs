//! Playlist `Identifier` ordering properties: always sortable, inserts
//! land where asked, rebalancing preserves order.

mod common;

use common::arb_step;
use dessplay_core::test_support::{file, run_script};
use dessplay_core::types::{ActorId, Ed2kHash, SharedTimestamp, UserId};
use dessplay_core::{CrdtState, NewPlaylistEntry};
use proptest::collection::vec;
use proptest::prelude::*;

fn new_entry(i: u8) -> NewPlaylistEntry {
    NewPlaylistEntry {
        hash: file(i),
        added_by: UserId::new("prop"),
        filename: format!("f{i}.mkv"),
        size_bytes: 1,
        duration_millis: None,
    }
}

proptest! {
    /// After any script, the playlist view is strictly sorted by
    /// (position, hash) — i.e. sortable with no duplicate keys.
    #[test]
    fn playlist_is_always_strictly_sorted(steps in vec(arb_step(), 0..60)) {
        let (state, _) = run_script(&steps);
        let entries = state.playlist_entries();
        for pair in entries.windows(2) {
            if let [a, b] = pair {
                let key_a = (&a.state.position, a.hash);
                let key_b = (&b.state.position, b.hash);
                prop_assert!(key_a < key_b, "entries out of order");
            }
        }
    }

    /// A targeted insert sequence: inserting after a random existing entry
    /// always places the new entry immediately after its anchor.
    #[test]
    fn insert_lands_after_anchor(
        anchors in vec(proptest::option::of(any::<prop::sample::Index>()), 1..16),
    ) {
        let actor = ActorId(1);
        let mut state = CrdtState::new();
        let mut t = 0u64;
        for (i, anchor) in anchors.iter().enumerate() {
            let i = i as u8; // bounded by the vec size (< 16)
            t += 1;
            let existing: Vec<Ed2kHash> =
                state.playlist_entries().iter().map(|e| e.hash).collect();
            let anchor_hash = anchor.as_ref().and_then(|index| {
                if existing.is_empty() {
                    None
                } else {
                    Some(existing[index.index(existing.len())])
                }
            });
            // Use raw hashes (not `file()`, which wraps mod FILES) so
            // every entry is distinct.
            let mut entry = new_entry(0);
            entry.hash = Ed2kHash([i + 1; 16]);
            state.add_playlist_entry_after(actor, SharedTimestamp(t), anchor_hash.as_ref(), entry);

            let after: Vec<Ed2kHash> = state.playlist_entries().iter().map(|e| e.hash).collect();
            let new_pos = after
                .iter()
                .position(|h| *h == Ed2kHash([i + 1; 16]))
                .ok_or_else(|| TestCaseError::fail("inserted entry missing"))?;
            match anchor_hash {
                None => prop_assert_eq!(new_pos, 0, "front insert not at front"),
                Some(anchor_hash) => {
                    let anchor_pos = after
                        .iter()
                        .position(|h| *h == anchor_hash)
                        .ok_or_else(|| TestCaseError::fail("anchor missing"))?;
                    prop_assert_eq!(new_pos, anchor_pos + 1, "not directly after anchor");
                }
            }
        }
    }

    /// Rebalancing never changes the observed order, and converges across
    /// replicas that receive the rebalance ops.
    #[test]
    fn rebalance_preserves_order(steps in vec(arb_step(), 0..60)) {
        let (mut state, _) = run_script(&steps);
        let before: Vec<Ed2kHash> = state.playlist_entries().iter().map(|e| e.hash).collect();

        let mut replica = state.clone();
        let ops = state.rebalance_playlist(ActorId::SERVER, SharedTimestamp(u16::MAX as u64 + 1));
        let after: Vec<Ed2kHash> = state.playlist_entries().iter().map(|e| e.hash).collect();
        prop_assert_eq!(&before, &after);

        for op in ops {
            replica.apply(op);
        }
        prop_assert_eq!(&replica, &state);
    }
}

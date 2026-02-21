#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use dessplay_core::crdt::chat::Chat;
use dessplay_core::crdt::lww::LwwRegister;
use dessplay_core::crdt::playlist::Playlist;
use dessplay_core::protocol::PlaylistAction;
use dessplay_core::types::{FileId, UserId};
use proptest::prelude::*;

fn arb_file_id() -> impl Strategy<Value = FileId> {
    (0..10u8).prop_map(|n| {
        let mut id = [0u8; 16];
        id[0] = n;
        FileId(id)
    })
}

fn arb_user_id() -> impl Strategy<Value = UserId> {
    prop::sample::select(vec!["alice", "bob", "carol", "dave"])
        .prop_map(|s| UserId(s.to_string()))
}

fn arb_playlist_action() -> impl Strategy<Value = PlaylistAction> {
    prop_oneof![
        (arb_file_id(), prop::option::of(arb_file_id()))
            .prop_map(|(fid, after)| PlaylistAction::Add { file_id: fid, after }),
        arb_file_id().prop_map(|fid| PlaylistAction::Remove { file_id: fid }),
        (arb_file_id(), prop::option::of(arb_file_id()))
            .prop_map(|(fid, after)| PlaylistAction::Move { file_id: fid, after }),
    ]
}

/// Shuffle a vec using a seed, in a reproducible way.
fn shuffle_with_seed<T>(items: &mut [T], seed: u64) {
    // Simple Fisher-Yates shuffle with deterministic RNG
    let len = items.len();
    let mut state = seed;
    for i in (1..len).rev() {
        // Simple xorshift-style PRNG for shuffling
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let j = (state as usize) % (i + 1);
        items.swap(i, j);
    }
}

proptest! {
    #[test]
    fn lww_convergence(
        writes in proptest::collection::vec((0..5u32, 1..1000u64, 0..100i32), 1..30),
        perm_seed in any::<u64>(),
    ) {
        // Apply in original order
        let mut reg_a = LwwRegister::new();
        for (key, ts, val) in &writes {
            reg_a.write(*key, *ts, *val);
        }

        // Apply in a shuffled order
        let mut shuffled = writes.clone();
        shuffle_with_seed(&mut shuffled, perm_seed);

        let mut reg_b = LwwRegister::new();
        for (key, ts, val) in &shuffled {
            reg_b.write(*key, *ts, *val);
        }

        prop_assert_eq!(reg_a, reg_b);
    }

    #[test]
    fn playlist_convergence(
        ops in proptest::collection::vec(
            (1..1000u64, arb_playlist_action()),
            1..30
        ),
        perm_seed in any::<u64>(),
    ) {
        // Apply in original order
        let mut pl_a = Playlist::new();
        for (ts, action) in &ops {
            pl_a.apply(*ts, action.clone());
        }

        // Apply in shuffled order
        let mut shuffled = ops.clone();
        shuffle_with_seed(&mut shuffled, perm_seed);

        let mut pl_b = Playlist::new();
        for (ts, action) in &shuffled {
            pl_b.apply(*ts, action.clone());
        }

        prop_assert_eq!(pl_a.snapshot(), pl_b.snapshot());
    }

    #[test]
    fn chat_convergence(
        messages in proptest::collection::vec(
            (arb_user_id(), 0..20u64, 1..1000u64, "[a-z]{1,10}"),
            1..30
        ),
        perm_seed in any::<u64>(),
    ) {
        // No dedup filter needed — Chat::append() uses LWW on duplicate
        // (user, seq), so convergence holds even with conflicting content.

        // Apply in original order
        let mut chat_a = Chat::new();
        for (uid, seq, ts, text) in &messages {
            chat_a.append(uid.clone(), *seq, *ts, text.clone());
        }

        // Apply in shuffled order
        let mut shuffled = messages.clone();
        shuffle_with_seed(&mut shuffled, perm_seed);

        let mut chat_b = Chat::new();
        for (uid, seq, ts, text) in &shuffled {
            chat_b.append(uid.clone(), *seq, *ts, text.clone());
        }

        // Per-user message order should be identical
        prop_assert_eq!(chat_a, chat_b);
    }
}

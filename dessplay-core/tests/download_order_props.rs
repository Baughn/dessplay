//! Properties of `derive::anchored_download_order`: it permutes the
//! playlist, puts the anchor first, keeps after-anchor entries in
//! playlist order ahead of everything behind the anchor, and walks the
//! behind-anchor tail nearest-first.

use dessplay_core::CrdtState;
use dessplay_core::derive::anchored_download_order;
use dessplay_core::playlist::NewPlaylistEntry;
use dessplay_core::types::{ActorId, Ed2kHash, SharedTimestamp, UserId};
use proptest::prelude::*;

fn playlist_of(n: u8) -> Vec<dessplay_core::playlist::PlaylistEntry> {
    let mut state = CrdtState::new();
    for i in 1..=n {
        state.push_playlist_entry(
            ActorId::SERVER,
            SharedTimestamp(i as u64),
            NewPlaylistEntry {
                hash: Ed2kHash([i; 16]),
                added_by: UserId::new("prop"),
                filename: format!("ep{i}.mkv"),
                size_bytes: 1,
                duration_millis: None,
            },
        );
    }
    state.playlist_entries()
}

proptest! {
    #[test]
    fn anchored_order_is_a_permutation_ranked_around_the_anchor(
        n in 0u8..30,
        // Deliberately allowed to exceed the playlist (unknown anchor) —
        // that case must degrade to plain playlist order.
        anchor in proptest::option::of(1u8..40),
    ) {
        let playlist = playlist_of(n);
        let order = anchored_download_order(&playlist, anchor.map(|a| Ed2kHash([a; 16])));

        // Permutation: same entries, nothing dropped or invented.
        let mut got: Vec<Ed2kHash> = order.iter().map(|e| e.hash).collect();
        let mut want: Vec<Ed2kHash> = playlist.iter().map(|e| e.hash).collect();
        got.sort();
        want.sort();
        prop_assert_eq!(&got, &want);

        let anchor_index = anchor
            .filter(|&a| a >= 1 && a <= n)
            .map(|a| (a - 1) as usize);
        match anchor_index {
            None => {
                // No (or unknown) anchor: plain playlist order.
                let got: Vec<Ed2kHash> = order.iter().map(|e| e.hash).collect();
                let want: Vec<Ed2kHash> = playlist.iter().map(|e| e.hash).collect();
                prop_assert_eq!(got, want);
            }
            Some(i) => {
                // Anchor first; then after-anchor in playlist order
                // (monotone by distance); then before-anchor reversed
                // (nearest-first). Concretely the whole ranking is
                // [i..n) ++ rev([0..i)), which encodes all three claims.
                let want: Vec<Ed2kHash> = playlist[i..]
                    .iter()
                    .chain(playlist[..i].iter().rev())
                    .map(|e| e.hash)
                    .collect();
                let got: Vec<Ed2kHash> = order.iter().map(|e| e.hash).collect();
                prop_assert_eq!(got, want);
            }
        }
    }
}

//! Playlist ordering on top of `crdts::Identifier` dense positions.
//!
//! Entries live in the playlist map keyed by file hash; display order is
//! the LWW-resolved entries sorted by `(position, hash)`. Adding and
//! moving compute a fresh `Identifier` between the relevant neighbors.
//! The underlying rationals are kept from growing without bound by the
//! server's daily compaction, which rebuilds the playlist from its
//! resolved order with flat `0, 1, 2, ...` identifiers via
//! [`CrdtState::push_playlist_entry`] (see [`crate::compact::rebuild`]).
//! [`CrdtState::rebalance_playlist`] does the same reassignment in place
//! and is exercised by the playlist property/fuzz suites, but it is not
//! the live compaction path.

use crdts::Identifier;

use crate::state::{CrdtOp, CrdtState};
use crate::types::{ActorId, Ed2kHash, PlaylistFileState, SharedTimestamp, UserId};

/// A resolved playlist entry.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlaylistEntry {
    /// The file's id.
    pub hash: Ed2kHash,
    /// The LWW-winning entry state.
    pub state: PlaylistFileState,
}

/// Everything the adder knows about a new entry except its position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewPlaylistEntry {
    /// The file's id.
    pub hash: Ed2kHash,
    /// Who is adding it.
    pub added_by: UserId,
    /// Original filename.
    pub filename: String,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Duration if known.
    pub duration_millis: Option<u64>,
}

impl CrdtState {
    /// The playlist in display order: LWW winners sorted by `(position,
    /// hash)`.
    pub fn playlist_entries(&self) -> Vec<PlaylistEntry> {
        let mut entries: Vec<PlaylistEntry> = self
            .playlist
            .iter()
            .filter_map(|entry| {
                let (hash, reg) = entry.val;
                // Outer None: never written. Inner None: tombstoned.
                crate::lww::resolve_value(reg)
                    .flatten()
                    .map(|state| PlaylistEntry { hash: *hash, state })
            })
            .collect();
        entries.sort_by(|a, b| {
            a.state
                .position
                .cmp(&b.state.position)
                .then_with(|| a.hash.cmp(&b.hash))
        });
        entries
    }

    /// Add a file after `anchor` (`None` = at the front). Appending means
    /// passing the last entry's hash. If `anchor` is absent from the
    /// playlist, the entry lands at the end.
    pub fn add_playlist_entry_after(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        anchor: Option<&Ed2kHash>,
        new: NewPlaylistEntry,
    ) -> CrdtOp {
        let position = self.position_after(anchor, &new.hash, actor);
        let entry = PlaylistFileState {
            position,
            added_by: new.added_by,
            filename: new.filename,
            size_bytes: new.size_bytes,
            duration_millis: new.duration_millis,
        };
        self.set_playlist_entry(actor, ts, new.hash, entry)
    }

    /// Append a file at the end of the playlist.
    pub fn push_playlist_entry(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        new: NewPlaylistEntry,
    ) -> CrdtOp {
        let anchor = self.playlist_entries().last().map(|entry| entry.hash);
        self.add_playlist_entry_after(actor, ts, anchor.as_ref(), new)
    }

    /// Move an existing entry to sit after `anchor` (`None` = to the
    /// front). Returns `None` if the entry doesn't exist or `anchor` is
    /// the entry itself.
    pub fn move_playlist_entry_after(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        hash: Ed2kHash,
        anchor: Option<&Ed2kHash>,
    ) -> Option<CrdtOp> {
        if anchor == Some(&hash) {
            return None;
        }
        let mut state = self
            .playlist_entries()
            .into_iter()
            .find(|entry| entry.hash == hash)?
            .state;
        state.position = self.position_after(anchor, &hash, actor);
        Some(self.set_playlist_entry(actor, ts, hash, state))
    }

    /// Reassign fresh, small identifiers to every entry, preserving
    /// order. Run by the server at compaction.
    pub fn rebalance_playlist(&mut self, actor: ActorId, ts: SharedTimestamp) -> Vec<CrdtOp> {
        let entries = self.playlist_entries();
        let mut ops = Vec::with_capacity(entries.len());
        let mut prev: Option<Identifier<ActorId>> = None;
        for mut entry in entries {
            // between(prev, None) yields a single-level identifier one
            // above prev, so the sequence is 0, 1, 2, ...
            let fresh = Identifier::between(prev.as_ref(), None, actor);
            prev = Some(fresh.clone());
            entry.state.position = fresh;
            ops.push(self.set_playlist_entry(actor, ts, entry.hash, entry.state));
        }
        ops
    }

    /// Compute a position after `anchor`, before whatever currently
    /// follows it — ignoring `moving` (the entry being repositioned).
    /// `anchor: None` means the front; a missing anchor means the end.
    fn position_after(
        &self,
        anchor: Option<&Ed2kHash>,
        moving: &Ed2kHash,
        actor: ActorId,
    ) -> Identifier<ActorId> {
        let entries: Vec<PlaylistEntry> = self
            .playlist_entries()
            .into_iter()
            .filter(|entry| entry.hash != *moving)
            .collect();

        let anchor_index = match anchor {
            None => None,
            Some(hash) => match entries.iter().position(|entry| entry.hash == *hash) {
                Some(index) => Some(index),
                // Anchor vanished (concurrent remove): append at the end.
                None => entries.len().checked_sub(1),
            },
        };

        let low = anchor_index.and_then(|i| entries.get(i));
        let high = match anchor_index {
            None => entries.first(),
            Some(i) => entries.get(i + 1),
        };
        Identifier::between(
            low.map(|entry| &entry.state.position),
            high.map(|entry| &entry.state.position),
            actor,
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    const ACTOR: ActorId = ActorId(1);

    fn ts(t: u64) -> SharedTimestamp {
        SharedTimestamp(t)
    }

    fn hash(i: u8) -> Ed2kHash {
        Ed2kHash([i; 16])
    }

    fn entry(i: u8) -> NewPlaylistEntry {
        NewPlaylistEntry {
            hash: hash(i),
            added_by: UserId::new("baughn"),
            filename: format!("ep{i}.mkv"),
            size_bytes: 1_000 * i as u64,
            duration_millis: Some(1_440_000),
        }
    }

    fn order(state: &CrdtState) -> Vec<Ed2kHash> {
        state.playlist_entries().iter().map(|e| e.hash).collect()
    }

    #[test]
    fn push_appends_in_order() {
        let mut state = CrdtState::new();
        let mut t = 0;
        for i in 1..=4 {
            t += 1;
            state.push_playlist_entry(ACTOR, ts(t), entry(i));
        }
        assert_eq!(order(&state), vec![hash(1), hash(2), hash(3), hash(4)]);
    }

    #[test]
    fn add_after_none_inserts_at_front() {
        let mut state = CrdtState::new();
        state.push_playlist_entry(ACTOR, ts(1), entry(1));
        state.push_playlist_entry(ACTOR, ts(2), entry(2));
        state.add_playlist_entry_after(ACTOR, ts(3), None, entry(3));
        assert_eq!(order(&state), vec![hash(3), hash(1), hash(2)]);
    }

    #[test]
    fn add_in_the_middle() {
        let mut state = CrdtState::new();
        state.push_playlist_entry(ACTOR, ts(1), entry(1));
        state.push_playlist_entry(ACTOR, ts(2), entry(2));
        state.add_playlist_entry_after(ACTOR, ts(3), Some(&hash(1)), entry(3));
        assert_eq!(order(&state), vec![hash(1), hash(3), hash(2)]);
    }

    #[test]
    fn add_after_missing_anchor_appends() {
        let mut state = CrdtState::new();
        state.push_playlist_entry(ACTOR, ts(1), entry(1));
        state.add_playlist_entry_after(ACTOR, ts(2), Some(&hash(9)), entry(2));
        assert_eq!(order(&state), vec![hash(1), hash(2)]);
    }

    #[test]
    fn move_to_front_and_middle() {
        let mut state = CrdtState::new();
        for i in 1..=3 {
            state.push_playlist_entry(ACTOR, ts(i as u64), entry(i));
        }
        state
            .move_playlist_entry_after(ACTOR, ts(4), hash(3), None)
            .unwrap();
        assert_eq!(order(&state), vec![hash(3), hash(1), hash(2)]);

        state
            .move_playlist_entry_after(ACTOR, ts(5), hash(3), Some(&hash(1)))
            .unwrap();
        assert_eq!(order(&state), vec![hash(1), hash(3), hash(2)]);
    }

    #[test]
    fn move_nonexistent_or_self_anchor_is_noop() {
        let mut state = CrdtState::new();
        state.push_playlist_entry(ACTOR, ts(1), entry(1));
        assert!(
            state
                .move_playlist_entry_after(ACTOR, ts(2), hash(9), None)
                .is_none()
        );
        assert!(
            state
                .move_playlist_entry_after(ACTOR, ts(2), hash(1), Some(&hash(1)))
                .is_none()
        );
    }

    #[test]
    fn remove_then_readd() {
        let mut state = CrdtState::new();
        state.push_playlist_entry(ACTOR, ts(1), entry(1));
        state.push_playlist_entry(ACTOR, ts(2), entry(2));
        state.remove_playlist_entry(ACTOR, ts(3), hash(1));
        assert_eq!(order(&state), vec![hash(2)]);
        state.push_playlist_entry(ACTOR, ts(3), entry(1));
        assert_eq!(order(&state), vec![hash(2), hash(1)]);
    }

    #[test]
    fn rebalance_preserves_order_with_flat_identifiers() {
        let mut state = CrdtState::new();
        // Repeated front-inserts make identifiers grow.
        for i in 1..=6 {
            state.add_playlist_entry_after(ACTOR, ts(i as u64), None, entry(i));
        }
        let before = order(&state);
        let ops = state.rebalance_playlist(ActorId::SERVER, ts(100));
        assert_eq!(ops.len(), 6);
        assert_eq!(order(&state), before);

        // Fresh identifiers are single-level (small rationals). The Vec
        // inside Identifier is private; its Display separates levels with
        // commas.
        for entry in state.playlist_entries() {
            let rendered = format!("{}", entry.state.position);
            assert!(
                !rendered.contains(','),
                "expected single-level identifier, got {rendered}"
            );
        }

        // A replica that only sees the ops converges to the same order.
        let mut replica = CrdtState::new();
        // Rebuild history: replay the original adds via a second state is
        // not needed — merge carries everything.
        replica.merge(state.clone());
        assert_eq!(order(&replica), before);
    }
}

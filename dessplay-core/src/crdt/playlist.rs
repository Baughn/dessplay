use serde::{Deserialize, Serialize};

use crate::protocol::PlaylistAction;
use crate::types::{FileId, SharedTimestamp};

/// Playlist CRDT — an ordered set maintained by a base state plus operation log.
///
/// The `base` is the materialized playlist from the last compaction (epoch
/// snapshot). The `ops` are changes since that compaction. The snapshot
/// (ordered list of FileIds) is produced by starting from `base` and replaying
/// `ops` in timestamp order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    base: Vec<FileId>,
    ops: Vec<(SharedTimestamp, PlaylistAction)>,
}

impl Default for Playlist {
    fn default() -> Self {
        Self::new()
    }
}

impl Playlist {
    pub fn new() -> Self {
        Self {
            base: Vec::new(),
            ops: Vec::new(),
        }
    }

    /// Apply an operation. Inserts into the log in sorted timestamp order.
    /// Returns true if the operation was new (not a duplicate).
    pub fn apply(&mut self, timestamp: SharedTimestamp, action: PlaylistAction) -> bool {
        // Check for duplicate (same timestamp and action)
        if self.ops.iter().any(|(ts, a)| *ts == timestamp && *a == action) {
            return false;
        }

        // Insert in sorted order by (timestamp, action) for deterministic ordering
        let pos = self
            .ops
            .partition_point(|(ts, a)| (*ts, a) <= (timestamp, &action));
        self.ops.insert(pos, (timestamp, action));
        true
    }

    /// Replay all operations on top of the base to produce the current ordered playlist.
    pub fn snapshot(&self) -> Vec<FileId> {
        let mut list: Vec<FileId> = self.base.clone();

        for (_, action) in &self.ops {
            match action {
                PlaylistAction::Add { file_id, after } => {
                    // Skip if already present
                    if list.contains(file_id) {
                        continue;
                    }
                    match after {
                        Some(anchor) => {
                            if let Some(pos) = list.iter().position(|id| id == anchor) {
                                list.insert(pos + 1, *file_id);
                            } else {
                                // Anchor not found, append at end
                                list.push(*file_id);
                            }
                        }
                        None => {
                            list.push(*file_id);
                        }
                    }
                }
                PlaylistAction::Remove { file_id } => {
                    list.retain(|id| id != file_id);
                }
                PlaylistAction::Move { file_id, after } => {
                    // Remove from current position
                    let Some(old_pos) = list.iter().position(|id| id == file_id) else {
                        continue; // Not present, skip
                    };
                    list.remove(old_pos);

                    // Insert at new position
                    match after {
                        Some(anchor) => {
                            if let Some(pos) = list.iter().position(|id| id == anchor) {
                                list.insert(pos + 1, *file_id);
                            } else {
                                list.push(*file_id);
                            }
                        }
                        None => {
                            // None means "move to beginning"
                            list.insert(0, *file_id);
                        }
                    }
                }
            }
        }

        list
    }

    /// Deterministic hash of base + ops. Used for version vectors.
    ///
    /// When hashes differ between peers, all ops are exchanged and the
    /// receiver deduplicates. This avoids the bug where a lower-timestamp
    /// op was invisible to timestamp-based version tracking.
    pub fn version(&self) -> u64 {
        // FNV-1a style hash over base + ops
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        // Hash base (materialized state from last compaction)
        h = h.wrapping_mul(0x0100_0000_01b3).wrapping_add(self.base.len() as u64);
        for fid in &self.base {
            h = Self::hash_file_id(h, fid);
        }
        // Hash ops (changes since compaction)
        h = h.wrapping_mul(0x0100_0000_01b3).wrapping_add(self.ops.len() as u64);
        for (ts, action) in &self.ops {
            h = h.wrapping_mul(0x0100_0000_01b3).wrapping_add(*ts);
            match action {
                PlaylistAction::Add { file_id, after } => {
                    h = h.wrapping_mul(0x0100_0000_01b3).wrapping_add(0);
                    h = Self::hash_file_id(h, file_id);
                    h = Self::hash_option_file_id(h, after.as_ref());
                }
                PlaylistAction::Remove { file_id } => {
                    h = h.wrapping_mul(0x0100_0000_01b3).wrapping_add(1);
                    h = Self::hash_file_id(h, file_id);
                }
                PlaylistAction::Move { file_id, after } => {
                    h = h.wrapping_mul(0x0100_0000_01b3).wrapping_add(2);
                    h = Self::hash_file_id(h, file_id);
                    h = Self::hash_option_file_id(h, after.as_ref());
                }
            }
        }
        h
    }

    fn hash_file_id(mut h: u64, fid: &FileId) -> u64 {
        for chunk in fid.0.chunks(8) {
            let mut buf = [0u8; 8];
            buf[..chunk.len()].copy_from_slice(chunk);
            h = h.wrapping_mul(0x0100_0000_01b3).wrapping_add(u64::from_le_bytes(buf));
        }
        h
    }

    fn hash_option_file_id(mut h: u64, fid: Option<&FileId>) -> u64 {
        match fid {
            Some(fid) => {
                h = h.wrapping_mul(0x0100_0000_01b3).wrapping_add(1);
                Self::hash_file_id(h, fid)
            }
            None => h.wrapping_mul(0x0100_0000_01b3).wrapping_add(0),
        }
    }

    /// All operations that the remote may be missing.
    ///
    /// `remote_version` is the remote's `version()` hash. If it differs from
    /// ours, we send ALL ops (receiver deduplicates). If equal, nothing to send.
    pub fn ops_since(&self, remote_version: u64) -> Vec<(SharedTimestamp, PlaylistAction)> {
        if remote_version == self.version() {
            return Vec::new();
        }
        self.ops.clone()
    }

    /// Load from a compacted snapshot (materialized playlist, no ops).
    pub fn from_materialized(files: Vec<FileId>) -> Self {
        Self {
            base: files,
            ops: Vec::new(),
        }
    }

    /// Load from an op log (used for gap-fill and op-based sync).
    pub fn from_ops(ops: Vec<(SharedTimestamp, PlaylistAction)>) -> Self {
        let mut playlist = Self::new();
        for (ts, action) in ops {
            playlist.apply(ts, action);
        }
        playlist
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn fid(n: u8) -> FileId {
        let mut id = [0u8; 16];
        id[0] = n;
        FileId(id)
    }

    #[test]
    fn add_single() {
        let mut pl = Playlist::new();
        pl.apply(1, PlaylistAction::Add { file_id: fid(1), after: None });
        assert_eq!(pl.snapshot(), vec![fid(1)]);
    }

    #[test]
    fn add_multiple() {
        let mut pl = Playlist::new();
        pl.apply(1, PlaylistAction::Add { file_id: fid(1), after: None });
        pl.apply(2, PlaylistAction::Add { file_id: fid(2), after: None });
        assert_eq!(pl.snapshot(), vec![fid(1), fid(2)]);
    }

    #[test]
    fn add_after() {
        let mut pl = Playlist::new();
        pl.apply(1, PlaylistAction::Add { file_id: fid(1), after: None });
        pl.apply(2, PlaylistAction::Add { file_id: fid(3), after: None });
        pl.apply(3, PlaylistAction::Add { file_id: fid(2), after: Some(fid(1)) });
        assert_eq!(pl.snapshot(), vec![fid(1), fid(2), fid(3)]);
    }

    #[test]
    fn add_duplicate_ignored() {
        let mut pl = Playlist::new();
        pl.apply(1, PlaylistAction::Add { file_id: fid(1), after: None });
        pl.apply(2, PlaylistAction::Add { file_id: fid(1), after: None });
        assert_eq!(pl.snapshot(), vec![fid(1)]);
    }

    #[test]
    fn remove() {
        let mut pl = Playlist::new();
        pl.apply(1, PlaylistAction::Add { file_id: fid(1), after: None });
        pl.apply(2, PlaylistAction::Add { file_id: fid(2), after: None });
        pl.apply(3, PlaylistAction::Remove { file_id: fid(1) });
        assert_eq!(pl.snapshot(), vec![fid(2)]);
    }

    #[test]
    fn remove_absent_ignored() {
        let mut pl = Playlist::new();
        pl.apply(1, PlaylistAction::Remove { file_id: fid(99) });
        assert_eq!(pl.snapshot(), Vec::<FileId>::new());
    }

    #[test]
    fn move_item() {
        let mut pl = Playlist::new();
        pl.apply(1, PlaylistAction::Add { file_id: fid(1), after: None });
        pl.apply(2, PlaylistAction::Add { file_id: fid(2), after: None });
        pl.apply(3, PlaylistAction::Add { file_id: fid(3), after: None });
        // Move fid(3) to after fid(1)
        pl.apply(4, PlaylistAction::Move { file_id: fid(3), after: Some(fid(1)) });
        assert_eq!(pl.snapshot(), vec![fid(1), fid(3), fid(2)]);
    }

    #[test]
    fn move_absent_ignored() {
        let mut pl = Playlist::new();
        pl.apply(1, PlaylistAction::Add { file_id: fid(1), after: None });
        pl.apply(2, PlaylistAction::Move { file_id: fid(99), after: None });
        assert_eq!(pl.snapshot(), vec![fid(1)]);
    }

    #[test]
    fn concurrent_adds_same_anchor_sorted_by_timestamp() {
        // Two adds with the same anchor, applied in timestamp order
        let mut pl = Playlist::new();
        pl.apply(1, PlaylistAction::Add { file_id: fid(1), after: None });
        // Both add after fid(1) — earlier timestamp first
        pl.apply(2, PlaylistAction::Add { file_id: fid(2), after: Some(fid(1)) });
        pl.apply(3, PlaylistAction::Add { file_id: fid(3), after: Some(fid(1)) });
        // fid(2) was inserted first (ts=2), then fid(3) after fid(1) (ts=3)
        // After replaying: [1, 2, 3] because fid(2) goes after 1 first, then fid(3) goes after 1 (pushing 2 right)
        assert_eq!(pl.snapshot(), vec![fid(1), fid(3), fid(2)]);
    }

    #[test]
    fn concurrent_moves_last_wins() {
        let mut pl = Playlist::new();
        pl.apply(1, PlaylistAction::Add { file_id: fid(1), after: None });
        pl.apply(2, PlaylistAction::Add { file_id: fid(2), after: None });
        pl.apply(3, PlaylistAction::Add { file_id: fid(3), after: None });
        // Two moves of the same item — last timestamp wins
        pl.apply(4, PlaylistAction::Move { file_id: fid(3), after: Some(fid(1)) });
        pl.apply(5, PlaylistAction::Move { file_id: fid(3), after: None }); // Move to beginning
        assert_eq!(pl.snapshot(), vec![fid(3), fid(1), fid(2)]);
    }

    #[test]
    fn add_then_remove() {
        let mut pl = Playlist::new();
        pl.apply(1, PlaylistAction::Add { file_id: fid(1), after: None });
        pl.apply(2, PlaylistAction::Remove { file_id: fid(1) });
        assert_eq!(pl.snapshot(), Vec::<FileId>::new());
    }

    #[test]
    fn version_tracking() {
        let mut pl = Playlist::new();
        let v0 = pl.version();
        pl.apply(10, PlaylistAction::Add { file_id: fid(1), after: None });
        let v1 = pl.version();
        assert_ne!(v0, v1, "version must change when ops are added");
        pl.apply(5, PlaylistAction::Add { file_id: fid(2), after: None });
        let v2 = pl.version();
        assert_ne!(v1, v2, "version must change when more ops are added");
    }

    #[test]
    fn duplicate_op_rejected() {
        let mut pl = Playlist::new();
        assert!(pl.apply(1, PlaylistAction::Add { file_id: fid(1), after: None }));
        assert!(!pl.apply(1, PlaylistAction::Add { file_id: fid(1), after: None }));
    }

    // --- Regression test: version hash must include FileId ---
    #[test]
    fn version_differs_for_different_file_ids() {
        // Regression: version() only hashed action discriminant, not FileId.
        // Two playlists with different FileIds but the same timestamps and
        // action types produced identical hashes, causing sync to miss ops.
        let mut pl_a = Playlist::new();
        pl_a.apply(100, PlaylistAction::Remove { file_id: fid(1) });

        let mut pl_b = Playlist::new();
        pl_b.apply(100, PlaylistAction::Remove { file_id: fid(2) });

        assert_ne!(
            pl_a.version(),
            pl_b.version(),
            "version must differ when FileIds differ",
        );
    }

    // --- Regression test: ops_since must not miss lower-timestamp ops ---
    #[test]
    fn ops_since_misses_lower_timestamps() {
        let mut pl = Playlist::new();
        pl.apply(10, PlaylistAction::Add { file_id: fid(1), after: None });

        // Record version after first op
        let v1 = pl.version();

        // Add a second op with LOWER timestamp
        pl.apply(5, PlaylistAction::Add { file_id: fid(2), after: None });

        // ops_since(v1) must include the ts=5 op
        let ops = pl.ops_since(v1);
        assert!(
            ops.iter().any(|(ts, _)| *ts == 5),
            "ops_since must return the lower-timestamp op; got: {ops:?}"
        );
    }

    // --- Base + ops (compaction) tests ---

    #[test]
    fn from_materialized_snapshot() {
        let pl = Playlist::from_materialized(vec![fid(1), fid(2), fid(3)]);
        assert_eq!(pl.snapshot(), vec![fid(1), fid(2), fid(3)]);
    }

    #[test]
    fn base_plus_add() {
        let mut pl = Playlist::from_materialized(vec![fid(1)]);
        pl.apply(100, PlaylistAction::Add { file_id: fid(2), after: None });
        assert_eq!(pl.snapshot(), vec![fid(1), fid(2)]);
    }

    #[test]
    fn base_plus_remove() {
        let mut pl = Playlist::from_materialized(vec![fid(1), fid(2)]);
        pl.apply(100, PlaylistAction::Remove { file_id: fid(1) });
        assert_eq!(pl.snapshot(), vec![fid(2)]);
    }

    #[test]
    fn base_plus_move() {
        let mut pl = Playlist::from_materialized(vec![fid(1), fid(2), fid(3)]);
        pl.apply(100, PlaylistAction::Move { file_id: fid(3), after: Some(fid(1)) });
        assert_eq!(pl.snapshot(), vec![fid(1), fid(3), fid(2)]);
    }

    #[test]
    fn base_add_duplicate_ignored() {
        let mut pl = Playlist::from_materialized(vec![fid(1)]);
        pl.apply(100, PlaylistAction::Add { file_id: fid(1), after: None });
        assert_eq!(pl.snapshot(), vec![fid(1)]);
    }

    #[test]
    fn version_changes_with_base() {
        let empty = Playlist::new();
        let with_base = Playlist::from_materialized(vec![fid(1)]);
        assert_ne!(
            empty.version(),
            with_base.version(),
            "empty vs non-empty base must have different versions",
        );
    }

    #[test]
    fn compaction_round_trip() {
        // Build a playlist via ops
        let mut pl = Playlist::new();
        pl.apply(1, PlaylistAction::Add { file_id: fid(1), after: None });
        pl.apply(2, PlaylistAction::Add { file_id: fid(2), after: None });
        pl.apply(3, PlaylistAction::Add { file_id: fid(3), after: None });
        pl.apply(4, PlaylistAction::Remove { file_id: fid(2) });
        pl.apply(5, PlaylistAction::Move { file_id: fid(3), after: None });

        let materialized = pl.snapshot();

        // "Compact" by creating from materialized state
        let compacted = Playlist::from_materialized(materialized.clone());
        assert_eq!(compacted.snapshot(), materialized);

        // New ops should layer on top correctly
        let mut pl_after = compacted.clone();
        pl_after.apply(10, PlaylistAction::Add { file_id: fid(4), after: Some(fid(1)) });

        let mut pl_from_scratch = Playlist::new();
        pl_from_scratch.apply(1, PlaylistAction::Add { file_id: fid(1), after: None });
        pl_from_scratch.apply(2, PlaylistAction::Add { file_id: fid(2), after: None });
        pl_from_scratch.apply(3, PlaylistAction::Add { file_id: fid(3), after: None });
        pl_from_scratch.apply(4, PlaylistAction::Remove { file_id: fid(2) });
        pl_from_scratch.apply(5, PlaylistAction::Move { file_id: fid(3), after: None });
        pl_from_scratch.apply(10, PlaylistAction::Add { file_id: fid(4), after: Some(fid(1)) });

        assert_eq!(pl_after.snapshot(), pl_from_scratch.snapshot());
    }
}

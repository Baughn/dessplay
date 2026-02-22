pub mod chat;
pub mod lww;
pub mod playlist;
pub mod version;

use serde::{Deserialize, Serialize};

use crate::protocol::{CrdtOp, CrdtSnapshot, LwwValue, RegisterId, VersionVectors};
use crate::types::{AniDbMetadata, FileId, FileState, SharedTimestamp, UserId, UserState};

use self::chat::Chat;
use self::lww::LwwRegister;
use self::playlist::Playlist;

/// Container holding all CRDT state. Single entry point for the sync engine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrdtState {
    epoch: u64,
    pub user_states: LwwRegister<UserId, UserState>,
    pub file_states: LwwRegister<(UserId, FileId), FileState>,
    pub anidb: LwwRegister<FileId, Option<AniDbMetadata>>,
    pub playlist: Playlist,
    pub chat: Chat,
}

impl Default for CrdtState {
    fn default() -> Self {
        Self::new()
    }
}

impl CrdtState {
    pub fn new() -> Self {
        Self {
            epoch: 0,
            user_states: LwwRegister::new(),
            file_states: LwwRegister::new(),
            anidb: LwwRegister::new(),
            playlist: Playlist::new(),
            chat: Chat::new(),
        }
    }

    /// Apply a single CRDT operation, dispatching to the correct sub-CRDT.
    ///
    /// Returns true if the operation changed state (was not a duplicate/stale).
    /// Returns false (and ignores the op) if validation fails.
    pub fn apply_op(&mut self, op: &CrdtOp) -> bool {
        if !Self::validate_op(op) {
            return false;
        }
        match op {
            CrdtOp::LwwWrite { timestamp, value } => self.apply_lww_write(*timestamp, value),
            CrdtOp::PlaylistOp { timestamp, action } => {
                self.playlist.apply(*timestamp, action.clone())
            }
            CrdtOp::ChatAppend {
                user_id,
                seq,
                timestamp,
                text,
            } => self.chat.append(user_id.clone(), *seq, *timestamp, text.clone()),
        }
    }

    /// Reject ops with invalid fields: timestamp 0 (reserved sentinel),
    /// non-finite f32 progress (breaks PartialOrd tiebreak in LWW).
    fn validate_op(op: &CrdtOp) -> bool {
        match op {
            CrdtOp::LwwWrite { timestamp, value } => {
                if *timestamp == 0 {
                    return false;
                }
                if let LwwValue::FileState(_, _, FileState::Downloading { progress }) = value
                    && !progress.is_finite()
                {
                    return false;
                }
                true
            }
            CrdtOp::PlaylistOp { timestamp, .. } => *timestamp != 0,
            CrdtOp::ChatAppend { timestamp, .. } => *timestamp != 0,
        }
    }

    /// Apply a typed LWW write to the correct register.
    fn apply_lww_write(&mut self, timestamp: SharedTimestamp, value: &LwwValue) -> bool {
        match value {
            LwwValue::UserState(uid, val) => {
                self.user_states.write(uid.clone(), timestamp, *val)
            }
            LwwValue::FileState(uid, fid, val) => {
                self.file_states
                    .write((uid.clone(), *fid), timestamp, val.clone())
            }
            LwwValue::AniDb(fid, val) => self.anidb.write(*fid, timestamp, val.clone()),
        }
    }

    /// Produce a full snapshot of the current state.
    pub fn snapshot(&self) -> CrdtSnapshot {
        CrdtSnapshot {
            user_states: self.user_states.clone().into_inner(),
            file_states: self.file_states.clone().into_inner(),
            anidb: self.anidb.clone().into_inner(),
            playlist: self.playlist.snapshot(),
            chat: self.chat.clone().into_inner(),
        }
    }

    /// Build version vectors summarizing our current state.
    pub fn version_vectors(&self) -> VersionVectors {
        let mut vv = VersionVectors::new(self.epoch);

        for (key, (ts, _)) in self.user_states.iter() {
            vv.lww_versions
                .insert(RegisterId::UserState(key.clone()), *ts);
        }
        for (key, (ts, _)) in self.file_states.iter() {
            let (uid, fid) = key;
            vv.lww_versions
                .insert(RegisterId::FileState(uid.clone(), *fid), *ts);
        }
        for (key, (ts, _)) in self.anidb.iter() {
            vv.lww_versions.insert(RegisterId::AniDb(*key), *ts);
        }

        for uid in self.chat.users() {
            if let Some(seq) = self.chat.version(uid) {
                vv.chat_versions.insert(uid.clone(), seq);
            }
        }

        vv.playlist_version = self.playlist.version();

        vv
    }

    /// Return ops that the remote is missing, based on their version vectors.
    pub fn ops_since(&self, remote: &VersionVectors) -> Vec<CrdtOp> {
        let mut ops = Vec::new();

        // LWW registers: send any that are newer or equal (equal timestamps may
        // hide value differences from Ord tiebreaking)
        for (key, (ts, val)) in self.user_states.iter() {
            let reg = RegisterId::UserState(key.clone());
            let remote_ts = remote.lww_versions.get(&reg).copied().unwrap_or(0);
            if *ts >= remote_ts {
                ops.push(CrdtOp::LwwWrite {
                    timestamp: *ts,
                    value: LwwValue::UserState(key.clone(), *val),
                });
            }
        }

        for (key, (ts, val)) in self.file_states.iter() {
            let (uid, fid) = key;
            let reg = RegisterId::FileState(uid.clone(), *fid);
            let remote_ts = remote.lww_versions.get(&reg).copied().unwrap_or(0);
            if *ts >= remote_ts {
                ops.push(CrdtOp::LwwWrite {
                    timestamp: *ts,
                    value: LwwValue::FileState(uid.clone(), *fid, val.clone()),
                });
            }
        }

        for (key, (ts, val)) in self.anidb.iter() {
            let reg = RegisterId::AniDb(*key);
            let remote_ts = remote.lww_versions.get(&reg).copied().unwrap_or(0);
            if *ts >= remote_ts {
                ops.push(CrdtOp::LwwWrite {
                    timestamp: *ts,
                    value: LwwValue::AniDb(*key, val.clone()),
                });
            }
        }

        // Playlist ops since remote's known version
        for (ts, action) in self.playlist.ops_since(remote.playlist_version) {
            ops.push(CrdtOp::PlaylistOp {
                timestamp: ts,
                action,
            });
        }

        // Chat entries since remote's known versions
        for uid in self.chat.users() {
            let remote_seq = remote.chat_versions.get(uid).copied();
            for entry in self.chat.entries_since(uid, remote_seq) {
                ops.push(CrdtOp::ChatAppend {
                    user_id: uid.clone(),
                    seq: entry.seq,
                    timestamp: entry.timestamp,
                    text: entry.text,
                });
            }
        }

        ops
    }

    /// Replace all state from a compacted snapshot (used after epoch change).
    pub fn load_snapshot(&mut self, epoch: u64, snap: CrdtSnapshot) {
        self.epoch = epoch;
        self.user_states = LwwRegister::from_inner(snap.user_states);
        self.file_states = LwwRegister::from_inner(snap.file_states);
        self.anidb = LwwRegister::from_inner(snap.anidb);
        self.playlist = Playlist::from_materialized(snap.playlist);
        self.chat = Chat::from_inner(snap.chat);
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Set the epoch (used by compaction).
    pub(crate) fn set_epoch(&mut self, epoch: u64) {
        self.epoch = epoch;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::protocol::PlaylistAction;
    use crate::types::AniDbMetadata;

    fn uid(name: &str) -> UserId {
        UserId(name.to_string())
    }

    fn fid(n: u8) -> FileId {
        let mut id = [0u8; 16];
        id[0] = n;
        FileId(id)
    }

    fn make_lww_op(value: LwwValue, timestamp: SharedTimestamp) -> CrdtOp {
        CrdtOp::LwwWrite { timestamp, value }
    }

    #[test]
    fn apply_user_state() {
        let mut state = CrdtState::new();
        let op = make_lww_op(LwwValue::UserState(uid("alice"), UserState::Ready), 100);
        assert!(state.apply_op(&op));
        assert_eq!(
            state.user_states.read(&uid("alice")),
            Some(&UserState::Ready)
        );
    }

    #[test]
    fn apply_file_state() {
        let mut state = CrdtState::new();
        let op = make_lww_op(
            LwwValue::FileState(uid("alice"), fid(1), FileState::Missing),
            100,
        );
        assert!(state.apply_op(&op));
        assert_eq!(
            state.file_states.read(&(uid("alice"), fid(1))),
            Some(&FileState::Missing)
        );
    }

    #[test]
    fn apply_playlist_op() {
        let mut state = CrdtState::new();
        let op = CrdtOp::PlaylistOp {
            timestamp: 1,
            action: PlaylistAction::Add {
                file_id: fid(1),
                after: None,
            },
        };
        assert!(state.apply_op(&op));
        assert_eq!(state.playlist.snapshot(), vec![fid(1)]);
    }

    #[test]
    fn apply_chat_op() {
        let mut state = CrdtState::new();
        let op = CrdtOp::ChatAppend {
            user_id: uid("alice"),
            seq: 0,
            timestamp: 100,
            text: "hello".into(),
        };
        assert!(state.apply_op(&op));
        let view = state.chat.merged_view();
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].1.text, "hello");
    }

    #[test]
    fn snapshot_round_trip() {
        let mut state = CrdtState::new();

        // Add some state
        state.apply_op(&make_lww_op(
            LwwValue::UserState(uid("alice"), UserState::Paused),
            10,
        ));
        state.apply_op(&CrdtOp::PlaylistOp {
            timestamp: 1,
            action: PlaylistAction::Add {
                file_id: fid(1),
                after: None,
            },
        });
        state.apply_op(&CrdtOp::ChatAppend {
            user_id: uid("bob"),
            seq: 0,
            timestamp: 50,
            text: "hi".into(),
        });

        let snap = state.snapshot();

        let mut state2 = CrdtState::new();
        state2.load_snapshot(0, snap);

        assert_eq!(state.user_states, state2.user_states);
        assert_eq!(state.playlist.snapshot(), state2.playlist.snapshot());
        assert_eq!(state.chat.merged_view().len(), state2.chat.merged_view().len());
    }

    #[test]
    fn version_vectors_reflect_state() {
        let mut state = CrdtState::new();

        state.apply_op(&make_lww_op(
            LwwValue::UserState(uid("alice"), UserState::Ready),
            42,
        ));
        state.apply_op(&CrdtOp::ChatAppend {
            user_id: uid("bob"),
            seq: 0,
            timestamp: 100,
            text: "msg".into(),
        });
        state.apply_op(&CrdtOp::PlaylistOp {
            timestamp: 77,
            action: PlaylistAction::Add {
                file_id: fid(1),
                after: None,
            },
        });

        let vv = state.version_vectors();
        // LWW versions are still timestamp-based
        assert_eq!(
            vv.lww_versions.get(&RegisterId::UserState(uid("alice"))),
            Some(&42)
        );
        // Chat and playlist versions are now hashes — just check they're present
        assert!(vv.chat_versions.contains_key(&uid("bob")));
        assert_ne!(vv.playlist_version, 0, "playlist version should be non-zero after adding ops");
    }

    #[test]
    fn ops_since_returns_missing() {
        let mut state = CrdtState::new();

        state.apply_op(&make_lww_op(
            LwwValue::UserState(uid("alice"), UserState::Ready),
            42,
        ));
        state.apply_op(&CrdtOp::ChatAppend {
            user_id: uid("bob"),
            seq: 0,
            timestamp: 100,
            text: "msg".into(),
        });

        // Remote knows nothing
        let remote_vv = VersionVectors::new(0);
        let ops = state.ops_since(&remote_vv);
        assert_eq!(ops.len(), 2);
    }

    #[test]
    fn ops_since_returns_empty_when_up_to_date() {
        let state = CrdtState::new();
        let vv = state.version_vectors();
        let ops = state.ops_since(&vv);
        assert!(ops.is_empty());
    }

    // --- Regression tests for fuzz-discovered bugs ---

    #[test]
    fn test_nan_filestate_rejected() {
        let mut state = CrdtState::new();
        let op = make_lww_op(
            LwwValue::FileState(
                uid("alice"),
                fid(1),
                FileState::Downloading {
                    progress: f32::NAN,
                },
            ),
            100,
        );
        assert!(!state.apply_op(&op));
        assert_eq!(state.file_states.read(&(uid("alice"), fid(1))), None);

        // Also reject infinity
        let op_inf = make_lww_op(
            LwwValue::FileState(
                uid("alice"),
                fid(1),
                FileState::Downloading {
                    progress: f32::INFINITY,
                },
            ),
            100,
        );
        assert!(!state.apply_op(&op_inf));
        assert_eq!(state.file_states.read(&(uid("alice"), fid(1))), None);
    }

    #[test]
    fn test_timestamp_zero_rejected() {
        let mut state = CrdtState::new();

        // LWW with timestamp 0
        let lww_op = make_lww_op(
            LwwValue::UserState(uid("alice"), UserState::Ready),
            0,
        );
        assert!(!state.apply_op(&lww_op));
        assert_eq!(state.user_states.read(&uid("alice")), None);

        // Playlist with timestamp 0
        let playlist_op = CrdtOp::PlaylistOp {
            timestamp: 0,
            action: PlaylistAction::Add {
                file_id: fid(1),
                after: None,
            },
        };
        assert!(!state.apply_op(&playlist_op));
        assert!(state.playlist.snapshot().is_empty());

        // Chat with timestamp 0
        let chat_op = CrdtOp::ChatAppend {
            user_id: uid("alice"),
            seq: 0,
            timestamp: 0,
            text: "hello".into(),
        };
        assert!(!state.apply_op(&chat_op));
        assert!(state.chat.merged_view().is_empty());
    }

    #[test]
    fn playlist_sync_lower_timestamp_op() {
        // Peer A has ops at ts=10 and ts=5
        let mut peer_a = CrdtState::new();
        peer_a.apply_op(&CrdtOp::PlaylistOp {
            timestamp: 10,
            action: PlaylistAction::Add {
                file_id: fid(1),
                after: None,
            },
        });
        peer_a.apply_op(&CrdtOp::PlaylistOp {
            timestamp: 5,
            action: PlaylistAction::Add {
                file_id: fid(2),
                after: None,
            },
        });

        // Peer B has only the ts=10 op
        let mut peer_b = CrdtState::new();
        peer_b.apply_op(&CrdtOp::PlaylistOp {
            timestamp: 10,
            action: PlaylistAction::Add {
                file_id: fid(1),
                after: None,
            },
        });

        // Peer A should send the ts=5 op to peer B
        let b_vv = peer_b.version_vectors();
        let ops = peer_a.ops_since(&b_vv);
        let playlist_ops: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, CrdtOp::PlaylistOp { .. }))
            .collect();
        assert!(
            !playlist_ops.is_empty(),
            "ops_since must send the lower-timestamp playlist op"
        );

        // Apply and verify convergence
        for op in &ops {
            peer_b.apply_op(op);
        }
        assert_eq!(peer_a.playlist.snapshot(), peer_b.playlist.snapshot());
    }

    #[test]
    fn test_chat_gap_fill_with_noncontiguous_seqs() {
        // Peer A ("behind") has only seq 3
        let mut behind = CrdtState::new();
        behind.apply_op(&CrdtOp::ChatAppend {
            user_id: uid("alice"),
            seq: 3,
            timestamp: 400,
            text: "msg3".into(),
        });

        // Peer B ("ahead") has seq 1 and seq 3
        let mut ahead = CrdtState::new();
        ahead.apply_op(&CrdtOp::ChatAppend {
            user_id: uid("alice"),
            seq: 1,
            timestamp: 200,
            text: "msg1".into(),
        });
        ahead.apply_op(&CrdtOp::ChatAppend {
            user_id: uid("alice"),
            seq: 3,
            timestamp: 400,
            text: "msg3".into(),
        });

        // Behind has a version hash for alice (it has seq 3)
        let behind_vv = behind.version_vectors();
        assert!(behind_vv.chat_versions.contains_key(&uid("alice")));

        // Hashes differ so ahead should send all its ops
        let ops = ahead.ops_since(&behind_vv);
        let chat_ops: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, CrdtOp::ChatAppend { .. }))
            .collect();
        assert_eq!(chat_ops.len(), 2);

        // Apply them — behind now has seq 1 and 3
        for op in &ops {
            behind.apply_op(op);
        }
        let entries = behind.chat.entries_since(&uid("alice"), None);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].seq, 1);
        assert_eq!(entries[1].seq, 3);
    }

    // --- Regression tests for LWW tiebreak divergence (fuzz round 3) ---

    #[test]
    fn test_ops_since_lww_tiebreak_same_timestamp() {
        // Regression: ops_since used `>` instead of `>=`, so when remote had the
        // same timestamp but a different (losing) value from a tiebreak, the
        // winning value was never sent.
        let user = uid("alice");

        // "behind" peer writes Ready at ts=100
        let mut behind = CrdtState::new();
        behind.apply_op(&make_lww_op(
            LwwValue::UserState(user.clone(), UserState::Ready),
            100,
        ));

        // "ahead" = clone of behind, then receives a concurrent write at the
        // SAME timestamp but with a higher value (NotWatching > Ready in Ord)
        let mut ahead = behind.clone();
        ahead.apply_op(&make_lww_op(
            LwwValue::UserState(user.clone(), UserState::NotWatching),
            100,
        ));

        // Sanity: ahead resolved the tiebreak to NotWatching (higher Ord value)
        assert_eq!(
            ahead.user_states.read(&user),
            Some(&UserState::NotWatching),
        );
        // behind still has Ready
        assert_eq!(
            behind.user_states.read(&user),
            Some(&UserState::Ready),
        );

        // behind's version vector says ts=100 for this register
        let behind_vv = behind.version_vectors();
        assert_eq!(
            behind_vv
                .lww_versions
                .get(&RegisterId::UserState(user.clone())),
            Some(&100),
        );

        // ops_since must return the winning write so behind can catch up
        let catch_up = ahead.ops_since(&behind_vv);
        assert!(
            !catch_up.is_empty(),
            "ops_since must send the tiebreak-winning value when timestamps are equal",
        );

        // Apply catch-up and verify convergence
        for op in &catch_up {
            behind.apply_op(op);
        }
        assert_eq!(ahead.snapshot(), behind.snapshot());
    }

    #[test]
    fn test_ops_since_lww_tiebreak_all_register_types() {
        // Verify the fix applies to all three LWW register types:
        // user_states, file_states, and anidb.
        let user = uid("bob");
        let file = fid(1);

        let mut behind = CrdtState::new();
        // FileState: Ready (discriminant 0) at ts=200
        behind.apply_op(&make_lww_op(
            LwwValue::FileState(user.clone(), file, FileState::Ready),
            200,
        ));
        // AniDb: None at ts=300
        behind.apply_op(&make_lww_op(LwwValue::AniDb(file, None), 300));

        let mut ahead = behind.clone();
        // FileState: Missing (discriminant 1 > 0) at same ts=200
        ahead.apply_op(&make_lww_op(
            LwwValue::FileState(user.clone(), file, FileState::Missing),
            200,
        ));
        // AniDb: Some(metadata) > None at same ts=300
        let meta = AniDbMetadata {
            anime_id: 1,
            anime_name: "Test".into(),
            episode_number: 1,
            episode_name: "Ep1".into(),
            group_name: "Grp".into(),
        };
        ahead.apply_op(&make_lww_op(LwwValue::AniDb(file, Some(meta)), 300));

        let behind_vv = behind.version_vectors();
        let catch_up = ahead.ops_since(&behind_vv);

        // Must include both the FileState and AniDb ops
        let file_state_ops = catch_up
            .iter()
            .filter(|op| matches!(op, CrdtOp::LwwWrite { value: LwwValue::FileState(..), .. }))
            .count();
        let anidb_ops = catch_up
            .iter()
            .filter(|op| matches!(op, CrdtOp::LwwWrite { value: LwwValue::AniDb(..), .. }))
            .count();

        assert_eq!(file_state_ops, 1, "must send FileState tiebreak winner");
        assert_eq!(anidb_ops, 1, "must send AniDb tiebreak winner");

        for op in &catch_up {
            behind.apply_op(op);
        }
        assert_eq!(ahead.snapshot(), behind.snapshot());
    }

}

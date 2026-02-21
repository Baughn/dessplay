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
    pub fn apply_op(&mut self, op: &CrdtOp) -> bool {
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
            playlist_ops: self.playlist.ops().to_vec(),
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

        // LWW registers: send any that are newer than remote's known version
        for (key, (ts, val)) in self.user_states.iter() {
            let reg = RegisterId::UserState(key.clone());
            let remote_ts = remote.lww_versions.get(&reg).copied().unwrap_or(0);
            if *ts > remote_ts {
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
            if *ts > remote_ts {
                ops.push(CrdtOp::LwwWrite {
                    timestamp: *ts,
                    value: LwwValue::FileState(uid.clone(), *fid, val.clone()),
                });
            }
        }

        for (key, (ts, val)) in self.anidb.iter() {
            let reg = RegisterId::AniDb(*key);
            let remote_ts = remote.lww_versions.get(&reg).copied().unwrap_or(0);
            if *ts > remote_ts {
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
        self.playlist = Playlist::from_ops(snap.playlist_ops);
        self.chat = Chat::from_inner(snap.chat);
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::protocol::PlaylistAction;

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
        assert_eq!(
            vv.lww_versions.get(&RegisterId::UserState(uid("alice"))),
            Some(&42)
        );
        assert_eq!(vv.chat_versions.get(&uid("bob")), Some(&0));
        assert_eq!(vv.playlist_version, 77);
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

}

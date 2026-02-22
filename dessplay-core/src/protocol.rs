use std::collections::BTreeMap;
use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use crate::types::{AniDbMetadata, FileId, FileState, PeerId, SharedTimestamp, UserId, UserState};

// ---------------------------------------------------------------------------
// CRDT Operations
// ---------------------------------------------------------------------------

/// Identifies which LWW register a write targets (key without value).
///
/// Used in version vectors and gap-fill requests where only the register
/// identity is needed.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RegisterId {
    UserState(UserId),
    FileState(UserId, FileId),
    AniDb(FileId),
}

/// A typed LWW register value, combining register identity and payload.
///
/// This replaces the previous `RegisterId` + `Vec<u8>` representation so that
/// the wire format cannot carry a value mismatched for its register type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
pub enum LwwValue {
    UserState(UserId, UserState),
    FileState(UserId, FileId, FileState),
    AniDb(FileId, Option<AniDbMetadata>),
}

// Manual Eq: FileState contains f32 (progress), but values are always finite.
impl Eq for LwwValue {}

impl LwwValue {
    /// Extract the register identity (for version vectors and gap fill).
    pub fn register_id(&self) -> RegisterId {
        match self {
            LwwValue::UserState(uid, _) => RegisterId::UserState(uid.clone()),
            LwwValue::FileState(uid, fid, _) => RegisterId::FileState(uid.clone(), *fid),
            LwwValue::AniDb(fid, _) => RegisterId::AniDb(*fid),
        }
    }
}

/// A single playlist mutation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
pub enum PlaylistAction {
    Add {
        file_id: FileId,
        after: Option<FileId>,
    },
    Remove {
        file_id: FileId,
    },
    Move {
        file_id: FileId,
        after: Option<FileId>,
    },
}

/// A CRDT operation — the unit of replication.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
pub enum CrdtOp {
    /// LWW Register write with strongly-typed value.
    LwwWrite {
        timestamp: SharedTimestamp,
        value: LwwValue,
    },

    /// Playlist operation.
    PlaylistOp {
        timestamp: SharedTimestamp,
        action: PlaylistAction,
    },

    /// Chat message append.
    ChatAppend {
        user_id: UserId,
        seq: u64,
        timestamp: SharedTimestamp,
        text: String,
    },
}

// ---------------------------------------------------------------------------
// Version Vectors & Gap Fill
// ---------------------------------------------------------------------------

/// Compact summary of a peer's CRDT state, used to detect missed operations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionVectors {
    pub epoch: u64,
    /// Per-register: latest timestamp seen.
    pub lww_versions: BTreeMap<RegisterId, SharedTimestamp>,
    /// Per-user: highest chat sequence number seen.
    pub chat_versions: BTreeMap<UserId, u64>,
    /// Playlist: latest op timestamp seen.
    pub playlist_version: SharedTimestamp,
}

/// Request for missing ops from a peer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GapFillRequest {
    pub lww_needed: Vec<(RegisterId, SharedTimestamp)>,
    pub chat_needed: Vec<(UserId, u64)>,
    pub playlist_after: Option<SharedTimestamp>,
}

/// Response containing the requested ops.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GapFillResponse {
    pub ops: Vec<CrdtOp>,
}

// ---------------------------------------------------------------------------
// CRDT Snapshot
// ---------------------------------------------------------------------------

/// A compacted snapshot of all CRDT state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrdtSnapshot {
    pub user_states: BTreeMap<UserId, (SharedTimestamp, UserState)>,
    pub file_states: BTreeMap<(UserId, FileId), (SharedTimestamp, FileState)>,
    pub anidb: BTreeMap<FileId, (SharedTimestamp, Option<AniDbMetadata>)>,
    pub playlist: Vec<FileId>,
    pub chat: BTreeMap<UserId, Vec<ChatEntry>>,
}

/// A single chat message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatEntry {
    pub seq: u64,
    pub timestamp: SharedTimestamp,
    pub text: String,
}

// ---------------------------------------------------------------------------
// Peer Info
// ---------------------------------------------------------------------------

/// Information about a connected peer, distributed by the rendezvous server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerInfo {
    pub peer_id: PeerId,
    pub username: String,
    pub addresses: Vec<SocketAddr>,
    pub connected_since: SharedTimestamp,
}

// ---------------------------------------------------------------------------
// Rendezvous Control Messages
// ---------------------------------------------------------------------------

/// Messages on the client ↔ rendezvous server control stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RvControl {
    // Client → Server
    Auth { password: String, username: String },
    TimeSyncRequest { client_send: u64 },
    AniDbLookup { file_id: FileId, file_size: u64 },

    // Server → Client
    AuthOk { peer_id: PeerId, observed_addr: SocketAddr },
    AuthFailed,
    PeerList { peers: Vec<PeerInfo> },
    TimeSyncResponse {
        client_send: u64,
        server_recv: u64,
        server_send: u64,
    },

    // Bidirectional (state sync)
    StateSnapshot {
        epoch: u64,
        crdts: CrdtSnapshot,
    },
    StateOp {
        op: CrdtOp,
    },
    StateSummary {
        versions: VersionVectors,
    },
}

// ---------------------------------------------------------------------------
// Peer-to-Peer Control Messages
// ---------------------------------------------------------------------------

/// Messages on the peer ↔ peer control stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeerControl {
    Hello { peer_id: PeerId, username: String },

    // State sync
    StateOp { op: CrdtOp },
    StateSummary {
        epoch: u64,
        versions: VersionVectors,
    },
    StateSnapshot {
        epoch: u64,
        crdts: CrdtSnapshot,
    },

    // File transfer
    FileAvailability { file_id: FileId, bitfield: Vec<u8> },
}

// ---------------------------------------------------------------------------
// Peer Datagram Messages
// ---------------------------------------------------------------------------

/// Messages sent via QUIC unreliable datagrams between peers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PeerDatagram {
    /// Playback position (ephemeral LWW, 100ms/1s interval).
    Position {
        timestamp: SharedTimestamp,
        position_secs: f64,
    },

    /// Seek command (debounced 1500ms).
    Seek {
        timestamp: SharedTimestamp,
        target_secs: f64,
    },

    /// Best-effort eager push of a state operation.
    StateOp { op: CrdtOp },
}

// f64 in Position/Seek prevents auto-derive of Eq, but our values are always finite.
impl Eq for PeerDatagram {}

// ---------------------------------------------------------------------------
// Relay Envelope
// ---------------------------------------------------------------------------

/// Wrapper for relayed messages through the rendezvous server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelayEnvelope {
    /// Forward enclosed message to the specified peer.
    Forward { to: PeerId, message: Vec<u8> },
    /// A message forwarded from another peer.
    Forwarded { from: PeerId, message: Vec<u8> },
}

// ---------------------------------------------------------------------------
// File Transfer
// ---------------------------------------------------------------------------

/// Request for file chunks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkRequest {
    pub file_id: FileId,
    pub chunks: Vec<u32>,
}

/// A single file chunk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkData {
    pub index: u32,
    pub data: Vec<u8>,
}

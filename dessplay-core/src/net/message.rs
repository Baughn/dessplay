//! Wire message types for the client <-> server protocol.
//!
//! Everything is postcard-encoded. [`WireMessage`] is the top-level type
//! on both the control stream and datagrams; its postcard discriminant
//! (one byte for <128 variants) is the "message type tag" from
//! docs/network-design.md. Stream messages are length-prefixed by the
//! framing layer; datagrams are self-contained.

use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use crate::state::{CrdtOp, StateSnapshot};
use crate::types::{Ed2kHash, Epoch, UserId};

/// Top-level wire message. Phase 9 adds a `Relay` variant for file
/// transfer envelopes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WireMessage {
    /// Client <-> server control traffic.
    Control(ServerControl),
}

/// A client's role, declared at auth time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// Full experience: TUI, player, a human. Gates playback.
    Interactive,
    /// Headless auto-fetcher. Never gates playback.
    Seeder,
}

/// The server's view of a peer's liveness (not CRDT state).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Presence {
    /// Normal operation; counted in playback gating.
    Present,
    /// 30s without traffic; everyone pauses.
    Lost,
    /// 60s without traffic; removed from gating, shown dimmed.
    Departed,
}

/// One entry in the peer list the server pushes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerInfo {
    /// The username the server authenticated.
    pub username: UserId,
    /// Interactive or seeder.
    pub role: Role,
    /// Current presence stage.
    pub presence: Presence,
    /// Observed + self-reported addresses (v4 + v6). Informational.
    pub addresses: Vec<SocketAddr>,
    /// Shared-clock millis when the peer connected.
    pub connected_since: u64,
}

/// Client <-> server control messages. See docs/network-design.md for
/// the connection flow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServerControl {
    // ---- Client -> Server
    /// First message on the control stream.
    Auth {
        /// Self-chosen nickname; the server rejects none but supersedes
        /// an existing connection with the same name.
        username: UserId,
        /// Shared room password, plaintext under TLS.
        password: String,
        /// Interactive or seeder.
        role: Role,
        /// Last known epoch; drives snapshot-vs-merge on the server.
        epoch: Epoch,
    },
    /// NTP-style probe. Sent as a datagram when supported.
    TimeSyncRequest {
        /// Client clock at send, millis.
        client_send: u64,
    },
    /// Player reached end of file. A report, not state: the server owns
    /// the EOF -> next-file transition.
    EofReached {
        /// Which file ended.
        file: Ed2kHash,
    },

    // ---- Server -> Client
    /// Auth accepted.
    AuthOk {
        /// The client's address as the server saw it. Informational.
        observed_addr: SocketAddr,
    },
    /// Auth rejected; the server closes the connection after sending.
    AuthFailed,
    /// Full peer list; pushed on every join/leave/presence change.
    PeerList {
        /// All known peers, including the recipient.
        peers: Vec<PeerInfo>,
    },
    /// NTP-style probe response.
    TimeSyncResponse {
        /// Echoed from the request.
        client_send: u64,
        /// Server clock at receive, millis.
        server_recv: u64,
        /// Server clock at send, millis.
        server_send: u64,
    },

    // ---- Bidirectional state sync (consumed from Phase 4 on)
    /// Full state replacement (stale epoch or compaction broadcast).
    StateSnapshot(StateSnapshot),
    /// One CmRDT operation.
    StateOp(CrdtOp),
    /// Full CvRDT state for merge-based reconnection sync.
    StateMerge(StateSnapshot),

    // ---- Divergence alarm (see sync-state.md)
    /// Server -> client, every 30s: hash of the server's resolved view
    /// (excluding playback positions).
    StateHash {
        /// The server's current epoch.
        epoch: Epoch,
        /// `CrdtState::view_hash()` output.
        hash: [u8; 32],
    },
    /// Client -> server: my view hash mismatched twice in a row; please
    /// send a `StateMerge`.
    RequestMerge,
}

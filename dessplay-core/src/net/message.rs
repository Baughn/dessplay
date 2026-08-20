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
use crate::types::{AniDbSeriesId, Ed2kHash, Epoch, UserId};

/// The wire protocol version, checked at auth time (design.md request
/// #23). Bump on **any** change to wire messages, `CrdtOp`, or the
/// encoding of a CRDT value type. Append enum variants and struct
/// fields, never reorder or remove — and never reshape `Auth` itself:
/// its stability is what lets a mismatched future client still be
/// decoded and answered with a readable [`ServerControl::ProtocolMismatch`]
/// instead of a silent decode failure.
/// v11 (2026-08-17): appended `SeriesRelations::short_titles`.
/// v12 (2026-08-18): appended `ServerControl::SetAnthropicToken`
/// (wire-only; the persisted layout is unchanged from v11).
/// v13 (2026-08-21): appended `ServerControl::SyncStatus` — the client
/// half of the connect handshake (wire-only; the persisted layout is
/// unchanged from v11).
pub const PROTOCOL_VERSION: u32 = 13;

/// Top-level wire message: only control traffic. File-transfer relay
/// envelopes are **not** a `WireMessage` variant -- they are framed as
/// `RelayEnvelope` (see `net::transfer`) on a dedicated relay stream,
/// kept off the control channel so bulk transfer never head-of-line-
/// blocks state sync (docs/network-design.md, Transfer Stream / Relay
/// Envelope).
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

/// A username the server has seen before, with when it last connected or
/// disconnected — design.md #15: lets the group name and act on
/// (`n` / `/skip <name>`) someone who hasn't connected *this session*
/// (possibly not since a server restart), which the live [`PeerInfo`] list
/// alone cannot represent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownUser {
    /// The username.
    pub username: UserId,
    /// Shared-clock millis of the last connect or disconnect.
    pub last_seen: u64,
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
        /// Last known epoch. Informational/log-only since protocol v13:
        /// the snapshot-vs-merge decision moved to the post-auth
        /// [`Self::SyncStatus`] handshake, which also carries the state
        /// hash. Kept in place — `Auth` is never reshaped (see above).
        epoch: Epoch,
        /// The client's [`PROTOCOL_VERSION`]. Deliberately the *last*
        /// field: a pre-versioning client's `Auth` is a strict prefix
        /// of this shape, so it fails decode cleanly and the server can
        /// answer with the (old-decodable) `AuthFailed` rather than a
        /// `ProtocolMismatch` the old binary cannot read.
        protocol_version: u32,
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
    /// Graceful quit (`/quit`, Ctrl-C). The server removes the user
    /// immediately (no Lost stage) and pauses playback. A connection
    /// that dies without this goes through Lost -> Departed instead.
    Goodbye,

    // ---- Server -> Client
    /// Auth accepted.
    AuthOk {
        /// The client's address as the server saw it. Informational.
        observed_addr: SocketAddr,
        /// One-time token binding the **transfer connection** to this
        /// session: the client presents it in [`Self::TransferAuth`] on
        /// the second, DSCP-tagged QUIC connection it dials next (to the
        /// control port + 1). Regenerated on every successful `Auth`, so
        /// a stale token from a superseded connection is refused.
        /// Appended field (bump policy): protocol v8.
        transfer_token: u64,
    },
    /// Auth rejected; the server closes the connection after sending.
    AuthFailed,
    /// Full peer list; pushed on every join/leave/presence change.
    PeerList {
        /// All known peers, including the recipient.
        peers: Vec<PeerInfo>,
        /// Known usernames not currently in `peers` (design.md #15):
        /// this-session departures and never-connected-today users alike,
        /// from the server's persisted registry, within the retention
        /// window. Replaces the old plain "departed" display -- see
        /// [`KnownUser`].
        known_offline: Vec<KnownUser>,
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
    /// One CmRDT operation. Epoch-tagged: ops generated against a
    /// different epoch are dropped by both sides. Without the tag, an
    /// op in flight across a compaction would land on the rebuilt state
    /// and pollute its freshly-reset per-actor dot sequences — the
    /// sender's next post-adoption ops would then be silently deduped
    /// as "already seen".
    StateOp {
        /// The epoch the op was generated against.
        epoch: Epoch,
        /// The operation.
        op: CrdtOp,
    },
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

    // ---- AniDB name search (backs the AniDbSearch modal)
    /// Client -> server: search the anime-titles index by name. The
    /// search runs locally on the server over the daily titles dump —
    /// the UDP API has no multi-result search.
    AniDbSearch {
        /// The (partial, informal) name to search for.
        query: String,
    },
    /// Server -> client: results, echoing the query so a slow reply to
    /// a superseded search can be ignored.
    AniDbSearchResults {
        /// The query these results answer.
        query: String,
        /// Best matches, one per series.
        results: Vec<AniDbSearchHit>,
    },

    // ---- Protocol version gate
    /// Server -> client: your `Auth` carried a different
    /// [`PROTOCOL_VERSION`]; admission refused. The server closes the
    /// connection after sending; the client must not retry. Appended
    /// after the pre-existing variants so their discriminants never
    /// move.
    ProtocolMismatch {
        /// The server's [`PROTOCOL_VERSION`].
        server_version: u32,
    },

    // ---- Manual mark-watched (design.md #10)
    /// Client -> server: cycle a file's group watched flag from the
    /// episode browser. Unlike `EofReached` this is not scoped to
    /// now-playing and touches no playback register -- just the watched
    /// flag, plus (when setting `true`) the same List `next_ep`
    /// auto-advance the EOF path gets. Appended last so no existing
    /// discriminant moves (see the bump policy above); logically it
    /// belongs with the other client -> server requests.
    MarkWatched {
        /// The file whose watched flag to set.
        file: Ed2kHash,
        /// The new value.
        watched: bool,
    },

    // ---- Transfer connection binding (protocol v8)
    /// Client -> server: the first (and only expected) control frame on
    /// the **transfer connection** — the second QUIC connection, dialed
    /// to the control port + 1 with the bulk DSCP tag. Binds it to the
    /// session `AuthOk` authenticated: the server validates `token`
    /// against the one it issued to `username`'s live control
    /// connection. All relay (file-transfer) streams ride the transfer
    /// connection; presence remains keyed to the control connection
    /// alone, so a dead transfer link degrades transfers, never
    /// liveness. Appended last (bump policy).
    TransferAuth {
        /// Who this transfer connection belongs to.
        username: UserId,
        /// The token from this session's [`Self::AuthOk`].
        token: u64,
    },

    // ---- AI curator credential (protocol v12)
    /// Client -> server: store (`Some`) or clear (`None`) the Anthropic
    /// API token the server's short-title curator uses (design.md, The
    /// List). Client-provisioned on purpose: the token lives in one
    /// client's settings, is pushed on connect (when set) and on any
    /// settings edit that changes it, and the server persists it in its
    /// SQLite — so the settings screen is also the interface for
    /// rotating or removing the server-side credential. Plaintext under
    /// TLS, like the room password. Never logged, either side. Appended
    /// last (bump policy).
    SetAnthropicToken {
        /// The token, or `None` to clear the stored one.
        token: Option<String>,
    },

    // ---- Connect-handshake sync status (protocol v13)
    /// Client -> server: sent once per connection, right after `AuthOk`
    /// reaches the sync actor. Carries what the client actually holds,
    /// so the server can decide the initial sync *curatively*: it
    /// answers `StateMerge` iff both the epoch AND the view hash match
    /// its own, and `StateSnapshot` otherwise — a bare epoch match must
    /// never buy a merge, because after a server DB restore the epoch
    /// counter can collide while the states differ, and merging then
    /// re-pollutes the freshly restored state with the clients' stale
    /// unions (the 2026-08 tsugumi restore incident). Appended last
    /// (bump policy).
    SyncStatus {
        /// The epoch the client's replica is at.
        epoch: Epoch,
        /// The client's `CrdtState::view_hash()` output.
        state_hash: [u8; 32],
    },
}

/// One AniDB name-search result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AniDbSearchHit {
    /// The series id to link.
    pub series: AniDbSeriesId,
    /// The series' primary title, for display.
    pub title: String,
    /// The title/synonym the query matched (often an informal name).
    pub matched: String,
}

impl ServerControl {
    /// The variant's name, for logging. Payloads (state snapshots in
    /// particular) can be huge; log this plus the encoded byte size
    /// instead of the message contents.
    pub fn variant_name(&self) -> &'static str {
        match self {
            ServerControl::Auth { .. } => "Auth",
            ServerControl::TimeSyncRequest { .. } => "TimeSyncRequest",
            ServerControl::EofReached { .. } => "EofReached",
            ServerControl::Goodbye => "Goodbye",
            ServerControl::AuthOk { .. } => "AuthOk",
            ServerControl::AuthFailed => "AuthFailed",
            ServerControl::PeerList { .. } => "PeerList",
            ServerControl::TimeSyncResponse { .. } => "TimeSyncResponse",
            ServerControl::StateSnapshot(_) => "StateSnapshot",
            ServerControl::StateOp { .. } => "StateOp",
            ServerControl::StateMerge(_) => "StateMerge",
            ServerControl::StateHash { .. } => "StateHash",
            ServerControl::RequestMerge => "RequestMerge",
            ServerControl::AniDbSearch { .. } => "AniDbSearch",
            ServerControl::AniDbSearchResults { .. } => "AniDbSearchResults",
            ServerControl::ProtocolMismatch { .. } => "ProtocolMismatch",
            ServerControl::MarkWatched { .. } => "MarkWatched",
            ServerControl::TransferAuth { .. } => "TransferAuth",
            ServerControl::SetAnthropicToken { .. } => "SetAnthropicToken",
            ServerControl::SyncStatus { .. } => "SyncStatus",
        }
    }
}

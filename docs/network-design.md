# DessPlay Network Protocol Design

Last updated: 2026-02-16

Sub-design for the network module. See `docs/design.md` for overall architecture.

## Layer Overview

```
┌─────────────────────────────────────────────┐
│  Application Channels                        │
│  (player state, user states, playlist, chat) │
├─────────────────────────────────────────────┤
│  Sync Engine                                 │
│  (LWW registers, append logs, gossip)        │
├─────────────────────────────────────────────┤
│  Clock Sync                                  │
│  (NTP-like shared timestamps)                │
├─────────────────────────────────────────────┤
│  Connection Manager                          │
│  (QUIC via quinn, rendezvous, TURN relay)    │
└─────────────────────────────────────────────┘
```

Each layer only talks to the one directly below it.

## Layer 1: Connection Manager

### Responsibility

Abstracts away all connectivity concerns. Upper layers see two communication
channels:
- **Datagrams** — unreliable, unordered, low-latency (for gossip snapshots, clock sync)
- **Reliable streams** — ordered, guaranteed delivery (for gap fill responses)
- Peer discovery notifications

Handles internally: rendezvous server communication, TURN relay fallback,
per-peer connection state machine.

### Transport: QUIC via quinn

All peer-to-peer communication uses QUIC (`quinn` crate). This provides:

- **TLS 1.3 encryption** built-in (no separate encryption layer needed)
- **Reliable streams** for gap fill and large messages
- **Unreliable datagrams** (QUIC datagram extension) for gossip and clock sync
- **Multiplexing** — multiple streams over one connection, no head-of-line blocking
- **Connection migration** — survives IP changes (mobile, VPN toggle)
- **Built-in congestion control** — no manual flow control needed
- **Keep-alive** — QUIC idle timeout replaces manual heartbeats

No fragmentation layer is needed. QUIC handles packet-level fragmentation
transparently, and reliable streams handle arbitrarily large payloads.

### Rendezvous Server

Located on VPS. Separate binary (`dessplay-rendezvous`).

Responsibilities:
1. **Peer registration** — clients announce themselves
2. **Peer list** — clients learn about other peers in their room
3. **STUN** — tell clients their public IP:port (observed address in Register response)
4. **TURN relay** — fallback when direct QUIC connection fails

**Protocol**: QUIC connection between client and server, with a long-lived
bidirectional control stream using length-prefixed postcard messages:

```rust
// Client → Server
enum ClientMessage {
    Register { peer_id: String, password: String },
    Keepalive,
}

// Server → Client
enum ServerMessage {
    Registered { peers: Vec<PeerEntry>, your_addr: SocketAddr },
    PeerList { peers: Vec<PeerEntry> },
    AuthFailed { reason: String },
}
```

**Authentication**: Password sent in plaintext over the TLS-encrypted QUIC
connection. No HMAC key derivation needed — TLS provides confidentiality.
Server configured via `--password-file` or `DESSPLAY_PASSWORD` env var.

**TLS: TOFU (Trust On First Use)**: SSH-style certificate trust for the
rendezvous server. Server generates a self-signed cert on first run, persists
it to disk (stable identity), and prints its fingerprint on startup. Client
stores `server_name → SHA256 fingerprint` on first connect; subsequent
connections verify the fingerprint matches. Mismatch rejects the connection.
Peer-to-peer connections continue using skip-verification.

**Relay protocol**: Datagrams and uni streams on the rendezvous QUIC connection,
with a simple header: `[1 byte: peer_id length][peer_id UTF-8 bytes][payload]`.
Server swaps the peer_id (destination → source) when forwarding.

The rendezvous server does NOT participate in state sync. Once peers are
connected, they communicate directly.

### Connection Lifecycle (per peer)

```
Disconnected → Connecting → Direct (QUIC handshake to peer address succeeds)
                           → Relayed (relay via rendezvous server)
                           → Disconnected (handshake timeout)

Direct/Relayed → Disconnected (QUIC idle timeout)
Relayed → Direct (periodic direct probe succeeds)
```

QUIC's built-in keep-alive (configured idle timeout) replaces manual heartbeat
logic. A connection transitions to Disconnected when QUIC reports the connection
as lost. No hole-punching — all peers expected to have IPv6 connectivity.

### API Surface

```rust
#[async_trait]
pub trait ConnectionManager: Send + Sync {
    /// Send an unreliable datagram to a specific peer.
    /// Used for gossip snapshots and clock sync messages.
    /// May be silently dropped by the network.
    async fn send_datagram(&self, peer: PeerId, data: &[u8]) -> Result<()>;

    /// Receive the next incoming datagram from any peer.
    async fn recv_datagram(&self) -> Result<(PeerId, Vec<u8>)>;

    /// Send data reliably to a specific peer via a QUIC stream.
    /// Used for gap fill responses that must not be lost.
    /// Returns Err if the peer is disconnected or partitioned.
    async fn send_reliable(&self, peer: PeerId, data: &[u8]) -> Result<()>;

    /// Receive the next reliable message from any peer.
    async fn recv_reliable(&self) -> Result<(PeerId, Vec<u8>)>;

    /// Subscribe to connection events.
    fn subscribe(&self) -> broadcast::Receiver<ConnectionEvent>;

    /// List currently connected peers.
    fn connected_peers(&self) -> Vec<PeerId>;
}

enum ConnectionEvent {
    PeerConnected(PeerId),
    PeerDisconnected(PeerId),
    ConnectionStateChanged { peer: PeerId, state: ConnectionState },
}

enum ConnectionState {
    Disconnected,
    Connecting,
    Direct,
    Relayed,
}
```

### Dual Channel Rationale

**Datagrams** are used for gossip snapshots and clock sync because:
- They're fire-and-forget — no retransmission overhead
- Old snapshots are worthless if a newer one arrives first
- Loss is tolerable (next broadcast is <1s away)

**Reliable streams** are used for gap fill responses because:
- Missing append log entries must not be lost
- Entries must arrive in order and complete
- A dropped gap fill would trigger another request (wasted round trip)

## Layer 2: Clock Sync

### Responsibility

Establish a shared clock across all peers so timestamps are comparable.
All timestamps in layers above use this clock.

### Protocol

NTP-like exchange against the rendezvous server (the one stable reference).
Clock sync messages are sent as **QUIC datagrams** (low-latency, loss-tolerant):

1. Client sends `Ping { t1: local_time }`
2. Server responds `Pong { t1, t2: server_time, t3: server_time }`
3. Client receives at `t4`
4. Offset = `((t2 - t1) + (t3 - t4)) / 2`
5. Repeat periodically, use rolling median to filter outliers

### Peer-to-Peer Refinement

Once peers are connected, they also exchange timestamps directly with each
other via QUIC datagrams. This guards against rendezvous server downtime and
allows continuous drift correction without server involvement.

Each peer periodically sends `Ping` to every other peer. Offsets are computed
the same way. The shared clock uses the rendezvous server as the initial
reference and peer-to-peer exchanges to detect and correct drift.

### API Surface

```rust
pub struct SharedClock { /* offset from local monotonic clock */ }

impl SharedClock {
    /// Current shared time.
    pub fn now(&self) -> SharedTimestamp;

    /// Convert shared timestamp to local Instant (for scheduling).
    pub fn to_local(&self, ts: SharedTimestamp) -> Instant;
}
```

`SharedTimestamp` is a newtype over `u64` (microseconds since epoch).

## Layer 3: Sync Engine

### Responsibility

Replicate state across peers using gossip. Provides two primitives that
application channels build on.

### Primitive 1: LWW Register

Last-Writer-Wins register. A single value with a timestamp. On conflict,
highest timestamp wins.

Used for: player position, per-user states, current file.

On receiving a remote value:
- Remote timestamp > local → adopt remote value
- Otherwise → ignore

### Primitive 2: Per-User Append Log

Each user maintains a monotonically numbered sequence of entries. Derived
state is computed by replaying all users' logs in timestamp order.

Used for: chat messages, playlist actions.

**Eager path**: entries are broadcast immediately when created (fire-and-forget
via datagrams).

**State vector**: gossip state includes `HashMap<UserId, SequenceNumber>` — the
latest sequence number seen from each user, per log type.

**Gap fill**: on receiving a state vector showing a peer has entries we're
missing, request them. Gap fill requests are sent via datagrams, but responses
use **reliable streams** (entries must not be lost). Prefer the origin peer,
fall back to any peer that has them (supports the case where origin is
disconnected).

### Gossip Behavior

- **Broadcast interval**: 100ms while playing, 1s while paused
- **Rate transition**: when derived state changes (e.g., someone pauses or
  unpauses), broadcast at 100ms for 1 second, then return to the rate
  appropriate for the new state. This ensures state changes propagate quickly
  even during paused periods.
- **Forward rule**: on receiving state newer than local, adopt it AND forward
  to all other peers. This provides resilience to single-connection failures
  (A↔C↔B works even if A↔B is down).

### Wire Format

Inside QUIC, messages are already encrypted by TLS 1.3, so no HMAC is needed.
The wire format is simply:

```
[version_byte][postcard_payload]
```

The version byte (currently `1`) allows protocol evolution. Peers drop messages
with unknown versions and log a warning.

```rust
// No version byte — WireMessage already provides one.
struct GossipMessage {
    origin: PeerId,
    timestamp: SharedTimestamp,
    payload: GossipPayload,
}

enum GossipPayload {
    /// Periodic full state snapshot
    StateSnapshot {
        player_state: PlayerStateSnapshot,
        user_states: HashMap<PeerId, (UserState, SharedTimestamp)>,
        file_states: HashMap<PeerId, (FileState, SharedTimestamp)>,
        chat_vectors: HashMap<PeerId, SequenceNumber>,
        playlist_vectors: HashMap<PeerId, SequenceNumber>,
    },
    /// Eager-push of a new append log entry (via datagram)
    AppendEntry {
        log_type: LogType,
        user: PeerId,
        seq: SequenceNumber,
        data: Vec<u8>,
        timestamp: SharedTimestamp,
    },
    /// Request missing entries (via datagram)
    GapFillRequest {
        log_type: LogType,
        user: PeerId,
        from_seq: SequenceNumber,
        to_seq: SequenceNumber,
    },
    /// Response with missing entries (via reliable stream)
    GapFillResponse {
        log_type: LogType,
        user: PeerId,
        entries: Vec<(SequenceNumber, Vec<u8>, SharedTimestamp)>,
    },
}
```

Serialization: `serde` + `postcard` (compact binary format).

## Layer 4: Application Channels

Each channel declares which sync primitive it uses and provides its
domain-specific types and merge/replay logic.

### Player State (LWW Registers)

Playback position and current file are LWW registers:

```rust
struct PlayerStateSnapshot {
    file_id: Option<ItemId>,
    position: PositionRegister,
}

struct PositionRegister {
    position: f64,  // seconds
    timestamp: SharedTimestamp,
}
```

Merge rule: highest timestamp wins.

### Derived Pause Logic

Playback state (playing vs paused) is **derived**, not synced as a register.
The video plays iff:
- Every connected user's User State is Ready or NotWatching, AND
- Every connected user's File State permits playback (Ready, or Downloading
  with sufficient progress)

This design eliminates an entire class of conflicts. There is no PauseRegister
that can race with seeks or be overwritten during partitions. Instead:

- **User pauses**: their User State → Paused (LWW register update), which
  immediately makes the derived state "paused" for everyone
- **User unpauses**: their User State → Ready, which makes derived state
  "playing" only if everyone else is also Ready/NotWatching
- **Network partition**: each partition independently computes derived state
  from the users they can see. On reconnection, User State registers merge
  via LWW and the derived state recomputes correctly.

The sync engine broadcasts User State changes; the application layer computes
the derived play/pause state locally.

### User States (Per-user LWW)

Each user broadcasts their own state. No conflicts — each user only
writes their own.

```rust
enum UserState {
    Ready,
    Paused,
    NotWatching,
}
```

### Playlist (Per-user Append Log)

Playlist items have stable IDs (e.g. `(UserId, SequenceNumber)`) rather than
positional indices. This makes concurrent operations commutative — two users
removing different items simultaneously won't corrupt each other's intent.

```rust
/// Stable identifier for a playlist item, assigned on creation.
type ItemId = (UserId, SequenceNumber);

enum PlaylistAction {
    Add { id: ItemId, filename: String, after: Option<ItemId> },
    Remove { id: ItemId },
    Move { id: ItemId, after: Option<ItemId> },
}
```

Current playlist = replay all users' actions in timestamp order. Concurrent
adds from different users both survive because they live in different logs.
`Remove` and `Move` on a nonexistent ID are no-ops (item was already removed
by a concurrent action).

### Chat (Per-user Append Log)

```rust
struct ChatMessage {
    text: String,
    timestamp: SharedTimestamp,
}
```

Display = all messages from all users sorted by timestamp.

Chat messages are delivered via the append log mechanism: eager-push on
creation (datagram), state vectors in snapshots for gap detection, gap fill
via reliable streams for recovery.

### Growth Management

Both append logs grow without bound during a session. In practice, a single
evening produces a few hundred playlist ops and a few thousand chat messages —
negligible for v1.

**TODO (design before v2):** Compaction. Once all peers have ACKed up to
sequence N, compact everything before N into a materialized snapshot. This
requires:
- An ACK protocol (peers report their state vectors)
- A snapshot format for each log type
- A way to bootstrap new peers from snapshot + tail of log

Design this early even if implementation is deferred — the append log data
structures should be built with compaction in mind.

### Late Join / Bootstrapping

There is no distinction between "early" and "late" joiners. When a new peer
connects (via `PeerConnected` event), existing peers immediately send a full
`StateSnapshot`. The new peer applies it, discovers gaps in append logs via
the state vectors embedded in the snapshot, and requests gap fills via
reliable streams.

This means a peer that joins mid-episode automatically receives: the current
playlist, playback position, user states, and enough chat history to fill gaps.

## Implementation Order

Each layer is a session-sized chunk, built bottom-up:

1. **Connection Manager** — QUIC via quinn, peer discovery. SimulatedNetwork for testing. ✅
2. **Rendezvous + Relay** — Rendezvous server (QUIC), TOFU TLS, TURN relay, mesh bootstrap. ✅
3. **Clock Sync** — NTP-like protocol over QUIC datagrams. ✅
4. **Sync Engine** — LWW + append log primitives, gossip forwarding, gap fill. ✅
5. **Application Channels** — Wire up player state, user states, playlist, chat. ✅

Each layer can be tested independently with the SimulatedNetwork (see
`docs/testing-strategy.md` for details).

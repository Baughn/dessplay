# Network Design

Last updated: 2026-02-20

This document covers connection establishment, wire protocols, hole punching,
relay, and file transfer. For the replicated data types built on top of this
layer, see [sync-state.md](sync-state.md).

## Table of Contents

1. [Overview](#overview)
2. [QUIC Transport](#quic-transport)
3. [Rendezvous Protocol](#rendezvous-protocol)
4. [Hole Punching](#hole-punching)
5. [TURN Relay](#turn-relay)
6. [Peer-to-Peer Protocol](#peer-to-peer-protocol)
7. [Time Synchronization](#time-synchronization)
8. [State Sync Wire Protocol](#state-sync-wire-protocol)
9. [File Transfer](#file-transfer)
10. [Reconnection](#reconnection)

---

## Overview

```
                    ┌──────────────────┐
                    │  Rendezvous      │
                    │  Server           │
                    │  (QUIC endpoint) │
                    └──┬─────┬─────┬──┘
                       │     │     │
              QUIC     │     │     │     QUIC
            ┌──────────┘     │     └──────────┐
            │                │                │
       ┌────▼───┐       ┌───▼────┐       ┌───▼────┐
       │ Peer A │◄─────►│ Peer B │◄─────►│ Peer C │
       └────────┘ QUIC  └────────┘ QUIC  └────────┘
                    (direct or relayed)
```

Every client maintains a QUIC connection to the rendezvous server and attempts
direct QUIC connections to every other peer. If direct connection fails, traffic
is relayed through the server.

---

## QUIC Transport

### Connection Types

| Connection | Initiator | Purpose |
|------------|-----------|---------|
| Client → Rendezvous | Client | Auth, peer discovery, time sync, state sync, TURN relay |
| Client → Client | Both (simultaneous open) | Direct state sync, file transfer |

### Channel Usage

Each QUIC connection uses three kinds of channels:

1. **Control stream** — a single long-lived bidirectional stream, opened
   immediately after the connection is established. Carries authentication,
   state sync operations, state summaries, and file availability announcements.
   Messages are length-prefixed and serialized with postcard.

2. **Datagrams** — QUIC unreliable datagrams. Used for playback position
   updates, seek events, and best-effort eager-push of state ops. No
   ordering or delivery guarantees.

3. **On-demand streams** — short-lived bidirectional streams opened as needed
   for gap fill (recovering missed state ops) and file chunk transfer. Opened
   by the requesting peer, closed when the transfer completes.

### TLS and Identity

- **Rendezvous server**: Generates a persistent self-signed certificate.
  Clients use TOFU (Trust On First Use) — the server's certificate fingerprint
  is stored locally on first connection and verified on subsequent connections.
- **Peer-to-peer**: Ephemeral self-signed certificates. Identity is established
  at the application layer (username in Hello message), not at the TLS layer.

### Serialization

All structured messages use **postcard** (serde, compact binary). Messages on
streams are length-prefixed with a `u32` (little-endian) byte count. Datagrams
are self-contained (no length prefix needed since QUIC datagrams are framed).

Every message starts with a `u8` message type tag, followed by the
postcard-encoded body.

---

## Rendezvous Protocol

### Connection Flow

```
Client                          Rendezvous Server
  │                                     │
  │─── QUIC connect (TOFU) ───────────►│
  │                                     │
  │─── Open control stream ───────────►│
  │                                     │
  │─── Auth { password } ────────────►│
  │◄── AuthOk { observed_addr } ───────│
  │                                     │
  │─── TimeSync request ─────────────►│
  │◄── TimeSync response ──────────────│
  │                                     │
  │◄── PeerList { peers } ─────────────│
  │                                     │
  │◄── StateSnapshot { epoch, data } ──│
  │                                     │
  │    (bidirectional state sync and    │
  │     periodic time sync ongoing)     │
```

### Messages (Client ↔ Rendezvous)

```rust
enum RvControl {
    // Client → Server
    Auth { password: String },
    TimeSyncRequest { client_send: u64 },

    // Server → Client
    AuthOk { observed_addr: SocketAddr },
    AuthFailed,
    PeerList { peers: Vec<PeerInfo> },
    TimeSyncResponse {
        client_send: u64,
        server_recv: u64,
        server_send: u64,
    },

    // Bidirectional (state sync — same as peer-to-peer)
    StateSnapshot { epoch: u64, crdts: CrdtSnapshot },
    StateOp { op: CrdtOp },
    StateSummary { versions: VersionVectors },
}

struct PeerInfo {
    username: String,
    addresses: Vec<SocketAddr>,  // observed + self-reported
    connected_since: u64,
}
```

### Peer List Updates

The server pushes an updated `PeerList` whenever a peer joins or leaves.
Clients diff the list against their current peer set and initiate/tear down
connections accordingly.

### Authentication

The password is sent as plaintext in the `Auth` message, protected by QUIC's
TLS 1.3 encryption. The server verifies it against the configured password.
On success, the server responds with `AuthOk` including the client's observed
address (for hole punching). On failure, the server sends `AuthFailed` and
closes the connection.

---

## Hole Punching

Peers are assumed to be on public IPv6 addresses behind stateful firewalls.
The rendezvous server facilitates hole punching:

### Choreography

```
Peer A                  Rendezvous                  Peer B
  │                        │                           │
  │  (both registered,     │                           │
  │   addresses known)     │                           │
  │                        │                           │
  │◄── PeerList ───────────│─── PeerList ────────────►│
  │    (includes B's addr) │    (includes A's addr)   │
  │                        │                           │
  │─── QUIC Initial ──────────────────────────────────►│
  │◄──────────────────────────────────── QUIC Initial ─│
  │                        │                           │
  │    (firewall sees outgoing packet,                 │
  │     allows incoming response)                      │
  │                        │                           │
  │◄═══════════ QUIC connection established ══════════►│
```

### Retry Strategy

1. Both peers begin sending QUIC Initial packets simultaneously upon
   receiving each other's address in the PeerList
2. Retry with exponential backoff: 100ms, 200ms, 400ms, 800ms, 1600ms
3. If no connection after 5 seconds: fall back to TURN relay
4. Continue periodic direct connection attempts in the background
   (every 30s) even while relaying, to recover if the firewall state changes

### Multiple Addresses

The PeerList includes both the server-observed address and any self-reported
addresses (e.g., link-local, additional interfaces). The connecting peer
attempts all addresses in parallel and uses whichever succeeds first.

---

## TURN Relay

When direct connection fails, the rendezvous server relays traffic between
peers. The server **terminates** QUIC — each peer has its own QUIC connection
to the server, and the server bridges them at the application layer.

### Architecture

```
Peer A ◄── QUIC ──► Rendezvous Server ◄── QUIC ──► Peer B
                    (decrypts, re-encrypts)
```

The server acts as an application-layer proxy:
- Messages from A addressed to B are decrypted from A's connection, then
  re-encrypted and sent on B's connection
- Both control stream messages and datagrams are forwarded
- On-demand streams are proxied: when A opens a stream addressed to B, the
  server opens a corresponding stream to B and copies data bidirectionally

### Relay Addressing

Messages that need relay include a `relay_to: PeerId` header so the server
knows where to forward them. For direct connections, this field is absent.

### Relay Message Wrapper

```rust
enum RelayEnvelope {
    /// Forward enclosed message to the specified peer
    Forward { to: PeerId, message: Vec<u8> },
    /// A message forwarded from another peer
    Forwarded { from: PeerId, message: Vec<u8> },
}
```

Relay envelopes wrap the standard peer-to-peer messages. The inner `message`
bytes are decoded by the recipient as normal `PeerControl` / datagram messages.

### Transparency

The peer-to-peer protocol layer does not need to know whether a connection is
direct or relayed. The network layer abstracts this: it provides a `send(peer,
message)` interface, and routes through direct connection or relay as
appropriate.

---

## Peer-to-Peer Protocol

### Connection Flow

```
Peer A                              Peer B
  │                                    │
  │─── QUIC connect (simultaneous) ──►│
  │                                    │
  │─── Open control stream ──────────►│
  │                                    │
  │─── Hello { username } ──────────►│
  │◄── Hello { username } ────────────│
  │                                    │
  │─── StateSummary { versions } ───►│
  │◄── StateSummary { versions } ─────│
  │                                    │
  │    (gap fill if needed, then       │
  │     ongoing bidirectional sync)    │
```

### Messages (Peer ↔ Peer)

```rust
enum PeerControl {
    Hello { username: String },

    // State sync
    StateOp { op: CrdtOp },
    StateSummary { epoch: u64, versions: VersionVectors },

    // File transfer
    FileAvailability { file_id: FileId, bitfield: BitVec },
}
```

### Datagram Messages

```rust
enum PeerDatagram {
    /// Playback position (ephemeral LWW, 100ms/1s interval)
    Position { timestamp: u64, position_secs: f64 },

    /// Seek command (debounced 1500ms)
    Seek { timestamp: u64, target_secs: f64 },

    /// Best-effort eager push of a state operation
    StateOp { op: CrdtOp },
}
```

Datagrams include the sender's user ID implicitly (identified by the QUIC
connection they arrive on).

---

## Time Synchronization

NTP-style protocol run over the rendezvous control stream.

### Exchange

```
Client                          Server
  │                                │
  │  t1 = local_clock()            │
  │─── TimeSyncRequest(t1) ──────►│
  │                                │  t2 = server_clock()  [receive]
  │                                │  t3 = server_clock()  [send]
  │◄── TimeSyncResponse(t1,t2,t3) │
  │                                │
  │  t4 = local_clock()            │
  │                                │
  │  rtt = (t4 - t1) - (t3 - t2)  │
  │  offset = ((t2-t1) + (t3-t4)) / 2
```

### Usage

- Run on initial connection, then every 30 seconds
- Maintain a rolling average of the offset (discard outliers where RTT > 2x
  the median)
- All CRDT operation timestamps and playback positions use
  `local_clock() + offset` to produce shared-clock timestamps
- Precision target: <50ms (sufficient for 3s sync tolerance)

---

## State Sync Wire Protocol

This section describes how the CRDT operations from [sync-state.md](sync-state.md)
are mapped onto the wire.

### CrdtOp Encoding

```rust
enum CrdtOp {
    /// LWW Register write (strongly typed)
    LwwWrite {
        timestamp: u64,
        value: LwwValue,
    },

    /// Playlist operation
    PlaylistOp {
        timestamp: u64,
        action: PlaylistAction,
    },

    /// Chat message
    ChatAppend {
        user_id: UserId,
        seq: u64,
        timestamp: u64,
        text: String,
    },
}

/// Typed LWW register value — the register identity is embedded in the variant.
enum LwwValue {
    UserState(UserId, UserState),
    FileState(UserId, FileId, FileState),
    AniDb(FileId, Option<AniDbMetadata>),
}

/// Register identity without a value (used in version vectors and gap fill).
enum RegisterId {
    UserState(UserId),
    FileState(UserId, FileId),
    AniDb(Ed2kHash),
}

enum PlaylistAction {
    Add { file_id: FileId, after: Option<FileId> },
    Remove { file_id: FileId },
    Move { file_id: FileId, after: Option<FileId> },
}
```

### Version Vectors

```rust
struct VersionVectors {
    epoch: u64,
    /// Per-register: latest timestamp seen
    lww_versions: HashMap<RegisterId, u64>,
    /// Per-user: highest chat sequence number seen
    chat_versions: HashMap<UserId, u64>,
    /// Playlist: latest op timestamp seen
    playlist_version: u64,
}
```

### Sync Flow

1. **On connect**: Both peers exchange `StateSummary` containing their version
   vectors.
2. **Detect gaps**: Each peer compares the received versions against its own.
   For any CRDT where the remote peer has newer data, open an on-demand stream
   and request the missing ops.
3. **Ongoing**: State ops are sent on the control stream (reliable) and
   simultaneously pushed via datagram (best-effort, for lower latency). The
   recipient deduplicates by `(CrdtOp type, timestamp)`.
4. **Periodic summary**: Every 1s, peers exchange `StateSummary` on the control
   stream. If gaps are detected, open an on-demand stream for gap fill.

### Gap Fill Stream

```
Requester                          Provider
  │                                    │
  │─── Open stream ──────────────────►│
  │─── GapFillRequest { ... } ──────►│
  │◄── GapFillResponse { ops } ───────│
  │◄── (stream closed) ───────────────│
```

```rust
struct GapFillRequest {
    /// Which CRDTs need filling, with the requester's known version
    lww_needed: Vec<(RegisterId, u64)>,     // register, known_timestamp
    chat_needed: Vec<(UserId, u64)>,         // user, known_seq
    playlist_after: Option<u64>,             // known_timestamp
}

struct GapFillResponse {
    ops: Vec<CrdtOp>,
}
```

---

## File Transfer

A chunk-based peer-to-peer file transfer protocol for distributing missing
media files across the mesh. Simpler than BitTorrent — all peers are trusted,
and the mesh is small (~4 users).

### Chunks

- Files are divided into **256 KiB chunks** (last chunk may be smaller)
- Chunks are identified by `(file_id, chunk_index)`
- A typical 1.4 GB video file has ~5600 chunks

### Availability Tracking

Each peer maintains a bitfield per file indicating which chunks it has.
Bitfield updates are broadcast on the control stream:

```rust
/// Sent on the control stream
FileAvailability {
    file_id: FileId,
    bitfield: BitVec,  // 1 = have chunk, 0 = don't
}
```

- Sent when a peer begins serving a file (complete bitfield)
- Sent when a downloading peer completes new chunks (updated bitfield)
- Update frequency: at most every 1s during active transfer (batch updates)

### Chunk Selection: Rarest First

When a downloader decides which chunk to request next:

1. Collect availability bitfields from all peers
2. For each missing chunk, count how many peers have it
3. Request the chunk available from the **fewest** peers
4. Break ties randomly

This maximizes the rate at which rare chunks propagate through the mesh.
With 1 seeder and 3 leechers, the seeder sends different chunks to each
leecher; those leechers can then serve each other, roughly tripling effective
throughput.

### Upload Prioritization

When a peer has multiple pending chunk requests:

- Prioritize chunks that the requester is the only one missing (they can't
  get it elsewhere)
- Otherwise, prioritize rarest chunks (same logic as download selection)
- Round-robin between requesting peers to ensure fairness

### Transfer Stream

Each chunk transfer uses an on-demand bidirectional stream:

```
Downloader                         Uploader
  │                                    │
  │─── Open stream ──────────────────►│
  │─── ChunkRequest { file_id,       ►│
  │        chunks: [idx, idx, ...] }   │
  │◄── ChunkData { idx, data } ───────│
  │◄── ChunkData { idx, data } ───────│
  │◄── ... ────────────────────────────│
  │◄── (stream closed) ───────────────│
```

```rust
struct ChunkRequest {
    file_id: FileId,
    chunks: Vec<u32>,  // chunk indices, in preferred order
}

struct ChunkData {
    index: u32,
    data: Vec<u8>,     // up to 256 KiB
}
```

A single stream can request multiple chunks (pipelining). The uploader sends
them in order. The downloader may open parallel streams to different peers.

### Flow Control

- Maximum **4 concurrent transfer streams** per downloading peer (across all
  uploaders)
- Maximum **16 chunks** per request (pipeline depth)
- QUIC flow control handles backpressure naturally
- If a peer's upload bandwidth is saturated, QUIC will slow the sender

### Integration with Playback

When a file is being downloaded for immediate playback:

- Chunk selection switches from rarest-first to **sequential** for the next
  ~20% of the file ahead of the current playback position
- Rarest-first continues for chunks outside the playback window
- This ensures smooth playback while still distributing rare chunks

### Temporary Storage

Downloaded chunks are written to a temporary directory. When 50% of the file's
duration has been watched, the completed file is moved to
`<download_root>/<series>/<season>/<original_filename>` (see design.md, File
Matching).

---

## Reconnection

### Client Reconnects to Rendezvous

1. Re-establish QUIC connection
2. Re-authenticate
3. Re-sync time
4. Send `StateSummary` with local epoch and version vectors
5. Server compares epoch:
   - **Same epoch**: server sends ops since the client's last known versions
   - **Stale epoch**: server sends full `StateSnapshot` with current epoch
6. Resume normal sync

### Client Reconnects to Peer

1. Re-establish QUIC connection (direct or via relay)
2. Exchange `Hello` and `StateSummary`
3. Gap fill as needed
4. Resume normal sync

### Graceful Disconnect

On clean shutdown, a peer closes its control streams. QUIC's connection close
mechanism notifies all connected peers. No explicit "goodbye" message is needed
— the rendezvous server detects the closed connection and pushes an updated
`PeerList` to remaining peers.

### Ungraceful Disconnect

QUIC idle timeout (default: 30s) detects dead connections. On timeout:
- Peers remove the disconnected user from their local peer list
- The rendezvous server pushes an updated `PeerList`
- The disconnected user's state (User State, File State) remains in the CRDTs
  until overwritten on reconnection

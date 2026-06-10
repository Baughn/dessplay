# Network Design

Last updated: 2026-03-04

This document covers connection establishment, wire protocols, relay, and file
transfer. For the replicated data types built on top of this layer, see
[sync-state.md](sync-state.md).

## Table of Contents

1. [Overview](#overview)
2. [QUIC Transport](#quic-transport)
3. [Rendezvous Protocol](#rendezvous-protocol)
4. [Time Synchronization](#time-synchronization)
5. [State Sync Wire Protocol](#state-sync-wire-protocol)
6. [Peer-to-Peer File Transfer](#peer-to-peer-file-transfer)
7. [File Transfer Relay](#file-transfer-relay)
8. [Reconnection](#reconnection)

---

## Overview

```
                    +------------------+
                    |  Rendezvous      |
                    |  Server           |
                    |  (QUIC endpoint) |
                    +--+-----+-----+--+
                       |     |     |
              QUIC     |     |     |     QUIC
           (state sync)|     |     | (state sync)
                       |     |     |
       +-------+   +---+----+  +--+-----+
       | Peer A |   | Peer B |  | Peer C |
       +---+---+   +---+----+  +---+----+
           |            |          |
           +--- P2P file transfer -+
              (direct or relayed)
```

**Hub-and-spoke for state:** Every client maintains a QUIC connection to the
rendezvous server. All CRDT state sync flows through the server. Clients do
not sync state with each other directly.

**Peer-to-peer for files:** File transfers go directly between peers when
possible. When direct connection fails, file transfer traffic can be relayed
through the server.

---

## QUIC Transport

### Connection Types

| Connection | Initiator | Purpose |
|------------|-----------|---------|
| Client -> Server | Client | Auth, state sync, time sync, peer discovery, file relay |
| Client -> Client | Either | File transfer only |

### Channel Usage (Client <-> Server)

Each client-server QUIC connection uses three kinds of channels:

1. **Control stream** -- a single long-lived bidirectional stream, opened
   immediately after connection. Carries authentication, state sync operations,
   state summaries, and peer list updates. Messages are length-prefixed and
   serialized with postcard.

2. **Datagrams** -- QUIC unreliable datagrams. Used for best-effort eager-push
   of state ops (lower latency than the control stream). Also used for
   high-frequency playback position updates.

3. **On-demand streams** -- short-lived bidirectional streams opened as needed
   for gap fill (recovering missed state ops) and file transfer relay. Opened
   by the requesting side, closed when the transfer completes.

### Channel Usage (Client <-> Client)

Peer-to-peer connections are used exclusively for file transfer:

1. **On-demand streams** -- short-lived bidirectional streams for chunk
   transfer. Opened by the downloading peer.

2. **Control stream** -- for file availability announcements (bitfields).

No datagrams are used on peer-to-peer connections.

### TLS and Identity

- **Rendezvous server**: Generates a persistent self-signed certificate.
  Clients use TOFU (Trust On First Use) -- the server's certificate fingerprint
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
  |                                     |
  |--- QUIC connect (TOFU) ----------->|
  |                                     |
  |--- Open control stream ----------->|
  |                                     |
  |--- Auth { password } ------------->|
  |<-- AuthOk { observed_addr } -------|
  |                                     |
  |--- TimeSync request -------------->|
  |<-- TimeSync response --------------|
  |                                     |
  |<-- PeerList { peers } -------------|
  |                                     |
  |<-- StateSnapshot { epoch, data } --|
  |                                     |
  |    (bidirectional state sync and    |
  |     periodic time sync ongoing)     |
```

### Messages (Client <-> Server)

```rust
enum ServerControl {
    // Client -> Server
    Auth { password: String },
    TimeSyncRequest { client_send: u64 },

    // Server -> Client
    AuthOk { observed_addr: SocketAddr },
    AuthFailed,
    PeerList { peers: Vec<PeerInfo> },
    TimeSyncResponse {
        client_send: u64,
        server_recv: u64,
        server_send: u64,
    },

    // Bidirectional (state sync)
    StateSnapshot { epoch: u64, crdts: CrdtSnapshot },
    StateOp { op: CrdtOp },
    /// Full CvRDT state for merge-based sync on reconnection
    StateMerge { epoch: u64, crdts: CrdtSnapshot },
}

struct PeerInfo {
    username: String,
    addresses: Vec<SocketAddr>,  // observed + self-reported
    connected_since: u64,
}
```

### Peer List Updates

The server pushes an updated `PeerList` whenever a peer joins or leaves.
Clients use this for file transfer peer discovery. The peer list includes
addresses for direct connection attempts.

### Authentication

The password is sent as plaintext in the `Auth` message, protected by QUIC's
TLS 1.3 encryption. The server verifies it against the configured password.
On success, the server responds with `AuthOk` including the client's observed
address (useful for peer-to-peer file transfer connections). On failure, the
server sends `AuthFailed` and closes the connection.

---

## Time Synchronization

NTP-style protocol run over the server control stream.

### Exchange

```
Client                          Server
  |                                |
  |  t1 = local_clock()            |
  |--- TimeSyncRequest(t1) ------>|
  |                                |  t2 = server_clock()  [receive]
  |                                |  t3 = server_clock()  [send]
  |<-- TimeSyncResponse(t1,t2,t3) |
  |                                |
  |  t4 = local_clock()            |
  |                                |
  |  rtt = (t4 - t1) - (t3 - t2)  |
  |  offset = ((t2-t1) + (t3-t4)) / 2
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

Each variant wraps the native `Op` type from the corresponding `crdts` crate
type. All ops are serializable via serde/postcard.

```rust
/// A single CRDT operation, sent over the wire.
/// Each variant wraps the native crdts Op type for that field.
enum CrdtOp {
    Playlist(<Map<Ed2kHash, MVReg<Lww<PlaylistFileState>, ActorId>, ActorId> as CmRDT>::Op),
    NowPlaying(<MVReg<Lww<Option<Ed2kHash>>, ActorId> as CmRDT>::Op),
    SeekAuthority(<MVReg<Lww<ActorId>, ActorId> as CmRDT>::Op),
    SeriesPreference(<Map<(UserId, AniDbSeriesId), MVReg<Lww<SeriesWatchState>, ActorId>, ActorId> as CmRDT>::Op),
    ManualOverride(<Map<UserId, MVReg<Lww<Option<ManualState>>, ActorId>, ActorId> as CmRDT>::Op),
    FileAvailability(<Map<(UserId, Ed2kHash), MVReg<Lww<FileAvailability>, ActorId>, ActorId> as CmRDT>::Op),
    AniDbMetadata(<Map<Ed2kHash, MVReg<Lww<Option<AniDbMetadata>>, ActorId>, ActorId> as CmRDT>::Op),
    PlaybackPosition(<Map<UserId, MVReg<Lww<PlaybackPosition>, ActorId>, ActorId> as CmRDT>::Op),
    Chat(<GList<ChatMessage> as CmRDT>::Op),
    LookupRequest(<GSet<FileHashInfo> as CmRDT>::Op),
}
```

In practice, `Map::Op` is `map::Op::Up(key, dot, mvreg_op) | map::Op::Rm(key, vclock)`,
and `MVReg::Op` carries the value and a dot for causality tracking. The
concrete types are determined by the crdts crate -- we just wrap and tag them.

### Sync Flow

1. **On connect**: Client sends its epoch to the server.
2. **Epoch check**: Server compares epochs.
   - **Same epoch**: Server sends its full CvRDT state. Client merges.
   - **Stale epoch**: Server sends the compacted snapshot with new epoch.
     Client replaces its local state entirely.
3. **Ongoing**: CmRDT ops are sent on the control stream (reliable) and
   simultaneously pushed via datagram (best-effort, for lower latency).
   The crdts types handle deduplication internally via causality tracking.

No custom version vectors or gap-fill protocol is needed. Reconnection
uses CvRDT merge (idempotent, commutative, associative).

---

## Peer-to-Peer File Transfer

File transfer uses direct peer-to-peer connections when possible. This is
the only functionality that uses P2P connections -- all state sync goes
through the server.

### Peer Discovery for File Transfer

When a client needs a file:
1. Check the `PeerList` from the server for other connected clients
2. Check file availability CRDTs to see who has the file
3. Attempt direct connection to peers who have needed chunks

### Connection Establishment

Peers attempt direct QUIC connections using addresses from the PeerList.
Since peers may be behind firewalls, both sides attempt connection
simultaneously (hole punching):

1. Both peers begin sending QUIC Initial packets upon learning each other's
   addresses
2. Retry with exponential backoff: 100ms, 200ms, 400ms, 800ms, 1600ms
3. If no connection after 5 seconds: fall back to server relay
4. Continue periodic direct connection attempts (30s) while relaying

### Peer-to-Peer Messages

```rust
enum PeerControl {
    Hello { username: String },
    FileAvailability { file_id: FileId, bitfield: BitVec },
}
```

### Chunks

- Files are divided into **256 KiB chunks** (last chunk may be smaller)
- Chunks are identified by `(file_id, chunk_index)`
- A typical 1.4 GB video file has ~5600 chunks

### Availability Tracking

Each peer maintains a bitfield per file indicating which chunks it has:

```rust
FileAvailability {
    file_id: FileId,
    bitfield: BitVec,  // 1 = have chunk, 0 = don't
}
```

- Sent when a peer begins serving a file (complete bitfield)
- Updated when a downloading peer completes new chunks
- Update frequency: at most every 1s during active transfer

### Chunk Selection: Rarest First

When a downloader decides which chunk to request next:

1. Collect availability bitfields from all peers
2. For each missing chunk, count how many peers have it
3. Request the chunk available from the **fewest** peers
4. Break ties randomly

This maximizes the rate at which rare chunks propagate. With 1 seeder and 3
leechers, the seeder sends different chunks to each leecher; those leechers
can then serve each other.

### Upload Prioritization

When a peer has multiple pending chunk requests:

- Prioritize chunks that the requester is the only one missing
- Otherwise, prioritize rarest chunks
- Round-robin between requesting peers for fairness

### Transfer Stream

```
Downloader                         Uploader
  |                                    |
  |--- Open stream ------------------>|
  |--- ChunkRequest { file_id,       >|
  |        chunks: [idx, idx, ...] }   |
  |<-- ChunkData { idx, data } -------|
  |<-- ChunkData { idx, data } -------|
  |<-- ... ----------------------------|
  |<-- (stream closed) ---------------|
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

### Flow Control

- Maximum **4 concurrent transfer streams** per downloading peer
- Maximum **16 chunks** per request (pipeline depth)
- QUIC flow control handles backpressure naturally

### Integration with Playback

When a file is being downloaded for immediate playback:

- Chunk selection switches from rarest-first to **sequential** for the next
  ~20% of the file ahead of the current playback position
- Rarest-first continues for chunks outside the playback window
- This ensures smooth playback while still distributing rare chunks

### Temporary Storage

Downloaded chunks are written to a temporary directory. When 50% of the file's
duration has been watched, the completed file is moved to
`<download_root>/<series>/<season>/<original_filename>`.

---

## File Transfer Relay

When direct peer-to-peer connection fails for file transfer, the server
relays the traffic.

### Architecture

```
Peer A <-- QUIC --> Server <-- QUIC --> Peer B
                    (application-layer proxy)
```

The server acts as an application-layer proxy for file transfer:
- Chunk requests from A addressed to B are forwarded on B's connection
- Chunk responses from B are forwarded back to A
- The server does not cache or store file data

### Relay Envelope

```rust
enum RelayEnvelope {
    /// Forward enclosed message to the specified peer
    Forward { to: PeerId, message: Vec<u8> },
    /// A message forwarded from another peer
    Forwarded { from: PeerId, message: Vec<u8> },
}
```

File transfer messages are wrapped in relay envelopes when sent through the
server. The inner `message` bytes are decoded by the recipient as normal
peer-to-peer file transfer messages.

### Transparency

The file transfer layer does not need to know whether a connection is direct
or relayed. The network layer provides a `send(peer, message)` interface and
routes through direct connection or relay as appropriate.

---

## Reconnection

### Client Reconnects to Server

1. Re-establish QUIC connection
2. Re-authenticate
3. Re-sync time
4. Client sends its epoch to the server
5. Server compares epoch:
   - **Same epoch**: server sends `StateMerge` with full CvRDT state.
     Client calls `.merge()` on each CRDT field (idempotent).
   - **Stale epoch**: server sends `StateSnapshot` with compacted state
     and new epoch. Client replaces its local state entirely.
6. Resume normal CmRDT operation sync

### Client Reconnects to Peer (File Transfer)

1. Re-establish QUIC connection (direct or via relay)
2. Exchange `Hello` and file availability bitfields
3. Resume chunk transfers

### Graceful Disconnect

On clean shutdown, a client closes its control stream. QUIC's connection close
mechanism notifies the server. The server pushes an updated `PeerList` to
remaining clients.

### Ungraceful Disconnect

QUIC idle timeout (default: 30s) detects dead connections. On timeout:
- The server removes the client from the peer list and pushes an update
- The disconnected user's CRDT state remains until overwritten on reconnection
- File transfers to/from the disconnected peer are interrupted; other peers
  can pick up the slack

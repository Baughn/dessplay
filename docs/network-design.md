# Network Design

Last updated: 2026-06-10

This document covers connection establishment, wire protocols, relay, and file
transfer. For the replicated data types built on top of this layer, see
[sync-state.md](sync-state.md).

## Table of Contents

1. [Overview](#overview)
2. [QUIC Transport](#quic-transport)
3. [Rendezvous Protocol](#rendezvous-protocol)
4. [Time Synchronization](#time-synchronization)
5. [State Sync Wire Protocol](#state-sync-wire-protocol)
6. [File Transfer](#file-transfer)
7. [Relay Mechanics](#relay-mechanics)
8. [Reconnection](#reconnection)

---

## Overview

```
              NAS (home server, NixOS)
        +--------------------------------+
        |  Rendezvous Server   Seeder    |
        |  (QUIC endpoint) <-> (loopback)|
        +--+--------+--------+-----------+
           |        |        |
    QUIC   |        |        |   QUIC
 (state +  |        |        | (state +
  transfer)|        |        |  transfer)
           |        |        |
      +----+---+ +--+-----+ ++-------+
      | Peer A | | Peer B | | Peer C |
      +--------+ +--------+ +--------+
```

**Hub-and-spoke for everything:** Every client maintains a single QUIC
connection to the rendezvous server. All CRDT state sync flows through the
server, and **all file transfer is relayed through the server** -- there are
no client-to-client connections in v2.

This is a deliberate simplification. The server runs on a home connection
with an unmetered 250Mbit uplink and sits on the same machine as the primary
seeder (which connects over loopback), so relay bandwidth is free in
practice and the common transfer path (seeder -> peer) crosses the wire
exactly once. NAT hole punching and direct peer connections were cut: they
optimized the rare peer-to-peer transfer at the cost of a second
connection-management path full of untestable NAT-timing behavior. Direct
peer connections may return as a future optimization.

**Deployment:** the rendezvous server and the NAS seeder are two separate,
colocated processes (`dessplay-rendezvous` and `dessplay --seeder`). The
seeder is an ordinary client; anyone can run another one.

---

## QUIC Transport

### Connection Types

| Connection | Initiator | Purpose |
|------------|-----------|---------|
| Client -> Server | Client | Auth, state sync, time sync, peer discovery, file transfer (relayed) |

There are no client-to-client connections. Both IPv4 and IPv6 are supported
(the server is dual-stack); `PeerInfo.addresses` carries both families.

### Channel Usage (Client <-> Server)

Each client-server QUIC connection uses three kinds of channels:

1. **Control stream** -- a single long-lived bidirectional stream, opened
   immediately after connection. Carries authentication, state sync operations,
   state summaries, and peer list updates. Messages are length-prefixed and
   serialized with postcard.

2. **Datagrams** -- QUIC unreliable datagrams. Used for best-effort eager-push
   of state ops (lower latency than the control stream), for high-frequency
   playback position updates (datagram-only -- see below), and for time sync
   probes (so RTT measurements are not polluted by stream retransmission
   delays on lossy links).

   **Size rule:** an op whose encoded size exceeds the QUIC datagram limit
   (typically ~1200 bytes; query `max_datagram_size` at runtime) is sent on
   the control stream only. Long chat messages and large playlist ops simply
   skip the eager-push optimization.

3. **On-demand streams** -- short-lived bidirectional streams opened as needed
   for gap fill (recovering missed state ops) and relayed file transfer.
   Opened by the requesting side, closed when the transfer completes.

**Stream priority:** the control stream is prioritized above transfer
streams (quinn `set_priority`), so a bulk download never starves state sync.
QUIC connection-level flow control windows must be sized for bulk transfer
(the quinn defaults are tuned for request/response traffic, not for pushing
video files); this is a config item on both client and server endpoints.

### TLS and Identity

The rendezvous server generates a persistent self-signed certificate.
Clients use TOFU (Trust On First Use) -- the server's certificate fingerprint
is stored locally on first connection and verified on subsequent connections.
With no client-to-client connections, no other certificates exist; peer
identity is the username the server authenticated.

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
    Auth {
        username: String,
        password: String,
        role: Role,        // Interactive | Seeder
        epoch: u64,        // last known epoch; drives snapshot-vs-merge below
    },
    TimeSyncRequest { client_send: u64 },
    /// Player reached end of file. A report, not state -- the server owns
    /// the EOF -> next-file transition (see sync-state.md, Now Playing).
    EofReached { file: Ed2kHash },

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

    // Divergence alarm (see sync-state.md, Divergence Alarm)
    /// Server -> client, every 30s: hash of the server's resolved view
    /// (excluding playback positions).
    StateHash { epoch: u64, hash: [u8; 32] },
    /// Client -> server: view hashes mismatched twice in a row; please
    /// send a StateMerge.
    RequestMerge,
}

enum Role { Interactive, Seeder }

enum Presence { Present, Lost, Departed }

struct PeerInfo {
    username: String,
    role: Role,
    presence: Presence,
    addresses: Vec<SocketAddr>,  // observed + self-reported (v4 + v6); informational
    connected_since: u64,
}
```

Time sync requests/responses are sent as datagrams when the path supports
them, falling back to the control stream otherwise.

### Peer List Updates

The server pushes an updated `PeerList` whenever a peer joins, leaves, or
changes presence state. Clients use it for presence-aware playback gating
and to know which transfer sources are online. Addresses are informational
(see Authentication below).

### Presence

The server is the source of truth for presence (it is not CRDT state).
Clients send QUIC keep-alives every 10s; position updates double as liveness
while playing.

| Stage | Trigger | Server action |
|-------|---------|---------------|
| Present | Normal traffic | -- |
| Lost | 30s silence (QUIC idle timeout) | Push `PeerList` update; clients pause playback and post a system chat message |
| Departed | 60s silence | Push `PeerList` update; peer leaves the gating set; playback stays paused (no auto-resume); server takes seek authority if the departed peer held it |

Graceful disconnects (clean control-stream close) skip the Lost stage and go
straight to removal -- but still pause playback if it was running.

The full presence semantics, including UI treatment, are in
[design.md](design.md#presence).

### Authentication

The password is sent as plaintext in the `Auth` message, protected by QUIC's
TLS 1.3 encryption. The server verifies it against the configured password.
On success, the server responds with `AuthOk` including the client's observed
address (informational -- it costs nothing to report and would be needed if
direct peer connections ever return). On failure, the server sends
`AuthFailed` and closes the connection.

`Auth` also carries the client's username, role (seeders are excluded from
gating and listed separately), and last known epoch (used to choose between
`StateMerge` and `StateSnapshot` -- see State Sync Flow).

**Duplicate usernames:** a successful `Auth` with a username that already
has a live connection *supersedes* it -- the server closes the old
connection ("superseded by a new connection") and registers the new one.
This is the reconnect-before-timeout path: a client that crashes and
restarts must not be locked out by its own zombie connection. With five
trusted friends, impersonation is out of scope (see the threat model).

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
- Probes are sent as datagrams (stream fallback if datagrams are unsupported)
  so RTT samples are not inflated by stream retransmissions on lossy links
- Maintain a rolling average of the offset (discard outliers where RTT > 2x
  the median)
- All CRDT operation timestamps and playback positions use
  `local_clock() + offset` to produce shared-clock timestamps
- Precision target: <50ms (sufficient for slew-band drift correction)

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
    WatchedFlag(<Map<Ed2kHash, MVReg<Lww<bool>, ActorId>, ActorId> as CmRDT>::Op),
    NowPlaying(<MVReg<Lww<Option<Ed2kHash>>, ActorId> as CmRDT>::Op),
    SeekAuthority(<MVReg<Lww<ActorId>, ActorId> as CmRDT>::Op),
    SeriesPreference(<Map<(UserId, AniDbSeriesId), MVReg<Lww<SeriesWatchState>, ActorId>, ActorId> as CmRDT>::Op),
    ManualOverride(<Map<UserId, MVReg<Lww<Option<ManualState>>, ActorId>, ActorId> as CmRDT>::Op),
    FileAvailability(<Map<(UserId, Ed2kHash), MVReg<Lww<FileAvailability>, ActorId>, ActorId> as CmRDT>::Op),
    AniDbMetadata(<Map<Ed2kHash, MVReg<Lww<Option<AniDbMetadata>>, ActorId>, ActorId> as CmRDT>::Op),
    SeriesRelations(<Map<AniDbSeriesId, MVReg<Lww<SeriesRelations>, ActorId>, ActorId> as CmRDT>::Op),
    ListEntry(<Map<ListEntryId, MVReg<Lww<SeriesListEntry>, ActorId>, ActorId> as CmRDT>::Op),
    ListNextEp(<Map<ListEntryId, MVReg<Lww<NextEpState>, ActorId>, ActorId> as CmRDT>::Op),
    PlaybackPosition(<Map<UserId, MVReg<Lww<PlaybackPosition>, ActorId>, ActorId> as CmRDT>::Op),
    Chat(<GList<ChatMessage> as CmRDT>::Op),
    LookupRequest(<GSet<FileHashInfo> as CmRDT>::Op),
}
```

In practice, `Map::Op` is `map::Op::Up(key, dot, mvreg_op) | map::Op::Rm(key, vclock)`,
and `MVReg::Op` carries the value and a dot for causality tracking. The
concrete types are determined by the crdts crate -- we just wrap and tag them.

### Sync Flow

1. **On connect**: Client sends its epoch to the server (in `Auth`).
2. **Epoch check**: Server compares epochs.
   - **Same epoch**: Server sends its full CvRDT state. Client merges.
   - **Stale epoch**: Server sends the compacted snapshot with new epoch.
     Client replaces its local state entirely.
3. **Ongoing**: CmRDT ops are sent on the control stream (reliable) and
   simultaneously pushed via datagram (best-effort, for lower latency,
   subject to the datagram size rule).
   The crdts types handle deduplication internally via causality tracking.

**Exception -- playback position:** position ops are datagram-only at the
100ms cadence, with one reliable send per second as a catch-up baseline.
Reliable delivery of every stale position is exactly the head-of-line
blocking we are avoiding; dropped intermediate positions are superseded
within 100ms anyway.

**Compaction broadcast:** at the daily compaction (see
[sync-state.md](sync-state.md)), connected clients receive an unsolicited
`StateSnapshot { new_epoch, ... }` on the control stream and replace local
state, exactly as in the stale-epoch path.

No custom version vectors or gap-fill protocol is needed. Reconnection
uses CvRDT merge (idempotent, commutative, associative).

---

## File Transfer

All file transfer flows through the server as relayed peer messages: the
downloader exchanges messages with the *logical* peer (seeder, or whoever
has the file), but the bytes always travel downloader <-> server <-> uploader.
The transfer protocol below is peer-to-peer in its semantics and
relay-transported in its mechanics; see [Relay Mechanics](#relay-mechanics).

### Finding Sources

When a client needs a file:
1. Check the file availability CRDTs to see who has it (or has chunks of it)
2. Filter against the `PeerList` for peers that are currently Present
3. Exchange availability bitfields and request chunks via relay

### Peer Messages

Messages addressed to other peers (always wrapped in relay envelopes):

```rust
enum PeerMessage {
    FileAvailability { file_id: FileId, bitfield: BitVec },
    BlockHashRequest { file_id: FileId },
    BlockHashes { file_id: FileId, hashes: Vec<Md4Hash> },  // ed2k blocks
    ChunkRequest { file_id: FileId, chunks: Vec<u32> },
    ChunkData { file_id: FileId, index: u32, data: Vec<u8> },
}
```

No Hello message is needed: the server authenticated every peer, and relay
envelopes carry the sender's identity.

### Chunks

- Files are divided into **256 KiB chunks** (last chunk may be smaller)
- Chunks are identified by `(file_id, chunk_index)`
- A typical 1.4 GB video file has ~5600 chunks
- The chunk count is derived from `size_bytes` in the playlist entry
  (filled in by whoever added the file)

### Verification: ed2k Block Hashes

ed2k is internally a list of MD4 hashes over 9,728,000-byte blocks. Whenever
a client hashes a file it keeps the **per-block hashes**, not just the root,
and serves them to downloaders on request (a `BlockHashes` message on the
control stream). This gives 9.28 MB-granularity verification:

- Each completed block of a download is verified immediately; a bad block is
  re-fetched (from a different peer) without restarting the file
- On startup, a client with a partial download verifies the blocks it has on
  disk and resumes from a trustworthy bitfield -- no separate persistence of
  download progress is needed beyond the chunk data itself
- The block hashes themselves are validated against the file's ed2k root
  (which is the playlist key) before use

### Upload Limiting

Clients may set an upload rate cap (`upload_limit`). A peer seeding while
also watching should not starve its own playback; when unset, uploads are
unthrottled. Seeders typically leave this unset.

### Availability Tracking

Each peer maintains a bitfield per file indicating which chunks it has
(1 = have chunk), exchanged via `PeerMessage::FileAvailability`:

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
Downloader                  Server                    Uploader
  |                            |                          |
  |--- Open stream ----------->|                          |
  |--- Forward{to: U,          |--- Open stream --------->|
  |      ChunkRequest{...}} -->|--- Forwarded{from: D,    |
  |                            |      ChunkRequest{...}} >|
  |<-- Forwarded{from: U,      |<-- Forward{to: D,        |
  |      ChunkData{idx,data}} -|      ChunkData{...}} ----|
  |<-- ...  -------------------|<-- ...  -----------------|
  |<-- (streams closed) -------|                          |
```

`ChunkRequest.chunks` lists chunk indices in preferred order; `ChunkData`
carries up to 256 KiB. The extra server hop roughly doubles request latency,
which the 16-chunk pipeline depth absorbs.

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

### Storage

Downloaded chunks are written into the download cache
(`$XDG_CACHE_HOME/dessplay/files/`) and the completed file stays there,
subject to the retention policy. Promotion into a media root happens only
via the explicit archive action. See design.md,
[Download Cache and Retention](design.md#download-cache-and-retention).

---

## Relay Mechanics

All peer messages travel through the server as an application-layer proxy.

### Architecture

```
Peer A <-- QUIC --> Server <-- QUIC --> Peer B
                    (application-layer proxy)
```

- Messages from A addressed to B are forwarded on B's connection
- The server does not cache, store, or inspect file data
- The server drops envelopes addressed to non-Present peers

**Bandwidth:** the server shares the NAS's unmetered 250Mbit uplink and
5Gbit downlink, and the primary seeder talks to it over loopback -- so the
dominant transfer pattern (seeder -> peers) costs each transferred byte one
trip over the uplink, the same as a direct connection would. Peer-to-peer
transfers (e.g. Kim seeding a file the NAS doesn't have yet) cost one
downlink trip plus one uplink trip per recipient. Both are well within
budget for episode-sized files.

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
server. The inner `message` bytes are decoded by the recipient as a
`PeerMessage`.

### Transparency

The file transfer layer addresses logical peers through a
`send(peer, message)` interface and never deals in connections. If direct
peer connections are ever added back as an optimization, they slot in below
this interface without touching transfer logic.

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

### Transfer Resumption

Transfers have no connection of their own to re-establish. After a server
reconnect, the downloader re-exchanges availability bitfields with its
sources and re-requests outstanding chunks; blocks already on disk are
revalidated against the ed2k block hashes, so nothing is re-fetched
unnecessarily.

### Graceful Disconnect

On clean shutdown, a client closes its control stream. QUIC's connection close
mechanism notifies the server. The server pushes an updated `PeerList` to
remaining clients, and playback pauses if it was running (see
[Presence](#presence)).

### Ungraceful Disconnect

Handled by the presence stages: Lost at 30s (everyone pauses), Departed at
60s (removed from gating; playback stays paused pending a human decision).
Additionally:
- The disconnected user's CRDT state remains until overwritten on
  reconnection, but is ignored by playback gating while they are not Present
- File transfers to/from the disconnected peer are interrupted; other peers
  can pick up the slack

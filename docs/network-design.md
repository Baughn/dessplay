# Network Design

Last updated: 2026-07-09

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

**Dialing.** The client resolves *all* of the server's A/AAAA addresses and
tries them in family-interleaved order (the resolver's preferred family
first, then the other family, alternating), with a **10s per-address
handshake budget** before moving to the next address. One address family
being silently black-holed is a real failure mode — a Mac waking from sleep
had a stale-NDP IPv6 path that ate packets for ~90s while IPv4 worked
(2026-07-06) — and waiting out the full 30s idle timeout against a single
dead AAAA address read as a hang. Client endpoints are created lazily, one
per address family; the TOFU pin is keyed by server name and shared across
addresses. Each connection attempt (initial or reconnect) is surfaced to
the UI as a `Connecting { attempt }` event so the status bar can show link
state (design.md, UI principles: no silent long-running work).

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
        /// The client's PROTOCOL_VERSION (see Protocol Versioning below).
        /// Deliberately the last field: a pre-versioning client's Auth is
        /// a strict prefix of this shape, so it fails decode cleanly and
        /// can be told apart from garbage.
        protocol_version: u32,
    },
    TimeSyncRequest { client_send: u64 },
    /// Player reached end of file. A report, not state -- the server owns
    /// the EOF -> next-file transition (see sync-state.md, Now Playing).
    EofReached { file: Ed2kHash },
    /// Graceful quit (`/quit`, Ctrl-C): the server removes the user
    /// immediately (no Lost stage) and forces playback intent to Paused.
    /// The client waits for the server's close after sending, so the
    /// frame is flushed before teardown.
    Goodbye,
    /// Manual mark-watched from the episode browser (design.md #10): set
    /// a file's group watched flag directly, not scoped to now-playing.
    /// Setting `true` also runs the EOF path's List `next_ep`
    /// auto-advance. Appended after `ProtocolMismatch` on the wire (the
    /// bump policy forbids reordering existing variants) though it
    /// belongs here logically.
    MarkWatched { file: Ed2kHash, watched: bool },

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
    /// Epoch-tagged: both sides drop ops generated against another
    /// epoch. Without the tag, an op in flight across a compaction
    /// would land on the freshly rebuilt state and pollute its reset
    /// per-actor dot sequences (the sender's later ops would be
    /// silently deduped as already-seen). Ops in flight at the
    /// compaction edge are dropped *by design* -- the daily schedule
    /// keeps that window away from watch-party hours.
    StateOp { epoch: u64, op: CrdtOp },
    /// Full CvRDT state for merge-based sync. Server -> client on
    /// reconnection and divergence healing; client -> server as the
    /// upward half of the reconnect handshake (recovers ops that died
    /// in flight with the old connection; the server rebroadcasts).
    StateMerge { epoch: u64, crdts: CrdtSnapshot },

    // Divergence alarm (see sync-state.md, Divergence Alarm)
    /// Server -> client, every 30s: hash of the server's resolved view
    /// (excluding playback positions).
    StateHash { epoch: u64, hash: [u8; 32] },
    /// Client -> server: view hashes mismatched twice in a row; please
    /// send a StateMerge.
    RequestMerge,

    /// Server -> client: your Auth carried a different PROTOCOL_VERSION;
    /// admission refused, connection closed. The client exits with a
    /// "please update" message instead of retrying.
    ProtocolMismatch { server_version: u32 },
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
| Lost | 30s silence (QUIC idle timeout) | Push `PeerList` update; force playback intent to Paused (interactive peers only); system chat message |
| Departed | 60s silence | Push `PeerList` update; peer leaves the gating set; intent forced Paused again (no auto-resume); server takes seek authority if the departed peer held it |

Implementation note: with 10s keep-alives, "30s without traffic" coincides
exactly with the QUIC idle timeout killing the connection — so the server
marks Lost when the connection dies, and a 1s sweeper promotes Lost entries
to Departed 30s later (60s of silence total). Lost and Departed entries stay
in the `PeerList` (with their presence) until the user reconnects; a
reconnecting user's new connection supersedes the old entry.

Graceful disconnects (`Goodbye`) skip the Lost stage and go straight to
removal -- but still force the intent to Paused, and hand seek authority to
the server if the quitter held it.

The full presence semantics, including UI treatment, are in
[design.md](design.md#presence).

### Authentication

The password is sent as plaintext in the `Auth` message, protected by QUIC's
TLS 1.3 encryption. The server verifies it against the configured password.
On a bad password the server sends `AuthFailed`, then waits (up to 2s) for
the client to close before closing itself — closing immediately would
discard the unflushed frame, and the client would see only a generic
connection loss and retry forever. On success, the server responds with
`AuthOk` including the client's observed
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

### Protocol Versioning

`Auth` carries the client's `PROTOCOL_VERSION` (a constant in
`dessplay-core::net`). The server refuses a mismatch **before** checking
the password (a stale client should hear "update", not "bad password"):
it sends `ProtocolMismatch { server_version }`, waits briefly for the
client to act on it (the same flush-before-close dance as `AuthFailed`),
and closes. The client surfaces the message and exits without retrying.

A **pre-versioning** client (from before the field existed) cannot be
answered with `ProtocolMismatch` -- its binary predates the variant. It
is recognized instead by its `Auth` failing to decode: the version field
is deliberately the *last* field, so the old shape is a strict prefix of
the new one and dies with a clean truncation error. Such clients get
`AuthFailed`, whose discriminant has never moved -- a generic refusal,
but a decodable and terminal one.

**Bump policy:** increment `PROTOCOL_VERSION` on *any* change to wire
messages, `CrdtOp`, or the encoding of a replicated value type. Append
enum variants and struct fields, never reorder or remove; and never
reshape `Auth` itself -- its stability is what keeps a future mismatched
client distinguishable from garbage and refusable with a readable
message.

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

**The relay transfer below is the fallback, not the first choice.**
Fetching is torrent-first: a missing file is searched on nyaa.si and
downloaded via an embedded BitTorrent engine when a matching public
torrent exists, which is the common case for current-season releases —
see design.md, [BitTorrent Downloads]. The relay path engages when the
torrent route can't deliver (no match, dead swarm, verify mismatch), and
remains the only route for genuinely rare files that exist nowhere but a
group member's disk. Everything below describes that relay path.

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
    FileAvailability { file_id: FileId, bitfield: Bitfield },
    BlockHashRequest { file_id: FileId },
    BlockHashes { file_id: FileId, hashes: Vec<Ed2kBlockHash> },  // ed2k blocks
    ChunkRequest { file_id: FileId, chunks: Vec<u32> },
    ChunkData { file_id: FileId, index: u32, data: Vec<u8> },
    Cancel { file_id: FileId, chunks: Vec<u32> },  // retract requests
    CannotServe { file_id: FileId },  // definitive "I can never serve this"
}
```

No Hello message is needed: the server authenticated every peer, and relay
envelopes carry the sender's identity. `Bitfield` is a compact `Vec<u8>`
newtype (LSB-first bits + a length), not a `bitvec` dependency. `Cancel`
retracts outstanding requests — used by endgame and when dropping a
silent source (see [Chunk Selection](#chunk-selection-rarest-first)).

`CannotServe` answers a `BlockHashRequest` from a holder whose local copy
is **known** to hash to a different identity — a manual mapping to a
different encode, which design.md deliberately leaves playable locally
(filename-trusted) and therefore advertised Ready. Ready normally implies
servable; this is the one designed exception, so the holder says so
explicitly and the requester drops it as a source for that download and
never re-solicits it (unlike a snub, which retries after a cooldown). A
holder that merely hasn't finished hashing stays silent instead — that
state is transient and resolves into either `BlockHashes` or
`CannotServe` on a later solicitation.

### Chunks

- Files are divided into **256,000-byte chunks** (250 KiB; the last chunk
  may be smaller). **Chosen as `ED2K_BLOCK_SIZE / 38`** so chunks align
  exactly to ed2k block boundaries: the block size (9,728,000) is fixed
  by the AniDB-compatible root hash, but the chunk size is ours, and
  9,728,000 = 2¹²·5³·19 has 256,000 as a divisor (38 chunks/block). A
  chunk therefore never straddles a block, so block verification maps to
  a contiguous chunk group with no shared-chunk bookkeeping.
- Chunks are identified by `(file_id, chunk_index)`
- A typical 1.4 GB video file has ~5500 chunks
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

The download scheduler (`dessplay/src/download.rs`) is a synchronous,
deterministic policy core (events in, actions out — like the player
wiring), so it is unit-testable without async or real time. Chunk order:

1. A **sequential window** of the next ~20% of the file ahead of the
   playback position is requested first, in order, so playback can start.
2. Outside the window, **rarest-first**: count how many sources advertise
   each missing chunk and request the rarest; ties break by index
   (deterministic, not random — reproducible tests).

This maximizes the rate at which rare chunks propagate. With 1 seeder and 3
leechers, the seeder sends different chunks to each leecher; those leechers
can then serve each other.

### Scheduling: pipeline, snub, endgame

Informed by BitTorrent — and notably, there is **no per-chunk timeout**:

- **Pipeline depth** outstanding chunk requests *per source* (the
  `--pipeline-depth` flag, default 16), across up to **4 concurrent
  sources**. The depth is the queue: the N+1th chunk isn't requested
  until one completes, so no request queue piles up behind the in-flight
  set.
- **Snub, not timeout.** A source that sends *nothing* for 30s is dropped
  and its outstanding chunks are `Cancel`led and requeued to other
  sources. A chunk in transit is never re-requested — only a silent
  *source* is — which avoids both spurious duplicate fetches (too-short
  timeout) and stalled playback (too-long timeout). Real chunk loss is
  rare and surfaces as a snub.
- **Endgame.** When the remaining work fits in one pipeline, a chunk may
  be requested from *several* sources at once; whichever arrives first
  wins and the losers are `Cancel`led — so the tail isn't stuck behind
  one slow source.
- Block hashes are fetched and validated against the file's ed2k root
  before any chunk can verify; an invalid list is rejected and re-asked
  from another source.

### Upload Prioritization

Serving is a FIFO queue drained within the [upload limit](#upload-limiting);
`Cancel` removes queued chunks. (Rarest-aware upload prioritization across
competing requesters is future work — for the 5-friends-and-a-seeder scale,
the seeder's uplink is the bottleneck and FIFO suffices.)

### Transfer Stream

Each peer opens **one dedicated relay `BiStream`** to the server on
connect (a QUIC stream separate from the control stream). All of that
peer's transfer envelopes ride it:

```
Downloader                  Server                    Uploader
  |--- Forward{to: U,        |                          |
  |      ChunkRequest} ----->|--- Forwarded{from: D,    |
  |                          |      ChunkRequest} ----->| (U's relay stream)
  |<-- Forwarded{from: U,    |<-- Forward{to: D,        |
  |      ChunkData} ---------|      ChunkData} ---------|
```

On open, a client writes a `RelayEnvelope::Hello` first. QUIC reveals a
bidirectional stream to the peer only when bytes are first written, so a
peer that only ever *receives* on its relay stream (an idle source/seeder
waiting to serve) would otherwise never have its stream `accept_bi`'d by
the server, and every message addressed to it would be dropped. `Hello`
forces registration; the server reads and ignores it. (The simulated
transport establishes streams eagerly, so this real-QUIC-only failure was
caught in the field, not in tests — there is now a real-QUIC regression
test, `quic_localhost::a_relayed_message_reaches_a_receive_only_peer`.)

**Why a separate QUIC stream, not the control stream:** QUIC multiplexes
streams with independent per-stream flow control, so bulk transfer on the
relay stream never head-of-line-blocks state sync on the control stream
(unlike TCP). The server doesn't correlate per-transfer streams — one
relay stream per peer suffices, since send and recv on a `BiStream` are
independent directions, so inbound `ChunkData` never blocks outbound
`ChunkRequest`s. `ChunkData` carries up to 250 KiB. The extra server hop
roughly doubles request latency, which the pipeline depth absorbs.

### Flow Control

- **Pipeline depth** (`--pipeline-depth`, default 16) chunk requests per
  source, across up to **4 concurrent sources** (so up to 64 in flight)
- The transfer stream's QUIC flow-control window is sized ≥ the
  bandwidth-delay product, so the app-level pipeline depth is the limiter,
  not QUIC backpressure
- The relay stream is a distinct QUIC stream from control, so QUIC handles
  backpressure per-stream without starving control traffic

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

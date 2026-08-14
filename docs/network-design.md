# Network Design

Last updated: 2026-08-15

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

**Hub-and-spoke for everything:** Every client maintains two QUIC
connections to the rendezvous server — a **control** connection (state
sync, presence, datagrams) and a **transfer** connection (bulk file
data), split so each can carry its own DSCP tag (see
[Connection Types](#connection-types)). All CRDT state sync flows through
the server, and **all file transfer is relayed through the server** --
there are no client-to-client connections in v2.

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

| Connection | Initiator | Port | DSCP | Purpose |
|------------|-----------|------|------|---------|
| **Control** | Client | configured | AF41 (34) | Auth, state sync, time sync, peer discovery, presence, datagrams |
| **Transfer** | Client | control + 1 | AF21 (18) | Relayed file transfer: the peer-message relay stream and per-transfer data streams |

There are no client-to-client connections. Both IPv4 and IPv6 are supported
(the server is dual-stack); `PeerInfo.addresses` carries both families.

**Why two connections (protocol v8):** DSCP is a per-packet IP-header
field, but quinn exposes only per-transmit ECN — tagging is per-socket
via `IP_TOS`/`IPV6_TCLASS`, so differentiating traffic classes means
separate endpoints, and streams within one connection cannot be tagged
apart. (The deeper reason streams *couldn't* carry distinct DSCP even
with kernel support: a QUIC connection has one congestion controller and
one loss detector, and packets sampling two different router queues
would poison both.) The tag matters at the sender's own router/uplink
egress — the queue that actually hurts — so ISP bleaching en route is
irrelevant. Torrents (librqbit's own sockets) stay untagged at CS0;
"torrents lowest" is achieved by raising dessplay. Both sides tag: the
server binds each listener's socket with the matching codepoint, so the
downlink direction is classifiable too. Windows silently ignores the
setsockopt; tagging is best-effort everywhere.

**The quinn-udp fork (vendor/quinn-udp).** Socket-level tagging alone
does not survive stock quinn: quinn-udp attaches a per-packet
`IP_TOS`/`IPV6_TCLASS` control message to every datagram whose value is
derived solely from the transmit's ECN codepoint, and a per-packet cmsg
overrides the socket option — so the wire TOS byte came out as bare ECN
(DSCP 0) on Linux and macOS, both families. Upstream declined to add
DSCP support (quinn-rs/quinn#1749), so the workspace pins a vendored
fork via `[patch.crates-io]`: `UdpSocketState::new` captures the
socket's TOS byte (ECN bits masked off) with one `getsockopt`, and
`prepare_msg` ORs it into the cmsg value. Since `bind_socket` tags the
socket before handing it to `quinn::Endpoint::new`, the captured base is
always the intended codepoint. Patch sites are marked `DESSPLAY PATCH`
in `vendor/quinn-udp/src/unix.rs`; the wire-level regression test is
`dessplay-core/tests/dscp_wire.rs` (send through quinn-udp on a tagged
socket, read the TOS byte back with raw `recvmsg`). When bumping quinn,
re-vendor and re-apply — the test fails if the patch is lost.

**Binding.** `Auth`/`AuthOk` happen on the control connection; `AuthOk`
carries a per-session `transfer_token`. The client then dials the
transfer connection (same host, port + 1 by convention — the server
always binds both, one cert covers them) and presents
`TransferAuth { username, token }` as its first control frame. A
reconnect regenerates the token, so a stale transfer connection cannot
bind to a superseded session.

**Presence is keyed to the control connection alone.** The transfer
link redials itself on a backoff; its death degrades transfers (which
retry at their own layer) and never marks a user Lost. The server
closes a session's transfer connection when its control connection
ends. A transfer link that will not come up is not fully silent,
though: past three consecutive failed dial/setup attempts the network
actor tells the session (`TransferLinkDown`), and the health line's
advisor shows "file transfer link down — is the transfer port
(control port + 1) open?" — the port is opened separately, and a
blocked one leaves auth, chat, and presence looking healthy while
every download sits at 0%. The first few failures stay silent
(transient blips self-heal on the backoff); recovery clears the
advisory.

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

Resolution is not frozen at startup: when a connect pass exhausts every
known address, the connector **re-resolves** the server's hostname, tries
any genuinely new addresses in the same pass, and stores the fresh set
for later attempts — so a mid-session record change (dynamic IPv6 prefix
rotation, a DNS move) recovers on the next reconnect cycle instead of
requiring a process restart (2026-07-19 audit).

### Channel Usage (Client <-> Server)

The **control connection** uses two kinds of channels:

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

The **transfer connection** carries two kinds of streams, each
classified by its first frame (a `RelayEnvelope` header):

3. **Relay stream** (`Hello`) -- one long-lived bidirectional stream
   opened by each client after `TransferAuth`. Carries the *small*
   relayed peer messages (availability bitfields, block hashes,
   `CannotServe`) as `Forward`/`Forwarded` envelopes. Bulk data never
   rides it.

4. **Data streams** (`OpenTransfer { to, file }`) -- one per active
   (source, file) transfer, opened by the **downloader**; the server
   opens a matching stream toward the uploader (headed
   `TransferFrom { from, file }`) and pumps bytes verbatim between the
   two. After the header the stream carries bare `PeerMessage` frames:
   `ChunkRequest`/`Cancel` toward the uploader, `ChunkData` back. See
   [Flow Control](#flow-control).

   **Stream opens follow an answered-request contract**: the network
   actor answers every open request from the transfer layer — with the
   live stream, or with an explicit failure event — and buffers
   requests that arrive while the link is still coming up
   (reconnect-until-AuthOk). It never drops one silently: the transfer
   layer asks exactly once per pending transfer and retries on its own
   tick only when answered with failure, so a lost answer would wedge
   that transfer until restart. Stream lifecycle events are
   **generation-stamped**: a fresh stream for the same (peer, file)
   replaces its predecessor, and a predecessor's late close/end event
   names the generation it belongs to, so it can never tear down the
   replacement.

Reconnection uses full CvRDT merge rather than a gap-fill stream; there
are no other application streams in the current protocol.

**Congestion control is BBR** on every connection, both sides — the
default loss-based Cubic filled deep, AQM-less home-router buffers
(Starlink-class) until the standing queue *was* the path RTT (~25s
probe RTT while seeding, 2026-07-28). Flow-control windows stay sized
for bulk transfer (16 MiB/stream, 64 MiB/connection): they are
receive-side memory bounds, not queue-builders — the queue was Cubic's
doing. The control stream is still prioritized above other streams
(quinn `set_priority`) within its own connection.

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
    // Server -> Client
    AuthOk {
        observed_addr: SocketAddr,
        /// Binds the transfer connection to this session (v8): the
        /// client presents it in TransferAuth on the second, bulk-DSCP
        /// connection. Regenerated per Auth, so a superseded session's
        /// token is refused.
        transfer_token: u64,
    },
    AuthFailed,
    PeerList {
        peers: Vec<PeerInfo>,
        known_offline: Vec<KnownUser>,
    },
    TimeSyncResponse {
        client_send: u64,
        server_recv: u64,
        server_send: u64,
    },

    // Bidirectional (state sync)
    StateSnapshot(StateSnapshot),
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
    StateMerge(StateSnapshot),

    // Divergence alarm (see sync-state.md, Divergence Alarm)
    /// Server -> client, every 30s: hash of the server's resolved view
    /// (excluding playback positions).
    StateHash { epoch: u64, hash: [u8; 32] },
    /// Client -> server: view hashes mismatched twice in a row; please
    /// send a StateMerge.
    RequestMerge,

    // AniDB title search (client request + server response)
    AniDbSearch { query: String },
    AniDbSearchResults { query: String, results: Vec<AniDbSearchHit> },

    /// Server -> client: your Auth carried a different PROTOCOL_VERSION;
    /// admission refused, connection closed. The client exits with a
    /// "please update" message instead of retrying.
    ProtocolMismatch { server_version: u32 },

    /// Manual mark-watched from the episode browser (design.md #10): set
    /// a file's group watched flag directly, not scoped to now-playing.
    /// Setting `true` also runs the EOF path's List `next_ep`
    /// auto-advance. Appended here after `ProtocolMismatch` because the
    /// bump policy forbids reordering existing wire variants.
    MarkWatched { file: Ed2kHash, watched: bool },

    /// Client -> server: first (and only expected) control frame on the
    /// **transfer connection**, binding it to the session whose AuthOk
    /// issued `token` (v8). Presence stays keyed to the control
    /// connection; a dead transfer link degrades transfers, never
    /// liveness.
    TransferAuth { username: UserId, token: u64 },
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
| Departed | 60s silence | Push `PeerList` update; peer leaves ordinary gating; do **not** write playback intent again; server takes seek authority if the departed peer held it |

Implementation note: with 10s keep-alives, "30s without traffic" coincides
exactly with the QUIC idle timeout killing the connection — so the server
marks Lost when the connection dies, and a 1s sweeper promotes Lost entries
to Departed 30s later (60s of silence total). Lost and Departed entries stay
in the `PeerList` (with their presence) until the user reconnects; a
reconnecting user's new connection supersedes the old entry.

The Lost -> Departed timeout promotion deliberately does not force another
pause. Lost already wrote Paused; if the present users deliberately resume
during the next 30 seconds (valid for an absent Maybe user), the sweep must
not overwrite that decision. Playback does not auto-resume merely because a
peer departs: the Lost write remains latched until a user presses play.

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

Each variant wraps the native operation type for the corresponding field.
`LwwMapOp<K, V>` is `crdts::map::Op<K, LwwCell<V>, ActorId>` and
`LwwRegOp<V>` is the timestamped `Lww<V>` value itself. All ops are
serializable via serde/postcard.

```rust
/// A single CRDT operation, sent over the wire.
/// Each variant wraps the native crdts Op type for that field.
enum CrdtOp {
    Playlist(LwwMapOp<Ed2kHash, Option<PlaylistFileState>>),
    Watched(LwwMapOp<Ed2kHash, bool>),
    NowPlaying(LwwRegOp<Option<Ed2kHash>>),
    SeekAuthority(LwwRegOp<SeekAuthority>),
    PlaybackIntent(LwwRegOp<PlaybackIntent>),
    SeriesPreference(LwwMapOp<(UserId, ListEntryId), SeriesPreference>),
    ManualOverride(LwwMapOp<UserId, Option<ManualState>>),
    FileAvailability(LwwMapOp<(UserId, Ed2kHash), FileAvailability>),
    AniDbMetadata(LwwMapOp<Ed2kHash, Option<AniDbMetadata>>),
    SeriesRelations(LwwMapOp<AniDbSeriesId, SeriesRelations>),
    FileCatalog(LwwMapOp<Ed2kHash, FileCatalogEntry>),
    ListEntry(LwwMapOp<ListEntryId, SeriesListEntry>),
    ListNextEp(LwwMapOp<ListEntryId, NextEpState>),
    LookupRequest(FileHashInfo),
    Chat(glist::Op<ChatMessage>),
    PlaybackPosition(LwwMapOp<UserId, PlaybackPosition>),
    AcknowledgeAbsent((Ed2kHash, UserId)),
}
```

In practice, `Map::Op` can represent `Up` or `Rm`, but DessPlay emits only
`Up`: removals are LWW `None` tombstones. Standalone LWW registers carry no
causal metadata; applying or merging them keeps `max((timestamp, value))`.

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

**The relay transfer below is the only automatic fetch path.** A
missing playlist file is always fetched from peers through the server;
the embedded BitTorrent engine exists solely for the Playlist pane's
explicit Nyaa browse import — see design.md,
[BitTorrent Downloads] — and never fetches playlist entries.
Everything below describes the relay path.

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
retracts outstanding requests — used when an urgent chunk's duplicate
race resolves (endgame included) and when dropping a silent source (see
[Chunk Selection](#chunk-selection-rarest-first)).

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

### Scheduling: request window, snub, urgent set

Informed by BitTorrent — with per-chunk re-requests reserved for the
chunks whose absence gates a deadline:

- **Request window** of 64 outstanding chunk requests *per source*,
  across up to **4 concurrent sources**. Since protocol v9 the window
  is a **latency-hider, not a throttle**: chunk data flows on a
  dedicated per-transfer stream paced by BBR and end-to-end stream
  backpressure, so the window only needs to keep the uploader's queue
  non-empty across the relay round trip (64 × 250 KiB = 16 MB covers
  the plausible bandwidth-delay product with margin). Excess requests
  queue as *indices* at the uploader — not as bytes in network buffers.
  The old `--pipeline-depth` flag is gone with the old role.
- **Snub for silence.** A source that sends *nothing* for 30s is dropped
  and its outstanding chunks are `Cancel`led and requeued to other
  sources. A bulk chunk in transit is never re-requested — only a
  silent *source* is. A **closed or reset data stream is the same
  signal, acted on immediately**: the source's in-flight chunks requeue
  and re-plan at once (`Downloads::on_source_stream_lost`) rather than
  sitting dead until the 30s timeout — the source itself is kept (the
  stream, not the peer, died; the next request toward it opens a fresh
  one, see [Transfer Resumption](#transfer-resumption)).
- **Urgency for stalls.** The snub alone cannot protect the playable
  window: a slow-but-delivering source never trips it, and a source
  whose stream keeps dying re-arms its snub clock on every requeue
  cycle — either way it can sit on the window's chunks for the whole
  transfer (the 2026-08-14 regression: playback started at ~100%
  instead of 20%). So a needed chunk turns **urgent** when its absence
  gates a deadline: it lies in the playable window's *blocks*
  (playability is block-granular — the window rounded out to ed2k
  block boundaries is what actually gates) and was first requested
  more than `urgent_age` (5s) ago without landing, **or** the whole
  remaining set fits in one pipeline — endgame, the special case where
  the completion deadline gates everything left. An urgent chunk may
  be requested from *several* sources at once, including sources past
  the concurrency cap; whichever arrives first wins and the losers are
  `Cancel`led — so neither playback nor the tail is stuck behind one
  slow source. The age runs from the chunk's *first* request
  precisely so requeue-and-reassign cycles cannot reset it.
- Block hashes are fetched and validated against the file's ed2k root
  before any chunk can verify; an invalid list is rejected and re-asked
  from another source.

### Upload Prioritization

Each incoming data stream gets its own **serve task**: it reads
`ChunkRequest`/`Cancel` frames into a per-transfer queue (request order
is serve order — the downloader front-loads its sequential window) and
streams `ChunkData` back as fast as the stream accepts. The tasks share
the [upload limit](#upload-limiting)'s token bucket, so the cap covers
their sum, but each blocks only on its own stream — one slow or stalled
downloader backpressures itself and nobody else (the pre-v9 shared serve
queue starved every recipient behind one saturated peer; regression:
`transfer::a_stalled_downloader_does_not_starve_another`).

### Relay and Data Streams

Each peer opens **one relay stream** to the server on connect for the
small peer messages, and the downloader opens **one data stream per
active (source, file) transfer**; the server pumps each data stream
byte-for-byte to its target (see [Relay Mechanics](#relay-mechanics)):

```
Downloader                  Server                     Uploader
  |--- relay stream -------->|--- relay stream ------->|   (Hello; availability,
  |    Forward{BlockHashReq} |    Forwarded{...}       |    block hashes, CannotServe)
  |                          |                         |
  |=== data stream =========>|=== data stream ========>|   (OpenTransfer / TransferFrom
  |    ChunkRequest, Cancel  |        byte pump        |    header, then bare PeerMessage
  |<== ChunkData ============|<========================|    frames; one per transfer)
```

On open, a client writes a `RelayEnvelope::Hello` first on the relay
stream. QUIC reveals a bidirectional stream to the peer only when bytes
are first written, so a peer that only ever *receives* on its relay
stream (an idle source/seeder waiting to serve) would otherwise never
have its stream `accept_bi`'d by the server, and every message addressed
to it would be dropped. `Hello` forces registration; the server reads and
ignores it. Data streams carry their own classifying header
(`OpenTransfer` from the downloader; `TransferFrom` on the pumped side),
so the first frame of *any* stream on the transfer connection names what
the stream is. (The simulated transport establishes streams eagerly, so
the lazy-reveal failure was caught in the field, not in tests — there is
a real-QUIC regression test,
`quic_localhost::a_relayed_message_reaches_a_receive_only_peer`.)

**Why per-transfer streams (protocol v9):** QUIC multiplexes streams
with independent per-stream flow control. Giving each transfer its own
stream makes that flow control the transfer's pacing — end to end,
through the server's bounded byte pump — and isolates transfers from
each other: a slow downloader stalls its own stream, never a sibling's
(the old single shared relay stream head-of-line-blocked every recipient
behind the slowest). `ChunkData` carries up to 250 KiB. The extra server
hop roughly doubles request latency, which the request window absorbs.

### Flow Control

Transfers pace themselves the way TCP does — no application-level
throttle:

- **BBR** (not loss-based Cubic) on every connection bounds in-flight
  data to ~the bandwidth-delay product instead of filling bottleneck
  buffers until loss; the 2026-07-28 bufferbloat incident (~25s probe
  RTT while seeding over a deep-buffered uplink) is the regression this
  guards against.
- **End-to-end stream backpressure**: the uploader's serve task blocks
  on its stream write; the server's pump copies with bounded buffers;
  the downloader's read pace closes the loop. A stalled receiver stops
  its sender within a buffer's worth of data.
- The **request window** (64 chunks/source, up to 4 sources) only hides
  relay latency — excess queues as indices at the uploader (see
  [Scheduling](#scheduling-request-window-snub-urgent-set)).
- QUIC flow-control windows (16 MiB/stream, 64 MiB/connection) are
  receive-side memory bounds sized to never be the binding constraint
  under BBR. The concurrent-bidi-stream cap is likewise raised to 1024
  (quinn's default 100 makes `open_bi` silently *wait* at the limit,
  and one stream per transfer means a seeder redeploy legitimately
  runs hundreds); the stream open also runs off the network actor's
  link loop, so a wait at the cap parks only its own task.
- The relay and data streams live on the transfer connection, so bulk
  transfer never competes with state sync on the control connection at
  the QUIC layer — and the two connections carry different DSCP tags
  (see [Connection Types](#connection-types)).

### Integration with Playback

When a file is being downloaded for immediate playback:

- Chunk selection switches from rarest-first to **sequential** for the next
  ~20% of the file ahead of the current playback position, rounded out to
  ed2k block boundaries (playability is block-granular, so the enclosing
  blocks are what actually gate playback)
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

All peer traffic travels through the server as an application-layer proxy.

### Architecture

```
Peer A <-- QUIC --> Server <-- QUIC --> Peer B
                    (application-layer proxy)
```

- Relay-stream messages from A addressed to B are forwarded on B's
  relay stream (`Forward` -> `Forwarded`)
- A data stream A opens with `OpenTransfer { to: B, file }` gets a
  matching stream opened on B's transfer connection (headed
  `TransferFrom { from: A, file }`); the server then **pumps bytes
  verbatim, both directions,** until either side closes. The pump's
  bounded copy buffers are what let QUIC stream backpressure propagate
  downloader <-> uploader end to end.
- The server does not cache, store, or inspect file data (it never
  decodes past a stream's header frame)
- The server drops envelopes — and data streams — addressed to peers
  with no bound transfer connection

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
    /// First frame of a relay stream (forces lazy-QUIC registration)
    Hello,
    /// Relay stream: forward enclosed message to the specified peer
    Forward { to: PeerId, message: Vec<u8> },
    /// Relay stream: a message forwarded from another peer
    Forwarded { from: PeerId, message: Vec<u8> },
    /// Data-stream header (downloader -> server): pump this stream to `to`
    OpenTransfer { to: PeerId, file: Ed2kHash },
    /// Data-stream header (server -> uploader): the pump's other end
    TransferFrom { from: PeerId, file: Ed2kHash },
}
```

The first frame of every stream on the transfer connection is a
`RelayEnvelope`, which classifies the stream. On the relay stream,
small peer messages are wrapped in `Forward`/`Forwarded` envelopes (the
inner bytes decode as a `PeerMessage`); on a data stream, everything
after the header is a bare length-prefixed `PeerMessage` frame — the
stream itself is the address.

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

Transfer *state* has no connection of its own to re-establish: data
streams die with the transfer connection (which redials itself on a
backoff, with the fresh session's token after a control reconnect), and
the next chunk request toward a source simply opens a fresh one. After a
reconnect the downloader re-exchanges availability bitfields with its
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

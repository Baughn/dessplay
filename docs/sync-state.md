# Sync State Design

Last updated: 2026-02-20

DessPlay uses **operation-based CRDTs** (CmRDTs) to synchronize state across
peers. Each piece of shared state is a replicated data type with its own
conflict resolution strategy. Operations are broadcast to all peers and
replayed in timestamp order to produce a local snapshot.

## Table of Contents

1. [Core Concepts](#core-concepts)
2. [Replicated Data Types](#replicated-data-types)
3. [Transport](#transport)
4. [Compaction](#compaction)
5. [Failure Modes](#failure-modes)

---

## Core Concepts

### Shared Clock

All operation timestamps use a shared clock established via the NTP-style
protocol with the rendezvous server (see design.md, Time Synchronization).
This provides a consistent total ordering of operations across all peers.

### Operation Log

Each replicated data type maintains an **operation log** (op log): a sequence
of timestamped operations. Every peer:

1. Receives operations from the network (or generates them locally)
2. Inserts them into the op log in timestamp order
3. Replays the log to produce a **snapshot** — the materialized state that
   application code reads

The op log is the source of truth. The snapshot is a cache derived from it.

### Epochs

An **epoch** is a generation counter incremented each time the rendezvous
server compacts the op log into a snapshot (see [Compaction](#compaction)).
When a client connects, it receives the current epoch and snapshot, then
applies any subsequent ops on top.

If a client reconnects and sees a newer epoch, it discards its local op log
and starts fresh from the server's snapshot.

---

## Replicated Data Types

### LWW Register

A **Last-Writer-Wins Register** holds a single value. Each write is an
operation `(timestamp, value)`. The snapshot is simply the value with the
highest timestamp.

Used for:

| Register | Key | Value | Owner |
|----------|-----|-------|-------|
| User State | `user_id` | Ready / Paused / Not Watching | Each user writes their own |
| File State | `(user_id, file_id)` | Ready / Missing / Downloading | Each user writes their own |
| AniDB metadata | `ed2k_hash` | `None \| JSON` | Server wins (server overwrites client entries) |

**Conflict resolution:** Highest timestamp wins. For AniDB metadata, the
server's writes always have a higher logical timestamp (server is authoritative).

### Playlist (Op Log CRDT)

The playlist is an **ordered set** maintained by a log of playlist operations.
Each operation targets items by **file ID** (not index), making concurrent
edits commutative in practice.

Operations:

| Op | Fields | Semantics |
|----|--------|-----------|
| `Add` | `file_id, after: Option<file_id>, timestamp` | Insert file after the given item (or at end if `None`) |
| `Remove` | `file_id, timestamp` | Remove file from playlist |
| `Move` | `file_id, after: Option<file_id>, timestamp` | Move file to after the given item |

Replay rules:
- Operations are applied in timestamp order
- `Add` of an already-present file ID is ignored
- `Remove` / `Move` of an absent file ID is ignored
- Concurrent `Add`s with the same `after` target: sort by timestamp (earlier
  goes first)
- Concurrent `Move`s of the same item: last timestamp wins

With ~4 users and manual playlist management, true simultaneous conflicts are
rare. When they occur, the playlist converges to *some* deterministic order
and someone fixes it manually if needed.

### Chat (Append-Only Log)

Chat is a **per-user append-only log**. Each user's messages are sequenced
independently with a monotonic sequence number.

Entry: `(user_id, seq, timestamp, message_text)`

There are no conflicts — each user exclusively appends to their own log.
The merged chat view interleaves all users' logs by timestamp.

State vectors (mapping each `user_id` to the highest `seq` seen) are
exchanged periodically to detect gaps. Missing entries are recovered via
reliable-stream gap fill.

### Playback Position (Ephemeral LWW)

Playback position is **not** a persistent CRDT. It is a fire-and-forget
**ephemeral LWW value** sent via unreliable datagrams.

Each datagram contains: `(user_id, timestamp, position_seconds)`

- Sent every 100ms during playback, every 1s when paused
- No op log, no compaction, no reliability
- Receivers simply keep the latest value per user
- Stale/lost updates are harmless — the next one arrives momentarily
- Not persisted to SQLite; only lives in memory

This is the one piece of state that explicitly does *not* go through the
CRDT sync engine.

### Seek Events

Seeks are also sent via datagrams but have different semantics from position
updates:

Entry: `(user_id, timestamp, target_position)`

- Debounced at 1500ms on the sender side (only broadcast after scrubbing stops)
- On receipt: if `|local_position - target_position| > 3s`, seek the local player
- No log — a seek is an imperative command, not replicated state

---

## Transport

### Two Channels

State sync uses two transport mechanisms:

1. **Datagrams** (unreliable, unordered): Used for eager-push of new
   operations, playback position updates, and seek events. Low latency,
   acceptable to lose.

2. **Reliable streams**: Used for:
   - Initial state transfer (snapshot + op log) on connect
   - Gap fill when a peer detects missing operations
   - Bulk recovery after reconnection

### Operation Broadcast

When a client generates a new operation:

1. Apply it locally (immediate feedback)
2. Send it to all connected peers via datagram
3. The operation is also included in the next periodic state summary

### Periodic State Summary

Every peer periodically sends a compact summary of its state:
- The epoch it's operating on
- Per-CRDT version vectors or latest timestamps
- Used by receivers to detect if they've missed any operations

Interval: 1s (sufficient given the small peer count).

If a peer detects it has missed operations (e.g., a datagram was lost),
it opens a reliable stream to the sender and requests the missing ops.

---

## Compaction

### When It Happens

The rendezvous server compacts the op log when **no clients have been
connected for more than 5 minutes**. This is the typical "end of session"
scenario — everyone closes their laptops after watching anime.

### How It Works

1. Server replays all operations using the same CRDT logic as clients
2. Produces a snapshot for each replicated data type
3. Discards the op log
4. Increments the epoch counter
5. Stores the snapshot as the new baseline

### Client Reconnection After Compaction

1. Client connects, sends its last known epoch
2. If epoch matches: server sends only new ops since the client's last seen
   timestamp
3. If epoch is stale: server sends the full snapshot + any ops since
   compaction. Client replaces its local state entirely.

---

## Failure Modes

### Lost Datagrams

Normal and expected. The periodic state summary (1s interval) ensures gaps
are detected within a second. Reliable-stream gap fill recovers missing ops.
For playback position, lost datagrams are simply ignored.

### Client Crash / Unclean Disconnect

The client persists its op log and snapshot to SQLite. On restart, it loads
local state, reconnects, and reconciles with peers via the standard
epoch-check and gap-fill mechanism.

### Network Partition During Compaction

If a client is partitioned while the server compacts:

- The client may hold ops that predate the new snapshot.
- These ops are already incorporated into the snapshot (the server saw them
  before compacting) — so they can be safely discarded.
- If the client generated ops *during* the partition that the server never
  saw: these ops are lost. For this use case (friends watching anime ~1hr/day),
  this requires being partitioned for the entire session *and* for 5 minutes
  after everyone else disconnects. Acceptable risk.

### Server Unavailable

Peers can still sync directly with each other via the full mesh. However:
- No compaction occurs
- No AniDB lookups happen
- New peers cannot discover the group (no rendezvous)
- The op log grows unbounded until the server returns

For short outages this is fine. For extended outages, clients could implement
a local compaction as a fallback (not planned for v1).

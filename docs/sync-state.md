# Sync State Reference

*Last updated: 2026-02-17*

This document describes every piece of state replicated by the sync engine,
its CRDT strategy, merge semantics, and wire format.

---

## Register Summary

| Register | Type | Strategy | Writer | Merge Rule |
|----------|------|----------|--------|------------|
| File register | `FileRegister` | Global LWW | Any peer | `(timestamp, origin) >` — higher PeerId wins ties |
| Position register | `PositionRegister` | Global LWW, conditional | Any peer | Reject if `for_file != file_register.file_id`, then `(timestamp, origin) >` |
| User state | `UserState` per peer | Per-peer LWW | Only owning peer | `timestamp >=` |
| File state | `FileState` per peer | Per-peer LWW | Only owning peer | `timestamp >=` |
| Peer generation | `SharedTimestamp` per peer | Per-peer LWW | Only owning peer | `generation >=`; if strictly newer, clears old user/file state |
| Chat messages | `ChatMessage` | Per-user append log | Only owning user | Seq-based; dedup by `(sender, seq)` |
| Playlist actions | `PlaylistAction` | Per-user append log | Any user (own log) | Seq-based; replay sorted by `(timestamp, action_sort_key)` |

### Derived state (not synced)

| State | Derived from | Description |
|-------|-------------|-------------|
| `is_playing` | All peers' UserState + FileState | True iff every peer is Ready/NotWatching and FileState permits playback |
| `ReadyState` | Per-peer UserState + FileState | UI display enum (Ready, Paused, NotWatching, DownloadingReady, DownloadingNotReady) |
| Peer list | Connection events | Ephemeral `Vec<PeerId>`, updated on connect/disconnect |

---

## Register Details

### File Register

```rust
struct FileRegister {
    file_id: Option<ItemId>,
    timestamp: SharedTimestamp,
    origin: PeerId,
}
```

Controls which file is currently loaded. Any peer can change the file (e.g.,
advancing the playlist). When a new file register is accepted, the position
register is automatically reset to `0.0` for the new file (same timestamp and
origin as the file change).

**Merge**: accept if `(incoming.timestamp, incoming.origin) > (stored.timestamp, stored.origin)`.
This uses `PeerId`'s lexicographic ordering as a deterministic tiebreaker when
timestamps are equal.

### Position Register

```rust
struct PositionRegister {
    position: f64,
    for_file: Option<ItemId>,
    timestamp: SharedTimestamp,
    origin: PeerId,
}
```

Tracks playback position within a file. The `for_file` field tags which file
this position belongs to, preventing stale seeks from overriding file switches.

**Merge**:
1. If `incoming.for_file != current file_register.file_id` → reject (wrong file)
2. Accept if `(incoming.timestamp, incoming.origin) > (stored.timestamp, stored.origin)`

**Design rationale**: Position and file are separate registers because a seek
should never overwrite a file change. The `for_file` tag acts as a natural
guard — once a file switch is accepted, all pending position updates for the
old file are automatically invalidated.

### User State (per peer)

```rust
enum UserState { Ready, Paused, NotWatching }
```

Each peer controls only their own user state. Stored as
`HashMap<PeerId, (UserState, SharedTimestamp)>`.

**Merge**: `timestamp >=` — accepts equal timestamps. This is safe because only
the owning peer writes, so equal-timestamp conflicts are always the same peer
re-sending (idempotent). The generation counter protects against stale state
from old sessions.

### File State (per peer)

```rust
enum FileState { Ready, Missing, Downloading { progress: f32, speed_sufficient: bool } }
```

Describes each peer's ability to play the current file. Same merge semantics
as user state.

### Peer Generation

Each peer records `clock.now()` at join time as their generation. This is
included in every `StateSnapshot`. When a peer reconnects with a newer
generation, the old session's user/file state is cleared.

**Merge**: `generation >=` — if strictly newer, clear the peer's user_states
and file_states entries. Peer generations are NOT removed on disconnect
(needed to reject stale state if the peer reconnects).

### Chat Messages

```rust
struct ChatMessage {
    sender: PeerId,
    text: String,
    timestamp: SharedTimestamp,
    seq: SequenceNumber,
}
```

Per-user append log. Each user has a monotonically increasing sequence.
Delivered via eager datagram push with gap fill over reliable streams.
Deduplicated in SharedState by `(sender, seq)`.

**Display order**: sorted by `(timestamp, sender)` for deterministic rendering.

### Playlist Actions

```rust
enum PlaylistAction {
    Add { id: ItemId, filename: String, after: Option<ItemId> },
    Remove { id: ItemId },
    Move { id: ItemId, after: Option<ItemId> },
}
```

Per-user append log. The canonical playlist is derived by replaying all actions
sorted by `(timestamp, action_sort_key)` where `action_sort_key` is
`(item_id.user, item_id.seq)`. This ensures deterministic ordering across all
peers, even for concurrent actions at the same timestamp.

---

## Snapshot Processing Order

When a `StateSnapshot` is received, fields are merged in this order:

1. **Peer generations** — must be processed first to gate per-peer state acceptance
2. **File register** — must precede position register (position is conditional on file_id)
3. **Position register** — conditional merge, may be rejected if for wrong file
4. **User states** — per-peer LWW, skipped if generation is stale
5. **File states** — per-peer LWW, skipped if generation is stale
6. **Chat/playlist vectors** — trigger gap fill requests for missing entries

After merging, if any LWW field was updated, the snapshot is forwarded to
peers other than the sender and origin (epidemic dissemination).

---

## Wire Format

Wire version: **2** (bumped from 1 for the register split + generation counter).

Messages use `[version_byte][postcard_payload]`. The version byte ensures
clean rejection between incompatible peers.

```
WireMessage::Application(Vec<u8>)
  └─ GossipMessage { origin, timestamp, payload }
       └─ GossipPayload::StateSnapshot {
              file_register,
              position_register,
              user_states,
              file_states,
              peer_generations,
              chat_vectors,
              playlist_vectors,
          }
```

Append log entries are delivered separately:
- **Eager push**: `GossipPayload::AppendEntry` via datagram (low latency)
- **Gap fill**: `GossipPayload::GapFillRequest` (datagram, debounced 500ms) →
  `GossipPayload::GapFillResponse` (reliable stream)

---

## Broadcast Intervals

| Condition | Interval |
|-----------|----------|
| Playing | 100ms |
| Paused | 1s |
| Burst mode (after state change) | 100ms for 1s |

Position updates do NOT trigger burst mode (already at 100ms during playback).
File changes, user/file state changes, and playlist actions trigger burst mode.

---

## Future Work

### Append Log Compaction

Both chat and playlist logs grow without bound. For v1 this is acceptable
(sessions are short, action counts are low). For v2, compaction is needed:

**Approach**: Snapshot-based compaction with ACK protocol.
- Periodically, a peer creates a compacted snapshot of the log state
- Peers ACK the snapshot, indicating they have all entries up to that point
- Once all peers ACK, entries before the snapshot can be discarded
- Late joiners bootstrap from the snapshot + entries after it

**Open design questions**:
- Who initiates compaction? (Any peer? Designated leader?)
- How to handle peers that are offline during compaction?
- What does a playlist snapshot look like? (Resolved item list + compaction sequence number)

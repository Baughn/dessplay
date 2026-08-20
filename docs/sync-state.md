# Sync State Design

Last updated: 2026-08-21

DessPlay uses the **`crdts`** crate for state synchronization. All shared state
is expressed as CRDT types from this library, synced through the server as
central coordinator.

## Table of Contents

1. [Core Concepts](#core-concepts)
2. [Replicated Data Types](#replicated-data-types)
3. [Transport](#transport)
4. [Compaction](#compaction)
5. [Snapshot Storage](#snapshot-storage)
6. [Failure Modes](#failure-modes)

---

## Core Concepts

### Shared Clock

All operation timestamps use a shared clock established via the NTP-style
protocol with the rendezvous server (see design.md, Time Synchronization).
This provides a consistent total ordering of operations across all peers.

### Dual-Mode Sync: CmRDT + CvRDT

The `crdts` crate types implement both CmRDT (operation-based) and CvRDT
(state-based) replication. DessPlay uses both:

- **CmRDT (normal operation):** Clients generate operations locally and send
  them to the server. The server applies them and broadcasts to other clients.
  Each operation is the native `Op` type of the underlying CRDT.

- **CvRDT (reconnection / gap recovery):** When a client reconnects or
  detects it has missed operations, the server sends its full state. The client
  calls `.merge()` on each CRDT field. This is idempotent, commutative, and
  associative -- safe to apply at any time.

There is no custom operation log, version vector, or gap-fill protocol. The
`crdts` types track causality internally via vector clocks.

### Epochs

An **epoch** is a generation counter incremented each time the rendezvous
server compacts the state (see [Compaction](#compaction)). When the connect
handshake decides on a snapshot, the client replaces its local state
entirely with the server's.

The server is **authoritative**: at connect time the client adopts the
server's snapshot *regardless of how the epochs compare*. The counter is a
bare u64, so it is not monotonic across operator interventions — restoring
the server's database from a backup legitimately rolls it backwards (or
lands it on a value clients already hold). A client that refused
lower-epoch snapshots at connect wedged forever after exactly such a
restore (2026-08, tsugumi); only a *mid-session* backward snapshot is
treated as a stale frame and ignored. Because a bare epoch match proves
nothing after a restore, the handshake also compares state hashes — see
[Reconnection Sync](#reconnection-sync).

### Actor IDs

Every participant in the CRDT system has an `ActorId`:
- Each client derives a fresh **session-scoped** ActorId at startup
  (hash of username + random nonce)
- The server has a well-known ActorId used for authoritative actions

Actors are per-session because Map ops carry per-actor sequence numbers
(dots): a client restarting from a stale snapshot would re-allocate
numbers its previous incarnation already spent — double-spent dots,
i.e. state corruption. Fresh actors make that structurally impossible.
The cost is one map-clock entry per session, which **compaction must
collapse by rebuilding the state from its resolved view under the
server actor** (a Phase 5 requirement; LwwCell values lose nothing in
a rebuild).

The server ActorId is used when:
- Advancing now-playing on EOF (only the server does this)
- Becoming seek authority on file change
- Writing AniDB metadata (server-authoritative)

### LWW Conflict Resolution via `LwwCell<V>`

All register state uses last-writer-wins semantics, implemented by our
own `LwwCell<V>`: a pure **max-merge register**. The op type *is* the
timestamped value; applying an op and merging a state are the same
operation — keep `max((timestamp, value))`:

```rust
struct Lww<V> { timestamp: SharedTimestamp, value: V }  // Ord = (ts, value)

struct LwwCell<V> { current: Option<Lww<V>> }
// CmRDT::apply(op)  = current.max(op)
// CvRDT::merge(rhs) = current.max(rhs.current)
// ResetRemove        = no-op (nothing is ever causally retracted)
```

This is commutative, associative, and idempotent under *any* delivery
order, and carries **no causal metadata** — highest timestamp wins, with
value-based tiebreaking on equal timestamps.

**Why not `crdts::MVReg<Lww<V>>` (the original design)?** Property
testing found two view-divergence bugs in the `Map` + nested-`MVReg`
composition, both from the same impedance mismatch: nested put clocks
are map-global while the Map's remove/merge machinery reasons
entry-scoped. `Map::rm` racing a concurrent re-add leaves ghost values
on some replicas (Phase 1); a plain CvRDT `merge` trims value clocks
and breaks dominance between sequential writes, resurrecting overwritten
values (Phase 3 — see `dessplay-core/tests/regressions.rs`). Since every
DessPlay register resolves to an LWW winner anyway, the multi-value
register bought nothing but the bug surface.

**Timestamp discipline:** under pure LWW, a causally-later write with
an *older or equal* timestamp can lose (equal stamps fall to the value
tiebreak). Writers must issue **Lamport-monotonic** timestamps:
`max(shared_now, last_issued + 1)`, where `last_issued` is raised not
only by the writer's own stamps (Phase 4) but by every LWW timestamp it
*observes* — remote ops, merges, snapshots, and state loaded from
storage at startup (Phase 5). Self-monotonicity alone is not enough:
two actors writing in the same shared-clock millisecond tie, and the
causally-later write — the server forcing intent to Paused right after
applying a client's Playing — could lose the tiebreak. Found by the
Phase 5 EOF tests; pinned in
`dessplay/src/actors/sync.rs::stamps_dominate_observed_remote_timestamps`.

### crdts Crate Integration

We use the following types from the `crdts` crate:

| Type | Our usage |
|---|---|
| `LwwCell<V>` (ours) | Max-merge LWW register; the only register type |
| `Map<K, V, A>` (crdts) | Keyed collections (playlist, per-user maps, file availability) |
| `GList<V>` (crdts) | Grow-only ordered list (chat) |
| `GSet<V>` (crdts) | Grow-only set (lookup requests, acknowledged-absent) |
| `Identifier<T>` (crdts) | Dense ordering for playlist positions |

The standard pattern is `Map<K, LwwCell<V>, ActorId>`. The Map's keys are
effectively grow-only (we never use `Map::rm`; see below), so its
observed-remove machinery sits unused.

Per-user state uses **compound keys**: e.g. `Map<(UserId, ListEntryId), ...>`
rather than separate CRDT instances per user. This keeps the state table flat
and the wire protocol uniform.

#### crdts API Notes

- **Map return types:** `Map::get()` returns `ReadCtx<Option<V>>` (cloned value
  via `.val`), while `Map::iter()` yields `ReadCtx<(&K, &V)>` (references via
  `.val`). Always access the `.val` field to get the inner data.
- **GSet::apply():** Takes the element directly as the op (no separate insert +
  apply pattern).
- **GList::read():** Returns references (`FromIterator<&T>`), not owned values.
- **`Map::rm` is banned.** Property testing (Phase 1) found that op-based
  `Map::Rm` is not view-convergent when nested registers carry map-global
  put clocks: a remove racing a concurrent re-add wholesale-drops the
  entry on some replicas but leaves a resurrected "ghost" value on
  others. All removal in DessPlay is expressed as an **LWW tombstone**
  (the register value is an `Option`, `None` = removed), and the server
  purges tombstones at compaction. (With `LwwCell` values there are no
  put clocks left to corrupt, but the tombstone design stays — it is
  simpler and compaction wants it anyway.)
- **Nested `MVReg` is banned.** The second divergence (Phase 3,
  `tests/regressions.rs`): `Map::merge` computes "information the other
  side has deleted" against entry-scoped clocks and `reset_remove`s it
  from value clocks, which are map-global — breaking dominance between
  sequential writes and resurrecting overwritten values, with no removal
  involved anywhere. This is why registers are `LwwCell` (no causal
  metadata at all) rather than `MVReg<Lww<V>>`.
- **Delivery requirements.** `LwwCell` values converge under any
  delivery order. `Map` update ops still carry per-actor sequence
  numbers (dots); ops from one origin must be applied in the order that
  origin generated them, or later dots mask earlier ones and ops are
  lost silently. The hub-and-spoke topology provides per-origin FIFO
  for free (ordered control streams; one server broadcast order; own
  ops applied in own order). **Phase 4 constraint:** the datagram fast
  path must not apply an op ahead of undelivered earlier ops from the
  same origin — hold (or drop) such datagrams until the reliable stream
  catches up.
- **Internal state vs. view equality:** replicas applying the same ops
  in different (valid) orders can end up with differing internal Map
  bookkeeping while resolving to the same view. Convergence is defined —
  and tested — on the resolved view, not on raw CRDT equality.

---

## Replicated Data Types

### Complete State Table

| State | CRDT Type | Owner |
|---|---|---|
| Playlist | `Map<Ed2kHash, LwwCell<Option<PlaylistFileState>>, ActorId>` | Any peer |
| Watched flags | `Map<Ed2kHash, LwwCell<bool>, ActorId>` | Server only (at EOF) |
| Now Playing | `LwwCell<Option<Ed2kHash>>` | Any peer; server on EOF |
| Seek Authority | `LwwCell<SeekAuthority>` (`Server \| User(UserSeek)`) | Whoever last seeked; the user variant records the explicit scrub for attribution; server on file change or authority departure |
| Playback intent | `LwwCell<PlaybackIntent>` (`Playing \| Paused`) | Any user (play/pause); server forces Paused on Lost, graceful quit, and EOF-advance; not on the later timeout promotion to Departed |
| Series preference | `Map<(UserId, ListEntryId), LwwCell<SeriesPreference>, ActorId>` | Any user (design.md #7/#13: `SeriesPreference { state, set_by }` lets one user write another's) |
| Manual override | `Map<UserId, LwwCell<Option<ManualState>>, ActorId>` | Owning user; *anyone* may write `Away` |
| Acknowledged absent | `GSet<(Ed2kHash, UserId)>` | Any peer inserts; cleared on compaction |
| File availability | `Map<(UserId, Ed2kHash), LwwCell<FileAvailability>, ActorId>` | Each user writes own |
| AniDB metadata | `Map<Ed2kHash, LwwCell<Option<AniDbMetadata>>, ActorId>` | Server only |
| File catalog | `Map<Ed2kHash, LwwCell<FileCatalogEntry>, ActorId>` | Server only |
| Series relations | `Map<AniDbSeriesId, LwwCell<SeriesRelations>, ActorId>` | Server only |
| The List | `Map<ListEntryId, LwwCell<SeriesListEntry>, ActorId>` | Any peer |
| List next-ep | `Map<ListEntryId, LwwCell<NextEpState>, ActorId>` | Any peer; server auto-advance |
| Lookup requests | `GSet<FileHashInfo>` | Any peer inserts; cleared on compaction |
| Chat | `GList<ChatMessage>` | Any peer appends; trimmed on compaction |
| Marquee | `LwwCell<Option<MarqueeMessage>>` | The commentary-running client (design.md, AI Commentary); cleared on compaction |
| Playback position | `Map<UserId, LwwCell<PlaybackPosition>, ActorId>` | Each user writes own |

### Playlist

The playlist is a
`Map<Ed2kHash, LwwCell<Option<PlaylistFileState>>, ActorId>`.
The value is an `Option` because removal is a tombstone write (`None`),
not a `Map::rm` — see the API notes above.

`PlaylistFileState` contains:
- `position: Identifier<ActorId>` -- dense ordering via `crdts::Identifier`
- `added_by: UserId` -- who added this file
- `filename: String` -- original filename for display and matching
- `size_bytes: u64` -- filled by the adder; downloaders need it for chunk counts
- `duration_millis: Option<u64>` -- filled by the adder; drives the bitrate
  unpause rule and watched thresholds for files still downloading

**Watched flags live in a separate map** (`Map<Ed2kHash, LwwCell<bool>>`,
server-only writes at EOF) rather than inside `PlaylistFileState`. Keeping
them out of the struct avoids a real LWW race: a user moving an entry
(rewriting the whole struct with a new position) concurrent with the server
marking it watched would silently drop one of the two writes. With a single
writer (the server) on its own map, the race cannot occur.

**Ordering:** To display the playlist, collect all entries, resolve each
`LwwCell` to its winner, sort by `position` (ascending), with `Ed2kHash`
as tiebreaker.

**Adding:** To add a file after item X:
`Identifier::between(Some(&x.position), next.map(|n| &n.position), my_actor_id)`.
If adding at the end, `Identifier::between(Some(&last.position), None, my_actor_id)`.

**Moving:** To move a file to a new position: compute a new `Identifier`
between the two items it should sit between, write the new `PlaylistFileState`
with the updated position.

**Removing:** write a `None` tombstone over the entry (plain LWW write).
A concurrent update and remove resolve by timestamp — whichever was later
wins, identically on every replica. (`Map::rm`'s observed-remove semantics
were the original design, but proved non-convergent in `crdts`; see the
API notes above.) Tombstones are purged at compaction, so they never
accumulate beyond one epoch.

**Rebalancing:** After many moves, `Identifier` values grow in size (the
underlying `BigRational` denominators increase). The server rebalances during
compaction by reassigning fresh `Identifier` values with simple rationals
while preserving order.

**Conflict resolution:** Concurrent writes to the same key's `PlaylistFileState`
resolve by `LwwCell`'s max-merge (highest timestamp wins, with
value-based tiebreaking). With ~5 users and manual playlist management, true
simultaneous conflicts are rare. When they occur, the playlist converges to
*some* deterministic order and someone fixes it manually.

**Known limitation:** because the playlist is keyed by `Ed2kHash`, the same
file cannot appear twice (e.g. a same-session rewatch). Re-select the watched
entry with Enter instead. This is a deliberate consequence of the key choice,
not an oversight.

### Now Playing

`LwwCell<Option<Ed2kHash>>` -- a standalone register.

Any peer can set now-playing by selecting a playlist entry. The server sets
it on EOF (advancing to next item). Last writer wins via `Lww`.

**EOF transition:** clients report end-of-file to the server via the
`EofReached { file }` control message (not a CRDT op -- it is a report, not
state). On the first report matching the current now-playing value from a
present, watching user, the server: sets the watched flag, advances
now-playing to the next unwatched playlist entry, takes seek authority, and
auto-advances the List's `next_ep` if the file's series is linked and
`next_ep` is numeric. Subsequent reports no longer match now-playing and are
ignored -- the transition is idempotent without any dedup bookkeeping.

### Seek Authority

`LwwCell<SeekAuthority>` where `SeekAuthority = Server | User(UserSeek)` --
whoever most recently initiated a seek. `UserSeek` contains the user, file,
unique Lamport occurrence stamp, and the initial/final positions of the
debounced scrub. This is control-plane causality: continuous position reports
remain lossy telemetry for drift correction and are never used to infer that a
seek occurred.

**How it works:**
1. User A seeks -> their client writes `User(UserSeek { user: A, ... })` to the seek authority register
2. All clients see "A is authoritative" and sync their position to A's
   `PlaybackPosition`
3. Normal playback continues; small drift between clients is ignored
4. User B seeks -> B becomes authoritative; everyone syncs to B's position
5. Drift threshold: if >3s off from authority's position, trigger a seek

**Debounce:** the PlayerActor captures the position at the beginning of a
scrub, updates its destination as the user continues, and emits one explicit
`UserSeeked { from_millis, to_millis }` after 1500ms of quiet. Only then does
the client write user seek-authority plus the final position. Automatic seeks
(file load to zero, drift correction, crash restore) are echo-suppressed and
never produce `UserSeek`.

**Echo suppression:** When you receive a seek authority change naming *you*,
ignore it -- you already performed the seek.

**File change:** When now-playing changes, the server becomes seek authority.
Everyone resets to position 0. The server's authority prevents spurious seeks
during the transition.

**Authority departure:** if the current seek authority becomes Departed
(see presence in [network-design.md](network-design.md)), the server takes
seek authority so remaining clients never sync to a ghost.

### Playback Intent

`LwwCell<PlaybackIntent>` where `PlaybackIntent = Playing | Paused`.
Defaults to Paused in a fresh state.

Whether video actually plays is *derived*:
`intent == Playing && all present users permit && nobody is Lost`.
The register is the latch that gating alone cannot express: without it,
playback would silently auto-resume the moment a paused/lost user departs
(departed users are removed from gating) or returns Ready.

**Writers:**
- Any user, on play/pause in their player. Pause also sets the user's
  manual override (so the UI shows *who* blocks resume); play clears
  their own override and writes `Playing` (if others still block, the
  local player is re-paused by derivation -- "you tried").
- The server forces `Paused` when a user becomes Lost, on graceful quit
  during playback (the immediate-departure path), and when EOF advances
  now-playing (the next episode loads paused). The later Lost -> Departed
  timeout sweep does not write it again: a deliberate resume during the
  Lost window must survive that bookkeeping transition.

### Series Preference

`Map<(UserId, ListEntryId), LwwCell<SeriesPreference>, ActorId>`. Keyed on
the [List](#the-list) entry, not the AniDB series id -- AniDB linking is
enrichment only, and a real number of series the group watches have no AniDB
entry at all. See design.md's [Series Identity](design.md#series-identity)
for how a file resolves to a `ListEntryId` (auto-creating an entry on first
commitment if none claims the file yet).

`SeriesPreference { state: SeriesWatchState, set_by: Option<UserId> }`
(design.md #7/#13). `set_by` mirrors `ManualState::Away`'s setter: `None`
for every self-directed write and system auto-write (rendered as the
subject by the narrator), `Some(actor)` when one user writes *another's*
preference (`n` on the Users pane, `/skip <name>` -- the "Kim tool": rule
on someone's commitment without waiting for them to reconnect). There is
no special-casing on read -- a later write from the subject themself wins
by plain LWW, same as any other concurrent write to the key.

`SeriesWatchState` is `Watching | NotWatching | Maybe` -- a user's
commitment to a series, with three values:

- **Watching** (committed): the group waits for this user on this series
  even when they are absent (Lost/Departed). This is the only state that
  gates across absence.
- **NotWatching**: the user never gates on this series, present or absent,
  and it is not auto-downloaded.
- **Maybe** (the default): opportunistic -- gates only while the user is
  *present* and not ready-to-play; an absent Maybe user does not block.

See [design.md](design.md#user-states) for how these compose with the
manual override into the derived user state, and
[Playback Rules](design.md#playback-rules) for the gating matrix.

**Absent entry == Maybe.** A user with no map entry for a series is treated
as Maybe -- the common case (you've never ruled on most series). The schema
*also* stores `Maybe` explicitly so a user can cycle a series back to the
default from `Watching`/`NotWatching`: removal is not an option, because
DessPlay never uses `Map::rm` (see the API notes above -- it is
non-convergent here), so "return to default" must be a real written value.

**`Maybe` is a *trailing* enum variant** (`Watching = 0`, `NotWatching =
1`, `Maybe = 2`). serde/postcard number variants by declaration order, so
appending `Maybe` keeps the existing two discriminants byte-identical:
**snapshots and ops written before the change still deserialize unchanged**
(an old binary, however, cannot read a `Maybe` written by a new one -- the
upgrade is all-at-once across the group). `SeriesWatchState` derives `Ord`,
which is the LWW same-timestamp tiebreak; appending a variant only *extends*
the total order (`Maybe` becomes the max), so it stays a deterministic
total order on every replica -- convergence-safe. The only visible effect
is that at an exact-millisecond tie, a concurrent `Maybe` write beats a
`Watching`/`NotWatching` one, which is vanishingly rare under the
Lamport-monotonic stamp discipline.

**The `SeriesPreference` wrapper (Phase 16) is a value-*shape* change**,
unlike the `Maybe`-variant addition above: the map's value type gained a
field (`set_by`), not just a new enum discriminant. Snapshot decode
migrated old on-disk blobs via what was then a versioned trial-decode
fallback chain (`CrdtStateV3` -- identical to that day's layout but with
the bare `SeriesWatchState` value): each old entry was resolved to its
`(timestamp, value)` winner and rewritten as `SeriesPreference { state,
set_by: None }` under a reserved migration-only `ActorId`, preserving the
timestamp (so a later real write from any actor still LWW-compared
correctly) without needing the old entries' original per-actor dot
structure -- safe because `ActorId`s are session-scoped (Phase 4), so no
live session's dot clock depends on a restart-time migration reproducing
it. (The chain itself has since been retired -- see
[Snapshot Storage](#snapshot-storage). Note added 2026-08-20: this
reasoning covered live sessions but missed migrated-replica-vs-
migrated-replica merges -- a single constant migration actor double-
spends dots across replicas holding different key sets, and the merge
destroys entries. Migration-time rebuilds must dot entries under a
**key-derived** actor; see Decoding and Migration below.)

**Re-keying `AniDbSeriesId` -> `ListEntryId` (Series Identity work) is a
key-*type* change**, not a value-shape change, so it needs the same
snapshot-migration + `PROTOCOL_VERSION` bump treatment as the moves above,
plus one wrinkle the `Maybe`-variant and `SeriesPreference`-wrapper
migrations didn't have: the new key doesn't exist yet for old data. The
migration must *synthesize* it: for every `AniDbSeriesId` referenced by an
old-layout `series_preference` entry, find the List entry already linked to
that id (linking predates this change, so most will have one) or synthesize
one -- name from the snapshot's own `anidb_metadata` map if a cached title
exists, else a placeholder pending a later fill -- and rewrite each old
`(UserId, AniDbSeriesId)` entry as `(UserId, ListEntryId)`, preserving the
original timestamp exactly as the `SeriesPreference`-wrapper migration does.
Because this mutates *two* maps (`series_preference` and `the_list`) from
one old snapshot, it runs as its own decode-time step, not folded into the
generic versioned-fallback chain used for pure value-shape changes.

**Mid-development layouts can become load-bearing.** A build from the
middle of the Phase 19 window — re-keyed and with
`local_aliases`/`manual_files`, but before `SeriesListEntry` gained
`anidb_unavailable` — was deployed to the rendezvous server (2026-07-04)
and wrote authoritative snapshots in that never-numbered shape; the next
day's binary then refused to start, since no fallback matched it. The
trial-decode chain then carried that layout as `CrdtStateV5`. The
general lesson stands under today's tagged envelope too: the server
persists whatever layout it *runs*, so any layout that ever reaches
tsugumi must remain decodable — the envelope makes the failure loud and
attributable (a version-mismatch error naming both versions) instead of
a silent misdecode, and a deliberate migration adds an explicit decode
arm for the old version (see [Snapshot Storage](#snapshot-storage)).

### Acknowledged Absent

`GSet<(Ed2kHash, UserId)>` -- a grow-only set of `(now-playing file,
committed-absent user)` pairs.

A **committed** (Watching) user who is absent keeps gating the now-playing
file (the "wait for me even if I've been gone a week" rule). To play past
them the group records a **per-file one-shot acknowledge**: inserting
`(now_playing, user)` here suppresses that user's committed-absent block
*for that file only*. Gating consults the set keyed by the **current**
now-playing file, so advancing to the next episode leaves the old entry
behind and the block re-raises -- it is re-acknowledged consciously each
file.

It is synced (not local) because gating is computed independently on every
client: if one client acknowledged locally and another did not, they would
disagree on whether to play and desync. A `GSet` fits because the signal is
purely additive within a session -- no entry is ever retracted -- and it is
**cleared at compaction** (daily, far from watch hours), like the lookup
set. This is deliberately *not* a reuse of the per-user `Away` override:
`Away` is per-user and would persist across episodes until the marked user
returned, whereas acknowledge must be per-file and one-shot.

### Manual Override

`Map<UserId, LwwCell<Option<ManualState>>, ActorId>`,
where `ManualState` is `Paused | Away { set_by: UserId }`.

`Paused` is set when the user manually pauses, cleared (set to `None`) when
the user attempts to unpause. Takes priority over the series preference.

`Away` is the exception to "each user writes own": *any* user may write
`Away` into another user's register (for the friend who walked off without
quitting). `set_by` records who did it, for display. Any input from the
marked user's client clears the override back to `None` (or `Paused`-then-
unpause semantics as normal). For playback gating, `Away` behaves like
NotWatching -- it does not block.

### File Availability

`Map<(UserId, Ed2kHash), LwwCell<FileAvailability>, ActorId>`.

`FileAvailability` is `Ready | Missing | Downloading { progress_bps: u16 }
| DownloadingPlayable { progress_bps: u16 }` (basis points 0–10000, avoids
float Eq/Ord issues). Each user writes their own availability for each
file. This determines the file state column in the UI and whether the user
blocks playback.

The downloading split carries the holder's own **playable verdict**: the
downloading client — the only party holding both the chunk bitmap and its
player position — advertises `DownloadingPlayable` when every chunk in the
20% window ahead of its playback position is verified, and `Downloading`
otherwise. Gating then needs no arithmetic: `Downloading` blocks,
`DownloadingPlayable` doesn't (design.md, File State). The
`DownloadingPlayable` variant is **appended** after `Downloading` and the
variants are never reordered: postcard encodes the variant index, so
appending is what keeps pre-v10 snapshot bodies decodable by the current
type. (A Nyaa browse import never writes either variant — it is not a
playlist entry until its payload completes and verifies.)

### Lookup Requests

`GSet<FileHashInfo>` -- a grow-only set of files that clients want the server
to look up via AniDB.

`FileHashInfo` contains:
- `hash: Ed2kHash`
- `size: u64` -- file size in bytes (AniDB's FILE command requires this)
- `filename: String` -- for fallback metadata when AniDB doesn't know the file
- `mtime: Option<i64>` -- the file's mtime (unix millis) when the requester
  holds it locally; `None` for a playlist entry the client doesn't have. The
  server anchors the re-validation backoff on the *older* of this and when it
  first queued the file, so long-owned files AniDB doesn't know aren't
  re-polled on the aggressive new-file ladder after a queue reset (see
  "Parsing files" in design.md)
- `series_hint: Option<String>` -- a title-like containing-directory name
  (e.g. `RahXephon` for a file under `<root>/RahXephon/Season 1/...`),
  derived by the requester from the local path when it holds the file;
  `None` when unknown or when no ancestor directory looked like a title
  (season/disc folders and generic library containers are skipped). When
  AniDB doesn't know the file, the server prefers this over the filename
  stem for the fallback `series_name`, so a series' AniDB-unknown episodes
  group into one franchise instead of one entry per episode

Clients insert entries for **every** file their media-root scan finds that
still lacks metadata -- not just playlist entries (see "Media Library
Scanning" in design.md). The scan runs at startup and periodically (about once
a minute interactive, once a day for a seeder), re-hashing only files whose
`(mtime, size)` changed. The server drains entries into its AniDB lookup
queue. On compaction, the GSet is cleared -- all entries have been processed or
queued.

This re-arms naturally on reconnect and after compaction: clients start fresh,
re-scan their local files, check each hash against the metadata map, and
re-insert any that are still `None`. The server deduplicates against its
existing metadata and lookup queue.

### File Catalog

`Map<Ed2kHash, LwwCell<FileCatalogEntry>, ActorId>`.

Only the server writes these. When the server drains a lookup request it
records the file's identity, taken straight from the `FileHashInfo`, so any
client -- even one that has never held the file -- can construct a playlist
entry for it and download it. This is what lets the franchise browser add files
from the group's collective library rather than only locally-present ones.

```rust
struct FileCatalogEntry {
    filename: String,             // from the lookup request
    size_bytes: u64,              // from the lookup request
    duration_millis: Option<u64>, // None until an owner reports it / it downloads
}
```

`duration_millis` is absent from the lookup request, so it is filled lazily --
by an owner once the file is held, or at download time. Until then the
bitrate-based unpause rule can't be computed and the 20%-downloaded rule
governs alone. The catalog persists across compaction (unlike the lookup GSet),
so the browsable library does not evaporate at the daily compaction.

### AniDB Metadata

`Map<Ed2kHash, LwwCell<Option<AniDbMetadata>>, ActorId>`.

Only the server writes these. `None` means "not yet looked up." The server
fills in metadata from two sources:

1. **AniDB lookup succeeds**: full metadata (series name, ID, episode number)
2. **AniDB lookup fails**: filename-derived metadata. The series name is the
   requester's `series_hint` (a title-like containing-directory name) when one
   was supplied, else the filename minus its extension; no series ID, no
   episode number. The directory hint keeps a series' AniDB-unknown episodes
   grouped under their folder rather than splitting into one franchise per
   episode. Any smarter filename parsing (stripping group tags, episode
   numbers) is done at the display level so it can be updated without
   re-querying.

Either way, the register becomes `Some(AniDbMetadata)` -- downstream code
always has a series name to work with.

```rust
struct AniDbMetadata {
    source: MetadataSource,           // AniDb | FilenameDerived
    series_name: String,              // always present
    series_id: Option<AniDbSeriesId>, // None if filename-derived
    episode_number: Option<String>,   // None if unknown (AniDB uses "S1", "C1", etc.)
}
```

Code that needs franchise grouping (series browser) checks `series_id`.
Files without an AniDB series ID are grouped by `series_name` as a fallback
(less accurate, but functional).

### Series Relations

`Map<AniDbSeriesId, LwwCell<SeriesRelations>, ActorId>`.

Server-only writes. `SeriesRelations` holds the series' related-anime edges
(relation type + target series ID) plus display data (title, year, episode
count) fetched via the AniDB ANIME command, and `short_titles`
(appended in protocol v11): the **AI-curated** community short name —
zero or one entry (the `Vec` is wire form) — that The List renders
instead of the official title. The server walks relations recursively
as new series IDs appear, under the same rate limiter as file lookups,
caching results in its SQLite.

Short titles come from the curator (`anidb/curator.rs`), not the ANIME
reply and not raw from the titles dump (whose kind-3 rows are lowercase
search tags, useless for display): the worker's `curate_short_titles`
pass batches unsettled series to an Anthropic model — the dump's full
title rows as grounding context, fenced in the prompt as untrusted
data — and caches each answer durably in the `ai_short_titles` table,
including the "no short name exists" answer, so the API **answers each
series at most once**. Answers are keyed positionally to the batch
that was sent: a reply naming a series outside it (hallucinated, or
injected through the title rows) is unrepresentable past the parse
layer and is dropped with a warning; within the batch the answer is
trusted as returned. A series the model *doesn't* answer accrues a
durable attempt count (`attempts`/`settled` columns, schema v7) —
model-side failures like refusals, truncation, and timeouts count
against every series in the batch, transport failures against none —
and after five unanswered attempts it settles as a durable
no-short-name answer, mirroring the anime queue's `next_attempt =
NEVER` tombstone. Batch selection orders by fewest attempts first, so
an unanswerable batch rotates to the back instead of starving the
catalogue. The model call runs on its own task and is only polled by
the worker loop — a slow curator never delays metadata lookups. The
same pass reconciles the replicated `short_titles` with the settled
cache (one bulk read per pass) and quiesces; unsettled series are
never touched (a tokenless server clears nothing). Curation runs
only while an API token is stored in the kv table — client-provisioned
over the wire via `SetAnthropicToken` (protocol v12), pushed by the
token-holding client on connect and on settings edits, so it can
appear, rotate, or vanish without a server restart. Failures and
answerless replies back off ten minutes and cache no answer. Clients
compute franchise groupings (connected components over
sequel/prequel/side-story edges) locally from this map.

### The List

`Map<ListEntryId, LwwCell<SeriesListEntry>, ActorId>` for entry
data, plus `Map<ListEntryId, LwwCell<NextEpState>, ActorId>` for
the fast-changing progress fields.

`ListEntryId` is a random 128-bit ID generated at entry creation (or import).
See design.md, [The List](design.md#the-list-series-tracker), for the full
`SeriesListEntry` and `NextEpState` schemas and the CSV import rules.

The split into two maps exists because `next_ep` is written both by users
(weekly "episode 5 is out" updates) and by the server (auto-advance at EOF
for linked series); keeping it out of `SeriesListEntry` prevents those writes
from clobbering concurrent edits to notes/status, mirroring the
watched-flags reasoning above.

Entries are whole-struct LWW -- edit frequency is a few writes per week, and
losing one concurrent note edit is shrug-worthy. This also covers
`local_aliases`/`manual_files` (design.md's [Series
Identity](design.md#series-identity)): they ride along with the rest of the
entry rather than getting their own map, since they change about as often as
notes do and don't need finer-grained conflict resolution.

**The List survives compaction untouched and is never pruned**: it is a few
hundred rows of text and the history is the point.

### Chat

`crdts::GList<ChatMessage>` -- a grow-only ordered list.

Each `ChatMessage` contains:
- `sender: UserId`
- `text: String`
- `timestamp: SharedTimestamp`

The chat log's [system messages](../docs/design.md) (joins, pauses,
seeks, …) are local-only and derived per-client; they never enter this
GList. The one exception is the player-crash notice, written by the
crashing client as an ordinary `ChatMessage` so it persists and reaches
late joiners.

GList handles ordering and deduplication. Messages are displayed sorted by
the GList's internal ordering (which respects insertion order). Operations
are the GList's native `Op` type, sent via CmRDT.

Chat is trimmed to the most recent 500 messages at compaction. Before
trimming, the server archives the full history to its SQLite, so nothing is
truly lost -- the replicated state just stays bounded.

### Playback Position

`Map<UserId, LwwCell<PlaybackPosition>, ActorId>`.

`PlaybackPosition` contains:
- `position_millis: u64` -- milliseconds as integer (avoids float Eq/Ord issues)
- `timestamp: SharedTimestamp`
- `file: Ed2kHash` -- the file the sample was taken against. Drift correction
  follows a peer's position only when this equals now-playing, so a *stale*
  sample left over from the previous file (a peer advertises
  `FileAvailability::Ready` for the next episode on **prefetch**, before its
  player loads it) cannot elect a bogus leader after a now-playing
  transition. The tag is a **clock-free** identity check -- comparing a
  peer's sample timestamp against the now-playing write time would be
  unsound, since `SharedTimestamp`s are only reliably ordered *within* one
  machine, not across the loosely-NTP-synced fleet. (When this field was
  appended, pre-`file` blobs decoded via the then-current trial-decode
  fallback, which dropped their ephemeral positions; that chain has since
  been retired -- see [Snapshot Storage](#snapshot-storage).)

Updated at high frequency: every 100ms during playback, every 1s when paused.
These updates are **not persisted to SQLite on every update**. The server
compacts them to a single value per user.

This is a proper CRDT (not ephemeral gossip), which means:
- Position survives reconnection (the server has the last known value)
- No special-case code for "ephemeral" vs "persistent" state
- The CRDT machinery handles deduplication and ordering naturally

**Transport exception:** unlike every other op type, position ops are sent
via **datagram only** at the 100ms rate, with a 1s reliable-stream fallback
tick. Sending every position op reliably would queue stale positions behind
retransmissions on lossy links (head-of-line blocking) -- position is the
one type where dropping intermediate values is strictly correct. See
[network-design.md](network-design.md).

---

## Transport

### Server as Hub

All state sync flows through the server:

1. Client generates a CmRDT operation -> sends it to the server
2. Server applies it and broadcasts to all other clients
3. Clients receive ops from the server and apply locally

This eliminates:
- Peer-to-peer state reconciliation
- Clock skew between clients (only client-server offset matters)
- Complex relay/forwarding logic for state ops

### Operation Broadcast

When a client generates a new operation:

1. Apply it locally (immediate feedback)
2. Send it to the server via the control stream (reliable)
3. Also send via datagram (best-effort, for lower latency)
4. The server deduplicates, applies, and broadcasts to other clients

Every `StateOp` is **epoch-tagged**, and both sides drop ops whose
epoch is not their current one. This protects compaction: the rebuild
resets per-actor dot sequences, and an old-epoch op landing on the
fresh state would advance a dot clock the sender is about to restart
from 1 — its next ops would be silently deduped as already-seen. Ops
in flight at the compaction edge are dropped by design (the daily
schedule keeps that window away from watch-party hours).

### Reconnection Sync

When a client connects or reconnects (protocol v13, the `SyncStatus`
handshake):

1. After `AuthOk`, the client sends `SyncStatus { epoch, state_hash }` —
   its epoch plus its `view_hash()`, i.e. what it *actually* holds.
2. **Identical replica** (epoch AND hash match the server's): the server
   sends its full CvRDT state as a `StateMerge`. Client calls `.merge()`
   on each CRDT field — idempotent, and a no-op on an identical replica.
3. **Anything else** — stale epoch, *backward* epoch (server restored
   from a backup), or an epoch match with a differing hash — the server
   sends a `StateSnapshot`. The client, being in the connect window,
   adopts it wholesale regardless of epoch direction (logging loudly when
   the epoch went backwards). The hash gate is the restore defense: a
   bare epoch match must never buy a merge, because a restored backup
   colliding with the clients' epoch would then be re-polluted by every
   client's stale union.
4. **Upward merge:** the client re-applies any ops buffered while
   offline (playback positions coalesced to the latest) onto the
   adopted state and pushes its **full state** back as a `StateMerge` at
   the adopted epoch. The server merges it and rebroadcasts a merge to
   everyone if it changed anything.
5. Resume normal CmRDT operation broadcast.

**Accepted loss window:** ops generated while *connected* but still
undelivered when the connection died exist only in the client's replica;
they make the hashes differ, so the handshake answers with a snapshot and
adoption discards them. This window is seconds wide (the gap between last
delivery and death), the edits are low-stakes and simply redone, and the
offline buffer — everything generated while *down* — is always preserved
and replayed. The alternative (merging on a bare epoch match, which
recovered those ops) is exactly what re-polluted a restored server, so
the loss is accepted by design.

No version vectors or gap-fill protocol needed. The CvRDT merge handles
any missed operations implicitly.

### Divergence Alarm

A safety net, not a correctness mechanism — property testing has caught
two real CRDT divergence bugs, and if a third ever ships, it should be
a logged, self-correcting event rather than a silent one:

- Every 30s the server broadcasts `StateHash { epoch, hash }` on the
  control stream: a SHA-256 over the postcard encoding of its resolved
  view **excluding playback positions** (which churn every 100ms and
  would never match).
- The client compares against its own view hash on receipt. A single
  mismatch is expected churn (ops in flight); **two consecutive
  mismatches** trigger a loud log line and a `RequestMerge`, to which
  the server replies with a **`StateSnapshot`** (protocol v13; it was a
  `StateMerge` before). The heal is curative by design: a diverged
  replica holds — or lacks — something the server's does not, and a
  snapshot removes client-local garbage where an additive union would
  merge it back in and re-spread it. A false alarm costs one snapshot
  transfer and (mid-session, so the offline buffer is empty) nothing
  else.

### Escalation Ladder

Auto-healing is silent; the user hears about it only when it stops
working. The sync actor counts **consecutive failed heals** — alarms
fired without a matching `StateHash` in between:

- Attempts 1–2: silent (beyond the loud log). Two, not one, because of
  the one-cycle heal race: an op sent between our `RequestMerge` and
  the server's curative snapshot re-diverges the replica once through
  no fault of the mechanism, and re-converges within about one hash
  period.
- Attempt 3: `SyncEvent::DivergencePersisted` — a one-shot chat line
  plus a sticky advisor suggestion: *"state diverged and is not healing
  — run /resync (or dessplay --reset-sync)"*. Emitted once per
  escalation; it does not repeat while the divergence persists.
- Any matching hash after a failed heal (or after an escalation) emits
  `SyncEvent::DivergenceHealed`, which clears the sticky advisor flag.
  A merely *healthy* link (`HealthLevel::Ok`) deliberately does not:
  link health says nothing about replica equality.

### Manual Reset (`/resync`)

The remedy the escalation names — reachable as the `/resync` chat
command, the Settings → Account "Reset synced state" action row, or
offline as `dessplay --reset-sync` (the headless spelling, which clears
the client's `dessplay.sync.db` directly — see [Snapshot
Storage](#snapshot-storage)). `SyncCommand::ResetState`:

- Discards the replica wholesale: fresh `CrdtState`, epoch 0, offline
  buffer/position cleared, alarm and heal counters reset, flushed to
  storage immediately (a crash must not resurrect the discarded state).
- **Keeps `last_issued`**, the Lamport floor. Pre-reset stamps live on
  in the server's state; a post-reset write re-issuing one of them
  would tie — and could lose — the LWW comparison against a value it
  causally supersedes.
- Connected: sends `RequestMerge`; the reply is the curative snapshot
  at the server's epoch, adopted wholesale. Link down: sends nothing —
  the reconnect handshake covers adoption (`SyncStatus` reports epoch 0
  plus the empty-state hash, and the server answers any mismatch with a
  snapshot).

No confirmation step: both entry points are deliberate acts, and the
shared state is losslessly recoverable from the server. Local-only
tables (watch history, hash cache, manual mappings) are untouched;
file availability re-derives from local files and re-announces.

---

## Compaction

### When It Happens

**Scheduled daily** at a configured UTC time (default 12:00,
`--compact-at HH:MM`, or `never`) -- chosen to be maximally far from
watch-party hours. The old "no clients connected for 5 minutes" trigger
does not work here: seeders are always connected, so quiescence never
occurs. Tests use a fixed-period schedule under paused time instead.

Compaction runs with clients attached and needs no stop-the-world
phase: the epoch tag on `StateOp` (see Operation Broadcast) makes the
boundary safe — old-epoch ops in flight are dropped on both sides, and
connected clients adopt the broadcast snapshot like a stale-epoch
reconnect.

### How It Works

The heart is `dessplay_core::compact::rebuild`, a **pure function**
from a resolved view to a fresh state authored entirely by the server
actor — property-tested to preserve the view exactly, to be idempotent,
and to collapse all per-session actor clocks into the server's (the
debt taken on by session-scoped ActorIds). Around it, the server:

1. Takes its current resolved view (one lock).
2. Rebuilds: playlist tombstones are purged and `Identifier` positions
   reassigned small and flat; watched flags resolving `false` are
   dropped (absent already reads as unwatched) while `true` flags are
   kept for **all** files, playlist or not — the group's durable watch
   record, which The List's unwatched-file detection reads (bounded by
   files-ever-watched; until 2026-08-17 off-playlist flags were dropped
   wholesale, resurrecting long-finished episodes as "unwatched"); the
   lookup and acknowledged-absent GSets and
   the marquee register empty; chat keeps its
   trailing `chat_keep` messages (default 100); playback positions come
   along already coalesced (the view holds one per user); The List is
   untouched — permanent state. Stamps come from the server's Lamport
   clock, so they dominate everything in the old state.
3. Swaps the rebuilt state in and increments the epoch **while holding
   the state lock** (snapshots must never see a torn epoch/state pair).
4. Archives the full pre-compaction chat to SQLite (idempotent insert).
5. Broadcasts `StateSnapshot { new_epoch, state }` to all live
   connections and flushes to storage.

### Client Reconnection After Compaction

1. Client connects; after `AuthOk` it sends `SyncStatus { epoch,
   state_hash }` (the `Auth` message's epoch field is informational/
   log-only since protocol v13)
2. If epoch AND hash match: server sends full CvRDT state, client merges
3. Otherwise (including the post-compaction stale epoch): server sends
   the compacted snapshot + its epoch. Client replaces its local state
   entirely.

---

## Snapshot Storage

### The Client File Split (`dessplay.sync.db`)

Since 2026-08-21 the client keeps the replicated snapshot in its own
SQLite database, a sibling *derived* from the main database path
(`dessplay.db` → `dessplay.sync.db`; `--db` keeps working because the
sibling is derived, not configured). `SyncStorage` (sync_storage.rs)
owns it: one `crdt_state` table, its own append-only migration list,
`save_state`/`load_state`/`clear`. The main `Storage` has no snapshot
API at all any more, so nothing can write the legacy table back.

The point is remediation cost: everything in the sync database is a
replica of server-authoritative state and therefore disposable, while
the main database holds local-only data — most expensively `hash_cache`,
the block hashes of a TB-scale library. Blowing away sync state must
never cost hours of re-hashing (the tsugumi restore incident's operator
remedy, "delete the client DB", did exactly that).

**Remediation story**, in order of preference:

1. `/resync` in a running client (or Settings → "Reset synced state").
2. `dessplay --reset-sync` with the client stopped: takes the instance
   lock (refused while an instance runs — its next flush would undo the
   reset), clears via SQL `DELETE` (no orphaned `-wal`/`-shm`), and
   leaves the main database untouched. Respects `--db`.
3. `rm dessplay.sync.db*` with the client stopped — the documented
   manual fallback; equivalent, just messier.

**One-time move**: `SyncStorage::open` migrates the legacy row out of
the main database, idempotently and crash-safely — copy only when the
sync database has no row (a stale legacy row must never overwrite newer
state), then drop the legacy table only when the copy is known-good or
the table was empty; a crash between copy and drop re-enters via the
skip path and completes the drop. This is deliberately *not* a main-DB
migration entry: migrations run when the main database opens, before the
sync database exists, so a `DROP TABLE` migration would destroy the row
before it could be copied (comment on `storage::MIGRATIONS`). Blobs move
verbatim, envelope and all.

**Locking**: the sync database shares the main database's `<db>.lock`
instance-lock scope — its path is derived from the locked path and every
writer acquires that lock first — so it needs no lock file of its own
(instance_lock.rs). `--dump` runs unlocked beside a live client and is
therefore read-only here: it opens the sync database only if the file
exists (never performing the move); on a fresh install — or a legacy
database no post-split client has run against yet — the dump still
works, with the state-derived sections empty.

The **server** is deliberately not split (user decision): with
convergent connects a server restore needs zero client coordination, and
a single file restores as one unit.

### The Tagged Envelope

The SQLite `crdt_state` blob (client and server) is written as
`SNAPSHOT_MAGIC ++ PROTOCOL_VERSION (u32 LE) ++ postcard(CrdtState)` --
`CrdtState::encode_snapshot`. The magic's first byte is 0xFF, which no
untagged postcard state can begin with (the first field is the playlist
map's vclock length varint; 0xFF there would claim an absurd
continuation-varint size), so the envelope is an unambiguous
discriminator. A blob therefore **names its own layout**; decode never
guesses.

**Wire messages are unaffected.** `StateSnapshot`/`StateMerge` embed the
raw postcard `CrdtState` -- cross-version wire compatibility is owned
entirely by the `Auth` handshake's `PROTOCOL_VERSION` gate, which
refuses mismatched peers before any state crosses. The envelope exists
for storage because *blobs outlive deployments*; connections don't.

### Decoding and Migration

`CrdtState::decode_snapshot[_flagged]`:

- **Tagged, current version**: decode the body as the current layout.
- **Tagged, layout-compatible older version**
  (`LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS`): decode as the current
  layout, flagged (`migrated = true`). This is the explicit arm for
  protocol bumps whose old bodies the current type still decodes —
  v7 → v9 (wire-only changes) and v9 → v10 (an **appended** enum
  variant, `FileAvailability::DownloadingPlayable`, which postcard's
  variant indices tolerate) lived here until v11. Every entry in the
  list is a per-change assertion ("I checked the diff; the current type
  decodes the old body"), not a default — a bump that *does* reshape
  existing state must not be added there, and gets a frozen-layout
  decode arm instead (listed in `FROZEN_LAYOUT_SNAPSHOT_VERSIONS`).
  Caught in the field the same day the envelope landed:
  the server refused to start on its own v7-tagged authoritative
  snapshot after the bump, exactly as designed, and this arm is the
  deliberate migration the policy demands. Since then the
  forgotten-entry failure is a **red build, not a dead deploy**:
  `every_tagged_snapshot_version_is_deliberately_handled`
  (dessplay-core/tests/migration.rs) fails whenever any version in
  `FIRST_TAGGED_SNAPSHOT_VERSION..PROTOCOL_VERSION` is handled by
  neither list, so a `PROTOCOL_VERSION` bump — wire-only or not —
  cannot pass the test suite until someone makes the storage decision.
  Each listed version is also backed by a checked-in **fixture blob** in
  that version's real bytes (dessplay-core/tests/fixtures/): captured
  once at the moment of the bump — when the current encoder still
  reproduces the old bytes, by the very assertion that puts the version
  on the list — and **never regenerated**, so later drift of the current
  type against a listed version's bytes fails
  `layout_compatible_fixture_blobs_decode_to_the_expected_view` instead
  of decoding misaligned (postcard is positional; a wrong compat entry
  can otherwise succeed with silently wrong values). Today the list is
  **[11]**: v7–v10 left it for the frozen arm below when v11 reshaped
  `SeriesRelations`, and v11 → v12 appended only a wire message
  (`SetAnthropicToken`), leaving v11 bodies current-layout. Policy
  details in tests/fixtures/README.md.
- **Tagged, frozen older layout**
  (`FROZEN_LAYOUT_SNAPSHOT_VERSIONS`, today v7–v10): decode through a
  frozen copy of that layout plus an upgrade, flagged. v11 (2026-08-17)
  appended `short_titles` **inside** `SeriesRelations` — postcard is
  positional, so an old body simply ends where the new field would
  begin, and decoding it as the live type would misalign everything
  after it in the map. v7–v10 share one persisted layout (the old
  compat-list assertion), so one frozen struct (`CrdtStateV10`, whose
  `series_relations` holds the frozen `SeriesRelationsV10`) covers all
  four; the upgrade rebuilds that one map value-by-value, carrying each
  entry's LWW timestamp over unchanged (so later writes win or lose
  exactly as against the original) under a **key-derived** migration
  actor (`snapshot_migration_actor(series)`) — deliberately not
  `ActorId::SERVER`, whose per-origin dedup dots must never be forged.
  The key-derived actor is load-bearing: migration is **not** like
  compaction. A compacted state is broadcast with an epoch bump and
  replaces state on every replica, so compaction may rebuild map clocks
  freely; a migrated state is produced *independently on each replica*
  (server and every client decode their own blobs) and the results meet
  in an ordinary same-epoch **merge**, which is dot-based. The first
  version of this rebuild used one constant actor with
  position-derived counters, so replicas holding different key sets
  spent the same dots on different keys, and the reconnect merge
  silently destroyed entries on both sides — convergently, so no
  divergence alarm fired (2026-08-20 audit). With one actor per key
  and counter 1, every replica dots the same key identically and the
  merge degenerates to the per-key LWW union. Pinned by
  `independently_migrated_relations_merge_to_the_union` and the
  property `migration_commutes_with_relations_merge`
  (dessplay-core/tests/migration.rs): migration must be a merge
  homomorphism. Belt-and-braces, the **server also bumps the epoch on
  any migrated load** (`ServerStorage::load_state`), so clients — whose
  own local blobs may have been migrated by an *older* build with the
  unsafe rebuild — adopt the server's migrated snapshot wholesale
  instead of merging into it. The v7–v9 fixture blobs pin this arm now (their
  decode test moved with them); v10's fixture was captured **at the
  v11 bump** — the last moment the current encoder still produced its
  bytes — which is the standing rule for any future frozen version:
  capture the fixture at the bump, or its real bytes are gone.
- **Tagged, any other version**: hard error naming both versions -- the
  refuse-to-start posture. A deliberate migration adds an explicit
  decode arm for that version; nothing is ever guessed from bytes.
- **Untagged**: the one legacy layout, `CrdtStateUntaggedV6` -- the
  pre-envelope protocol-v6 shape every deployed database held when the
  envelope landed (2026-07-25). It decodes flagged (`migrated = true`),
  and the server backs up its whole database before first persisting
  the migrated result, exactly as before. "Frozen" here means the
  **top-level field list** plus, since v11, the one nested type that no
  longer drifts append-only: its `series_relations` holds the frozen
  `SeriesRelationsV10` (pre-envelope bodies predate `short_titles`
  too). The other nested value types are the live ones and have
  already drifted append-only since the envelope landed
  (`anidb_unavailable`, `DownloadingPlayable`) -- which postcard
  tolerates, so real pre-envelope blobs still decode. That property is
  pinned from 2026-08-13 by a checked-in binary fixture
  (tests/fixtures/snapshot-untagged-v6.bin, same written-once policy as
  above). The fallback, its fixture, and the test-support v6 encoder
  are deletable once every deployment (the tsugumi server and every
  client — since the file split its blob lives in `dessplay.sync.db`,
  moved verbatim) has run a post-2026-07-25 build once and thereby
  re-persisted its blob tagged -- verifiable per database by the blob's
  leading 0xFF; the condition and check live on `CrdtStateUntaggedV6`'s
  doc comment.

Any future `CrdtState` shape change is therefore: bump
`PROTOCOL_VERSION` (snapshots cross the wire untagged, so the group and
server upgrade together -- established practice), and the storage side
comes along for free since the envelope stamps the new version. Keeping
the old *tagged* version decodable is a choice made per change, not an
obligation the format imposes.

### History: the trial-decode chain

Before the envelope, blobs were untagged and identified by **trial
decode** through a chain of frozen layout structs
(`CrdtStateV1..V5`/`Protocol5`), grown by one entry per shape change --
including the `SeriesPreference` re-key migration and the
mid-development `V5` layout that reached production (see the Series
Preference section's migration notes). The chain worked, but each entry
was permanent code, discrimination relied on byte-length guards (the
trailing `protocol_version` field exists for exactly that), and a wrong
trial decode would have been silent. The envelope replaced the whole
chain with the single untagged-v6 fallback; the historical migrations
above are kept as documentation of *how* value-shape and key-type
migrations are done when one is needed again.

---

## Failure Modes

### Lost Datagrams

Normal and expected. Operations are also sent on the reliable control stream,
so datagrams are purely an optimization for lower latency. No special recovery
needed.

### Client Crash / Unclean Disconnect

The client persists its CRDT state to SQLite periodically. On restart, it
loads local state, reconnects, and the `SyncStatus` handshake catches it
up: a merge if its replica is identical to the server's, a snapshot
otherwise (see [Reconnection Sync](#reconnection-sync), including the
accepted seconds-wide loss window for ops undelivered at the crash).

### Network Partition During Compaction

If a client is partitioned while the server compacts:

- The client may hold state that predates the new epoch.
- On reconnection, it sees a newer epoch and replaces its state with the
  server's compacted snapshot.
- If the client generated ops *during* the partition that the server never
  saw: these ops are lost. With daily compaction at noon, this requires
  being offline with unsent ops across the noon boundary -- e.g. editing The
  List on a laptop at 11am, closing it, and reconnecting after noon. Rare
  and low-stakes (the edit is simply redone), but it is the one real data-loss
  window in the design. Acceptable risk.

### Server Unavailable

Without the server, nothing works: no state sync, no AniDB lookups, no file
transfer (all transfer is relayed through the server). For short outages,
clients buffer local operations (in memory only — a crash during an outage
drops them, deliberately) and replay them when the server returns.

The server lives on the host's home connection, so a home internet outage
takes the whole party down -- accepted, since the host could not watch
either way, and the others lose nothing but the evening's coordination.

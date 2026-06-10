# DessPlay Implementation Plan

Last updated: 2026-03-04

10 phases, bottom-up. Each phase produces testable artifacts. The first
user-facing demo (TUI with chat + shared playlist) arrives at Phase 6;
full watch-party experience at Phase 7.

## Workspace Layout

```
Cargo.toml                    (workspace root)
dessplay-core/                (shared library: types, CRDTs, protocol, sync)
dessplay/                     (client binary: actors, TUI, player, file mgmt)
dessplay-rendezvous/          (server binary: coordinator, compaction, AniDB)
```

---

## Phase 1: Foundation & CRDTs

**Goal**: Workspace, shared types, CRDT state using the `crdts` crate,
property tests. No networking -- pure logic.

### What gets built
- Cargo workspace with three crates
- Core types: `FileId` (ed2k hash), `UserId`, `ActorId`, timestamps
- `Lww<V>` wrapper for last-writer-wins conflict resolution via `MVReg`
- Wire protocol message types (CrdtOp wrapping native crdts Op types,
  postcard serialization)
- `CrdtState`: combined state container wrapping `crdts` types:
  - Playlist: `Map<Ed2kHash, MVReg<Lww<PlaylistFileState>>>`
  - Now Playing: `MVReg<Lww<Option<Ed2kHash>>>` (standalone)
  - Seek Authority: `MVReg<Lww<ActorId>>` (standalone)
  - Per-user series preference: `Map<(UserId, AniDbSeriesId), MVReg<Lww<SeriesWatchState>>>`
  - Per-user manual override: `Map<UserId, MVReg<Lww<Option<ManualState>>>>`
  - Per-user file availability: `Map<(UserId, Ed2kHash), MVReg<Lww<FileAvailability>>>`
  - AniDB metadata: `Map<Ed2kHash, MVReg<Lww<Option<AniDbMetadata>>>>`
  - Chat: `GList<ChatMessage>`
  - Lookup requests: `GSet<FileHashInfo>`
  - Playback position: `Map<UserId, MVReg<Lww<PlaybackPosition>>>`
- Playlist `Identifier<ActorId>`-based ordering (add, move, rebalance)
- CvRDT merge support (for reconnection sync)
- Snapshot generation and restoration
- ed2k hash computation

### Key crates
`crdts`, `serde`, `postcard`, `ed2k`

### Testing
- proptest: convergence (same ops in any order -> same state) for all CRDTs
- proptest: `Lww<V>` tiebreaking (same-timestamp, value-based resolution)
- proptest: playlist `Identifier` ordering properties
- proptest: CvRDT merge properties (commutative, associative, idempotent)
- Unit tests: individual op application, edge cases
- Fuzz targets: CrdtOp replay (never panics), convergence, CvRDT merge
  round-trip, playlist Identifier ordering

### Milestone
`cargo test` passes with comprehensive CRDT coverage. Types serialize
round-trip correctly.

---

## Phase 2: Storage & Configuration

**Goal**: SQLite persistence, config management.

### What gets built
- SQLite schema + migrations (rusqlite, bundled)
- Persist/restore CRDT snapshots and op logs (keyed by epoch)
- Local config: username, server, media roots, player choice, password
- Watch history: file hash -> watched, last-watched timestamp
- Manual file mappings
- AniDB validation queue
- TOFU certificate fingerprint store

### Key crates
`rusqlite` (bundled), `dirs` (XDG paths)

### Testing
- DB round-trip tests (write state, read back, verify)
- Migration tests (empty DB, upgrade from prior schema)

### Milestone
CRDT state and config survive process restarts.

---

## Phase 3: Network Layer

**Goal**: QUIC transport, server connection, time sync.
No state sync yet -- transport and connection management only.

### What gets built
- `Transport` trait (the testability seam)
- QUIC via quinn: server connection, length-prefixed postcard messages
- `SimulatedTransport`: in-process test transport with configurable loss,
  latency, partitions, reordering, bandwidth limits
- TOFU certificate management
- Client -> Server: auth flow, peer list, time sync
- NTP-style time synchronization (rolling average, outlier rejection)
- NetworkActor skeleton

### Key crates
`quinn`, `rustls`, `rcgen`, `tokio`

### Testing
- SimulatedTransport unit tests
- Time sync accuracy tests (simulated latency)
- Integration: two clients connect via localhost server

### Milestone
Client connects to server, authenticates, receives peer list,
synchronized clocks.

---

## Phase 4: State Sync Engine

**Goal**: CRDTs sync through server. Op broadcast, version vectors, gap fill.

### What gets built
- SyncActor: wraps CrdtState, handles local and remote ops
- Server-side sync: receive ops from clients, broadcast to others
- Eager push via datagrams + reliable send on control stream
- Periodic state summary exchange (1s)
- Gap detection from version vector comparison
- Gap fill over on-demand streams
- Op deduplication
- SQLite persistence integration (periodic flush, not per-op)

### Testing
- SimulatedTransport: N clients with packet loss -> verify convergence
- Partition/heal: ops on both sides of partition -> heal -> converge
- Reconnection: client misses ops -> reconnects -> full state recovery
- Fuzz: multi-actor sync with random partitions

### Milestone
Multiple clients modify CRDTs through server and converge to identical state.

---

## Phase 5: Application Core & Server

**Goal**: Derived state logic, server compaction, actor wiring.

### What gets built
- Main loop: actor creation, channel wiring, `tokio::select!` dispatch
- Derived playback state (play iff all users Ready/NotWatching, file states
  permit)
- User state derivation (series preference + manual override -> derived state)
- Seek authority logic (last seeker is authoritative, server on file change)
- Server compaction (5min after last disconnect -> epoch increment, playlist
  rebalance)
- Epoch handling on client reconnection
- Server ActorId for authoritative actions

### Testing
- Unit tests: derived state for all user/file state combinations
- Compaction round-trip: generate ops -> compact -> reconnect with stale epoch
- Seek authority transitions: user seek, file change, reconnect

### Milestone
Headless client connects and syncs. Server compacts correctly. Derived state
logic works.

---

## Phase 6: TUI

**Goal**: Full terminal interface using tui-realm.

### What gets built
- UiActor with tui-realm Application
- Components:
  - ChatPane (log + input)
  - SeriesPane (dual-mode: Recent Series / All Series)
  - UsersPane (colored ready states)
  - PlaylistPane (current highlighted, missing in red, played in muted)
  - PlayerStatus (progress bar, now-playing)
  - KeybindingBar (derived from focus)
- State -> Props mapping (CrdtSnapshot -> component data)
- Tab cycling: Chat -> Series -> Playlist
- Modal components: FileBrowser, Settings, EpisodeBrowser
- Keybinding bar (auto-derived from active component)
- `--dump` flag for debugging (print CRDT state, config, etc.)

### Key crates
`tui-realm`, `tui-realm-stdlib`, `ratatui`, `crossterm`

### Testing
- insta snapshot tests: render components -> buffer -> assert snapshot
- Message tests: input event -> correct Msg
- Update tests: Msg -> correct UserAction
- Edge cases: empty playlist, no users, long filenames

### Milestone
Interactive TUI client: connect, see peers, chat, manage shared playlist.

---

## Phase 7: Player Integration & Playback Sync

**Goal**: mpv integration, echo suppression, synchronized playback.

### What gets built
- `Player` trait + `MockPlayer` (for tests)
- `MpvPlayer`: JSON IPC over Unix socket
- PlayerActor: manages mpv process, echo filter, position broadcast
- Echo suppression: tag commands, filter echoed events, use mpv's
  user-initiated vs programmatic event distinction
- Play/pause sync (derived from user states)
- Seek authority: user seek -> write SeekAuthority + position, others follow
- Position broadcast (100ms playing, 1s paused)
- Seek-on-receipt when drift > 3s from authority's position
- Content hash verification (ed2k) before unpause
- OSD messages (chat on video)
- Crash handling (relaunch + seek; second crash within 30s -> global pause)
- Server EOF handling -> next file, server becomes seek authority

### Key crates
`serde_json` (mpv JSON IPC)

### Testing
- MockPlayer unit tests: correct commands for state transitions
- Echo suppression integration tests (gated behind `mpv-tests`)
- Seek authority tests with paused tokio time
- Debounce tests

### Milestone
**Full working watch party.** Multiple users, shared playlist, synced
video playback in mpv, chat on OSD.

---

## Phase 8: AniDB Integration

**Goal**: Server-side metadata lookups.

### What gets built
- AniDB UDP API client (client id: "dessplay")
- Login session management
- Rate limiter (4s minimum interval, 5s penalty on throttle)
- SQLite-backed validation queue with file_size
- ed2k hash -> file lookup -> series/season/episode
- Results written as server-authoritative LWW Register ops
- Re-validation schedule (30min <1d, 2h <1w, ...; >=3mo skip; known <=1/week)
- `anidb-probe` binary for manual API testing
- `episode_number` as String (AniDB uses "S1", "C1", etc.)

### Testing
- Rate limiter unit tests (paused tokio time)
- Response parsing tests
- Queue scheduling tests

### Milestone
Playlist files enriched with series/season/episode from AniDB.

---

## Phase 9: File Management & Transfer

**Goal**: Media scanning, file matching, watch tracking, P2P file transfer.

### What gets built
- FileActor: hashing, scanning, matching, download coordination
- Recursive media root scanning
- Automatic file matching (by filename)
- Known series vs unknown series detection
- Manual file mapping (file browser, sorted by edit distance)
- Watch tracking (85% duration = watched)
- Recent Series sorting
- Placeholder PNG for "not watching" state
- File mtime tracking for re-hashing
- P2P file transfer: 256KiB chunks, availability bitfields
- Rarest-first chunk selection (sequential near playback position)
- Hole punching: simultaneous QUIC opens, exponential backoff, relay fallback
- Server-side file transfer relay
- Max 4 concurrent streams, 16 chunks pipeline depth
- Temp storage -> download root after 50% watched
- Download progress in FileAvailability CRDT

### Key crates
`image` (PNG generation), `strsim` (edit distance)

### Testing
- File matching logic, series detection, sorting
- SimulatedTransport: file transfer integrity, rarest-first distribution
- Hole punch / relay fallback
- Bandwidth throttling

### Milestone
Missing files detected and shown in red. Manual mapping works. Files
downloaded from peers automatically. Works across firewalls.

---

## Phase 10: Hardening & Polish

**Goal**: Production readiness.

### What gets built
- Reconnection handling (all scenarios from network-design.md)
- Graceful shutdown (clean actor teardown)
- Error handling (no panics -- enforced by clippy lints)
- Full fuzz target suite
- System tests: tmux end-to-end
- Logging/tracing throughout
- `/exit`, `/quit`, `/q` commands
- VLC support (v2 scope decision)

### Testing
- Fuzz: >=10min per target
- System tests: full workflow
- Chaos testing: SimulatedTransport with high loss, partitions, reordering

### Milestone
Stable, production-ready. All documented failure modes handled.

# DessPlay Implementation Plan

Last updated: 2026-06-10

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

**Status: complete (2026-06-10).** Notable deviations from the original
sketch, all documented in sync-state.md: playlist removal is an LWW
tombstone (`Option<PlaylistFileState>`, `None` = removed) because
`crdts`' `Map::rm` proved non-convergent under concurrent re-add;
convergence is defined and tested at the resolved-view level through a
hub-and-spoke cluster model rather than naive op shuffling; ed2k uses the
eMule/AniDB ("red") variant with per-block MD4 hashes computed via `md4`
(the `ed2k` crate cross-checks the root in tests).

**Goal**: Workspace, shared types, CRDT state using the `crdts` crate,
property tests. No networking -- pure logic.

### What gets built
- Cargo workspace with three crates
- Core types: `FileId` (ed2k hash), `UserId`, `ActorId`, timestamps
- `Lww<V>` wrapper for last-writer-wins conflict resolution via `MVReg`
- Wire protocol message types (CrdtOp wrapping native crdts Op types,
  postcard serialization)
- `CrdtState`: combined state container wrapping `crdts` types:
  - Playlist: `Map<Ed2kHash, MVReg<Lww<PlaylistFileState>>>` (incl. size, duration)
  - Watched flags: `Map<Ed2kHash, MVReg<Lww<bool>>>` (server-only writes)
  - Now Playing: `MVReg<Lww<Option<Ed2kHash>>>` (standalone)
  - Seek Authority: `MVReg<Lww<ActorId>>` (standalone)
  - Per-user series preference: `Map<(UserId, AniDbSeriesId), MVReg<Lww<SeriesWatchState>>>`
  - Per-user manual override: `Map<UserId, MVReg<Lww<Option<ManualState>>>>`
    (`Paused | Away { set_by }`)
  - Per-user file availability: `Map<(UserId, Ed2kHash), MVReg<Lww<FileAvailability>>>`
  - AniDB metadata: `Map<Ed2kHash, MVReg<Lww<Option<AniDbMetadata>>>>`
  - Series relations: `Map<AniDbSeriesId, MVReg<Lww<SeriesRelations>>>`
  - The List: `Map<ListEntryId, MVReg<Lww<SeriesListEntry>>>`
    + `Map<ListEntryId, MVReg<Lww<NextEpState>>>`
  - Chat: `GList<ChatMessage>`
  - Lookup requests: `GSet<FileHashInfo>`
  - Playback position: `Map<UserId, MVReg<Lww<PlaybackPosition>>>`
- Playlist `Identifier<ActorId>`-based ordering (add, move, rebalance)
- CvRDT merge support (for reconnection sync)
- Snapshot generation and restoration
- ed2k hash computation (root + per-block hashes, kept for transfer
  verification)

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

**Status: complete (2026-06-10).** Storage lives in each binary per
architecture.md (`dessplay/src/storage.rs` + `config.rs`,
`dessplay-rendezvous/src/storage.rs`); the schema is documented in
design.md (Data Storage). TOFU pins are write-once (cert replacement
requires an explicit forget). The server's chat archive and AniDB queue
tables exist now; their consumers arrive in Phases 5 and 8.

**Goal**: SQLite persistence, config management.

### What gets built
- SQLite schema + migrations (rusqlite, bundled)
- Persist/restore CRDT snapshots, keyed by epoch (no op log: unsent ops
  are memory-only by design; a crash may lose the latest local edits)
- Local config in SQLite: username, server, media roots, player choice,
  password (plaintext, per threat model), cache retention, upload limit,
  subtitle pane. Flags/env override, never persisted. Seeder & server
  take flags/env only and persist no settings.
- Watch history: file hash -> watched, last-watched timestamp (keyed by
  hash/series so it survives cache eviction)
- Download cache state (last-access times for eviction)
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
- QUIC via quinn: server connection, length-prefixed postcard messages,
  10s keep-alives, datagram size rule (oversized ops skip eager-push)
- Stream priorities (control stream above transfer streams) and flow-control
  window sizing for bulk transfer
- `SimulatedTransport`: in-process test transport with configurable loss,
  latency, partitions, reordering, bandwidth limits
- TOFU certificate management
- Client -> Server: auth flow (username, password, role, epoch), peer list
  (role + presence), time sync
- NTP-style time synchronization over datagrams (rolling average, outlier
  rejection)
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
- Playback position exception: datagram-only at 100ms, reliable tick at 1s
- Periodic state summary exchange (1s)
- Gap detection from version vector comparison
- Gap fill over on-demand streams
- Op deduplication
- SQLite persistence integration (periodic flush, not per-op)

### Testing
- SimulatedTransport: N clients with packet loss -> verify convergence
  (the sync-only embryo of the multi-client harness)
- Partition/heal: ops on both sides of partition -> heal -> converge
- Reconnection: client misses ops -> reconnects -> full state recovery
- Fuzz: multi-actor sync with random partitions

### Milestone
Multiple clients modify CRDTs through server and converge to identical state.

---

## Phase 5: Application Core & Server

**Goal**: Derived state logic, server compaction, actor wiring.

### What gets built
- Main loop: actor creation, channel wiring, `tokio::select!` dispatch —
  factored as the **composition root** (`main()` is a thin shell; see
  architecture.md), so the multi-client harness can construct full clients
- Multi-client simulation harness, headless form (N full clients + server
  in-process; see testing-strategy.md)
- Seeder mode: `--seeder` spawns only Sync/Network/File actors
- Presence tracking on the server (Present -> Lost at 30s -> Departed at
  60s), `PeerList` pushes, pause-on-lost and pause-on-graceful-quit
- Derived playback state (play iff all *present* users Ready/Away/NotWatching,
  file states permit; seeders excluded)
- User state derivation (series preference + manual override -> derived
  state), including Away (set by others, cleared by owner activity)
- Seek authority logic (last seeker is authoritative; server on file change
  and on authority departure)
- EOF transition: `EofReached` report handling, watched flag, now-playing
  advance, idempotency
- Server compaction: scheduled daily (configurable time) -> pause ops,
  rebalance playlist, trim+archive chat, clear GSet, epoch increment,
  snapshot broadcast to connected clients
- Epoch handling on client reconnection
- Server ActorId for authoritative actions

### Testing
- Unit tests: derived state for all presence/user/file state combinations
- Presence transitions with paused tokio time (lost -> pause; departed ->
  unblock gating but stay paused)
- Compaction round-trip: generate ops -> compact -> reconnect with stale
  epoch; compact with clients connected -> snapshot broadcast adopted
- Seek authority transitions: user seek, file change, authority departure,
  reconnect
- EOF idempotency: duplicate reports are no-ops

### Milestone
Headless client connects and syncs. Seeder mode runs. Server compacts on
schedule with clients attached. Presence-aware derived state works.

---

## Phase 6: TUI

**Goal**: Full terminal interface using tui-realm.

### What gets built
- UiActor with tui-realm Application
- Components:
  - ChatPane (log + input; `/afk` command)
  - SubtitlePane (rolling log component; fed with real data in Phase 7)
  - SeriesPane (three modes: Recent Series / All Series / The List)
  - UsersPane (colored ready states incl. Away; departed + seeder lines;
    focusable, `a` = mark Away)
  - PlaylistPane (current highlighted, missing in red, watched in muted;
    `d` remove, `A` archive, `Ctrl-m` manual map)
  - PlayerStatus (progress bar, now-playing)
  - KeybindingBar (derived from focus)
- State -> Props mapping (CrdtSnapshot + presence -> component data)
- Tab cycling: Chat -> Series -> Users -> Playlist
- Modal components: FileBrowser, Settings, EpisodeBrowser, ListEntryEdit
  (AniDbSearch modal lands in Phase 8 with its server support)
- The List: entry display, editing, status grouping
- `dessplay import-list` CSV importer (spreadsheet port, watcher-initial
  mapping)
- Keybinding bar (auto-derived from active component)
- `--dump` flag for debugging (print CRDT state, config, etc.)

### Key crates
`tui-realm`, `tui-realm-stdlib`, `ratatui`, `crossterm`

### Testing
- insta snapshot tests: render components -> buffer -> assert snapshot
  (layout only)
- Message tests: input event -> correct Msg
- Update tests: Msg -> correct UserAction
- Whole-app TUI tests: scripted event sequences through the real
  Application on TestBackend, locator-style assertions
- Multi-client harness gains UI handles (inject input, query rendered
  buffers per client)
- Importer tests against the real exported sheets as fixtures
- Edge cases: empty playlist, no users, long filenames, unlinked List entries

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
- Play/pause sync (derived from presence + user states)
- Seek authority: user seek -> write SeekAuthority + position, others follow
- Position broadcast (100ms playing, 1s paused)
- Drift bands: ignore < 100ms, slew (±2% mpv `speed`) to 3s, hard seek above
- Content hash verification (ed2k) before unpause
- OSD messages (chat on video)
- Subtitle feed: observe mpv `sub-text`, emit `SubtitleLine` to the
  SubtitlePane
- Crash handling (relaunch + seek; second crash within 30s -> global pause)
- EOF reporting (`EofReached` to server) -> next file, server becomes seek
  authority

### Key crates
`serde_json` (mpv JSON IPC)

### Testing
- MockPlayer unit tests: correct commands for state transitions
- Multi-client harness gains player handles (full scenario tests: pause
  on A reaches B's and C's players)
- Echo suppression integration tests (gated behind `mpv-tests`)
- Drift band tests (boundary values; slew converges, releases speed to 1.0)
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
- Relations graph: ANIME lookups + recursive relation walks per new series
  ID, cached in server SQLite, replicated as `SeriesRelations`
- ANIME name search (backs the AniDbSearch modal for List entry linking)
- List integration: next_ep auto-advance on EOF for linked entries
- Results written as server-authoritative LWW Register ops
- Re-validation schedule (30min <1d, 2h <1w, ...; >=3mo skip; known <=1/week)
- `anidb-probe` binary for manual API testing
- `episode_number` as String (AniDB uses "S1", "C1", etc.)

### Testing
- Rate limiter unit tests (paused tokio time)
- Response parsing tests
- Queue scheduling tests

### Milestone
Playlist files enriched with series/season/episode from AniDB. Franchise
grouping works. List entries linkable; next_ep advances automatically.

---

## Phase 9: File Management & Transfer

**Goal**: Media scanning, file matching, watch tracking, relayed file
transfer, download cache.

### What gets built
- FileActor: hashing, scanning, matching, download coordination, cache
  management
- Recursive media root scanning
- Automatic file matching (by filename)
- Known series vs unknown series detection (AniDB series ID against watch
  history; name-parse fallback pre-metadata)
- Manual file mapping (file browser, sorted by edit distance)
- Watch tracking (85% duration = watched; personal vs group levels)
- Recent Series sorting
- Placeholder PNG for "not watching" state
- File mtime tracking for re-hashing
- Relayed file transfer: 256KiB chunks, availability bitfields, relay
  envelopes over the server connection (no client-to-client connections)
- ed2k block-hash verification (per-block validation, bad-block re-fetch,
  resume-after-restart from on-disk chunks)
- Rarest-first chunk selection (sequential near playback position)
- Max 4 concurrent streams, 16 chunks pipeline depth
- Upload rate cap (`upload_limit`)
- Download cache: retention policy (0/duration/infinite), eviction passes
  (startup + EOF-advance), archive action
- Prefetch: queued entries ahead of now-playing; seeder auto-fetch of
  everything
- Download progress in FileAvailability CRDT

### Key crates
`image` (PNG generation), `strsim` (edit distance)

### Testing
- File matching logic, series detection, sorting
- Eviction policy (retention boundaries; never evicts now-playing/queued)
- SimulatedTransport: relayed transfer integrity, rarest-first distribution,
  block verification and resume
- Bandwidth throttling

### Milestone
Missing files detected and shown in red. Manual mapping works. Files
downloaded through the relay automatically; the seeder fetches everything;
the laptop's cache cleans up after itself.

---

## Phase 10: Hardening & Polish

**Goal**: Production readiness.

### What gets built
- Reconnection handling (all scenarios from network-design.md)
- Graceful shutdown (clean actor teardown)
- Error handling (no panics -- enforced by clippy lints)
- Full fuzz target suite
- System tests: tmux end-to-end (including a seeder instance)
- Logging/tracing throughout
- `/exit`, `/quit`, `/q`, `/afk` commands
- NixOS deployment for the NAS (rendezvous + seeder services)
- VLC support (open scope decision: ship in v2 or defer)

### Testing
- Fuzz: >=10min per target
- System tests: full workflow
- Chaos testing: SimulatedTransport with high loss, partitions, reordering

### Milestone
Stable, production-ready. All documented failure modes handled.

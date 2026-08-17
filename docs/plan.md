# DessPlay Implementation Plan

Last updated: 2026-08-17

The initial 10 phases are bottom-up; later numbered phases capture feature
batches. Each phase produces testable artifacts. The first
user-facing demo (TUI with chat + shared playlist) arrives at Phase 6;
full watch-party experience at Phase 7.

Phases 11–18 are the **feature-request batch** (triaged 2026-07-02 from
the group's request sheet; request numbers `#N` refer to its rows).
Ordering is dependency-driven: the protocol version gate lands first so
later wire/schema changes are clean flag-days.

Phases 28–30 record **unplanned work** after the fact (2026-07/08 landed
without plan entries). Phases 31–33 are the **2026-08-17 usage-triage
batch** — see that triage section for what was closed or dropped.

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

**Status: complete (2026-06-10).** Notes: the transport seam lives in
`dessplay-core::net` (traits + framing + time sync + TOFU + quinn +
sim); both binaries became lib-with-thin-main so cross-crate tests run
real clients against the real server in-process (the composition-root
requirement, cashed in early). Duplicate-username auth supersedes the
old connection (documented in network-design.md). Sim reordering is
modeled as per-datagram jitter rather than a shuffle window
(testing-strategy.md updated).

Also in this phase: the property suite caught a second crdts
view-divergence (`Map::merge` corrupting nested-register clocks; pinned
in `dessplay-core/tests/regressions.rs`), and registers were rewritten
from `MVReg<Lww<V>>` to our own max-merge `LwwCell<V>` — see
sync-state.md. Consequence for Phase 4: op generation must issue
monotonic timestamps (`max(shared_now, last_issued + 1)`).

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

**Status: complete (2026-06-10).** Design changes, all documented in
sync-state.md: ActorIds are session-scoped (prevents double-spent dots
after crash-restore; compaction must rebuild state from the view --
Phase 5); SeekAuthority became `Server | User(UserId)`; the reconnect
handshake gained an **upward client->server StateMerge** after chaos
testing proved per-op replay loses ops that died in flight with the
old connection. The divergence alarm and FIFO datagram guard are in.

**Goal**: CRDTs sync through the server: op broadcast, eager datagram delivery,
and merge-based reconnection/recovery.

### What gets built
- SyncActor: wraps CrdtState, handles local and remote ops, issues
  monotonic timestamps (`max(shared_now, last_issued + 1)`)
- Server-side sync: receive ops from clients, broadcast to others
- Eager push via datagrams + reliable send on control stream; receivers
  apply a datagram op only if it is order-safe (per-origin FIFO guard:
  Map ops with out-of-sequence dots are dropped — the reliable copy
  arrives anyway)
- Playback position exception: datagram-only at 100ms, reliable tick at 1s
- Op deduplication (Map dots; LwwCell/GList/GSet are naturally idempotent)
- Divergence alarm: server broadcasts a periodic (30s) hash of its
  resolved view (excluding playback positions); a client mismatching
  twice in a row logs loudly and requests a StateMerge to self-heal
- Reconnection sync: epoch check -> StateMerge (same epoch) or
  StateSnapshot (stale epoch), per sync-state.md. No version vectors,
  no gap-fill protocol: mid-connection gaps are impossible (every op
  also travels the reliable ordered control stream)
- Offline buffering: ops queued in memory while disconnected (playback
  positions coalesced to latest), replayed on reconnect
- SQLite persistence integration (periodic flush ~30s + on shutdown,
  not per-op)

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

**Status: complete (2026-06-10).** Design changes, documented in
design.md / sync-state.md / network-design.md: the synced
`playback_intent` register (`Playing | Paused`) — gating alone cannot
express "stays paused after the blocker departs"; EOF-advance loads
the next episode paused. `StateOp` became epoch-tagged (an op crossing
a compaction boundary would pollute the rebuilt state's dot
sequences). A `Goodbye` message implements graceful quit. Timestamps
were upgraded from self-monotonic to **Lamport-monotonic** (bumped by
every observed remote stamp) after the EOF tests caught the server's
forced Paused losing a same-millisecond tiebreak to a client's
Playing. The compaction rebuild lives in `dessplay-core::compact` as a
pure, property-tested function; compaction time is UTC, not
server-local. The headless harness is
`dessplay-rendezvous/tests/common/mod.rs`.

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

**Status: complete (2026-06-10).** Design deviation, documented in
ui-architecture.md: we use tui-realm 4.1's component model, stdlib Input,
and test helpers, but replaced its threaded `Application` event loop
with a synchronous dispatcher (`ui::app::Ui`) so whole-app tests are
deterministic and thread-free; production wraps it in two plain threads
(`ui::shell`). The importer is calibrated against the real exported
sheets committed in `spreadsheet/` (249 entries, 6 flagged oddities) and
re-imports update entries by name instead of duplicating. Found and
fixed by the import test: a sync-actor deadlock when >256 events queued
against an undrained UI channel — StateChanged is now a lossy
edge-triggered signal. AniDbSearch modal deferred to Phase 8 (needs the
server side); playlist `A`/`M` bindings to Phase 9 (need files);
Ctrl-arrow word-movement in chat to polish. The interactive client is
now the binary's default mode (`--headless` opts out).

**Goal**: Full terminal interface using tui-realm.

### What gets built
- Synchronous `ui::app::Ui` dispatcher plus production terminal/input shell
- Components:
  - ChatPane (log + input; `/afk` command)
  - SubtitlePane (rolling log component; fed with real data in Phase 7)
  - SeriesPane (three modes: Recent Series / All Series / The List)
  - UsersPane (colored ready states incl. Away; departed + seeder lines;
    focusable, `a` = mark Away)
  - PlaylistPane (current highlighted, missing in red, watched in muted;
    `d` remove, `A` archive, `M` manual map)
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

**Status: complete (2026-06-11).** Notes and deviations:

- **Minimal file matcher pulled forward from Phase 9** (user-approved):
  exact-filename scan of media roots + ed2k verification, writing
  `FileAvailability` Ready/Missing (`dessplay/src/matcher.rs`). Without
  it only the adder could play anything. Phase 9 absorbs it into the
  FileActor (adding mtime tracking, a hash cache so unwatched playlist
  entries aren't re-hashed every session, manual-mapping UI, downloads).
- **Session policy layer** (`dessplay/src/session.rs`): the
  state↔player translation is a synchronous `PlayerWiring` (same
  philosophy as `ui::app::Ui`) plus an async `SessionShell` shared by
  `run_interactive` and the harness's player clients.
- Echo suppression is expected-state tracking (a queue of commanded
  pause flips, a counter of commanded seeks) — architecture.md's claim
  that mpv distinguishes user from programmatic events was wrong and
  has been corrected.
- One mpv instance persists per session (`--idle --keep-open`,
  spawn-on-first-load, `loadfile` to swap). The real-mpv test caught a
  genuine ordering bug: mpv's keep-open pause arrives *before*
  `eof-reached`, so the IPC layer holds a `pause=true` briefly to
  attribute it (user vs mechanics).
- A crashing player is always relaunched (paused, at the old position);
  the second crash within 30s *additionally* pauses globally + notifies
  chat, per design.
- Real-mpv coverage is one end-to-end journey behind `--features
  mpv-tests`; the test video is encoded by mpv itself from a lavfi
  source (no committed media, no ffmpeg dependency).
- Player harness scenarios touch the real filesystem (tempdir roots,
  blocking-pool matcher), so they are eventually-style rather than
  perfectly deterministic.
- **Post-milestone bug (2026-06-12), user-reported**: playlist-add
  hashing ran inline in the bridge loop's select arm, starving the loop
  for the duration of every multi-GB hash — frozen playlist UI,
  serialized adds, and a queued Ctrl-C Quit that was never read. Fixed
  by extracting the loop into a testable `run::SessionLoop` (liveness
  rule documented in architecture.md) and moving hashing into
  `SessionShell` background tasks with a completion channel. The
  supervision regression tests
  (`dessplay-rendezvous/tests/interactive_loop.rs`) hang a hash on a
  FIFO and assert quits and other adds still land. Follow-ups from the
  same report: debug-build MD4 hashed at ~70 MiB/s (20s per episode) —
  fixed with `[profile.dev.package.*] opt-level = 3` for the hash
  crates (now ~1.2 GiB/s, same as release; per-block parallelization
  considered and rejected — multiple streams behave badly on HDDs);
  and a new design rule (design.md, UI Principles): long-running work
  is never silent — hashing shows a non-input-capturing progress
  overlay, asserted end-to-end in the loop tests.

**Goal**: mpv integration, echo suppression, synchronized playback.

### What gets built
- `Player` trait + `MockPlayer` (for tests)
- `MpvPlayer`: JSON IPC over Unix socket
- PlayerActor: manages mpv process, echo filter, position broadcast
- Echo suppression: track expected command effects and filter matching mpv
  observations on our side (mpv does not identify event origin)
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

**Status: complete (2026-06-12).** Notes and deviations:

- **Name search uses the anime-titles dump, not the UDP API**
  (user-approved): `ANIME aname=` is an exact-title lookup with no
  candidate list, so the server fetches `anime-titles.dat.gz` daily
  (ureq + flate2, one blocking GET) into SQLite and searches locally —
  informal names resolve through synonyms. New wire messages
  `AniDbSearch`/`AniDbSearchResults`; results are not CRDT state.
- The fmask/amask bit tables were cross-checked against two independent
  client implementations (adbb, anidbcli) because the official wiki sits
  behind an interactive challenge; the mask constants have tests
  rebuilding them from named bit positions. Residual risk (accepted up
  front): a field-order or escaping subtlety may only surface on the
  first manual `anidb-probe` run.
- Lookup requests are issued by **any** client for playlist entries
  lacking metadata (hash/size/filename all live in the entry), not just
  the adder — covers offline adders and the GSet being cleared at
  compaction.
- The EOF List auto-advance also resets `available` to false (the new
  next episode is presumably not out yet) — design.md updated.
- The List's watchers→NotWatching wiring landed here with two guards:
  empty watchers sets mean "unrecorded" and write nothing, and existing
  preferences are never overridden.
- Queue tombstones: settled entries keep their row with
  `next_attempt = i64::MAX` instead of being deleted, so re-discovery
  (GSet re-inserts, relation re-walks) stays a no-op. A second queue
  table (`anime_queue`) persists the relations walk across restarts.
- Credentials via `DESSPLAY_ANIDB_USER`/`DESSPLAY_ANIDB_PASSWORD` (env
  fits the systemd deployment); both-or-neither enforced at startup.
  Plaintext-UDP AUTH accepted (account used for nothing else); ENCRYPT
  is future work.
- Testing is strictly offline (see testing-strategy.md, AniDB Tests):
  scripted-wire client tests under paused time, an in-memory host for
  the worker, canned-API integration scenarios over the sim transport.
  `anidb-probe` (ping/file/anime/scan) is the only real-API contact.
- **Record/replay fixtures** (user-proposed): `anidb-probe scan <dir>`
  hashes a directory, looks everything up through the recording wire
  (credentials/session keys redacted at write time), and stores the
  exchanges in `dessplay-rendezvous/testdata/anidb/`; the replay test
  re-parses them with the real codec forever after. This closes the
  "parser verified only against the spec" gap.

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

Split into two halves: **9A — local file management** (file actor,
matching, watch tracking, manual mapping, placeholder, cache
eviction/archive) and **9B — relayed transfer** (chunks, bitfields,
block verification/resume, rarest-first, prefetch, seeder auto-fetch).

**9A status: complete (2026-06-13).** Notes and deviations:

- **`matcher` absorbed into a `FileActor`** (`dessplay/src/actors/file.rs`),
  driven by the `SessionShell` over one `FileCommand`/`FileOutput`
  channel pair. Resolution is hash-cache-aware (client schema v2
  `hash_cache`, keyed by `(mtime, size)`): unwatched playlist entries no
  longer re-hash every session, and a touched file re-hashes exactly
  once. The two old resolution/hash channels collapsed into
  `file_outputs`.
- **85% watch tracking** in `PlayerWiring` (needs a known duration; the
  EOF report still marks group progress separately). Recent Series
  sorting was already reading `recent_watched`; populating watch history
  completes it.
- **Manual mapping** (`M`): a `FileBrowser` mapping mode that ranks
  files by `strsim` edit distance to the target. Opens at the media
  roots, not the series' last-used directory (that dir is file-actor
  state not yet in the UI snapshot) — noted in design.md. **Archive**
  (`A`) → `<download root>/<series>/<filename>`; the design's `Season #`
  level is collapsed (AniDB models each season as its own anime).
- **Placeholder PNG** via `image` + `ab_glyph` + an embedded DejaVu Sans
  (`dessplay/assets/`, license included).
- **Missing-file branch**: a known series stays Missing (blocks); an
  unknown series with an AniDB **series id** auto-marks NotWatching +
  shows the placeholder. **User-approved design decision:** a missing
  file whose series has no id keeps blocking, with the manual
  not-watching action as the escape hatch — no new CRDT state.
- **Deferred to 9B / later:** manual not-watching keybinding, the
  per-series mapping start directory, and everything transfer-related.

**9B status: complete (2026-06-13).** Relayed file transfer:

- **9B-1 relay**: `PeerMessage`/`RelayEnvelope`/`Bitfield` wire types; one
  dedicated relay QUIC stream per peer (separate from control, so bulk
  transfer doesn't head-of-line-block state sync — QUIC isolates
  streams); server forwards by username; client surfaces `NetworkEvent::Peer`.
- **9B-2 chunk store**: single-file assembly, ed2k per-block
  verification, sidecar-free resume. Chunks are **256,000 B (250 KiB) =
  block / 38**, aligned to ed2k blocks (no straddling) — the chunk size
  is ours, the block size fixed by the root hash.
- **9B-3 scheduling + serving**: `Downloads` coordinator (pipeline depth
  `--pipeline-depth` flag default 16 × ≤4 sources, sequential window +
  rarest-first, **source snub instead of per-chunk timeout**, endgame +
  `Cancel`); FileActor serves chunks/block-hashes from local copies
  within an upload-rate token bucket; wired into the live session
  (missing now-playing file → download). End-to-end tests report
  **100% goodput / 0% retransmit**.
- **9B-4 prefetch + seeder auto-fetch**: interactive clients fetch a
  lookahead window of queued entries ahead of now-playing; a seeder
  (`SeederTransfer`, headless) fetches and serves the *whole* playlist,
  persisting its hash cache (no re-hash on restart); prior downloads are
  re-discovered by the hash-addressed download-cache reconciliation every
  client runs at startup (not via a media-root scan).
- **Future**: disk/retention-aware prefetch depth, seek-aware download
  window, rarest-aware upload prioritization, choking for many-peer
  scale. *(2026-08-17: superseded — the window/prioritization items
  landed as Phase 30; the rest was closed in the 2026-08-17 triage.)*

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

Status: Completed as side-effects of phase 1-19. Insta replaced tmux tests. VLC defined as unnecessary.

---

## Phase 11: Protocol Version Gate (#23)

**Status: complete (2026-07-02).** Notes and deviations:

- `NetworkEvent::AuthFailed` was renamed to `Rejected { message }` (and
  `SessionEnd` follows): a protocol mismatch is not an auth failure, and
  the variant now carries the human-readable refusal to the terminal
  verbatim — including the "please update dessplay" text.
- `NetworkConfig` gained a `protocol_version` field (defaulting to
  `PROTOCOL_VERSION`) so the sim test drives the *real* client actor
  into the *real* server's refusal with a mismatched version.
- An undecodable first control frame (the pre-versioning signature) is
  now answered with `AuthFailed` before closing, where it previously got
  a silent close; a decodable-but-wrong first message still closes
  silently as a protocol violation.
- Policy documented in network-design.md (Protocol Versioning): version
  before password; append, never reorder; never reshape `Auth`.

**Goal**: Refuse mismatched clients with a clear message, so every later
wire/schema change is a clean flag-day instead of silent decode garbage.

### What gets built
- `PROTOCOL_VERSION: u32` constant in `dessplay-core::net`, carried in
  `Auth`. Adding the field is itself the one-time break: a pre-versioning
  client's `Auth` fails to decode on the new server.
- Server: on version mismatch (or `Auth` decode failure, which *is* a
  pre-versioning client), reply and close. Mismatching **new** clients get
  a new `ProtocolMismatch { server_version }` variant (appended to
  `ServerControl`, so enum indices stay stable) and print "please update";
  undecodable-old clients get `AuthFailed`, which their binary can still
  decode — a generic refusal beats a hang.
- Client: on `ProtocolMismatch`, exit with a clear message (no reconnect
  loop).
- Policy note in network-design.md: bump the constant on any change to
  wire messages, `CrdtOp`, or CRDT value types; append enum variants,
  never reorder.

### Testing
- Sim-transport integration: matching version connects; client with
  version±1 is refused with `ProtocolMismatch` and does not retry.
- A hand-encoded pre-versioning `Auth` frame gets `AuthFailed` + close.

### Milestone
An old binary pointed at the new server fails fast with a human-readable
reason. Later phases bump the version freely.

---

## Phase 12: State Wording & Narrator Polish (#17, #29, #27, #2, #18; closes #12)

**Status: complete (2026-07-02).** Notes and deviations:

- The pause-word rule is decided by **derived playback**: `NarratorState`
  now captures `playback_active` per snapshot; a pause narrates "X is not
  ready" unless video was actually playing, and an override-clear
  narrates "X is ready" unless playback actually starts (the last
  blocker's clear gets the "unpaused").
- The seek line's from-position is the previous sample **extrapolated**
  to the seek moment (equal to the raw sample when paused).
- `Tone::Paused` (yellow) covers only the plain manually-paused row;
  downloading-while-paused stays red per the Ready States table.
- The `/me` grey render keeps the sender's palette colour and mention
  highlighting (a `base` style parameter on the mention highlighter).
- "Offline" is a label-only rename; `Presence::Departed` and the
  `UsersProps::departed` field keep their names.
- #12 closed by `excused_users_never_block` in `dessplay-core::derive`
  tests: Away/NotWatching/seeder users never block, property-tested over
  presence × override × preference × availability.

**Goal**: Stop conflating "paused the video" with "not ready to watch",
in both the narrator's language and the Users pane's colors.

### What gets built
- **#17**: narrator wording — a manual-override clear *without* an intent
  change reads "Nero is ready"; "paused"/"unpaused" are reserved for
  transitions that actually stop/start video (intent + gating outcome).
  The no-cascade rule still holds: one line per user action.
- **#18**: Paused becomes its own Users-pane display state (yellow),
  distinct from red blockers (Missing file, committed-absent);
  attribution is kept — the pane still shows *who* paused. Ready States
  table in design.md updated.
- **#29**: "Departed" renders as **"Offline"** (Users pane dim line +
  narrator "left" lines untouched). Display-level rename only; the
  internal `Presence::Departed` name stays.
- **#27**: `/me` action lines render grey/dim in the chat log (and OSD).
- **#2**: seek narrator lines carry from→to: "Baughn skipped 08:12 →
  12:34". Phase 21 replaces the original position-diff inference, closing
  the first-seek attribution gap without narrating automatic seeks.
- **#12 closure**: a property test over the gating derivation — for all
  presence × file-state combinations, an Away or NotWatching user never
  blocks playback. Believed already true; the test pins it and the
  request closes.

### Testing
- Narrator: snapshot-diff unit tests (feed successive state views,
  assert exact lines) for each new/changed wording.
- UI: insta snapshots for the yellow Paused row and Offline line.
- The #12 property test (derive-level, proptest over state combinations).

### Milestone
"Nero ready" vs "Nero paused" mean what they say; a paused friend no
longer looks like a missing-file blocker.

---

## Phase 13: Player OSD Rework (#16, #30, #14)

**Status: complete (2026-07-03).** Notes and deviations:

- `Player::show_osd` was **replaced** by `set_osd_overlay(id, data)` —
  nothing else used the timed `show-text`, and the single-slot model was
  the bug. Two slots: chat log (id 1, top-left) and blocker summary
  (id 2, top-right); minimal ASS styling (`{\an7/\an9\fs26}`), braces
  and backslashes sanitized. Verified against real mpv (`mpv-tests`).
- The chat OSD has **no message-count display budget** — pure
  per-message 8s retention (the request was "minimum retention time");
  an 8-line cap guards pathological bursts only. Buffer + expiry live in
  the PlayerActor, which re-applies non-empty overlays after every
  relaunch/re-attach (fresh mpv slots are clean, so empty ones are
  skipped — this also keeps relaunch's first command the reload).
- The session's blocker producer keys its dedup on `(loaded file,
  text)`: a Load in the same directive batch spawns the player, and
  overlay commands sent before it land on nothing.
- The blocker text reuses `derive::playback_blockers` (which already
  existed) — no derivation moved out of `ui/props.rs`; the props code
  only shares the underlying derive, exactly as before.
- The design sentence "how many users are connected" on the OSD was
  dropped in favor of the Waiting-for line (the count lives in the TUI).

**Goal**: The video OSD becomes trustworthy: chat lines don't vanish
mid-read, your own lines don't echo, and the design.md blocker summary
(never implemented — confirmed 2026-07-02) finally exists as a
persistent "Waiting for X, Y, Z" display.

### What gets built
- **`osd-overlay` support** in the `Player` trait: `set_overlay(id,
  text)` / `clear_overlay(id)` alongside `show_osd` (mpv `osd-overlay`
  command; MockPlayer records overlay state). Two overlay slots: chat
  and blocker-summary — independent of each other and of `show-text`.
- **#16**: chat OSD becomes a rolling buffer rendered into the chat
  overlay — last N messages (default 4), each retained a minimum time
  (default 8s) and expiring individually; a burst never erases an unread
  line. Buffer + expiry timer live in the PlayerActor (it owns player
  state); the session keeps sending one message per new chat line.
- **#30**: the session's chat→OSD site (`session.rs`) skips messages
  whose sender is the local user.
- **#14**: a blocker-summary producer in the session: on every state
  change, derive who currently blocks playback (reusing/lifting the
  block-reason derivation that today lives in `ui/props.rs` — it moves
  to a shared derive site so TUI and OSD cannot disagree) and set/clear
  the overlay: "Waiting for Kim (downloading 34%), Nero (paused)".
  Shown to everyone, including non-blockers; cleared when playing.

### Testing
- PlayerActor: paused-time unit tests for buffer retention/expiry and
  overlay updates (MockPlayer assertions).
- Harness: A pauses → B's and C's MockPlayers show "Waiting for A";
  A resumes → overlay clears. Own-message OSD suppression asserted.
- Real-mpv smoke extends to one `osd-overlay` round trip.

### Milestone
A stalled unpause visibly names its blockers on the video itself; chat
on the OSD is readable under bursts.

---

## Phase 14: File Responsiveness (#26, #21)

**Status: complete (2026-07-03).** Notes and deviations:

- **#26** landed as a mismatch *watch* in the FileActor: 1s `stat` polls,
  re-resolve after 2 quiet polls — but **only if the file changed since
  the failed hash** (the hash-cache row, keyed by `(mtime, size)` at hash
  time, is the comparator). A stable mismatch — a genuine different
  encode — is never re-hashed, and its watch expires after 10 minutes
  (regression test `stable_mismatch_is_not_rehashed` pins the guard).
  `Done::Resolved` gained the filename so the actor can re-resolve
  without the session re-asking.
- **#21**: no live stall was ever captured, so the fix targets the
  nameable structural mechanism: scan hashing saturates the disk while
  transfers are latency-sensitive (a source silent for 30s is snubbed
  and its chunks requeued — with the seeder as only source, that loops
  for the whole indexing run). Scan **hashing** now defers while
  transfer traffic is recent (`FileConfig::scan_transfer_quiet`, default
  10s) and resumes via the 250ms tick; the stat-only walk still runs.
  One `info` line per deferral episode confirms the behavior in field
  logs. If a stall recurs *with* this fix, the existing handshake
  logging (debug) is the next diagnostic.
- Both regression tests were written first and confirmed failing.

**Goal**: Files that are *becoming* available stop looking broken.

### What gets built
- **#26 (mtime-quiesce recheck)**: when a resolve finds a candidate but
  hashing mismatches — the classic case being a hash check racing a
  still-running download/copy — the FileActor starts polling that path's
  mtime+size once per second (cheap `stat`); when they hold still for
  ~2s, it re-hashes. Repeats until Verified or the file vanishes; the
  poll is dropped if the entry resolves by other means. Kills the
  "download finished five seconds later but DessPlay took a minute to
  notice" lag.
- **#21 (downloads during indexing)**: diagnose with the already-landed
  logging, then fix. Suspected: the library scan monopolizing the
  FileActor's blocking-pool budget or serve queue. Whatever the cause,
  the fix must preserve the liveness rule (architecture.md): a scan in
  progress may never starve chunk serving or resolution.

### Testing
- FileActor paused-time test: a file whose bytes+mtime keep changing is
  not re-hashed; once quiet, it re-hashes and resolves Verified (the
  regression test is written first, per the bug-fixing rule).
- Harness regression for #21: start a scan over a large simulated tree,
  assert an in-flight download's chunks keep flowing.

### Milestone
A file that finishes downloading (or copying in) is Verified within
seconds, not minutes; indexing never stalls transfers.

---

## Phase 15: Episode Browser Rework (#31, #11, #10)

**Status: complete (2026-07-03).** Notes and deviations:

- Grouping key is the AniDB-parsed `(category, number)` from
  `episode_sort_key`'s own parser (factored out as
  `props::parse_episode_number`), not a name/hash heuristic: adjacent
  sorted files sharing a key merge, and a file with **no** parseable
  episode number always starts its own singleton group, even next to
  another unnumbered file — there is no evidence any two of them are the
  same episode.
- A multi-copy group renders as a `Header` (display-only, episode label,
  aggregate `watched` = all copies watched) plus one `Child` per file; a
  single-copy episode is one `Single` row combining the label, filename,
  and holders on one line, matching the design's literal example. `Enter`
  and `w` both decline (return `None`, the established "binding
  declines" convention already used by `PlaylistPane::act_archive`/
  `act_watch`) on a `Header` row — there's no single file to act on.
- Holders (`props::ready_holders`) and the episode grouping/muting
  (`props::episode_rows`, `props::first_unwatched`) are pure `StateView`
  mappings with their own unit tests; the modal only renders what it's
  handed. Holders render dim and right-aligned (mirroring
  `PlaylistPane`'s tag column), not per-user colored — kept simple since
  the design didn't call for it.
- `w` toggles the **raw group watched flag** (`view.watched`), not the
  combined muted-display value — a copy already muted by personal
  history alone still flips the group flag to `true` on first press,
  consistent with "cycles a group watched flag".
- `MarkWatched { file, watched }` was appended as the **last** variant of
  `ServerControl` (after `ProtocolMismatch`), not grouped with
  `EofReached`: the bump policy (Phase 11) forbids reordering existing
  variants, and this only needed one more discriminant, not a reshape.
  `PROTOCOL_VERSION` bumped to 2. `handle_mark_watched` mirrors
  `handle_eof`'s watched-flag + `list_advances` writes but isn't scoped
  to now-playing and touches no playback register; idempotent (a request
  that wouldn't change the flag is a no-op). Unmarking never rewinds
  `next_ep` — the auto-advance only ever runs forward, on `watched: true`.
- `UiSnapshot` gained `watched_hashes` (personal 85%-history set, fetched
  in `run.rs::snapshot()` exactly like `recency`) so the episode browser
  can mute by personal history without new plumbing beyond the existing
  per-tick snapshot.

**Goal**: The episode browser answers "which copy, who has it, what's
next" at a glance — today three same-named copies of an episode render
as three identical lines.

### What gets built
- **#31 (copies, filenames, holders)**: episode rows gain the filename
  and holders. Single-copy episodes stay one line
  (`Episode 03  [Judas] Frieren - 03.mkv   Baughn Nero Kim`); multiple
  copies expand into a lightweight tree, one child per file, holders
  listed per child. Holders derive from `FileAvailability::Ready`
  entries; the local user's own copy is what makes "pick the file *you*
  have" possible.
- **#11 (watched marks + next-unwatched)**: episodes watched personally
  (85% history) or by the group (watched flags) render muted, matching
  the playlist's convention; a `<` marker sits on the next unwatched
  episode and the cursor opens there.
- **#10 (manual mark-watched)**: a key in the episode browser / series
  pane cycles an episode's group watched flag. Watched flags are
  server-only writes by design, so this is a new control message
  (`MarkWatched { file, watched }`, mirroring `EofReached`): the server
  sets the flag and — matching the EOF path — auto-advances a linked
  List entry's `next_ep` when marking watched. Idempotent; protocol
  version bump (Phase 11 makes this painless).

### Testing
- Props-mapping unit tests: copy grouping (1 vs N copies), holder
  derivation, watched muting, next-unwatched cursor placement.
- insta snapshots: single-line and tree renders, long-filename clipping.
- Harness: client A marks an episode watched → B's browser shows it
  muted and B's List shows the advanced next_ep.

### Milestone
Three copies of SL2 episode 4 are three distinguishable lines with
owners; queueing tonight's episode starts with the cursor already on it.

---

## Phase 16: Presence & Watch-State Extensions (#7/#13, #15)

**Status: complete (2026-07-03).** Notes and deviations:

- `SeriesWatchState` (the enum) is unchanged; the new attribution struct
  is `SeriesPreference { state: SeriesWatchState, set_by: Option<UserId> }`
  wrapping it in the map. `set_by: None` means "the subject" (every
  self-directed write and system auto-write keeps writing `None`, unchanged
  behavior); only the two new other-targeting paths (`n`, `/skip <name>`)
  write `Some(actor)`. This kept the diff to the two new call sites instead
  of threading `Some(self.me)` through every existing one.
- The value-*shape* change (not just a trailing field on `CrdtState`, but a
  field added to an existing map's value type) needed a value-preserving
  migration, not just a whole-field move: `CrdtStateV3` freezes today's
  layout (bare `SeriesWatchState`), and `upgrade_series_preference` rebuilds
  the map entry-by-entry from resolved `(timestamp, value)` pairs under a
  reserved migration-only `ActorId(u128::MAX)` — safe because `ActorId`s are
  session-scoped (Phase 4), so no live session's dot clock depends on the
  old map's internal dot structure surviving a restart-time migration.
  `PROTOCOL_VERSION` bumped 2 → 3 (also covers #15's `PeerList` change; one
  bump for the whole phase).
- The existing `dessplay-core/tests/migration.rs` byte-truncation trick
  (chop a known trailing suffix off a *current*-layout encoding to fabricate
  an old blob) only works for trailing-field additions; it cannot fabricate
  a faithful old blob for a *middle*-field shape change without access to
  the private `CrdtStateVn` structs. `sample_state()` there was simplified
  to leave `series_preference` empty (an empty map encodes identically
  regardless of value shape) and the `SeriesPreference` migration is instead
  covered where `CrdtStateV3` is reachable: `state.rs`'s own `#[cfg(test)]`
  module.
- **#15 design choice** (a deviation from the terse plan wording): the new
  `known_offline` field *replaces* the old plain "departed" line rather than
  sitting beside it. The server's `known_users` table records `last_seen` on
  every connect *and* disconnect (both flow through the same code path that
  already flips `registry.peers` presence), so filtering it down to
  "everyone not currently Present" naturally covers both this-session
  departures and never-connected-today users in one richer, selectable list
  — a Kim who left 10 minutes ago and a Kim who hasn't logged in today are
  equally valid `n`/`/skip <name>` targets, which is the actual point of the
  request. The `CommittedAbsent`-blocker exclusion (a committed-but-absent
  user always gets a red blocker row, never the dim line) is preserved
  exactly as before; the finer Lost-vs-Departed-vs-committed dedup happens
  client-side in `props::users_props` against `rows`, not on the server.
- `ClientHandle` gained a second, independent `known_offline: watch::
  Receiver<Vec<KnownUser>>` channel alongside the existing `peers` one
  (rather than widening `peers`' value type), since `peers`'s type is
  threaded through the whole multi-client test harness
  (`dessplay-rendezvous/tests/common/mod.rs`) and widening it would have
  touched every harness test file for no benefit — the two channels update
  together (both are filled from the same `PeerList` push) but stay
  separately typed.
- The Users pane's selection cursor now ranges over `rows.len() +
  known_offline.len()`; `a`/`n` resolve the selected index into either list.

**Goal**: The group can manage *absent* members — the "Kim tool".

### What gets built
- **#7/#13 (mark others not-watching)**: `SeriesWatchState` entries gain
  attribution — the map value becomes a struct carrying `set_by:
  Option<UserId>` (snapshot decode falls back per the existing
  `CrdtStateVn` pattern; protocol version bumps). Surfaces: `n` on a
  Users-pane user (sets NotWatching for the now-playing series) and
  `/skip <name>`. The narrator names the real setter ("Baughn set Kim to
  not-watching Frieren"), replacing the "(by …)" placeholder. Guards
  mirror Away: any user may write, the subject's own later write wins by
  LWW.
- **#15 (known-but-offline users)**: the server persists a
  `known_users` table (username, last_seen, updated on
  connect/disconnect) and pushes it with the `PeerList` (offline users
  with a `last_seen` timestamp; hidden after 30 days). The Users pane
  shows them dim + italic with "last seen 3d ago" — and they are valid
  targets for `n`/`a`, which is the point: rule on someone's series
  commitment without waiting for them to show up.

### Testing
- CRDT: convergence property tests over the new attribution struct
  (append-only encoding asserted: old two/three-variant ops decode).
- Server: known_users persistence across restart; 30-day cutoff.
- Harness: A marks offline-Kim not-watching → B's pane and narrator
  agree; Kim reconnects and overrides back to Watching, LWW holds.

### Milestone
Witch Hat can be gated on Kim being present without Kim's absence
blocking every other show — recorded by whoever notices, attributed.

---

## Phase 17: /summon (#4)

**Status: complete (2026-07-03).** Notes and deviations:

- **`/summon` takes no arguments and is decided in two layers**, matching
  the existing `/ack` dispatch pattern: `Ui::command` answers "IRC bridge
  disabled" and "everyone's here" locally (both are already in
  `self.settings`/`self.snapshot.known_offline`, no round trip needed) and
  only emits `UserAction::Summon(Vec<UserId>)` when there is real work.
  Everything needing live IRC state (channel membership, nick matching,
  sending the PRIVMSG) lives in the IRC actor as two new `IrcCommand`/
  `IrcEvent` variants (`Summon` / `Summoned { pinged, unmatched }`),
  mirroring the existing pair rather than a new channel.
- **Membership tracking** is actor-local state inside `run_session`
  (`HashSet<String>`, reset per connection since a fresh NAMES reply
  always follows a JOIN): populated from `353` (NAMES), then kept live
  from `JOIN`/`PART`/`QUIT`/`NICK`. `PART`/`JOIN` are filtered to our one
  channel; `QUIT` has no channel param so it's a global removal (we only
  ever track one channel, so this is exact).
- **Nick matching** uses `strsim::normalized_levenshtein` (already a
  dependency, previously used for manual-file-mapping ranking in 9A) at a
  **0.4** similarity threshold, case-insensitive — `Nero`→`Nero200` scores
  ~0.57 and matches comfortably; an unrelated nick scores well below 0.4.
  Bridge nicks (`*Dess`) are excluded from the candidate pool via the
  existing `is_bridge_nick`.
- **The "not connected yet" case** (a `Summon` arriving while the actor is
  disabled or mid-reconnect-backoff) isn't named in the terse plan
  wording, but needed an answer: both the disabled-idle loop and
  `wait_backoff` now report `Summoned { pinged: vec![], unmatched: <all
  requested> }` immediately (same shape as "checked membership, found
  nobody") rather than silently dropping the command like `SendChat`
  does — a `/summon` that vanishes with zero feedback would violate the
  "local system line always reports" requirement. `wait_backoff` gained
  an `events` parameter for this; the SendChat-during-backoff
  non-abort behavior is untouched (Summon follows the identical
  fall-through-without-returning shape).
- The Dess-girl URL is a hardcoded `const DESS_GIRL_URL` in `irc.rs` (the
  exact link from this plan's row) — no settings surface, as scoped.

**Goal**: One command pings the missing people on IRC, with the
mandatory Dess-girl.

### What gets built
- IrcActor learns channel membership: parse NAMES on join, track
  JOIN/PART/QUIT/NICK. Membership is actor state, queried by command.
- `/summon`: the session computes absent known users (Phase 16's
  registry minus present peers), maps each to the closest-edit-distance
  channel nick (excluding `*Dess` bridge nicks; a normalized-distance
  threshold so nobody random gets pinged — Nero→Nero200 must match,
  an absent user with no plausible nick is skipped and reported), and
  sends one PRIVMSG:
  `Nero200, Quickshot: Dess? https://brage.info/GAN/019dea7a-e1ad-77e1-a719-82619e50944f.jpg`
  (URL a constant for now). Local system line reports who was pinged or
  why nobody was ("IRC bridge disabled", "everyone's here").

### Testing
- Pure-function tests: NAMES/NICK tracking, nick matching (Nero→Nero200,
  `*Dess` exclusion, threshold rejections), message formatting.
- Duplex-pipe actor test: `/summon` end-to-end against a scripted IRC
  server.

### Milestone
"It's time for dess" is one command instead of five manual pings.

---

## Phase 18: Layout & Input Polish (#6, #33, #22, #8)

**Status: complete (2026-07-03).** Notes and deviations:

- **#6**: `StatusBar::render` split into the bottom status bar (state +
  "Now Playing", unchanged position) and a new `render_progress`, called
  directly from `Ui::draw` (not through the `Component`/`view()` trait
  path — the same "inline render" pattern already used for the subtitle
  pane) into a `Constraint::Length(1)` row in the left column: between the
  chat input and the subtitle pane in Separate-pane mode, or at the
  bottom of the chat column otherwise (the position the subtitle pane
  would occupy if enabled).
- **#33**: bracketed paste is enabled via `CrosstermTerminalAdapter`'s
  inherent `enable_bracketed_paste()` (not part of the `TerminalAdapter`
  trait) right after entering the alternate screen; since the adapter's
  `restore()` never tracks or disables it, `run_ui_thread` explicitly
  issues `DisableBracketedPaste` on exit. `Ui::handle` gained a top-level
  `Event::Paste` branch (gated on no modal open, mirroring the existing
  global Tab/F2/F3 gate): a single existing-file path while Playlist is
  focused reuses `Msg::FileChosen` verbatim (anchored after the current
  selection, via a newly `pub(crate)` `PlaylistPane::selected_hash`);
  anything else lands in the chat input via a new `ChatPane::insert_text`
  (char-by-char, exactly like typing). No new `Msg` variant needed.
- **#22**: `subtitle_speaker_colors: bool` (default true) is a fully
  additive copy of the `irc_enabled` pattern end-to-end (`config.rs` load/
  save, a new settings-modal row appended at the end of the fixed-field
  list to avoid renumbering existing `FIELD_*` constants) plus one `if` in
  `Ui::draw`'s Separate-pane subtitle loop, gating speaker color to
  uniform dim when off.
- **#8**: `BrowserSort` (`Alphabetical` default / `Newest`) mirrors
  `SeriesSort` exactly, including the `set_sort`-from-settings-on-open /
  read-back-on-toggle pattern (`FileBrowser::sort()`, mirroring
  `SeriesPane::sort()`). `Newest` **replaces** rather than layers onto the
  purpose's default ordering — plain alphabetical *or* the Map browser's
  edit-distance-to-target ranking — since the request is an explicit
  "show me what's fresh" override, not a tiebreaker. Threading mtime
  required touching every layer between `Storage::library_paths()` (now
  `Vec<(PathBuf, Ed2kHash, i64)>`) and `BrowserLibrary`/`LibraryFile`/
  `DirRow` (`run.rs` → `UiInput::Browse` → `Ui::open_file_browser`); ~10
  existing test call sites picked up the third tuple element. The live
  directory-listing branch prefers the library index's mtime (an
  already-hashed file) and falls back to a live stat reusing the
  symlink-follow metadata call already made for `is_dir` when possible —
  so a freshly landed, not-yet-indexed file still sorts correctly, not
  just previously-scanned ones. `Tab` was the deliberate key choice (not
  `s`, which the design row for this phase implies but which collides
  with the type-to-search fall-through the moment a search starts with
  "s", e.g. "Sousou") — confirmed unused inside a modal (the global
  Tab-cycles-focus handler is gated on no modal being open) and bound in
  the File, Map, *and* Search keymaps so the toggle works mid-search too.

**Goal**: The remaining small, independent UI requests.

### What gets built
- **#6**: the progress bar + time move to their own line in the left
  column, between the chat input and the subtitle pane, so ready-state
  text stops shoving them around. TUI layout diagram in design.md
  updated.
- **#33 (drag-drop add)**: enable bracketed paste (crossterm
  `Event::Paste`). A paste that is a single existing-file path while the
  playlist pane is focused becomes an add (same path as the browser
  pick); any other paste inserts into the chat input as text (which the
  chat input gains as a side benefit).
- **#22**: `subtitle_speaker_colors` toggle in the settings screen
  (default on); when off, the separate subtitle pane renders uniformly
  dim regardless of speaker.
- **#8**: a sort toggle in the add/map file browser — alphabetical vs
  newest-mtime-first (mtime from the library index), persisted like the
  All Series sort. Freshly landed files float to the top.

### Testing
- insta snapshots for the new left-column layout and both browser sorts.
- Paste-event message tests: path-on-playlist → add; text → chat input;
  path-while-chat-focused → text.
- Settings round-trip for the two new persisted settings.

### Milestone
The request sheet's interface rows are done or consciously deferred.

---

## Phase 19: Series Identity & List Commitment (#9, #25)

**Status: complete (2026-07-05).** Notes and deviations:

- The auto-created entry id is **deterministic** (MD4 over a domain-tagged
  AniDB id or derived name, `series_identity::derive_entry_id`) rather
  than random: two clients racing to auto-create for the same series must
  converge on one entry, or gating forks (caught by a flaking e2e test).
  Import still mints random ids — a deliberate creation has no content to
  hash yet.
- The migration landed as unit tests over crafted legacy blobs
  (`state.rs::legacy_blob_*`: link-reuse and synthesis both asserted)
  rather than the promised property test over fixtures; the
  resolution-order property now generates series identities and verifies
  link > manual file > name > deterministic auto-create, including
  idempotence after persistence (`series_identity.rs`). Linked/unlinked
  gating tests landed 2026-07-05 with the scoped-review fixes
  (`derive.rs::absent_committed_user_blocks` parameterized). The
  disambiguation view is covered behaviorally (opens-the-browser +
  ranking assertions) rather than by an insta snapshot.
- The resolution function's step 2 (`manual_files`) runs *before* the
  metadata guard — it is a pure hash test and must work before any
  metadata syncs (2026-07-05 review).
- The edit modal's Aliases / Manual files rows landed 2026-07-05
  (semicolon-separated; manual files as ed2k hex, unparsable tokens
  dropped). `watchers` editing remains deferred to import.
- `PROTOCOL_VERSION` ended this phase at **5**, not 4: the review's manual-mapping
  fix added `PeerMessage::CannotServe` within the same upgrade window
  (network-design.md, Peer Messages).

**Goal**: series commitment and gating stop depending on an AniDB link
existing at all. Every committable series routes through its
[List](design.md#the-list-series-tracker) entry, and a file resolves to
that entry without assuming AniDB knows the series or that its episodes
share one directory — see design.md's
[Series Identity](design.md#series-identity) and
[Advancing next_ep](design.md#advancing-next_ep) (design discussion
2026-07-03/04). Unblocks the two long-deferred requests below: #9
(Nero-names surfacing) and #25 (auto-queue next episode), both of which
were waiting on exactly this.

### What gets built
- Protocol version gate: `PROTOCOL_VERSION` 3 -> 4 (Phase 11's gate refuses
  a mismatched reconnect, forcing a clean resync on upgrade).
- `series_preference` re-keyed `Map<(UserId, AniDbSeriesId), ...>` ->
  `Map<(UserId, ListEntryId), ...>` in `dessplay-core::state`. Snapshot
  migration per sync-state.md's Series Preference note: for each old
  `AniDbSeriesId`-keyed entry, find (or synthesize) the linked List entry
  and rewrite the key, preserving the original timestamp — mirroring the
  `SeriesPreference`-wrapper migration's approach but spanning two maps
  instead of one.
- `SeriesListEntry` gains `local_aliases: BTreeSet<String>` and
  `manual_files: BTreeSet<Ed2kHash>`.
- A single resolution function implementing design.md's 4-step order
  (AniDB link -> `manual_files` -> name/`local_aliases` match -> auto-create),
  replacing every `now_playing_series() -> Option<AniDbSeriesId>` call site
  (`/watch`/`/maybe`/`/skip`, the playlist `w` cycle key, Users-pane `n`,
  `watchers`-set wiring) with the `ListEntryId`-returning equivalent.
- EOF-time `next_ep` bump extended to unlinked entries: parse the
  just-finished file's own filename for an episode number (no ambiguity —
  it's the file already confirmed watched) and bump when it parses
  cleanly; otherwise left for a manual bump, same as any free-text entry.
- Candidate-ranked disambiguation: generalize the Episode Browser's
  existing "several files, one confirmed episode" tree into "several
  *candidate* files, ranked, no confirmed identity" for jumping to
  `next_ep` on an unlinked entry — scored on parsed episode number, edit
  distance to the expected label, mtime, and alias/`manual_files`
  membership. Picking a candidate runs the ordinary add-to-playlist flow;
  no Playlist CRDT change.
- Series pane default mode -> The List (`SeriesMode::TheList` as
  `#[default]`, was `Recent`).
- List edit modal gains fields for a linked-or-not entry's
  `local_aliases` / `manual_files`.

### Testing
- Property test: the re-keying migration preserves every
  (subject, resolved value) pair across representative old-snapshot
  fixtures, mirroring the existing `SeriesPreference`-wrapper migration
  test.
- Property test: resolution order (link > `manual_files` >
  `local_aliases` > auto-create) is deterministic and idempotent
  regardless of which committable action triggers it first.
- Gating property tests (`derive.rs`) parameterized over linked/unlinked:
  an unlinked entry's commitment blocks/unblocks playback identically to a
  linked one.
- Unit test: two files with different directory-derived hints, once both
  named in one entry's `local_aliases`, resolve to the same `ListEntryId`.
- insta snapshot: the candidate-ranked disambiguation tree, mirroring the
  existing multi-file-per-episode tree snapshot.

### Milestone
A series with no AniDB entry can be committed to, gates playback across
absence exactly like a linked one, and its next episode can be found and
queued without relying on a dedicated directory. #9 and #25 unblocked.

---

## Phase 20: Nyaa Playlist Search

**Status: complete (2026-07-10).**

Playlist `n` opens a local Nyaa Anime-category search. The client inspects the
first 20 RSS results' torrent metadata and lists only safe single-file payloads.
Selection downloads in the background under a temporary local import id;
reopening the modal shows active imports with cancellation. Completion assigns
the payload its ed2k identity, promotes the torrent into the ordinary cache and
seeding lifecycle, then emits the existing hash-keyed playlist mutation. No
wire or CRDT schema change was needed.

Tests use canned RSS/metainfo and the fake torrent engine; no automated test
contacts Nyaa. Coverage includes category/limit/order filtering, malformed and
multi-file metadata, unsafe names, actor completion/promotion and cancellation,
plus the full UI key/search/progress/reopen/cancel flow.

---

## Phase 21: Explicit User-Seek Attribution

**Status: complete (2026-07-11).**

Seek narration no longer infers user actions from continuous playback-position
samples. The debounced player output records the scrub's initial and final
positions; user seek-authority carries that explicit `UserSeek` occurrence.
This narrates the first genuine seek in an episode while automatic load-to-zero,
drift-correction, and restore seeks remain silent. Protocol version 6 changes
the seek-authority value encoding; persisted v5 snapshots migrate by preserving
durable state and resetting the old, unattributable transient authority to
Server.

Regression coverage includes the formerly missing first seek, gradual scrub
coalescing, wrong-file rejection, programmatic-seek echo suppression, v5
snapshot migration, and a two-client scenario asserting identical attribution.

---

## Phase 22: Disconnected Media-Root Retention

**Status: complete (2026-07-11).**

The library index now distinguishes an individually deleted file from a whole
media root disappearing. If any recorded file survives, missing siblings are
pruned immediately; if none survive, the root is marked vanished and its hashes
are retained indefinitely but hidden/inactive. Returning unchanged files are
cache hits. Roots removed from the effective runtime configuration retain their
hidden index for seven days so remove/re-add is cheap, then expire.

SQLite records root ownership on `hash_cache` and stores durable lifecycle in
`library_roots`; no protocol state changed. Regression, property, actor, and
migration/storage tests cover wholesale disappearance, partial pruning,
availability retraction, reconnect without hashing, and removal expiry.

---

## Phase 23: Categorised Settings

**Status: complete (2026-07-12).** See
[`docs/proposals/2026-07-12-settings-screen.md`](proposals/2026-07-12-settings-screen.md).

Replace the settings modal's mixed integer-indexed list with Account,
Playback, Files, and IRC tabs over typed semantic form rows. The shared Form
derives control rendering and activation from row data; Settings and List edit
stop dispatching fields by `usize`. Save remains atomic, all existing
first-run validation and runtime-override isolation stays intact, and controls
show their actual live/restart lifecycle.

Add the missing human-readable upload-limit editor. Also expose the persisted
mpv/VLC player choice as a clearly marked `WIP -- not applied` placeholder;
the composition root continues to use mpv in this phase.

Coverage includes typed-row uniqueness and edit-kind tests, upload-rate
round-trip properties, semantic selection across root reorder/removal,
per-category navigation and missing markers, secret rendering, invalid editor
commits, one 100x30 snapshot per category, and preservation of the existing
save-path and runtime-override regressions. No CRDT or wire change is needed.

Implementation notes: Form now owns `FormRow<RowId>` / `FormControl` and one
`FormEdit` mutation boundary, retains semantic selection across externally
inserted rows, and keeps header/notes/Save fixed around the scrolling list.
Long media-root paths clip from the left so their final components remain
distinguishable. Upload rates accept exact whole binary units through GiB/s
and round-trip without losing byte precision. The player choice persists but,
as designed, is visibly annotated `WIP — not applied` and does not alter the
mpv composition root.

---

## Phase 24: Terminal Color Depth & Subtitle Speaker Scaling

**Status: complete (2026-07-12).** The reported six-speaker ceiling was not a
literal cap: the old subtitle path hashed speakers into the shared finite
palette, so collisions could appear before or after its ten entries. A local
tracker now maintains the inclusive rolling five-minute active set and stable
slots for true-color generation. Active speakers retain their slots; expired
slots are recycled. Limited terminals deliberately retain the prior direct
name hash for backward-compatible rendering.

Production asks crossterm for terminal color depth once during setup and
injects the result into `Ui`. True-color mode gives the completed frame one
explicit dark theme across every pane, modal, and overlay, then generates
speaker colors without an application cap from a progressive HSLuv palette.
Each slot deterministically maximizes its minimum CIEDE2000 distance from the
quantized colors already assigned, while the known background makes contrast
testable. Limited-color mode remains a no-op theme pass, preserving the user's
terminal theme, and continues hashing speaker names into the existing
ten-color application palette.

The Playback tab gains the persisted **Color overflow** choice for an active
set larger than that finite palette: **Reuse colors** continues the existing
name hashing and is the default for backward compatibility; **Disable colors**
removes all speaker identity (uniform dim text) until enough speakers expire.
The window also advances on a quiet-scene UI clock tick, so recovery does not
require another cue. The existing **Speaker colors** setting remains the
master toggle on both terminal paths. This is entirely local presentation
state: no CRDT or wire change.

Regression coverage pins five-minute boundary inclusion, lease refresh,
expiry/reuse and backward-clock behavior; both limited overflow policies and
recovery; the persisted setting's default/cycle/round trip and Playback
snapshot; true-color continuation past ten speakers; whole-frame dark-theme
coverage; perceptual-separation bounds through the first 256 generated colors;
and at least 4.5:1 contrast for that prefix and every semantic foreground
against the fixed background.

**Compatibility follow-up (2026-07-17):** true-color dim text is now
materialized as the theme's explicit muted RGB foreground before terminal
output. This preserves the limited-color path's native SGR 2 behavior while
avoiding VTE-family differences when SGR 2 is combined with explicit RGB; a
whole-app watched-playlist regression and modifier-preservation property test
pin the boundary.

---

## Phase 25: Subtitle Speaker Names

**Status: complete (2026-07-13).**

The Playback tab now has a persisted **Speaker names** toggle, default off to
preserve the existing spoiler-safe presentation. When enabled, named ASS cues
render as `Name: dialogue` in both Intermixed and Separate modes; unnamed and
non-ASS cues remain unchanged. One display-time formatter serves both modes,
so toggling the setting updates already-buffered cues immediately without
changing subtitle collapsing, timestamps, or speaker tracking.

Intermixed subtitles remain uniformly dim. Separate-pane names and dialogue
share the line's existing speaker color, or become uniformly dim together when
speaker colors are disabled. This is a local key-value setting only: no CRDT,
wire, database-schema, or protocol-version change.

Coverage includes the missing-key default and persistence round trip, typed
Playback-row toggle/save behavior and snapshot, both rendering modes, unnamed
cues, live buffered-cue updates, and the existing color-policy interaction.

---

## Phase 26: Configurable Archive Subdirectory

**Status: complete (2026-07-13).**

The Files & transfers tab now has a persisted **Archive subdirectory**
toggle, default on to preserve the existing layout. Each `A` action carries
the current choice through the UI/session boundary to the file actor. Enabled
archives use `<download root>/<sanitized series>/<sanitized filename>`;
disabled archives use `<download root>/<sanitized filename>`.

Coverage pins the missing-key default and persistence round trip, typed Files
row toggle/save behavior, action propagation of the default, and both archive
destination layouts at the file-actor boundary.

---

## Phase 27: Browse-Only BitTorrent

**Status: complete (2026-08-09).**

The automatic torrent-first fetch path was removed: the matured peer
relay is the only automatic fetch route, and BitTorrent survives solely
as Phase 20's explicit Nyaa browse import. Deleted with it: the
`TorrentFetches` policy core (search/stall/cooldown/ban state machine),
the exact-filename nyaa match (`pick_match`), the `torrents` registry
table (dropped by migration v6), librqbit session persistence, and
startup torrent reconciliation. Imports now seed only for the session
that downloaded them — sessions typically clear 1:1, and resuming last
week's seeds at launch was judged unexpected behavior for a video
player — so startup simply sweeps `<cache>/torrents/`, sparing only a
directory hosting a registered cache file. `StartDownload` lost its
`filename` field (it existed for the nyaa query). For rare files, the
expectation is a manual search plus a dedicated BT client.

---

## Phase 28: Transfer Flow-Control Overhaul

**Status: complete (2026-07-28; hardened 2026-08-13).** Unplanned — see
[`docs/proposals/2026-07-28-transfer-flow-control.md`](proposals/2026-07-28-transfer-flow-control.md).

QUIC congestion control switched from Cubic to BBR; the client runs
split DSCP-tagged control and transfer connections; each transfer gets
its own data stream with end-to-end backpressure; quinn-udp is vendored
so DSCP tags survive the per-packet ECN cmsg. Position-anchored playable
gating and partial-file playback landed in the same window. The
2026-08-13 hardening pass added the answered-request contract for stream
opens, generation-stamped stream lifecycle events, stream-loss-as-snub,
and the permanently-unreachable-transfer-link advisory.

---

## Phase 29: AI Commentary & the Synced Marquee

**Status: complete (2026-07-25 → 2026-08-13).** Unplanned; documented in
design.md (AI Commentary). A generic synced marquee register
(`PROTOCOL_VERSION` 7) scrolls through the bottom line's middle slot;
the commentary engine feeds it in-character lines from an Anthropic
model, with screenshot capture through the player stack,
speaker-attributed subtitle context, per-thread commentators, and an AI
settings tab (token + interval). Follow-up hardening: JPEG frames (PNG
blew the API's 10 MiB image cap), frame/episode retention caps,
series-change survival, adaptive thinking, and merged-clock marquee
animation fixes.

---

## Phase 30: Anchored Download Policy

**Status: complete (2026-08-16).** Unplanned; documented in design.md
(Download Cache and Retention). The want-set became every unwatched
playlist entry, ordered by `anchored_download_order` around now-playing
(entries after it first, nearest first; watched back-catalog last); a
shared per-source chunk budget spends across files in that priority
order; the session plumbs `SetDownloadPriority` to the scheduler; the
seeder anchors at now-playing too; urgent-set scheduling generalizes
endgame to any deadline-gated chunk. Cross-file fuzz coverage pins the
policy. Supersedes Phase 9B's seek-aware-window and prioritization
future bullets.

---

## Other unplanned work, 2026-07 → 2026-08

Landed without plan entries; recorded for completeness: the borderless
connection-health line + advisor seam (2026-07-25), mouse support
(2026-07-20), Discord-style `||spoiler||` chat tags masked in both IRC
directions (2026-08-12/13), the tagged snapshot storage envelope with
frozen-byte compat fixtures (2026-07-25 / 2026-08-13), multi-address
server dial + DNS re-resolution (2026-07-06/19), known-offline gating
extended to a week (2026-07-18), and the drift controller's hysteresis +
tapered-slew rework (2026-07-23).

---

## 2026-08-17 Triage

The 2026-07-02 deferred batch, Phase 9B's future list, and design.md's
Future Plans, consolidated against six weeks of real usage.

**Closed / dropped:**

- Disk/retention-aware prefetch depth — obsoleted by hardware (the one
  constrained client now has 2 TiB of disk); was always a single-user
  problem.
- Bitrate-aware unpause (the download-speed-vs-bitrate rule half) — not
  worth automating; since the anchored download policy it comes up
  rarely, and the group decides by watching how fast the download
  percentage moves.
- Intermixed-subtitle interleave by in-video timestamp — arrival order
  is fine in practice.
- Web frontend (the `ViewSpec` web renderer) — no interest.
- Choking for many-peer scale — the group is a fixed handful.
- **#28** large uncorrected desync — believed fixed (drift-controller
  hysteresis rework, 2026-07-23); reopen with logs if it recurs.

**Still open, deferred:**

- **#5** subtitle near-duplicate lines — needs a concrete example.
- **#14 (sound half)** the audible "Dess?!" — blocked on an audio asset.
- **#19** GUI — deferred until everything else is done; mechanism open
  (the web-renderer approach is dropped).
- Automating The List's "episode is out" flag via AniDB air dates.

**New work from usage pain points: Phases 31–33, below.**

---

## Phase 31: Servable Ad-hoc Files & Drag-in Adds

**Status: complete (2026-08-17).** All four pieces landed as planned,
plus one scope change decided during implementation: the paste add is
now **unconditional on focus** (user decision 2026-08-17 — a valid
file path pastes as an add from any pane; there is no use for posting
a file *path* to chat; only modal text editors still capture the
paste). Delivered: hash-adds register the servable copy on
`Done::Hashed` *and* on the cache-hit fast path (`adopt_hash_added`);
out-of-root adds persist as manual-mapping rows, registered in place;
the not-held serve bail answers `CannotServe` + a warning instead of
silence; the stale-resolve race is guarded at both layers (the actor
drops a NotFound/mismatch for a held file; the session ignores a
non-pending downgrade of a Verified entry). Tests: five regression
tests written first and confirmed failing (serve-after-add fresh +
cache-hit, restart durability, CannotServe, stale resolve), paste
normalization units, dispatcher-level paste tests, and an end-to-end
harness test (`adhoc_files.rs`) over the promoted-to-common `LoopRig`:
A drags in an out-of-root file → B downloads it; A restarts → a fresh
client still downloads it from A.

**Goal**: a file dragged into the terminal — from anywhere, including
outside every media root — plays for everyone, not just the dragger.

Investigation (2026-08-17) found the add half already works: the
Phase 18 paste branch accepts any existing path with no root check, and
out-of-root `hash_cache` rows survive Phase 22's root-keyed pruning
(nullable `media_root`). What's broken is serving.

### What gets built
- **Bug fix (wider than drag-drop)**: a hash-added file is never
  registered in the FileActor's servable set (`local_files`), so *any*
  browse- or paste-added file advertises Ready yet serves nothing for
  the rest of the session — peers solicit, hit the silent
  `serve_block_hashes` bail, snub us, and stay Missing, gating the
  group. (In-root adds self-heal on the next restart via scan adoption;
  out-of-root adds never do.) Fix: adopt the local copy on
  `Done::Hashed` alongside `commit_fresh_hashes`; make the
  "advertised Ready but not held" serve path answer `CannotServe` (or
  log loudly) instead of silence. Regression test written first, per
  policy: an added file advertises Ready *and serves* in the same
  session.
- **Durable out-of-root registration**: persist the pasted path as a
  manual-mapping-style row — registered **in place** (user decision
  2026-08-17: no copy into the cache). The file must stay put; a moved
  file re-breaks availability, exactly like a manual mapping.
- **Paste normalization**: shell-unescape backslash escapes, strip
  quotes, accept `file://` URLs — the forms terminals actually produce
  on drag — before the existing single-existing-file test. Anything
  else still lands in the chat input.
- Fix the stale-resolve race found in the same investigation: a
  `NotFound` resolve landing after `note_local_file` overwrites the
  verified entry and starts a pointless download of a file we hold.

### Testing
- Paste normalization unit tests (escaped, quoted, `file://`,
  directory, multi-line).
- Harness: A pastes an out-of-root path → B downloads and plays it;
  A restarts → still servable.
- The serving regression test above, confirmed failing before the fix.

### Milestone
Ad-hoc episode selection is drag → everyone watches. No silent
Ready-but-unservable states remain.

---

## Phase 32: List Rework

**Status: complete (2026-08-17).** All four pieces landed as planned,
plus three details settled during implementation (user decisions
2026-08-17): the Recency partition is *watchable* — the weekly
`available` flag **or** an unwatched library file resolving to the
entry (Series Identity order), with the dim set exactly the bottom
partition; Recency is the fresh-install default, persisted thereafter
(`list_sort` setting, mirroring `series_sort`); and the local user's
own "Watching — ⟨user⟩" group renders first, the rest alphabetical.
"Most recently group-watched" is derived from local watch history (the
Recent Series source) — group watched flags carry no timestamps.

First-poke fixes (same day): Enter on a linked entry silently did
nothing when the linked season wasn't the franchise's component root
(the exact-key lookup missed) or held no files — List-entry Enter is
now one `BrowseListEntry` message resolved by full component
membership (`Franchise::members`, new), falling back candidate view →
editor, never silence. And "has unwatched files" was uselessly broad:
compaction dropped group watched flags for off-playlist files while
metadata rows persist forever, so every long-finished episode read as
unwatched. Two-part fix: **compaction now keeps `true` watched flags
for all files** (the durable group watch record; `false` flags drop —
absent already means unwatched), and the watchable test requires a
*held* copy (availability map) unwatched by both the group flag and
personal watch history (the episode browser's muting rule). Flags
compacted away before this fix are gone; a leaked "unwatched" episode
is repaired with `w` in the episode browser, and the flag now
survives.
Tests: props units (per-user groups incl. multi-membership and
unknown-user residual, live commitment column, both sorts + partition,
season ordinals over chains/cycles/missing links, unwatched-file
resolution via manual_files/aliases), two insta snapshots (both sorts
over one fixture with per-user groups + SnEnn), and a harness test
(`list_watching.rs`): kim's commitment lands in nero's List under
"Watching — kim".

**Goal**: The List answers "what are we watching, whose, and what's
next" at a glance. Today the users column shows import-time `watchers`
initials (never live state), grouping ignores per-user commitment, sort
is hardcoded name order, and next_ep renders as raw free text.

All client-side derivation: `series_preference` is already resolved in
the `StateView` at the `list_groups` call site — no CRDT or wire change.

### What gets built
- **Live commitment column**: the users column derives from
  `series_preference` (who has the series as Watching), replacing the
  static `watchers`-initials display. The `watchers` set keeps its
  existing role as a one-shot preference seed.
- **Per-user Watching groups**: one "Watching — ⟨user⟩" group per user
  (peers + known-offline) with Watching commitments; an entry appears in
  every applicable group. The shared status groups (Short List, Planned,
  Waiting, Hiatus, Finished/Dropped) render below, unchanged; an entry
  with Watching-tier status (`CurrentSeason`/`Active`) but no committed
  watcher falls into a residual shared Watching group so nothing
  vanishes.
- **Sort toggle** (`s`, currently unbound in List mode; mirrors the
  All-Series `SeriesSort` pattern including persistence): Alphabetical /
  Recency. In Recency mode, entries whose next episode is out
  (`available`) sort above those with nothing unwatched; within each
  partition, most recently group-watched first. Series without unwatched
  files are dimmed. Sort mode is the default at start.
- **SnEnn next-ep display**: for linked entries, derive the season
  ordinal by counting prequels along the replicated `SeriesRelations`
  chain (the franchise walk already exists) and render a parseable
  next_ep as `S2E05`; unlinked or unparseable values render verbatim.
  Column position unchanged (left of the users column).

### Testing
- Props unit tests: per-user group derivation (multi-membership,
  known-offline users, the residual group), the live-commitment column,
  both sort orders + the availability partition, season-ordinal
  derivation over relation chains (including cycles and missing links).
- insta snapshots: per-user groups, both sorts, SnEnn rendering.
- Harness: A `/watch`es a series → B's List shows it under
  "Watching — A".

### Milestone
The List is the nightly "what's next" surface: whose-turn groups, fresh
episodes floating up, `S2E05` at a glance.

---

## Phase 33: Nicknames & Short Titles

**Status: complete (2026-08-17).** All three pieces landed as planned:
`n` opens a minimal nero_name editor (`NeroNameModal` — Enter saves
trimmed with empty clearing, Esc cancels, unchanged commits write
nothing); `SeriesRelations` gained `short_titles` (kind-3 rows, x-jat
before en, deduped, from a new `ServerStorage::short_titles` query);
and a linked List entry renders the preferred short title in place of
the official name, alphabetizing under it. The "backfill" became an
idempotent every-pass reconcile (`apply_short_titles`, the
`apply_series_hints` shape) rather than one-shot — it also refreshes
short titles whenever the daily dump changes, quiesces when replicated
state matches, and treats an empty titles table as "no information".
The protocol bump (v10 → v11) reshaped a persisted value type for the
first time, so v7–v10 moved from `LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS`
to the tree's first frozen-layout decode arm (`CrdtStateV10` /
`SeriesRelationsV10`, value-level rebuild preserving LWW timestamps
under a dedicated migration actor); v10's fixture blob was captured at
the bump per the now-documented frozen-version rule
(tests/fixtures/README.md).

**Goal**: the names the group actually uses become first-class. Nero's
(re)names get a fast entry path, and AniDB's community short titles
(type-3 rows in the titles dump — already ingested unfiltered into
server SQLite, just never surfaced per-series) replace unwieldy official
titles in the List.

### What gets built
- **`n` in the Series pane, List mode**: opens a minimal single-field
  editor for the selected entry's `nero_name`. The field and its
  dim-quoted display after the title already exist; entry today requires
  the full edit modal. `n` is unbound in every SeriesPane keymap; this
  becomes the third pane-local meaning of `n` (design.md key table
  updated).
- **Short titles over the wire**: append `short_titles: Vec<String>`
  (kind-3 rows, x-jat/en preferred) to `SeriesRelations`, populated at
  ANIME-lookup time from the titles table, plus a one-time server
  backfill for already-settled `series_relations` rows (they are written
  once and never revisited). `PROTOCOL_VERSION` bump per policy;
  snapshot-compat fixtures checked.
- **Display**: a linked List entry with a short title renders it
  *instead of* the official name (user decision 2026-08-17: save the
  space — the full name still lives in the edit modal and episode
  browser), with `nero_name` appended dim as today.

### Testing
- Storage: kind-3 title query; worker population + backfill over canned
  dumps.
- Snapshot/protocol compat per the frozen-fixture policy.
- UI: short title over official name, nero_name still appended, `n`
  editor round-trip.

### Milestone
"GochiUsa" instead of "Gochuumon wa Usagi Desu ka??", and Nero's names
entered in two keystrokes.

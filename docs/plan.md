# DessPlay Implementation Plan

Last updated: 2026-10-07

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
injects the result into `Ui`. True-color mode gives the display one
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

**Redesigned 2026-08-18** after first contact with the real dump: the
kind-3 rows are lowercase search tags ("gochiusa s2", "s;g", "HnNKn"),
not display names, and only ~24% of series have one. The short-title
*source* is now an AI curator (`anidb/curator.rs`): the worker batches
never-asked series to an Anthropic model (claude-opus-5, effort low,
structured output; the series' full dump rows as context), trusts the
answer as returned (user decision — the community name is sometimes
absent from AniDB entirely), and caches it forever in the
`ai_short_titles` table, so the API is consulted once per series ever.
The wire shape (`short_titles: Vec<String>`, 0/1 entries) is unchanged.
Display gained a precedence rule (user decision): substitution only
when the entry's name still equals the official title — a human name
always wins, which is also the fix path for a bad pick. The API token
is client-provisioned: pushed from the token-holding client's existing
`anthropic_token` setting on connect and on settings edits
(`SetAnthropicToken`, protocol v11 → v12, wire-only — v11 joined the
layout-compatible list), persisted in the server's kv table, cleared by
clearing the setting.

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

---

## Phase 34: Auto-Archive Watched Downloads

**Status: complete (2026-09-01).**

The Files & transfers tab has a persisted **Auto-archive watched** toggle,
default off. When on, the personal watch record (the 85% rule) archives a
cache-only file exactly as `A` would; a file watched off a partial is
archived when its download completes. The archive policy (subdirectory
layout + auto trigger) moved into the file actor as `ArchivePolicy`, pushed
on settings save — the `A` action no longer carries the subdirectory flag,
so the manual and automatic paths share one destination rule. Cross-device
archive copies now run off the actor thread (the eviction pass skips a file
mid-copy), and the session re-keys its resolution and loaded path on
`Archived` without reloading the player.

Coverage: the default and persistence round trip, the Files row toggle/save,
the watch-record trigger (on, off, and a non-cached file), the
download-completion trigger, and the no-reload re-key at the wiring
boundary.

---

## Phase 35: Consequential expeditions and the awakened return

**Status: implemented (2026-09-06).**

The Waiting Below now treats leaving alive as a victory and ember retrieval
as the dangerous commitment. The adopted scope supersedes the short-return
experiment in the original [playtest proposal](proposals/2026-09-06-roguelike-playtest.md).

The implementation was divided into these dependent work units:

1. **Anatomy and equipment:** shared species-aware tissues, organs, lasting
   losses, functional grip/gait/sight/breath, finite automatic care, regional
   armor, and a small active/spare weapon kit.
2. **Generation and awakening:** procedural connected loops and branches,
   optional treasure/fountain/terrain set-pieces, dormant caverns, saved
   warning/outbreak/lull cycles, swarms, and route-preserving rockfalls.
3. **Simulation and observation:** integer elapsed-action scheduling,
   sprinting without walking recovery, readable creature commitments,
   interruptible calls, explicit pickup/interaction/equipment, structured
   journal, survival scoring and epilogues, and one honest `RunView`.
4. **Storage and controller:** version-2 saves behind the existing atomic
   save/history/report boundary; the authorized one-time v8 roguelike reset;
   committed views, paced cancellable care, inspection/journal screens, and
   live Full/Reduced/Off cosmetic settings.
5. **Validation and tuning:** seeded properties and recorded regressions,
   a structured fuzz target, full application/session tests, observation-only
   surveys, and two manual agents with separate initial and shared-seed
   follow-up cohorts. Findings and limitations are recorded in the proposal.

Anatomy, world generation, and UI work ran independently behind agreed
interfaces; integration retained one deterministic engine and the existing
party report pipeline. Initial testing found occupancy and perception bugs
and descent density that prevented the return phase from being exercised.
Those were addressed before the shared-seed follow-up. Difficulty remains a
playtesting judgment: a policy survey is not a promised human win rate.

The player guide, design rules, decision rationale, architecture, testing
strategy, and changelog describe the final behavior. No new protocol or
synced-state schema was introduced.

## Phase 36: Runtime-editable display layouts

**Status: implementation and automated verification complete (2026-09-09); physical terminal graphics checks pending.**

The accepted implementation is a staged migration to versioned local XML
component templates, CSS, and one width-first terminal geometry pipeline.
It retains synchronous Elm controllers. Native GUI rendering, scripting,
player OSD, and standalone diagnostic output are outside scope.

- [x] Embedded/custom bundle compiler, source diagnostics, typed entry schemas,
  template reuse, cascade/custom properties, flex/grid cell allocation,
  clipped scenes, and width-first text fragments.
- [x] UI-thread renderer ownership; shared form composition and semantic
  label/value/annotation rows; log header/body/footer and source anchors.
- [x] Local discovery, CLI export/check, debounced background compilation,
  stale/full-channel handling, and embedded F12 recovery tools.
- [x] Shared inline flows, typed rich spans, source/action paint records, and
  centered recovery overlays.
- [x] Log control fields, attached dropdown overlays, and semantic option rows.
- [x] Template-authored keyed repetition for small semantic lists, item-scoped
  binding validation, stable geometry identities, and form tabs/notes.
- [x] Authored virtual repetition for Users, Playlist, and Series, bounded
  item windows, sparse conditional rows, and shared dialog visibility/navigation.
- [x] Resolve terminal styles before painting, remove the production frame-wide
  theme pass, and share CSS precedence across semantic/primitive boundaries.
- [x] Chat/input/suggestion composition, recent projection, semantic separate
  subtitle rows, and timestamp-aligned bordered attachments with original scroll crops.
- [x] Chat rich message variants, authored first-line prefixes, source-mapped
  selection/spoiler actions, virtualized live tail, and paint-only animation caching.
- [x] Root composition, markup focus order, painted hit/splitter geometry,
  revision-keyed local drag persistence, and one-time PaneLayout import.
- [x] Users and Playlist semantic rows/fields, measured collection virtualization,
  and shared paint/hit geometry for wrapped items.
- [x] All Series modes, semantic List columns, and measured filter captions.
- [x] File/episode browsers, AniDB/Nyaa searches and imports, local-copy rows,
  confirmation/name dialogs, template modal sizing, and nested slot paint order.
- [x] Roguelike frame captions/notices/error and game-page composition, including
  labeled statistics/supplies and file-only sidebar relocation.
- [x] Roguelike recovery panel, inspection headings, equipment rows, documents,
  and expedition endings.
- [x] Changelog semantic entries and anchored document scrolling; status fields
  and repeated keybinding entries; authored root dimensions and outer margins.
- [x] Semantic health/progress fields with measured priority budgets, form modal
  entry sizing, Save/error templates, raw action labels, and slot-layer editor composition.
- [x] Shared log/dungeon/recent-chat page shell, typed work-progress overlays,
  intrinsic modal height, and explicit terminal cell rounding.
- [x] Roguelike condition/source navigation, measured bounded-summary children,
  semantic journal projections, and shared guide/journal document scrolling.
- [x] Remove production legacy list/table and split-layer adapters; audit pane
  root visibility/margins, selected CSS precedence, inline inspection, and
  cross-pane overlay/image geometry.
- [x] All six file-only demonstrations through real filesystem events and
  runtime bundle installation; atomic saves, relative paths, missing-directory
  creation, and whole-directory replacement.
- [x] Final automated compiler/geometry/interaction/image/reload audit: 1,522
  regular tests and 1,523 release-full tests passed, including performance checks.
- [ ] Physical terminal graphics protocol smoke checks using `layout_smoke`.

The current usable authoring subset, examples, and migration limitations are
documented in [ui-layouts.md](ui-layouts.md). Do not mark this phase complete
until all six demonstrations work with live reload and no Rust edits.

## Phase 37: Houseguest phase 3 — the room

**Status: done (2026-09-29).** Brief written 2026-09-28 at the end of
phase 2; progress notes below record each step. Design: [the houseguest
proposal](proposals/2026-09-28-houseguest.md) (sections *The room model*,
*Progression*, *Furnishings*, *Persistence*, *Routine and calendar*);
rules as built: design.md, Houseguest. Phases 1–2 are done: terrain,
line art, text layer, pulls, swaps, sneezes, speech, the needs brain,
and the stage.

### Goal

Osaka gets a home that outlives a visit: furniture she acquires, placed
around the panes, that gives her new things to do — and a record so the
room is still there next time.

### Settled with the user (2026-09-28)

1. **First items and sizes**: sofa 9×3, TV 6×4, bed 10×3, desk 7×3.
2. **Where props stand**: on floors she can reach, over blank cells
   only, in the quiet panes (Users, Playlist, a short List); never in
   the chat. A prop that no longer fits goes to the closet.
3. **Acquisition**: on an early visit a box brings the TV; after that,
   Chiyo-chichi's shopping channel on the TV sells her one item at a
   time.
4. **Cadence**: about one item per three visits; now and then she leaves
   for her part-time job, and the furnished room stands empty for 1–3
   minutes before she returns with a bag.
5. **Reset**: "Osaka moved out" is a row under Houseguest in F3 →
   Playback, with a confirmation; it wipes the ledger.

**Creative licence.** The proposal, the original screensaver and The
Sims are inspiration, not a spec. New scenes, props and gags are
welcome; the only goal is "watching her is kind of fun".

### What gets built (suggested order; each step testable in the stage)

1. **Anchors and one prop.** `room.rs`: `Anchor { surface, edge, frac,
   offset }` resolved against the current frame each paint (a pane's
   floor, found the way terrain finds floors); a prop whose anchor no
   longer resolves, or whose cells aren't blank, goes to the closet
   (hidden, still owned). One hard-coded sofa as coloured, unfilled line
   art (`art/` SVG, same resvg path as her). Stage: a key that gives
   her the sofa.
2. **Compositing.** Where she and a prop overlap, *one* image draws both
   in scene depth order (she sits in front of the sofa; under the bed
   covers; legs under the kotatsu), because placeholders from two images
   would cut each other out. The frame cache key then includes the prop;
   keep an eye on the number of distinct images (image-lifetime note in
   the proposal's Open questions).
3. **Props as offers.** Each prop advertises `Kind`s to the brain
   (`brain.rs`): sofa → sit/nap there, bed → proper sleep (answers
   *sleepy* far better than lying on a border), desk → homework, TV →
   watch. Adding a prop adds behaviour without touching `decide()`.
   Worth introducing an `Offers` struct here, which also removes the
   `#[allow(clippy::too_many_arguments)]` on `Osaka::start`.
4. **Terrain from props.** Decide whether a prop's top edge is a floor
   (standing on the TV, peering off the bed) — terrain is read from the
   rendered frame, and props are in the frame, but as images, not line
   glyphs.
5. **The ledger.** `ledger.rs`: local, never synced, same tier as
   `layout_sizes` and the roguelike save; JSON with a version and
   `#[serde(default)]`; owned props with anchors, closet, `visit_count`,
   `master_seed`, seen scenes, last-fired times. Visit RNG from
   `(master_seed, visit_index)`.
6. **Acquisition** per the user's answer to 3; Chiyo-chichi's shopping
   channel on the TV; a box sliding in and unpacking. **Room mutations
   commit at scene start** (the proposal's rule): the prop is owned
   before the delivery animation plays, so an interrupt loses nothing.
7. **Absences**: she leaves for her job and the room stands furnished.
8. The rest of the catalogue, ~8 props placed at most.

### Progress

- **Step 1 done (2026-09-28).** Art for all four pieces in
  `art/props.svg` (filled, reviewed on `props_sheet`:
  `HOUSEGUEST_PROPS=/tmp/props.png cargo test -p dessplay --lib
  props_sheet -- --ignored`). `room.rs`: `Furniture`, `Nook` (List,
  Users, Playlist, carried in `IdleView::nooks`), `Prop { item, at
  (‰ along the bottom border), facing }`, `Home::resolve` / `spot`.
  Pieces carry a `RoomKind` (living: sofa, TV; bedroom: bed, desk); a
  room lives in one pane (`Home::rooms`) and moves whole when its pane
  can't hold it. Later: she makes space for her things herself.
  Pieces are solid (their footprint and floor join the protected set)
  until step 2's compositing; one that would land on her waits in the
  closet. `Look::Prop` goes through the same `Graphics::paint` as her;
  ASCII drawings double as the goodbye's noise. Stage: `f` gives her the
  next piece. The `Home` lives on `Guest` (in memory until the ledger).
  SVG work goes to fork agents (the user's suggestion, to save context).
- **Steps 2–3 done (2026-09-28).** `graphics::Layer` and
  `Graphics::paint_layers`: one image over the union of several layers
  (her single paint is the one-layer case). Pieces are solid to text
  only; she walks in front, and the pieces her box overlaps are
  composited with her (`draw_art`; `art::Layer` Whole/Back/Bare/Front
  picks the bed's quilt over her, the sofa without the cushion she
  hugs). Uses (`room::Use`: Lounge, Nap, Sleep, Homework, Watch) are
  `Seat`s in `Chances::seats`, brain offers `Kind::Use`, and a
  `Job::Use` walks her there; `Act::Use` holds the pose. New poses
  (`Pose::Lounge`, `Nap`, `Sleep`, `Homework`) with rigs from the fork
  (`use_sheet`: `HOUSEGUEST_USE=/tmp/use.png cargo test -p dessplay
  --lib use_sheet -- --ignored`; seated legs are their own parts). Stage
  scenes for each use give her the piece if she lacks it. No `Offers`
  struct yet: seats ride in `Chances`, so `decide()` still grew one loop.
- **Step 5 done (2026-09-29): the ledger.** `houseguest/ledger.rs`:
  `Ledger { master_seed, visits, home }`, JSON v1 under settings key
  `houseguest`, unknown entries skipped, unreadable records kept (the
  guest runs with `keep_unsaved`). `run.rs` loads it into
  `Ui::houseguest_ledger`; the shell builds `Guest::restore` from it and
  sends `UserAction::SaveHouseguest` whenever `ledger_to_save` has one.
  F3 → Playback → "Osaka moved out" confirms, then `Msg::HouseguestMovedOut`
  → `Ui::houseguest_moved_out` → `Guest::move_out`. Not yet recorded:
  seen scenes and last-fired times (phase 4 rarity; `serde(default)`
  makes adding them free). No changelog entry yet: until she can get
  furniture on her own there's nothing for a user to keep.
- **Step 6 done (2026-09-29): deliveries and the shopping channel.**
  `Prop::boxed` and `Ledger { ordered, bought_on }`. The TV is ordered
  on visit 2 (`FIRST_TV_VISIT`); `advert()` offers the next of
  `CATALOGUE` when she watches ≥ `SHOP_EVERY` visits after the last
  purchase, nothing on order or boxed. `Osaka` reports `HomeEvent::
  Bought` (at the scene's start) and `Unpacked` (at its end); the guest
  records them. `furnish` delivers an order from an earlier visit as a
  boxed piece; `Use::Unpack` (base 40) is the only use of a boxed piece.
  Art: `render_parcel`, `render_tv(Channel::Snow | Shopping)` in
  `art/delivery.svg` (`delivery_sheet`). Stage: "a parcel", "shopping
  channel". Tests: the whole progression over twelve visits across a
  restart; an interrupted unpacking waits. Not built: the box sliding in
  (it appears in place), absences (step 7).
- **Step 8 done (2026-09-29): the rest of the catalogue.** Lamp,
  bookshelf, fridge, cat bed (8 pieces in all, the proposal's "~8
  placed" by construction); `RoomKind::Kitchen`; uses Read, Snack
  (answers the new *hungry* need), Pet. `art::PieceState` (lamp off,
  fridge open, cat, cat biting) through `Looks` per frame and
  `piece_state`; the cat is home on about half the visits, read off
  the visit seed (`cat_home`). `roomy()` now also requires a standing
  spot beside a piece used from beside it. Art and the reading, eating,
  petting rigs by a fork agent (`catalogue_sheet`). Not built from the
  proposal: the window (a wall piece; floors only so far), the kotatsu
  and seasonal closet rotation (with the calendar, phase 4), the
  credits roll when the catalogue is complete.
- **Step 7 done (2026-09-29): her part-time job.** `Kind::Work` (base
  9, offered with a home, once a visit, after `WORK_AFTER_MS`);
  `Osaka::go_to_work` leaves by an `Around` link or a `Door` with a
  `gap` of `SHIFT_MS`; `at_work` makes the return `Act::Home`
  (`Pose::Carry`, "I'm home!"). `look()` ignores chat while she's out.
  Carry rig by a fork agent: a bundle of leeks cradled across her chest
  (after the user's reference, ~/leek.webp; a first try with a shopping
  bag read as a leek growing out of her shoulder). Stage: "part-time
  job". The leeks are only for show so far — a natural hook for the
  proposal's *hungry* need (food).
- **Pits fixed (2026-09-28, the user's call: all three).** Terrain
  links `Route::Clamber` (over a divider, floors sharing no standing
  spot) and `Route::Around` (out at one screen edge, back in at another);
  `Act::Door` (keyframed, `osaka::DOOR`) when `go_to` finds no route or
  her floor has no links (Travel is always offered). Door art in
  `art/door.svg` (`door_sheet`). Stage scenes: clamber over, step out,
  door in space. The stage room's chat turned out not to be a pit at
  all once clambering existed (its lines are short); `pit_screen` in the
  tests is a real one.

### Testing

- Anchors survive arbitrary resize sequences (property): a prop is
  either placed on blank, unprotected cells or in the closet, never
  half-drawn over text. Extend `long_visits_never_touch_what_is_protected`
  with owned props.
- Composited frames: her image never shows a prop's placeholders cut out
  and vice versa; the frame cache stays bounded.
- Ledger: round-trip, version mismatch, unknown items ignored, missing
  file.
- Brain: a prop's offers appear only while it's placed; the needs
  statistics test gains "sleeps in the bed when she has one".
- Stage: every new scene cueable and covered by
  `every_scene_has_a_spot_in_the_stage_room`.

### Handoff notes from phases 1–2

- **Tools.** `cargo run -p dessplay --example houseguest [seed]` (the
  stage: scenes, needs keys 1–4, speed `[`/`]`, decisions logged to
  `houseguest-stage.log`); `HOUSEGUEST_SHEET=/tmp/sheet.png cargo test -p
  dessplay --lib model_sheet -- --ignored` (every pose, both facings);
  `cargo nextest run -p dessplay --lib reach_census --run-ignored only
  --no-capture` (how often chat offers her something); `perf.rs`
  `houseguest_visit_cpu_is_negligible` (release, `--profile full`).
- **Read before paint.** Everything that compares against the real
  frame (layer validation, terrain, the dissolve) reads it before any of
  her pixels or glyphs go on; props must follow the same order.
- **Placeholders hide what's under them.** Her whole box must be blank
  or redrawable lines, which is why pulls need a line sticking out
  beside her. Going underneath ratatui-image (cell-level transparency,
  then true translucency) is noted in the proposal as later work; don't
  start it inside phase 3 unless the user asks.
- **Mischief undoes itself on a schedule** (`Osaka::pending`), separate
  from her act; anything a prop scene changes in the text layer should
  schedule its own undo the same way.
- The user reviews in Ghostty and gives art feedback from the model
  sheet and the stage; show them the sheet for any new art before wiring
  it in.


## Phase 38: Houseguest mind and home

**Status: phases 0–4 done (2026-10-02 and 2026-10-03); phase 5 split into 5a (vignettes, done 2026-10-03) and 5b (the clock, done 2026-10-05; record below). Phase 5c, stillness, done 2026-10-07 (record below). master was pushed by the user at 5b step 5c (2026-10-04) to test it, and again at 5c step 11's record (2026-10-06); everything after that record is unpushed (step 11's review fixes, D7's docs, steps 8c–13 and the door art). Next: the door batch (below), then phase 6.** Design: [the mind
and home proposal](proposals/2026-10-02-houseguest-mind.md) (direction
agreed with the user; their answers are its *Decisions*). Its migration
plan numbers its own phases 0–8; this section records them.

### Phase 0 — the sofa class (done 2026-10-02)

- **Exit commits.** Home events are recorded right after the tick that
  produced them (`record` in mod.rs, shared with paint), so a visit
  ending before the next paint can't drop an order or an unpacking
  (`commits_survive_the_visit_ending`).
- **The nap.** A made piece in shape offers the real piece's uses; a
  nap answers *sleepy* (0.1) with a neutral floor (`for_its_own_sake`),
  a bridge until the proposal's comfort need
  (`sleepy_she_naps_on_the_sofa_she_made`).
- **Purposes on things.** `MadeId` (in `Scrap`), `PieceRef` on `Seat`
  (no more `makeshift: bool`), `Made { purpose, used }`, `Build.then` is
  a `Use`, `HomeEvent::{Crumpled, Used}(MadeId)`, `Chances.mine`;
  `Osaka::making` is gone. Continuation (`Osaka::leftover`): after the
  reflexes and the goal, nearest first, up to `TRIES` = 3 (a try per
  setting-off or failure to bind; crumpling resets them). Goals for
  another floor resolve by meaning (`Chances::offered`); `look()` drops
  a goal for a made piece so the return is a counted try.
  Tests: `an_interrupted_crumple_keeps_its_purpose`,
  `every_made_piece_is_used_or_let_go` (property; ~11 s at the gate's 32
  cases, the suite's long pole).
- **Found on the way.** Chat mid-clamber dropped her off the pole
  (`aloft()`; `chat_mid_clamber_doesnt_drop_her`). The 256-case runs
  found two more (saved as proptest regressions): `elsewhere` sampled
  one spot per floor and could miss every calm one; her frozen
  composite listed box cells her clipped image never drew, so a focus
  rain fell on a pane's border.
- **TV and layout.** The TV's floor is where she'd stand beside it;
  colliding pieces are laid side by side (`layout`); sofa build sites
  facing the TV weigh 5 (`pick_build`).
- **Not done from the phase-0 list.** The three pinned "37 s chat"
  bench cases: the bench and its logs weren't kept (sofa-diagnosis.md),
  so the property covers the class instead. `seed_7` needed no re-pin.

### Phase 1 — pure refactor (done 2026-10-02)

- **Golden trajectories first** (`houseguest::tests::golden`): four
  scenes × seeds 0–3 × {ASCII, kitty}, ~3 s at the gate. The scenes go
  beyond the proposal's list where it covered too little: the stage room
  is cued through swap, sneeze, pull, shopping, a bed and a parcel while
  chat arrives (Sneeze and PutBack never came up naturally), and the
  resident is cued at mischief in the chat and caught at it by a key
  press. A kitty image's id is random, so image cells hash by position;
  her appearance and image placement cover what they show.
  `HOUSEGUEST_GOLDEN_TRACE=<dir>` writes per-frame traces to diff.
- **`ActProps`** (`Stays`: job / rest / pass; `OnChat`: look / once
  landed / once back): `recheck`, `look`, `settle`'s first arm and
  `aloft` read it. `evict`, `errand`, `hidden`, `door` and the blink
  stay per-act matches: they act on payloads, not classify.
- **The job in the act.** `Walk { then: Then }` (`Nothing`, `Link`,
  `Job`); Pull, Tear, Swap, Giggle, Innocent and Use carry their own
  job type; `Use.what` is its seat's. `task` is gone, and with it the
  "no job after all" fallbacks. `JobRef` borrows a job's spot, side and
  hands row. `lost_grip` keeps the broad check (a walk's job too): a
  tick far behind can re-pick a pull before the paint reporting the
  lost grip.
- **`interrupt(Cause)`**: chat, restless, seat gone, lost grip, shaken,
  refused; the only builder of a Look. It clears `rest` for every cause
  (only `recheck` did); a stale one is unreadable before the next
  `decide` (commit d0f29db0 says why).
- All three steps left every golden hash unchanged; the gate passed
  after each.

**Checkpoint (2026-10-02).** The 37 s-chat bench of the diagnosis wasn't
kept, so `sofa_census` (ignored; `--release --ignored --nocapture`)
stands in, kept for later checkpoints: the stage room cued to make a
sofa, 100 seeds × {ASCII, kitty}, five-minute visits, chat every 37 s in
a phase shifted per seed (so it lands anywhere from the tear to the
sitting), or none.

| Chat | Mode | Made | Used | Made → used (median / p90 / max) | Let be | Lost | Waiting at the end |
|---|---|---|---|---|---|---|---|
| none | ASCII | 100 | 100 | 6 / 7 / 7 s | 0 | 0 | 0 |
| none | kitty | 100 | 100 | 6 / 7 / 7 s | 0 | 0 | 0 |
| 37 s | ASCII | 99 | 98 | 6 / 24 / 27 s | 0 | 0 | 1 |
| 37 s | kitty | 93 | 93 | 7 / 24 / 28 s | 0 | 0 | 0 |

The one waiting piece was started at 294.8 s and still being crumpled
when the visit ended. Fewer than 100 made with chat is most likely a
tear interrupted mid-reel, whose text goes back by design (not counted
separately; a cue finding no site that frame is the other possibility). Waits of ~25 s are a look, the
15 s watch and the walk back. The 256-case houseguest run passed (149
tests, 40 s, release), and so did the perf test. **The sofa class no
longer leaks; phase 2 needn't start by closing it.**

### Phase 2 — the mind as data (brief)

Proposal: *Choosing what, and how*, *Every want answers a need*, *Her
mood for the visit*, *Interruption and intent*, and the migration row.
Two steps, each its own commits:

1. **The table conversion, with today's numbers.** `Want`/`DesireDef`
   with named factors; the method table (ordered methods, pure guards,
   first that binds); `Heading` replaces `goal`, the one intent field
   still outside the act (×3 inertia in the roll; dropped with a
   glance); beats at the loss sites of the proposal's table, where
   `interrupt(Cause)` is now the single site for those that stop her
   short; credit by completed fraction (Q4); the mind's own rng stream
   (`visit_seed ^ MIND_SALT`, one draw per decision) and whims; the
   explain log; the lints. The golden hashes guard the behaviour-preserving
   commits (the table, the explain log); each behaviour-changing commit
   (methods and the mind stream, the heading, beats, credit by fraction)
   re-records them from the table the failure prints, with the reason in
   its message. Through the method table the statistics tests must pass
   unchanged (a failure is a conversion bug); only credit by fraction
   may move them, and a test that moves there waits for step 2's
   simulator rather than a hand re-pin.
2. **Needs for every want and the visit's mood**, tuned in the headless
   simulator (brought forward from phase 5) before any statistics test
   is re-pinned.

Then re-run `sofa_census` and the 256-case pass, and record them here.

### Phase 2, step 1 — the mind as data (done 2026-10-02)

- **The want table** (`brain::Want::def`, `DesireDef`, `Factor::InChat`;
  golden unchanged) and **the explain log** (`osaka::Decision`: bucket,
  method, want, top four, act, heading; the stage's `x`; golden
  unchanged).
- **Methods** (`mind.rs`): named, pure guards per want, first that
  binds; offers are what binds, so `start`, `places_for`, `Place` and
  the "couldn't after all" re-rolls are gone. `use/finish-my-heap` and
  `use/mine` come before real, made and new pieces (today's uniform pick
  among those is kept, so a made sofa facing the TV is still watched
  from). No method for `Use(Crumple)`. The mind's stream: one draw a
  decision, `Whims` hashed by label (FNV-1a + splitmix). Found on the
  way: `Want::ALL` lacked `Work` (now a test). Four tests assumed seed 7
  shows her at 95 s; they now step on until she's on screen
  (`run_until_seen`), and seed 7's snapshot is re-pinned. The statistics
  tests passed unchanged, but `a_sleepy_osaka_mostly_lies_down`, whose
  win was a tie broken by the old draws (now `..._mostly_dozes`).
- **The heading** (`mind::Heading`): the want and the job as she set
  off; text by glyphs in the same columns, up to `SCROLLED` = 4 rows
  above; ×3 inertia (`brain::INERTIA`); `drop_heading` the one remover;
  a heading toward a made piece stays a certain hop (continuation counts
  tries). Errands no longer clear it.
- **Beats** (`mind::{Loss, Beat, Lines}`, `Act::Glance`, `Osaka::owe`):
  the owed bucket after the reflexes; lines with a ten-minute cooldown
  and eight a visit. Loss sites: `tend_made` (an unused piece), the reel
  put-back, `drop_heading` (gone or other), the let-be after `TRIES`.
  "Ah, right!" on going back after an interruption.
  `a_lost_sofa_is_mourned`; the made-pieces property requires every
  unused loss to be mourned.
- **Credit by fraction** (`Osaka::credit`, `credit_done`): idles and
  uses by time share, pulls by offset/goal (whole when finished), swaps
  when made; walking and travel at the choice; continuation carries its
  purpose's credit. `an_interrupted_sleep_eases_only_what_she_slept`
  (fails under credit-at-choice). No statistics test moved.
- **Lints**: fixed lines fit a bubble; every want with a spot can be
  cued from the stage (Stand and Walk are the fillers); a `debug_assert`
  after every step that queued mischief has its undo scheduled (trips
  when a swap's restore is removed). "Acts over 2 s declare Stay or
  Pass" is `ActProps`' exhaustive match.
- Golden hashes re-recorded at each behaviour-changing commit (methods,
  heading, beats, credit). After credit: 256 cases pass (157 tests);
  `sofa_census`: no chat 100/100 used (both modes); chat every 37 s
  90/90 and 94/94 used, made → used median 7 s, max 28 s, none let be,
  lost or waiting.

### Phase 2, step 2 — needs for every want, moods, the simulator (done 2026-10-02)

- **The simulator** (`tests/census.rs`, `visit_census`, ignored,
  release; `CENSUS_MOODS=1` forces each mood): 30-minute visits in the
  stage room, a furnished home and a resident's room, 16 seeds: choices,
  time by act and by group (furniture, floor rest, spacing out, moving,
  mischief, exercise, standing), needs, beats, lines, headings. Its
  first run found the heading rolling at every hop (22 of 135 trips
  arrived at home): a landed hop now carries on
  (`an_uninterrupted_trip_runs_its_course`).
- **Needs**: comfort, fun (per-source tolerance, `Needs::fresh`,
  `enjoyed`), daydreams; `Needs` is an array by `Need`; new needs start
  at 0.5. Every want but Stand and Sneeze answers one. `brain::Spot`
  (real, made, floor) goes with each offer to `score` and to credit;
  `quality(need, spot)`. Fit = 0.1 + Σ need² × amount × 2 × quality
  (× freshness for fun): the proposal's formula with amounts, scaled so
  0.5 weighs a whole need, after the amount-free first cut let
  two-need wants win (homework 15% of choices at home). Floor dozes
  base 4. A parcel and work keep the 0.5 floor.
- **Moods** (`brain::Mood`, drawn in `begin_visit` from the visit seed;
  `Osaka::set_mood`; greeting by mood): shares 50/20/20/10 as proposed;
  rates in design.md.
- **Census, forced moods** (furniture / floor rest / spacing out /
  moving, % of time):

  | Room | Ordinary | Lazy | Industrious | Dreamy |
  |---|---|---|---|---|
  | stage | 3.8 / 3.6 / 7.4 / 36.6 | 15.1 / 1.3 / 6.1 / 35.8 | 1.7 / 1.4 / 5.4 / 39.2 | 2.6 / 3.7 / 10.9 / 38.5 |
  | home | 37.8 / 0 / 2.1 / 42.8 | 43.2 / 0 / 1.1 / 39.9 | 33.1 / 0 / 1.6 / 46.0 | 38.3 / 0.1 / 3.3 / 41.7 |
  | resident | 28.8 / 0.8 / 5.4 / 36.2 | 42.3 / 0.6 / 5.2 / 26.0 | 24.0 / 0.4 / 4.2 / 40.6 | 27.9 / 2.6 / 9.9 / 33.0 |

  At home her choices spread across her things (bed 12%, TV 11%, books
  10%, sofa 9%, homework 7%); no want takes more than 23% of a home visit
  (45% in the text-heavy stage room, pulls). With drawn moods: 256 cases
  pass (164 tests); `sofa_census`: no chat 120/120 and 104/104 used,
  chat every 37 s 102 of 104 and 93/93 (the two were started in the
  visit's last 15 s, still being crumpled), made → used max 28 s, none
  let be or lost.
- **Tests**: `at_home_her_furniture_beats_the_floor`, `her_mood_shows`
  (fails with every rate 1), `fun_wears_thin_and_freshens`,
  `moods_come_in_their_shares`, `a_sleepy_osaka_goes_to_bed` (was
  `..._mostly_dozes`: the floor is a poor answer to sleep now). No other
  statistics test needed re-pinning.
- **Left for later**: beauty and nesting (phase 4, with decor and
  rules); the mood's home-act cap (phase 4); the industrious mood's
  tidying shows little where text keeps tidy high. **For phase 5's line
  pools**: the simulator hears "Where was I?" (every door's end) 467
  times in 16 half-hour stage visits, about once a minute; it should
  join a pool with a cooldown, as the proposal's Character section says.

### Phase 3 — the home model (done 2026-10-02)

Built as briefed, in six commits, with these differences:

- **`Spec`** (room.rs): name, pitch, footprint, ASCII, ink, uses, where
  she sits (`sit`), offers, comfort (all 1.0; `Spot::Real` carries the
  piece and `quality` reads it). `art_id` was unused and went; art.rs's
  layer and state tables are composition and stayed. Golden hashes
  unchanged.
- **Strips, anchors, `project()`**: `Strip::Bottom(Nook)`, `Anchor { side,
  offset }`, `Prop { strip, anchor: Option, at, .. }` (no anchor until
  an older record's strip is first seen), `Extent`, `pack` in
  `order_key` order. A strip that's gone or can't hold its pieces moves
  them into any strip that holds them with its own (not only a free
  pane). Every frame of all 32 golden runs was diffed against the
  parent's traces: identical. The golden trace now hashes the record as
  saved (JSON), not the in-memory `Debug`.
- **The record**: `anchors` (item, strip, anchor) as a new top-level
  field; `rooms` derived for older builds from a ledger-private copy of
  the old kinds, a distinct pane each. Golden-file tests: the record as
  written, an older record read, what an older build reads of ours, an
  unknown strip.
- **Rooms**: `Offer`, `Role`, `ROLES` (first row met; no `base` or
  `min_width` yet, nothing needs them), `Home::rooms`; the stage's `x`
  row shows them. `RoomKind`, `Furniture::room`, `Home.rooms`,
  `strip_of` and room-bound placement are gone; `add` refuses only a
  second of a kind.
- **Deliveries** (the user's call, asked before step 3): through a
  **flap** in a strip's wall at the screen's edge (`Home::doorstep`,
  `Flap`, `draw_flap`, 800 ms), the parcel against the wall facing in,
  the strip's pieces making way only where all that show still fit.
  Anchor ties put the newer piece nearer its wall. The stage's gifts go
  anywhere they fit, at random.
- **`Faces`** (the user's call): same strip, gap 2–14 (`FACING_GAP`);
  facing counts from phase 4. Made sofas keep the same-floor rule.
  The test's split-floor case fails with the old condition put back.
- **Tests moved**: `a_furnished_home_gets_used_and_stays_cheap` asks
  that each piece is used (lounge *or* nap: naps were 5 in 8 twenty-
  minute visits before, 3 after, too rare for two seeds);
  `a_tv_anywhere_on_the_sofas_floor_is_watched_from_it` became
  `a_sofa_facing_the_tv_is_watched_from` (gaps in and out of range,
  both walls, a split floor). New: `packing_keeps_order_and_a_resize_and_back_restores`
  (property), `moved_pieces_may_join_another_rooms_strip`,
  `roles_come_from_contents`, `a_new_piece_goes_where_it_fits`,
  `a_parcel_comes_in_through_a_flap_at_the_screens_edge`.
- **Measured**: gate 1769 tests; 256 cases pass (172 tests); perf
  passes. `sofa_census` as after phase 2 (no chat 120/120 and 104/104;
  chat every 37 s 102 of 104, the two waiting, and 93/93; made → used
  max 28 s). `visit_census`, forced moods (furniture / floor rest /
  spacing out / moving, % of time):

  | Room | Ordinary | Lazy | Industrious | Dreamy |
  |---|---|---|---|---|
  | stage | 3.8 / 3.6 / 7.4 / 36.6 | 15.1 / 1.3 / 6.1 / 35.8 | 1.7 / 1.4 / 5.4 / 39.2 | 2.6 / 3.7 / 10.9 / 38.5 |
  | home | 38.6 / 0 / 2.2 / 42.0 | 44.3 / 0 / 1.2 / 38.7 | 33.8 / 0 / 1.3 / 44.8 | 40.4 / 0 / 3.6 / 39.6 |
  | resident | 24.5 / 0.8 / 5.1 / 41.7 | 32.4 / 0.5 / 5.1 / 37.1 | 19.8 / 0.4 / 4.4 / 44.0 | 24.3 / 1.8 / 8.1 / 39.8 |

  The stage room owns nothing, so it's unchanged. The home is within a
  point or so. The resident's furniture time fell (ordinary 28.8 →
  24.5, lazy 42.3 → 32.4): its sofa and TV are gifts, which now land
  anywhere, so they're often on different strips or too far apart to
  watch from the sofa. Phase 4's repair is what should bring that back;
  measure it there. With drawn moods, home choices: bed 15%, TV 10%,
  books 10%, homework 8%, sofa 8% lounging and 4% napping; no want
  over 24% of a home visit.

### Phase 4 — organising (brief)

Proposal: *Rules, not an objective*, *One repair a visit, prompted by a
grievance*, the nesting and beauty rows of *Every want answers a need*,
the mood's home-act cap, and the migration row. Phase 3 left these
hooks: `room::faces` (turn on its facing check here), `Home::rooms`
(roles), `order_key`/`pack`/`project` (a move is a new anchor,
projected on a copy), `Home::doorstep` and `Flap`, the `Spec` row (add
beauty and decor kinds there). In order, each against the gate:

1. **Rules as a table** (`Rule`, `RuleRow` with its grievance line),
   judged on the projection: `Faces` now with facing, `Near` (the lamp
   by the bed or the desk), `AgainstWall` (fridge, bookshelf: offset
   ≤ 1), `Apart { Bed, Tv }`. `Connected` waits for phase 6. A rule is
   broken or not; a satisfied rule never moves anything. Property: no
   move the repair makes breaks a satisfied rule.
2. **Unplaced deliveries.** Phase 3 sets every parcel against a flap,
   so rooms mix (a bed beside the TV, a sofa flush against it). Add a
   rule, or a flag persisted per piece (a new top-level ledger field),
   that a piece never yet set down by her is unsettled; the repair's
   cost then prefers, in order, a room whose role the piece completes
   (the TV joins the sofa), a room it doesn't spoil (no bed in a living
   room), an empty strip. These are the role tiers proposed for phase 3
   and set aside for the flap.
3. **Feeling a rule**: using a piece a broken rule names plays its
   grievance as an interlude ("Can't see the telly..."); a felt rule
   is remembered for the visit (and across visits? decide, default:
   the visit). **Nesting** rises only while a felt rule is broken.
   `Want::Arrange` (base 6) is offered then; the mood's cap (0–3 home
   acts a visit, by mood) gates it.
4. **The repair search**: each piece the rule names, every anchor on
   every strip, both facings, one piece at a time, on a copy projected
   with `project`; qualifies if it satisfies the rule, breaks no
   satisfied one, and fits blank cells snapshotted at paint; cheapest
   (fewest cells moved, then a whim). Budget about 2,000 candidates,
   ≤ 1 ms: measure in the perf test.
5. **The pocket carry**: methods `arrange/lift`, `carry`, `set-down`,
   `use-it`; `HomeEvent::SetDown { piece, anchor, facing }` committed
   at paint if it fits (else the old anchor stands); dropped carry,
   eviction and goodbye as the proposal says. Trials (up to three
   spots, keep with exp(−Δ/T)).
6. **Decor and beauty**: decor kinds as `Spec` rows (a potted plant, a
   poster; the gremlin waits for shelves), beauty as a room's summed
   decor, the channel selling decor.

Re-run `visit_census` (add a row for how many rules stand broken at
each visit's end, and home acts per visit by mood) and `sofa_census`,
and record them here.

### Phase 4 — organising (done 2026-10-03)

Built as briefed, in 15 commits (and two of docs), with these
differences. The working design, with its two rounds of amendments, is
[phase4-design.md](proposals/2026-10-02-houseguest-mind/phase4-design.md);
the approved decor art is in its `decor/` dir.

- **Layout apart from what fits** (`Home::layout`, a refactor first,
  goldens unchanged): the rules are judged on where her pieces are laid
  out on their strips, not on what shows, so text closeting a piece
  neither breaks nor mends one.
- **Faces with facing** (its own commit, before the rules): the watch
  seat takes the sofa's way round; a made sofa keeps the derivation.
- **The rules** (`rules.rs`): `Faces`, `Near` (lamp by bed or desk, ≤ 3
  cells), `AgainstWall` (fridge, bookshelf, ≤ 1 cell), `Apart { Bed, Tv }`
  and `Belongs` (an unsettled piece that spoils its room). Felt on the
  piece's own uses only: Faces on lounging and napping (not on watching
  from beside the TV), Apart on sleeping.
- **Unsettled deliveries**: a top-level ledger field `unsettled`, written
  only when non-empty and read leniently. Only doorstep parcels are
  unsettled; the stage's gifts (and so the census's homes) are settled.
  The brief's ranking became role tiers (a room the piece completes 0,
  one it doesn't spoil 1, an empty strip 2) and, where the rule would
  move an unsettled piece too, a settled piece behind them all (tier 3,
  `SETTLED_BEHIND`); a turn where it stands is exempt and cheapest.
- **The search runs at paint, not in her decision** (a deviation from
  the proposal): one definition of what fits, the frame's, throttled
  (again when her home, the panes, the rules she'd mend or the text she
  moved change, else once a second). The set-down is committed at paint
  too, judged again with the piece moved. Lift and set-down end in
  short timed acts, and each step waits while the frame hasn't judged it.
- **The carry**: tries count failures, not settings-off (a busy chat
  made her drop a piece she was carrying fine); one go per felt rule.
- **`Want::Arrange`** landed with `Scene::Arrange` (step 6), not with
  the feeling (step 4), for the exhaustive cue lint.
- **The image cache** drops the image shown longest ago instead of
  starting over (below).
- **Trials** shipped (below): whim order, a turn isn't another spot, the
  line is "hmm..." in ASCII dots.
- **Wall pieces** hang 4 rows up (the design's HEIGHT + 1 needed 7 clear
  rows; the home screen's Users pane has 7); a crowded wall hangs each
  piece that fits.
- **Beauty** is eased by rest and use in a pretty room, not exercise or
  chores; the channel sells decor first on a tie at the top of her
  needs (needs clamp at 1, where restlessness often sits).
- **Found by the census** (below): the search ran only for the first
  felt rule, so one no move mends blocked the rest of the visit
  (`fix(houseguest): what she can't mend doesn't stop her mending the
  next thing`).

**Image budget with a carry (step 6; design T4).** Before trials were
decided, `a_carry_stays_within_the_image_budget` (line art, Industrious,
nesting pressed, text-dense panes) counted the images encoded and the
times the frame cache (`CACHE_LIMIT` = 256) filled and started over
(`Graphics::counts`, test-only).

| Home | Set down at | Encoded from feeling it | Lift → 5 min after set-down | Cached | Clears |
|---|---|---|---|---|---|
| sofa turned from the TV (turn in place) | 152.6 s | 107 | 32 | 116 | 0 |
| unsettled TV carried downstairs | 139.0 s | 120 | 41 | 130 | 0 |

A carry costs some 30–40 distinct images (the piece hidden, then shown
again, under her and the poses around it). With trials and the trial
sit (measured again at the end), the same two homes encode 136 and 129
from feeling it, 61 and 50 from the lift on, with nothing dropped.

**The cache (step 6b review).** The furnished home
(`a_furnished_home_gets_used_and_stays_cheap`, ordinary, seeds 0 and 1)
caches 195 and 190 images in 20 minutes. Forced industrious over seeds
0–7 (`a_busy_furnished_home_stays_cheap`), seed 2 encoded 385 images:
the old cache, full, started over, so 120 were images she'd already
had. The carry wasn't the cause (lift, carry and use-it cost some
15–25 images a time); her walking and travelling over text is, where
each spot is a new image (industrious: 212–265 distinct images in 20
minutes, against 178–195 ordinary). So the cache now drops the image
shown longest ago (`graphics.rs`, a stamp per image and a `BTreeMap` of
them). Seed 2 then encodes 265 with 9 stale images dropped and none
encoded twice; the other seeds never fill it (189–242 encoded), and at
the end of the phase that still holds. The
test-only counts are images encoded, dropped, and encoded again; the
budget tests assert none encoded again (the busy one allows 4). Memory
is as before: 256 images of about 6,500 pixels each at 9×19 cells,
some 8 MB of encoded data.

**The carry's promises (step 6b; design T3–T5).**
`the_carry_keeps_every_promise` (both drawing modes, text-dense panes,
six deterministic broken homes, any mood, chat, resizes, a resident's
focus) checks every set-down against the rules on that frame (it mends
its rule and breaks none that held), the mood's cap, the covering
checks, and the goodbye. The design's "no piece returns within 3
visits" property was dropped: there's no mechanism to test (a
satisfied rule never moves anything and a move breaks none, so a piece
could only return after its layout changed, which the exemption
covers). In its place `moves_each_piece_once` (Industrious, no resize
or delivery): no piece moves twice, and a mendable home ends with
nothing broken. Cutting the "breaks none" check from the search makes
both fail. The property's two minutes hold one home act at most (277
cases at 256: 210 with none, 67 with one), so there it proves only that
lazy never arranges; `her_mood_caps_her_home_acts` (home 3, three
things wrong, both drawing modes) proves the other caps: ordinary and
dreamy put one thing right and leave what else they felt for ten
minutes, industrious goes on to a second. A cap of 3 for every mood but
lazy fails it.

**Trials (step 7; design D6, A3, T1).** The ties are the cheapest
repair's piece and tier within `TIE_CELLS` = 4, one per spot (the
search offers the sofa both ways round, the only piece whose rule looks
at its facing; a lamp turned where it would stand was a "trial" nobody
could see, caught by `no_piece_is_moved_twice`), tried in a whim's order
(in search order the second spot was always the cheapest, kept
outright, so a third was never tried). The keep roll is e^(−Δ/T) with T
her restlessness, which in practice sits near 1, so she keeps a dearer
spot about four times in five: on the sofa turned from the TV (three
spots, costs 1, 2, 2) she tries a second spot about one move in seven.
In the census she lifts a piece again 1–6 times in 16 half-hour visits
a mood. `she_tries_it_in_a_spot_or_two` (calm, so picky; two- and
three-spot visits in line art) asserts no image is encoded twice.
`the_carry_keeps_every_promise` reaches a second spot in only about 3%
of its cases, so interruptions mid-trial have their own property,
`a_trial_keeps_every_promise`: it starts each case already trying the
sofa (sitting on it in the first spot, or lifted again for another)
and sends chat, resizes and focus changes in the next 20 s, checked
per frame by the same `promised_frame`. About half the cases lift it
again after the first interruption. On these panes no resize makes a
spot stop holding, so the paths that skip such a spot are not reached.

**Measured (2026-10-03).** Gate 1848 tests; the 256-case
houseguest pass is clean (252 tests, 42 s, release); the perf test
passes (a visit: 0.00% of one core), and the search's forced-stride
worst case (three 400-wide strips, 1,173 moves) takes 0.44–0.46 ms in
release. `sofa_census` as after phases 2 and 3: no chat 120/120 and
104/104 used; chat every 37 s 102 of 104 (the two still waiting at the
end) and 93/93; made → used max 28 s; none let be or lost.

`visit_census` gained rows for her home (`test(houseguest): the census
counts what she does about her home`), and puts the shopping channel on
at the start of each visit, as on any visit it's due (it changes
nothing she chooses or how long anything takes; only the advert's lines
join what she says). Forced moods, furniture / floor rest / spacing out
/ moving, % of time:

| Room | Ordinary | Lazy | Industrious | Dreamy |
|---|---|---|---|---|
| stage | 3.8 / 3.6 / 7.4 / 36.6 | 15.1 / 1.3 / 6.1 / 35.8 | 1.7 / 1.4 / 5.4 / 39.2 | 2.6 / 3.7 / 10.9 / 38.5 |
| home | 38.0 / 0 / 1.8 / 42.3 | 44.3 / 0 / 1.2 / 38.8 | 33.5 / 0.1 / 1.3 / 45.3 | 36.4 / 0 / 3.4 / 43.5 |
| resident | 29.2 / 0.6 / 4.9 / 36.4 | 31.7 / 0.5 / 5.2 / 37.6 | 22.1 / 0.1 / 3.8 / 43.2 | 26.8 / 2.3 / 8.3 / 36.5 |

The resident's furniture time is back (ordinary 24.5 → 29.2, industrious
19.8 → 22.1, dreamy 24.3 → 26.8; lazy, who never arranges, 32.4 → 31.7):
its sofa, landed apart from the TV or turned from it, is moved or
turned in every visit but a lazy one, and she watches from it. The
stage room owns nothing and is unchanged. At home, dreamy lost 4
points of furniture time to moving; the carries themselves are small
(Lift and SetDown are 0.1–0.2% of her time), so most of it is visits
diverging after a move. Not investigated further.

Her home, 16 half-hour visits a mood (the home census's five pieces are
stage gifts, settled, scattered over two strips; the resident owns a
sofa and a TV):

| Room | | Ordinary | Lazy | Industrious | Dreamy |
|---|---|---|---|---|---|
| home | home acts a visit (visits with 0 / 1 / 2) | 0 / 16 / 0 | 16 / 0 / 0 | 0 / 10 / 6 | 0 / 16 / 0 |
| | set down / lifted again / dropped | 19 / 3 / 0 | 0 | 28 / 6 / 0 | 19 / 3 / 0 |
| | felt → mended, median / p90 | 159 / 311 s | — | 181 / 274 s | 129 / 214 s |
| | broken at the end (all felt) | faces 8, wall 5, apart 4 | faces 15, wall 13, apart 5 | faces 7, apart 4 | faces 9, wall 3, apart 5 |
| | visits ending with nothing broken | 1 | 0 | 5 | 1 |
| resident | home acts a visit | 16 × 1 | 16 × 0 | 16 × 1 | 16 × 1 |
| | set down / lifted again / dropped | 20 / 4 / 0 | 0 | 17 / 1 / 0 | 18 / 2 / 0 |
| | felt → mended, median / p90 | 198 / 429 s | — | 154 / 287 s | 190 / 326 s |
| | broken at the end | none | faces 16 | none | none |

She first feels a rule some 1–3½ minutes into a visit (median), and
mends it a median 2–3½ minutes after (p90 at most about 7), so nesting's
three-minute rise needs no tuning. No carry was dropped in any census
visit. With drawn moods, home choices: bed 14%, TV 11%, books 10%,
lounging 8%, homework 6%, napping 4%, arranging 1.5%; no want over 24%
of a home visit (resident 22%).

Before the fix the census found, the home's ordinary visits made their
one move in 11 of 16, dreamy in 9, and industrious did nothing about
their home in 3 (9 made one move, 4 two), each stuck on a first felt
rule no move mends. What's still broken at the
end at home is her cap (ordinary, dreamy: one move) or a rule no move
mends (industrious; see the known limits, and rooms by what they
forbid below).

**Beauty and decor.** Of 160 visits in rooms with a TV (forced and
drawn), 159 bought the next piece of furniture at her first watch
(minute 0–13); one, lazy in the resident's room, watched first at 16
minutes and bought a plant, beauty then at 1 and tied at the top. In a
plain home beauty reaches 1 within about a quarter of an hour (it ends
every census visit at 1.00: no census room has decor), so once it has,
any watch the channel is due on buys decor before the next furniture
unless another need is higher. The 1 in 160 is the census's, where she
watches early; in play it's as often as her first watch of a due visit
comes after a quarter of an hour in a plain room. Not tuned: a plant
then is in character, and the furniture comes on the next due visit.

**Rooms by what they forbid (after the phase, the user's call,
2026-10-03; `feat(houseguest): a room is spoilt only by what it
forbids`).** The phase shipped with *no room is worse* counting any
change of role: a sofa couldn't join a TV that stands with a desk (a
study would become a living room), nor a bed a desk, and `Belongs`
followed the role table's order (an unsettled desk in a kitchen spoilt
it, a fridge in a study didn't). Now a piece *spoils* a room when the
room's role without it forbids an offer it makes (`room::forbids`, the
table's `forbids` lists: a bed in a living room or a kitchen, a screen
or a fridge in a bedroom), for `Belongs` and the tiers alike; a move
makes a room *worse* only when it turns one that wasn't a den into a
den; and no piece may spoil a room it didn't, settled or not: the
moved piece the room it comes into, nor any other piece its own room
as the moved one comes or goes (design G7: the den test alone would let
a settled fridge join a bed and a desk, a study then, and the same room
comes of a bed joining a fridge and a desk, or a TV leaving all three;
`newly_spoilt` compares every piece's spoiling before and after), nor
the moved piece, unsettled, the room it's set down in. `wrong_home(5)` became mendable (the bed went up to the desk, a
bedroom then), so its bed now has its lamp beside it: moving the bed
breaks `Near`, the TV `Faces`. Forced moods, her home, 16 half-hour
visits:

| | Ordinary | Lazy | Industrious | Dreamy |
|---|---|---|---|---|
| home acts a visit (visits with 0 / 1 / 2 / 3) | 0 / 16 / 0 / 0 | 16 / 0 / 0 / 0 | 0 / 4 / 10 / 2 | 0 / 16 / 0 / 0 |
| set down / lifted again / dropped | 19 / 3 / 0 | 0 | 41 / 11 / 0 | 20 / 4 / 0 |
| felt → mended, median / p90 | 159 / 521 s | — | 195 / 477 s | 193 / 276 s |
| broken at the end (all felt) | faces 10, wall 7 | faces 15, wall 13, apart 5 | faces 3 | faces 10, wall 6, apart 1 |
| visits ending with nothing broken | 1 | 0 | 13 | 1 |
| furniture / floor rest / spacing out / moving, % | 37.2 / 0 / 1.7 / 43.4 | 44.3 / 0 / 1.2 / 38.8 | 32.3 / 0 / 1.1 / 46.5 | 36.1 / 0 / 3.6 / 43.5 |

Industrious visits ending with something broken: **3 of 16** (was 11).
Each of the 3 made one move and then felt `Faces` she couldn't mend;
the room rules don't refuse the move that would mend any of them (in
each the sofa could join the TV's room, or turn where it stands); what
does hasn't been looked into. Ordinary and dreamy
still end at their cap of one move. The stage and the resident's room
are unchanged; the furnished-home golden's seed 0 re-recorded (the sofa
now joins the TV and desk downstairs, lifted at 6.8 minutes, where
she used to give up; `the_sofa_joins_the_tv_in_her_study`, `wrong_home(7)`, has it
in both drawing modes). Gate 1852 tests; the 256-case houseguest pass
is clean (256 tests, 43 s, release).

**Known limits.**
- *One move at a time* can leave a home that no single move mends. Take
  a bed in the living room beside the TV the sofa faces, and a lamp
  upstairs. She feels the lamp first (Near comes before Apart in the
  table), so the lamp comes down beside the bed. After that, moving the
  bed breaks Near and moving the TV breaks Faces, so Apart stays broken
  and she leaves it (`wrong_home(5)` is that home, the lamp already
  down). That's the design working as intended, not a bug.
  If it ever matters, a later phase could weigh paired moves, or feel
  Apart before Near.
- A felt rule no move mends keeps *nesting* at 1 for the rest of the
  visit. Nothing comes of it (only arranging answers it, and that isn't
  on offer without a way), but the stage shows her keen.

**Later** (the user, 2026-10-03; not now): posters delivered in
cardboard tubes; more posters, paintings and plant varieties; perhaps a
greenhouse or solarium she builds.

### Phase 5 — vignettes and the clock (brief, 2026-10-03)

The proposal's row 5, split in two (the user's call). Decisions from the
user, this session:

- **5a, vignettes:** `Script`/`Key`/`Prop`, line pools, splices, the
  shopping channel converted, chopsticks (#40) and sata andagi (#41) with
  new art (a model sheet first), and four cheap extras: channel surfing
  (#37), riddle queen (#56), "Where was I?" as a pool, lamp off before
  bed (#70). Sata andagi: after a snack she says "Sata andagi." a few
  times, happier each time; a chat line ending in `?` meanwhile gets
  "Sata andagi." too, turned to the chat; then she eats it.
- **5b, the clock:** a **game clock** that runs only while dessplay is
  open, at about 6× real time, persisted, starting at 16:00 on a weekday
  and never resyncing. It drives her routine, sleep and absence (school,
  work) included: away, the home shows empty with rare dash-ins, and a
  resident leaves and comes back. Calendar **dates** are the real date
  (a starter set of cheap dates; art-heavy ones wait). Tiers, rarity and
  pity. A window and a wall-clock piece make game time legible (art).
  Later, maybe a cat who ignores the clock.

**5a working design:**
[phase5a-design.md](proposals/2026-10-02-houseguest-mind/phase5a-design.md)
(scripts hosted on `Act::Use` and `Act::SpaceOut`, every use's look a
script, a use's body with a prelude and a coda, choices at an act's start
hashed from her latest decision's whims). Commit order: scripts as data
(goldens unchanged) → lines → splice machinery with surfing and lamp →
chopsticks and andagi once the art is approved → census, the 256-case
pass, docs. 5b gets its own brief once 5a is recorded.

Deviations from the proposal's row 5 (mine, told to the user):
- **`Adverb` is deferred**: moods already carry her traits as rates, and
  no 5a vignette needs one.
- **No free-standing `Act::Script` in 5a**: every 5a vignette happens at
  a piece or while spacing out. 5b decides if calendar content or the
  dash-ins need one.
- **The purchase still commits the moment the channel comes on**
  (design.md unchanged), not "at key 1".
- **For 5b: "Tier is a factor" can't work as written.** `brain::choose`
  multiplies by factors, truncates to the top four, then rolls, so a
  factor below 1 doesn't make a want rare: it removes it whenever four
  others outscore it. Rarity and pity must gate *offering*, at the offer
  filter or in `mind::bind`
  ([map](proposals/2026-10-02-houseguest-mind/phase5a/map.md), A6).

### Phase 5a — vignettes (done 2026-10-03)

Built as the [working design](proposals/2026-10-02-houseguest-mind/phase5a-design.md)
and its round-1 amendments say (the code map and the three critiques are
in its `phase5a/` dir; the approved art, its review sheets and wiring
notes in `vignettes/`), in eleven commits: the design; Crumple's and
Unpack's switches made inclusive; scripts as data (goldens unchanged);
keys change on time; lines by pool; door lines, musings and riddles;
the splice machinery with channel surfing and the lamp; the art;
chopsticks and sata andagi; the census; design.md and decisions.md.
Each step was implemented, reviewed twice (correctness, and tests proven
by mutants) and fixed, minors included. Differences from the design:

- **The grid and bobs count from the start of the part that's playing**
  (prelude, body or coda), not `body_start`: only then is what shows in
  the body the same with or without a splice (the B2 property).
- **A pooled line spoken over in the same instant it's said** is taken
  back and doesn't cool: nothing was drawn in between.
- **No script cooldown for riddles, the shopping channel or bedtime.**
  Only splices and surfing consult it; a riddle is paced by its pool's
  chance and its question's line cooldown.
- **`Key.face` is a plain `Face`**, not `Option<Face>`: every key names
  its face.
- **`Pose::EatAndagi(0..=2)`** beside `Pose::Eat` (melon bread's hashed
  Debug unchanged): held up (in ASCII facing out, so the face ramp
  shows), bitten, and chewing, so the andagi never looks whole again
  after the bite. The ASCII frames and the chewing composition (the
  approved bitten piece in the held pose) weren't on the review sheet.
- **IRC lines past the first 100 went unseen**: `ChatMark.irc` was the
  trimmed log's length, so she stopped noticing IRC after 100 lines in a
  session. It's a counter that only rises now (fixed in the andagi
  commit, with a CHANGELOG entry).
- Cfg(test) splice rows (`TestSnack`, `TestSleep`, `TestBedtime`) stay
  beside the real ones for the tests that check proportions. Stage cues
  force a splice's branch; the example's `?` sends a chat line that asks.
- The stale commit messages: the splice commit's "taken by the next use"
  cue wording, and the census/docs commits' phrases predating their
  fixes, are superseded by this record.

**Measured (2026-10-03).** Gate 1914 tests. The 256-case houseguest pass
(313 tests, release) found no product bug: two wall-clock tests
(`repair_search_is_cheap`, `terrain_read_is_cheap`) failed under load
and now take their fastest run, and `every_made_piece_is_used_or_let_go`
takes 46–49 s at 256 cases (40 s before 5a: more steps from key-end
wakeups and vignettes), so it has its own nextest slow-timeout (killed
at 120 s). Then clean twice, about 46 s. Perf: a visit 0.00–0.33% of a
core (one 10 ms tick in 3 s is 0.33%). `sofa_census` as before: no chat
119/119 and 103/103 used; chat every 37 s 102 of 104 (2 waiting) and
93/93; made → used max 28 s.

`visit_census`'s home now owns a fridge and a lamp (so snacks and
lamp-lit bedtimes happen; it buys the cat bed first now), and every
other census chat line asks something. Drawn moods, per visit (visits
with any):

| Room | Scripts | Splices | Pooled lines |
|---|---|---|---|
| stage | riddle 0.69 (10 of 16) | none | door 8.2, musing 2.4, riddle 1.4 |
| home | bedtime 5.06, shopping 1, surf 0.62 (8), riddle 0.25 (3) | chopsticks clean 0.62 (9), bad 0.19 (2); andagi 0.12 (2) | musing 0.56, riddle 0.50 |
| resident | shopping 1, surf 1.25 (12), riddle 0.56 (7) | none | musing 0.88, riddle 1.12 |

All five door lines are heard about equally (25–29 each in 16 stage
visits). Forced moods, furniture / floor rest / spacing out / moving, %
of time:

| Room | Ordinary | Lazy | Industrious | Dreamy |
|---|---|---|---|---|
| stage | 4.7 / 3.1 / 6.3 / 38.1 | 16.7 / 1.5 / 6.1 / 34.3 | 2.2 / 1.0 / 5.7 / 39.6 | 6.5 / 2.0 / 11.3 / 36.4 |
| home | 37.9 / 0 / 1.5 / 43.4 | 41.8 / 0 / 1.1 / 41.1 | 33.6 / 0 / 1.0 / 45.9 | 37.5 / 0 / 2.5 / 43.8 |
| resident | 28.5 / 0.5 / 4.8 / 38.0 | 31.1 / 0.5 / 5.1 / 38.7 | 24.1 / 0.3 / 3.5 / 41.1 | 27.3 / 2.8 / 8.1 / 35.4 |

The home row isn't comparable with phase 4's: the census home gained two
pieces, and its gifts land breaking rules (the fridge off its wall in 14
of 16 visits, the lamp away from bed and desk in about 7), so most home
visits end with 3–4 felt rules broken and nesting at 1 (a new "broken at
the start" row shows it).

**Open, for the user.**
- **The sata andagi is rare:** 2 of 16 drawn home visits (forced moods
  0.2–0.4 a visit). Snacks are about 5% of her home choices, the odds are
  1 in 4, and the 10-minute cooldown applies. An answer to a question
  happened in 3 of 64 forced visits with chat every 90 s: a question must
  land in a coda of about 15 s. The chopsticks wrap about 1 homework in 7
  (the speech gate and the cooldown thin the 1 in 3).
- **Industrious with a fridge thrashes the image cache** (seed 2: about
  19 images encoded twice in 20 minutes, vignettes on or off;
  pre-existing). Her walk and exercise frames are the working set; the
  vignettes push it over 256 a little sooner.

**The user's answers (2026-10-03).** The andagi's rate is right: 2
visits in 16 is fine, since as events multiply any more would crowd
the rest. The census home starting broken is left alone for now; a later
improvement would be unlimited repairs in the census home. The image
cache is raised to suit Ghostty's capacity (measured; below).

**The image cache (2026-10-04; `perf(houseguest): a frame cache sized
for a long visit`).** `CACHE_LIMIT` 256 → 1024, sized by `image_census`
(`#[ignore]`d; `cargo test --release -p dessplay --lib image_census --
--ignored --nocapture`, about 40 s): on still screens (stage, home,
resident; every mood, 16 seeds, two hours) the most distinct images in a
visit were 421 by 20 minutes, 591 by 60 and 664 by 120, and the largest
working set 603, so 1024 encodes nothing twice. A cached image costs
about 36 KB of client memory (ratatui-image keeps its base64 RGBA
transmit string), so a full cache is 30–43 MB by room; Ghostty's default
`image-storage-limit` (320 MB a screen) holds 10,000–14,000 of her
images. **A live chat has no working set a cache can cover:** on the
client's own 200×50 layout with a line every 45 s she meets about 10 new
images a minute (her image includes the lines under her), 1669 in two
hours; 1024 encodes nothing twice for the first hour and 196 by two.
2048 was rejected (60–87 MB full, and only an hour later). Making her
image independent of the text under her is the real fix, if it matters.
The busy tests now allow no re-encodes and bound images a visit at 512;
an LRU property covers eviction at small test-only limits.

**For 5b's brief** (written first thing in its session; the decisions
are in the phase-5 brief above). Questions it must settle against the
code:
- **The clock in the ledger**: accumulated game minutes (never an
  absolute `now`, which restarts per process), accrued while the client
  runs, flushed in batches (each dirtying is a sqlite write), and drained
  on quit (the quit path skips the last `ledger_to_save()`; map d14).
- **"Away" against today's rules** (map d20): the idle delay alone brings
  her, a resident stays, an errand fetches her whatever the gate says.
  What does an errand do while she's at school, and what is a dash-in as
  an act (in, grab, out)?
- **Calendar content**: on the real date (`LocalTime` on `IdleView`,
  set in the shell after `idle_view`, `None` disables it), owed once on
  the first visit of the day; whether it needs a free-standing
  `Act::Script` (5a deferred it).
- **Rarity and pity** gate *offering* (above); pity from ledger counters
  of idle minutes; at most one unseen rare a visit. Ledger fields lenient
  and skipped when empty; an older build drops them on save.
- **Art**: a window piece and a wall-clock piece (model sheets first).

### Phase 5b — the clock (brief, 2026-10-04)

Working design: [phase5b-design.md](proposals/2026-10-02-houseguest-mind/phase5b-design.md)
(code map and critiques in its `phase5b/` dir). The user's calls this
session: school 08:30–12:30 (home from 12:45; HG's full day left every
other session mostly empty); an errand while she's at school is a dash
in and out by her door, and asleep she gets up groggy, pokes it and
goes back to bed; the wall clock arrives as a one-time parcel and the
window is sold after the cat bed, with a look-out use and a clock glance
at routine changes; real-date vacations (summer, year-end, spring)
cancel school. At 6× that's about 49% of open time home and awake, 37%
asleep (visible) and 14% away. Steps 0–9 are in the design.

### Phase 5b — the clock (done 2026-10-05)

Built as the [working design](proposals/2026-10-02-houseguest-mind/phase5b-design.md)
and its round-1 and 1b amendments say (code map, four critiques and their
synthesis in its `phase5b/` dir; the approved window and clock art in
`phase5b/art/`). The rules are in design.md (Houseguest) and the reasons
in decisions.md. Each step was implemented, reviewed twice (correctness,
and tests proven by mutants) and fixed, minors included. Commits, in
order: the exit save; her clock and the idle/pity counters; `routine.rs`,
the real date and `DayTime` (unfed); the mind's levers (unfed); the clock
fed and her night; the night's extras and the job on days off; no goodbye
from her where she isn't; away at school with her closed door; (the stop
hook stands down while subagents work); dashes and the errand at school,
plus its fixes; the calendar and time-of-day lines; rares and pity; pity
only for the unseen; the wall clock and window; looking out and the
glances; the week census; a chat line at her fridge; `--dump`'s
houseguest section; the docs. Several step commits' messages went stale
under their fixers (steps 1, 2, 3a, 3b, 4a, 6, 8a, 8b, 9a, 9b) and one
(5c's) couldn't take its fixes because master had been pushed; this
record supersedes them.

Differences from the design (the notable ones; design.md states them):

- **The date's single source is `Guest::date()`**, set by the shell
  before every `advance`; `IdleView.local` was dropped as a mirror with
  no reader. Each game day is judged by its own vacation flag, latched
  per game day (not persisted), so the longest night is **105** real
  minutes at a school-to-vacation edge, not 95.
- **The exit save also carries pane sizes** (`UiExit { houseguest,
  layout }`): the layout drain had the same lost-on-exit class. Exits
  that never join the UI thread (SIGHUP, a panic, the `?` returns in
  `run_interactive` between the UI's spawn and the session's end) stay
  unsaved.
- **The night:** one act on any surface, its wake time recomputed at
  every clock reading; the lamp's dark is `Osaka::dark(now)`, latched
  only in the Asleep slot; `begin_day` is the one new-day path (wake or a
  missed wake), and the line budget, the rare draw, the Dream, the
  midnight snack and the meal lines belong to the game day, carried
  across visits per process. Bedtime settles an arranging episode (a
  trial is kept, a pocketed piece let go). Parcels wait while she sleeps,
  until her morning line has shown, and while she's out or behind a door.
- **Two bugs older than 5b, found and fixed on the way:** a work shift
  cut short left her "at work" forever or said "I'm home!" without
  leaving (now `shift: Option<Shift>`); a boxed cat bed unpacked with the
  cat already in it (since 2026-10-01). Also new: an overlay ends an
  errand or a dash already under way (she used to paint over modals).
- **A hidden Osaka can't be placed** (`Placement::of` is the only
  constructor and is `None` while she's hidden): the hidden-goodbye class
  is unrepresentable.
- **Away:** the door's spot lives on the `Guest` (a no-home return comes
  out where she went in); a goodbye from Away rains the door out with no
  wave; a no-home door-end rains out what she moved or made. A dash reads
  the coming visit's cat.
- **The calendar:** delivered only when its line is actually drawn
  (`draw` returns the drawn bubble), retried at another spot at most
  three times a visit, then left owed; Jan 1's sunrise is a best-effort
  extra within the visit. With a calendar greeting she wakes with a plain
  "Mornin'." first (the combined line won't fit 24 characters).
- **Rarity:** seen rares roll their tier's base rate only; pity ramps
  only the choice of an unseen one (otherwise, once every rare had been
  seen, every seen rare would open every day). A day begun after midnight
  draws over the night and morning only. A restart the same game day may
  redraw.
- **The window and clock:** a parcel comes in only where she can stand to
  unpack it (her seat test), and a window first where she can look out
  of it (26 of 40 deliveries before, 40 of 40 after). The gift waits
  until her day's calendar entry is settled. A cued hour glance reads the
  game hour in any slot.

**Measured (2026-10-05).** Gate 2173 tests. The 256-case houseguest pass
(557 tests, release) passed three times and found no product bug; the
slowest are `every_made_piece_is_used_or_let_go` 45–47 s (120 s override)
and four tests at 29–39 s. Perf: a visit 0.00–0.33% of a core, playback
4.7–5.3%. `sofa_census` (now unfed) matches 5a exactly; `visit_census`
(unfed) is within 0.2 points of 5a for the stage and 1–3 for the home and
resident; `image_census` is unchanged. The new `day_census` (`#[ignore]`d;
`cargo test --release -p dessplay --lib day_census -- --ignored
--nocapture`, about 65 s) runs a fed game week from Monday 00:00 at three
seeds, on Oct 31 and with no date:

| Room | Here | Asleep | Away | Hidden in a visit |
|---|---|---|---|---|
| stage | 47.3 | 36.4 | 13.3 | 2.9 |
| home | 49.0 | 36.5 | 13.3 | 1.2 |
| resident | 49.0 | 36.5 | 13.3 | 1.1 |

(D2 predicted 49 / 37 / 14.) Out to school and home again 30 of 30;
dash-ins 10 a room (1 in 3); Halloween once a run, at the wake, never
undated; the clock parcel at Monday 07:00–07:18; look-outs about 19 a
game day in the home; 13–14 rares first seen a room, every run sees the
Dream. Moving is 35–45% of awake time in every slot. The census found one
product bug: a chat line as she reached her fridge on a dash (or a
midnight snack) cost her what she came for; fixed with a property each.

**Open, for the user.**
- **Restlessness** (the user, testing on 2026-10-04): she spends too much
  time walking about (35–45% of awake time; Restless fills in 90 s). To be
  discussed before phase 6. *(Fixed: phase 5c, stillness, below. She
  moves 5–31% of her time in sight by mood, a band the tuning holds.)*
- **Should a dash look at chat lines at all?** Today she's startled and
  looks, then carries on; the tests pin that. *(Decided: the user, in
  5c's brief: a dash keeps looking at chat lines, as built.)*
- **Look-outs vs. the afternoon clock glance:** in a full home look-outs
  (about 19 a game day) crowd out spacing out, so the afternoon glance
  came once in six home-weeks (nine each in the stage and resident). The
  glance is also once per visit, not per game day, and D4's "Gaze ×more
  with a window" isn't built. *(Fixed in 5c: looking out is a long
  daydream, about three a game day (base 5); the afternoon glance lost its
  one-in-three roll, and comes 20 times in the census home's six weeks,
  14 on the stage and 14 in the resident's; D4 was dropped, the window
  being the daydream itself. The glance stays once a visit, by decision:
  that never binds (decisions.md, "Her window is a long daydream").)*
- **Rares a home can't show:** pity can pick NoMelon in a fridgeless room
  (a wasted day's draw; nothing is lost). Scary is first seen in about one
  week in six (its window is 30 minutes on school nights).
- **A rain-out's last frame may linger** until the next redraw when it
  ends in a tick where nothing else changes (Visiting's `fading` is
  computed after `retain`). The fix moves 24 golden traces, unfed tables
  included, so it waits for a decision. Away's arm has the fix.
  *(Fixed in 5c step 1: the fades live on the guest and every arm drops
  them through one helper, so a fade's last frame is always drawn.)*
- **Her pane-corner column** belongs to no strip, so from there she sees no
  clock (as `beauty_at` already sees no decor).

### Phase 5c — stillness (brief, 2026-10-05)

Between 5b and phase 6, at the user's request. **The problem:** she spends
35–45% of her awake time moving (5b's week census, every slot, room and
mood), and movement catches the eye far more than her sitting still, so it
feels like more than it is. This is about **attention control**: walking
to a job counts as movement just as wandering does.

**The user's decisions (2026-10-05):**
- **A target band per mood** for moving, as a share of awake time, in the
  15–30% range: about 15 lazy, 20–25 ordinary and dreamy, up to 30
  industrious. It becomes a census band test per room and mood.
- **Walking to a job counts** as movement.
- **New art is fine this phase** (model sheets first, as always).
- **The window is a daydreaming place**, not a quick glance: look-outs
  become rarer and much longer (5b's census had about 19 short ones a game
  day in a furnished home, which also crowded out the afternoon clock
  glance).
- **A dash keeps looking at chat lines** (as built).
- **The rain-out's lingering last frame:** apply the fix (5b's record,
  "Open": Visiting's `fading` computed after `retain`), re-recording the 24
  traces it moves with the reason.

**Why she moves so much** (2026-10-05, brain.rs): movement wins the rolls
(Walk base 14, Travel 10, Pull 16; Stand, Sit and LieBack 4; SpaceOut and
Gaze 6); its needs refill fastest (Restless fills in 90 s, Tidy in 60 s;
Daydreams 10 min, Comfort 8 min); and stillness is brief (spacing out
6–14 s) while a walk lasts its whole path.

**Plan, in order:**
1. **Measure first:** split the census's "moving" by purpose (wandering:
   Walk and Travel; walking to a job: to text, to a seat; climbs, falls
   and doors), per room and mood, unfed (5a-comparable) and in the day
   census.
2. **The band test** per room and mood, failing at today's numbers.
3. **Cheap levers, then re-measure:**
   - slower movement needs (Restless toward about 5 min, Tidy toward
     about 4) and a lower Walk base;
   - longer still acts (spacing out 20–60 s, sitting or lying 30–90 s,
     longer lounging and watching) and **settling in**: when a still act
     ends she often settles further (sit, lie back, doze) instead of
     getting up;
   - prefer the nearest spot that answers a want;
   - daydream sessions: a long space-out with musings in a row, or lying
     back cloud-watching;
   - TV from the floor, cross-legged before it (check whether Watch binds
     without a sofa today; the cheapest win for a TV-only home).
4. **Bare-room stillness with new art:**
   - homework on the floor, lying on her front with a paper;
   - its discomfort drives a **makeshift desk**: she tears and crumples
     text into a cube and does homework at it, like the made sofa;
   - reading a pulled line like a book (HG #72), sitting on the floor;
   - **more makeshift furniture while she owns no real piece** for that
     use (for example, three times the weight on "make one"), dropping
     away as real furniture arrives.
5. **The window as a daydream:** rarer, much longer look-outs; then check
   the afternoon clock glance has room again.

Steps 1–3 should be re-measured before step 4. Step 4 may be smaller if
the band is met, but floor homework and the desk are wanted for their
own charm.

### Phase 5c — stillness (done 2026-10-07)

Built as the [working design](proposals/2026-10-02-houseguest-mind/phase5c-design.md)
and its Round-1 to Round-8 amendments say (code map, four critiques and
their synthesis, D7's own critique and mock script, and the approved art
in its `phase5c/` dir; every number in
[phase5c/baseline.md](proposals/2026-10-02-houseguest-mind/phase5c/baseline.md),
whose last section, "Census pass (step 13)", has the closing runs). The
rules are in design.md (Houseguest) and the reasons in decisions.md.
Each step was implemented, reviewed twice (correctness, and tests proven
by mutants) and fixed, minors included; the commit messages carry each
step's golden trace check.

**The aim.** She moved 35–45% of her awake time (5b's week census), and
moving draws the eye far more than sitting still. 5c is attention
control: a band per mood for her moving *in sight* (walking to a job
counts), reached through stiller habits rather than slower walking, and
the things on screen that changed for no reason (the TV's static, a
lingering rain frame) held still.

**What was built, by step:**
- **0.** The design, the code map, four critics and their synthesis (the
  user chose B1: in a still act she looks up where she is, no resume
  mechanism); the art agent's sheet.
- **1.** A rain-out's last frame is always drawn: the fades live on the
  guest, and every arm drops them through one helper (5b's "Open" item).
  A rain is cut short only when she's sent away or her room goes.
- **2.** The credit class: spacing out eases her daydreams and work her
  restlessness, as designed; a cut work shift is credited by the share
  worked; a class test runs every want by each of its methods.
- **3.** The census measures moving in sight by purpose (wandering, to a
  seat, to text, climbs and doors) and set-offs a minute; the fed
  afternoon census (each room fed at Tuesday 13:00 in a forced mood);
  `phase5c/baseline.md`.
- **4.** The band tests (`tests/band.rs`), ignored until the tuning.
- **5.** The watch after a chat line is 5 s, not 15 ("the act of looking
  is there solely to draw attention, which only happens during change").
- **6.** In a still act on restful terrain she looks up where she is
  (`!` then `?`, turned to the chat where the pose has a facing); dozes
  stir ("Mm?"). Walking, pulling, chores, making, exercise and anything
  on text are still cut.
- **7.** The levers, landed neutral: settling in (a still act that runs
  its course settles further where she is, skipping the roll), lingering
  by mood at the call sites, a daydream as one act of several musings,
  nearer spots (text on her own floor first, nearer seats likelier).
- **8.** A slow blink on held poses (150 ms every 6–12 s, from her
  whims); an easing lands on her needs as they are (`rise_to` before a
  credit). The tuning stopped: the design's per-room set-off cap held the
  furnished home near one set-off a minute while the stage needed three.
  The user's call: **per-mood caps**, the same in every room (Round 3).
- **8b.** The census and band drivers deliver each chat line at its own
  time; the per-mood rule in the band; watching out of lingering until
  the TV held a picture. The retune stopped again: under a per-mood cap
  the bare stage (2–3.5 s a set-off) sits a point or two over its floor.
  The user: "ship it, lower stage floors".
- **9.** Wiring the approved art folded into the steps that use each
  pose (Round 3); the sheet itself landed unwired after step 4.
- **10a.** Homework on the floor (on her front, or on her back with the
  book), reading on her back where no bookshelf stands (settling into a
  doze under the book), "My back..." after the first floor homework a
  visit, the paper desk (a 4×2 cube of torn text she kneels at), making
  three times as likely while she owns no real piece of the kind,
  cross-legged before the TV away from a sofa.
- **10b.** A borrowed line (HG #72): she reels a strip off a line beside
  her, reads it sat by the tear and slides it back; she never leaves it
  torn (grip is one class for held text and pulls).
- **11.** The window hangs low enough to lean on (and may stand behind a
  sofa, unlike any other piece); a look-out is a long daydream at the
  sill (one to three minutes, musing on the sky), settling to sitting
  under it, then a doze or watching the clouds (only ever under her
  window).
- **8c.** The golden driver cuts at each event's own time (its own
  re-record); **the levers ship** (`Stillness::TUNED`: lingering lazy
  ×1.5, ordinary ×1, dreamy and industrious ×0.85; settling lazy 0.6,
  ordinary and dreamy 0.35, industrious 0.15; restless over 5 minutes,
  tidy 90 s, mischief 6; Walk base 9, Travel 7; spacing out 10–28 s and
  worth doing for its own sake); the stage's floors lowered;
  `BAND_MINUTES` 17; the two short items in `SHORT`.
- **12a.** Her TV holds a drawn programme (news, weather, penguins, a
  cooking show) after 1.2 s of switch-on static; Chiyo-chichi bobs only
  through his hook; watching lingers.
- **12b.** In line art her TV shows a still of the held now-playing film,
  asked for as she heads to watch and once a minute while she does, into
  the TV's own private screenshot slot (never commentary's, whose frames
  go to Anthropic), read, treated and deleted in process; her trace and
  ASCII frames are identical with and without it. Every
  `screenshot-to-file` goes out async, and commentary shares the gate
  that the player shows the real video, not the placeholder.
- **12c.** The drivers paint her at every input, as the client does;
  the user's answers (an industrious watch isn't shortened, the film's
  saturation ×1.3, looking out at base 4, base 5 since T1); the stillness rule tested on
  drawn cells.
- **12d.** Two failures older than 5c that 5c's property runs surfaced, each
  fixed as a class: a moved wide glyph rains out whole, never half of one
  (as old as the text layer); a parcel comes in only where she can unpack
  it, and use it, with it there (as old as 5b's rule). Their pinned cases
  are regression tests now: `a_sofa_she_could_not_unpack` folded into
  `parcels_she_could_not_unpack`, and the away "half a wide glyph" line
  a saved regression of the day-long property, with the deterministic
  `a_wide_glyph_she_moved_rains_out_whole` cases.
- **13.** The census pass and these docs; the deep pass got its own
  nextest profile, `--profile deep` (flag at 90 s, kill at 180 s), so its
  256-case properties don't raise the gate's limits.

**Shipped numbers** (the band at full strength, N = 525 visits a cell of
17 minutes, line art, fed afternoons; moving % in sight / set-offs a
minute in sight):

| Room | Mood | Quiet | Chat | Band | Cap |
|---|---|---|---|---|---|
| stage | Lazy | 6.6 / 1.17 | 6.2 / 1.22 | 4–17 | 1.5 |
| stage | Ordinary | 7.1 / 1.62 | 7.3 / 1.69 | 6–26 | 2.25 |
| stage | Dreamy | 6.9 / 1.64 | 6.7 / 1.67 | 6–26 | 2.25 |
| stage | Industrious | 9.1 / 2.11 | **8.6** / 2.10 | 9–31 | 3.0 |
| home | Lazy | 12.0 / 0.62 | 11.9 / 0.68 | 5–17 | 1.5 |
| home | Ordinary | 22.5 / 1.24 | 21.5 / 1.32 | 8–26 | 2.25 |
| home | Dreamy | 22.1 / 1.25 | 21.7 / 1.36 | 8–26 | 2.25 |
| home | Industrious | 30.6 / 1.72 | 29.1 / 1.86 | 12–31 | 3.0 |
| resident | Lazy | 6.2 / 0.54 | 6.0 / 0.56 | 5–17 | 1.5 |
| resident | Ordinary | 11.0 / 1.10 | 10.7 / 1.16 | 8–26 | 2.25 |
| resident | Dreamy | 9.7 / 1.08 | 9.1 / 1.12 | 8–26 | 2.25 |
| resident | Industrious | 16.3 / 1.35 | 15.1 / 1.40 | 12–31 | 3.0 |

Every cell is in its band but the bare stage's industrious afternoon with
chat (8.6% against its floor of 9), and every set-off rate is under its
cap. The spreads (industrious ÷ lazy, N = 200): home 2.60, resident
2.68, the stage 1.38 against 1.6. The home's industrious quiet
afternoon (30.6 against 31) is in by noise only: the next change to what an
industrious Osaka does re-measures it at N = 525 first. The resident's
dreamy afternoon with chat measures 9.1, but is aimed at 8.3, just over
its floor of 8: the test at N = 525 holds it to 7.7, and enforcing the
floor itself takes N = 2266, so its value is read against the band by eye. Looking out: 3.3
a game day in the census home (140 in 42 game days, base 5 since T1; base 4 read 1.95). The unfed visit census
(5a-comparable, ASCII, with chat) reads 5–12% moving on the stage by
mood (22–30% before 5c, at 4–6 set-offs a minute), 11–26% in the home and
8–16% in the resident.

**Deviations from the design** (each in decisions.md):
- **B1, not D3's resume:** the user chose looking up in place; D3's
  `Resume` and its guards were never built.
- **Per-mood caps replaced M3's per-room cap** (Round 3), and the bare
  stage has lower floors (lazy 4, ordinary and dreamy 6, industrious 9).
- **Dreamy lingers ×0.85** (not in the brief): at ×1 the resident's dreamy
  afternoon with chat fell under its floor once 10a–11 were in.
- **The floor wants are base 6**, not the desk's 8 (at 8 a sleepy doze
  dropped out of a bare room's best four), and drop the chat factor;
  reading on her back is an activity, not a `Use(Read)` method.
- **D4 of 5b ("Gaze ×more with a window") was dropped**: the window is the
  daydream place itself.
- **The film refreshes about once a minute** (the user's Q2), overriding
  D7's critique's "never swapped mid-watch"; a picture swap cuts straight
  in with no static.
- **The golden driver's order within a moment** now keeps the shell's
  (T1, test-only: it advances her before it tells her of a key press,
  and cues the stage in that order by convention; no golden moved); the ~10 Hz snapshot redraws during
  playback stay unmodelled (testing-strategy.md).

**The user's decisions this phase** (beyond the brief's):
- **Art** (two review rounds, 2026-10-05): approved the paper desk
  kneeling, the torn strip, cross-legged TV, the sitting doze, looking up
  in place and the programme cards; floor homework with the paper in
  front of her, not under her head; reading on her back with the book
  over her face, and dozing with it turned side-on over her eyes; the
  window hung lower to lean on, allowed behind a sofa; cloud-watching only
  under a window (or a solarium, later).
- **The band:** per-mood caps in every room (lazy 1.5, ordinary and
  dreamy 2.25, industrious 3 a minute), share ceilings 17 / 26 / 31,
  floors 5 / 8 / 12, and 4 / 6 / 9 on the bare stage ("ship it, lower
  stage floors"). The gate runs the band at two seeds, full strength at
  step boundaries: "Seems fine. I'll know to check them if something
  feels off." Two items shipped short by the user's word (`SHORT`): the
  stage's industrious afternoon with chat and the stage's spread.
- **Looking out:** base 6 first, then the outcome over the number: about
  two long daydreams a game day (base 4, 1.95). "4.5 times a game day is
  fine. I'd accept anything in the 2-6 range": base 5 (3.3, mid-range)
  ships since T1, and the day census holds the 2 to 6 itself. The afternoon hour glance lost its roll; a borrowed strip
  doesn't turn her at a chat line.
- **The TV (D7):** the film, the mock's column 7 (centre crop, 1.3× zoom,
  a levels lift, saturation ×1.3, no sheen), drawn cards otherwise; a
  refresh about once a minute; no reaction from her (an idea for later,
  below). "Don't shorten industrious watching" (×1). The shopping hook's
  bob is left whole (up to 49 s lazy): "Continuous cyclical movement gets
  filtered out by the human optical system almost as quickly as static
  scenes."
- **A resident staying in the chat pane** isn't weighted (only going in
  is a tenth). The TV's retry 5 s after a question the player couldn't
  be asked stands (the orchestrator's call, unless the user objects).

**Measured at step 13** (phase5c/baseline.md, "Census pass (step 13)"):
the 256-case houseguest pass (731 tests, release) passed under its own
`--profile deep`, a 180 s kill (`her_film_never_moves_her` takes 72 s
alone and was killed at the default profile's 60 s); the slowest are
that one (81–85 s in the pass), `a_trial_keeps_every_promise` 52–62 s,
`every_made_piece_is_used_or_let_go` 48–51 s, and four or five at 30–42 s. No
product bug. The censuses read exactly step 12c's numbers (12d moved none of them).

**Phase 5c's tail, T1 (2026-10-07),** three commits: the golden driver
advances her before it tells her of a key press, as the shell does, and
cues the stage in that order by convention (test-only, no golden moved;
the two property drivers that press keys too; the ~10 Hz snapshot
redraws stay unmodelled); looking out's base 4 → 5 (3.3 a game day in the day
census, which now holds the user's 2 to 6; no band room has a window,
so the band is unmoved; baseline.md, "Phase 5c's tail, T1"); the
world's clock (her wall clock's dial, her window's sky) exempt from the
drawn stillness rule, only at a paint where her clock's reading
changed and scoped to its own change (the run checks, at each paint it
exempts, that one thing more changed is not), and its run
(`no_long_act_flips_drawn_cells_faster_than_a_frame_by_her_clock`, with
a run across the sky's day to dusk) in the gate.

**Open, for the user:**
- **The two `SHORT` items** above; the lead for the stage's spread is a
  mood factor on spacing out for its own sake, which weighs the same in
  every mood and fills an industrious stage afternoon most (14%).
- **From the drawn stillness test** (Round 8; each fix changes how she
  looks, a golden re-record): a bob's key ending off its frame grid (her
  homework nodding off within a frame of the writing's last flip; the
  test allows exactly that, "for now"); a stir by day ("Mm?", 1380 ms) is
  shorter than a frame. (The world's clock, her wall clock's dial and the
  window's sky stepping within a frame of her own changes, was resolved
  in T1: exempt, by the user's word, scoped to its own change.)
- **The drawn exemptions don't combine** (T1's review): a dial or sky
  step on the same 100 ms paint as another exempt change (a blink, a
  look up at the chat) is exempt by neither arm, and fails the drawn
  test as a flip; and in ASCII the sky's cells inside her box (her
  leaning at the sill) count as hers. Neither comes up in today's runs;
  the door batch moving pieces could. Checking the world's part and hers
  apart (strip the stepped piece's cells and look, then try the other
  exemptions on the rest) would answer both, at some looseness.
- **The by-her-clock drawn test's gate cost** (T1): about 10 s CPU,
  31.6 → 33.8 s wall for the stop hook's gate; it could run under
  `--profile deep` instead.
- **Commentary's frames in the real app:** screenshots are now async and
  gated on the real video being shown; the user should check commentary
  still attaches frames. A real `--vo=gpu` grab's time is unmeasured
  (the session logs each frame's at trace).
- **Desk before back:** where she can build at all, a desk she makes
  outweighs homework on the floor 4–5×, so she usually makes the desk
  before her back aches; gating the desk on the ache would make "a sore
  back, then the desk" the order (a design change, 10a).
- **A busy client's `school_out`** makes the empty home vanish with no
  rain (step 1's hand-off; another class).
- **Her door overlapping her furniture** (the user's report): the door
  batch, next.
- 5b's still-open items: rares a home can't show, her pane-corner column.
- In ASCII the bare stage's industrious afternoon sets off 3.05–3.26
  times a minute (fed, 15 minutes), over the line-art cap of 3; the band
  is pinned in line art (the user's client), and ASCII is dev and test
  only.

### Next: the door batch (planned, 2026-10-07)

The user's report (2026-10-06): *"she just left for school directly from
bed, from a door that's now stuck visible overlapping said bed."* The
closed door standing while she's out is by design ("there's really no
other way to know she *is* out"); the bug is where it stands. The ask:
*"make sure her external door has a reserved empty space next to the edge
of the screen."* The investigation is
[door/brief.md](proposals/2026-10-02-houseguest-mind/door/brief.md)
(sibling sites, geometry, regression tests; where it differs from this
section, this section wins), and the approved art is in
[door/art/](proposals/2026-10-02-houseguest-mind/door/art/) (committed,
not wired).

**Root cause.** The 08:15 school cut interrupts any use where she stands,
which for a bed, sofa or desk is *inside* the piece; `go_out` then opens
her door where she stands, the guest records that spot, and the only
check, `door_fits`, is `platform_at && restful`, and `restful` deliberately
accepts the cells of a piece her image takes in (that's how she lies in
her bed). The arrival reads an unfurnished terrain, so she comes home out
of the door inside the bed too. The class: her external door's spot comes
from her feet, and nothing asks that her door's box meet no piece. Work
by door, going out again after a dash, and the cold start in school hours
share it.

**The user's decisions:**
- **A reserved space** for her door at the screen's edge: floor against a
  wall, never over furniture; no piece stands in it, and no piece hangs
  low enough to meet it (the window included). **Poster and clock may hang
  above it.** The door's spot is computed from her home and the frame
  (one constructor, never her feet), so the overlap can't be represented
  inside the space; outside it (the space yielding, no home) a strict
  check that her box meets no piece's cover, the same in both modes.
- **The door is the approved sheet's B:** side-on, two columns, turned
  slightly toward the viewer, set in the wall line, with **the parcel flap
  in the door** (its lower half). Parcels come through it and rest just
  past the door's space.
- **While she's out:** her slippers on the floor before it and a card on
  the knob. The doorway's glimpse follows the window's sky. She visibly
  walks through, clipped at the wall line.
- **Work leaves through the same door** (screen edge first when her floor
  reaches one); every routine exit and return is through it.
- **Older homes** whose furniture fills the space: she clears it herself,
  through a felt rule ("Can't get to the door!") that phase 4's arranging
  answers by moving the piece. Until then the space yields for that frame
  and the door stands at the nearest spot meeting no piece.
- **Decided unless the user objects:** the no-home door uses the same edge
  chooser, unsaved; door-position logging (the spot at info as she goes
  out, a fallback or yield at debug with its reason).
- **Tests first:** the day-long property extended so a closed or open
  external door never meets a shown piece (both modes, with a bed, sofa
  or desk owned and school mornings); a bed, sofa and desk unit
  regression at 08:14; layout properties that no floor piece or window
  meets the space and that a home packing both ways keeps its anchors and
  order; an older record migrating without closeting anything.

### Later (ideas, the user)

Not planned yet; each wants its own brief:
- **A kitchen and a bathroom**, with their contents, for a **morning
  routine** (a bathroom-then-breakfast chain that ends with her walk to
  her door).
- **Osaka's comments on the actual onscreen episode**, replacing the
  marquee commentary (the user, after D7: she doesn't react to the film
  on her TV for now).
- From phase 4 (2026-10-03): posters delivered in cardboard tubes; more
  posters, paintings and plant varieties; a greenhouse or solarium she
  builds (cloud-watching under its glass too). From 5a/5b: maybe a cat
  who ignores the clock.

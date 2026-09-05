# Architecture

Last updated: 2026-08-21

This document describes DessPlay's internal structure: actor boundaries,
message flow, and concurrency model. For the external protocol, see
[network-design.md](network-design.md). For CRDT state, see
[sync-state.md](sync-state.md).

## Table of Contents

1. [Design Principles](#design-principles)
2. [Composition Root](#composition-root)
3. [Actor Model](#actor-model)
4. [Actor Definitions](#actor-definitions)
5. [Message Flow](#message-flow)
6. [Workspace Structure](#workspace-structure)
7. [Key Dependencies](#key-dependencies)

---

## Design Principles

### No Shared Mutable State

The prototype suffered from "a dozen threads and too many mutexes." DessPlay2
uses an **actor model**: each actor is a tokio task that owns its state and
communicates via typed message channels. No `Arc<Mutex<...>>`, no shared state.

### Typed Message Enums

Each actor has a dedicated message enum. The compiler enforces that only valid
messages are sent to each actor. This replaces the ad-hoc "event" types that
accumulated in the prototype.

### Unidirectional Data Flow

State flows in one direction: actors produce outputs in response to inputs.
There are no circular dependencies between actors. The event loop coordinates
without owning business logic.

### One Seam per Lifecycle Event

Every lifecycle event — a copy of a file turning up, a download stream dying,
a connection tearing down, a replica being adopted — funnels through exactly
one handler seam, so no channel that produces the event can skip the
consequences. The adoption seam is the model case (design.md, Download Cache
and Retention: every "copy turned up" channel routes through one adoption
point, so none can miss the redundant-download cancel).

The corollary is a review smell: a bug fix that adds handling *at one site*
for an event that can arise at several is treating the symptom. The 2026-08
audits found four instances of the same shape — a guard present on the close
path but not the install path, a requeue on one failure channel but not its
twin. When a fix wants a new handler, first ask whether the event deserves a
seam; if one exists, route through it instead.

### Testability at Every Level

Each actor can be tested in isolation by sending it messages and asserting on
outputs. The `SyncActor` can be tested without a network. The synchronous
`Ui` can be tested without a terminal. The `PlayerActor` can be tested with a mock
player.

Beyond isolation, the *whole client* must be constructible from a test —
see [Composition Root](#composition-root).

---

## Composition Root

`main()` is a thin shell. All actor creation and channel wiring lives in a
library-style entry point, roughly:

```rust
async fn run_client(
    config: Config,
    transport: impl Transport,      // quinn in prod, SimulatedTransport in tests
    player: impl Player,            // mpv in prod, MockPlayer in tests
    terminal: Backend,              // crossterm in prod, ratatui TestBackend in tests
    events: EventSource,            // real input in prod, injected events in tests
) -> Result<()>
```

Phase 5 status: the root exists in two layers.
`dessplay::client::spawn_client(connector, ClientConfig)` wires the
network and sync actors plus the event router — it is what the
multi-client harness calls (`dessplay-rendezvous/tests/common/mod.rs`).
`dessplay::run::run_headless` wraps it with the production inputs (QUIC
connector, TOFU pins, stored settings/state, Ctrl-C → Goodbye) and
serves both the headless client and `--seeder`. Phase 6 grows the
player/terminal/event parameters into the signature above.

The payoff is the multi-client simulation harness (see
[testing-strategy.md](testing-strategy.md)): N **complete** clients — every
actor, UI included — plus a real server actor, all in one
`current_thread` tokio runtime with paused time. This is the tier where
cross-client product behavior is tested ("A pauses, B's screen shows it")
without terminals, processes, or real sockets.

Anything `main()` does beyond argument parsing and constructing these
inputs is a bug: it would be behavior the harness cannot see.

---

## Actor Model

### Overview

```
                    +---------------+
                    |   Main Loop   |
                    | (coordinator) |
                    +---+-+-+-+-----+
                        | | | |
           +------------+ | | +------------+
           |              | |              |
   +-------v------+ +----v-v----+ +--------v-------+
   |  SyncActor   | | UI system | | PlayerActor    |
   | (CRDT state, | | (tui-realm| | (mpv IPC,      |
   |  op log)     | |  components| |  echo filter)  |
   +-------+------+ +-----------+ +--------+-------+
           |                               |
   +-------v------+               +--------v-------+
   | NetworkActor | (QUIC)        |   FileActor    |
   | (server conn,|               | (hashing, scan,|
   |  relay I/O)  |               |  cache, xfer)  |
   +--------------+               +----------------+
```

**Seeder composition:** in seeder mode (`--seeder`), only SyncActor,
NetworkActor, and FileActor are spawned -- no UI subsystem, no PlayerActor. The
main loop is the same; the routing arms for absent actors simply never fire.

The interactive client additionally spawns an [IrcActor](#ircactor) (the
optional IRC chat bridge), wired in by `run_interactive` rather than
`spawn_client` so seeders never get one.

### Communication

Actors communicate via `tokio::mpsc` channels. Each actor has:
- An **inbox** (`mpsc::Receiver<ActorMsg>`) -- messages it receives
- **Handles** to other actors' inboxes (`mpsc::Sender<OtherActorMsg>`)

The main loop holds handles to all actors and routes messages between them
using `tokio::select!`. It does not own business logic -- it's a mechanical
dispatcher.

### Lifecycle

1. Main function creates all channels
2. Spawns each actor as a `tokio::spawn` task
3. Enters the main select loop
4. On shutdown: drops senders, actors see closed channels and exit cleanly

---

## Actor Definitions

### SyncActor

**Owns:** All CRDT state (the `crdts` types), epoch counter.

**Receives:**
- `LocalOp(CrdtOp)` -- operation generated by this client (from UI or player)
- `RemoteOp(CrdtOp)` -- operation received from the server
- `RemoteMerge(CrdtSnapshot)` -- CvRDT state for merge (reconnection sync)
- `Snapshot(epoch, CrdtSnapshot)` -- full state replacement from server
- `GetState` -- request for current snapshot (for UI rendering)

**Produces:**
- `StateChanged(CrdtSnapshot)` -- sent through the event router to the UI
- `OutboundOp(CrdtOp)` -- sent to NetworkActor for transmission to server

**Persistence:** Periodically flushes state to SQLite. Playback position
updates are batched (not persisted on every 100ms tick).

### NetworkActor

**Owns:** QUIC connection to server (the only connection -- all peer traffic
is relayed through it), connection state, time sync state.

**Receives:**
- `SendOp(CrdtOp)` -- send operation to server (reliable + datagram, subject
  to the datagram size rule; playback position ops are datagram-only with a
  1s reliable fallback)
- `ConnectToServer(addr, password)` -- initiate server connection
- `ReportEof(Ed2kHash)` -- send `EofReached` to the server
- `SendPeerMessage(PeerId, PeerMessage)` -- file transfer traffic, wrapped
  in a relay envelope

**Produces:**
- `ServerOp(CrdtOp)` -- operation received from server
- `ServerSnapshot(epoch, CrdtSnapshot)` -- full state from server (reconnect
  with stale epoch, or daily compaction broadcast)
- `ServerMerge(CrdtSnapshot)` -- CvRDT state for merge on reconnection
- `PeerListUpdate(Vec<PeerInfo>)` -- updated peer list (role + presence)
- `TimeSyncUpdate(offset)` -- clock offset update
- `PeerMessageReceived(PeerId, PeerMessage)` -- relayed file transfer traffic
- `ConnectionStateChange(...)` -- connected/disconnected/error
- `LinkHealth(LinkHealthReport)` -- a 1Hz connection-health sample while
  authenticated (design.md, Connection Health Line): median time-sync probe
  RTT, consecutive unanswered probes, age of the last server frame (the
  server's 30s `StateHash` broadcast makes this a stalled-sync detector),
  and byte rates from per-connection counters covering the control,
  datagram, and relay planes. All measured with local monotonic `Instant`s
  inside the actor, so the sim stays deterministic (`Transport::link_stats`
  supplies quinn's path stats as display-only supplement; the sim returns
  `None`). **Not routed through the event stream**: the client router
  consumes it into a `watch::Receiver<Option<LinkHealthReport>>` on
  `ClientHandle` (latest value wins; cleared on disconnect) — 1Hz metrics
  in the event channel could fill it and stall the router, and with it
  state sync, behind droppable numbers.

### UI subsystem

There is no asynchronous `UiActor` task. `ui::app::Ui` is the synchronous
dispatcher that owns component state, focus, and the modal stack. Production's
`ui::shell` owns the terminal/input threads and exchanges `UiInput` and
`UserAction` values with the bridge loop; tests drive the same `Ui` directly.

**Receives:**
- `StateUpdate(CrdtSnapshot)` -- new CRDT state to display
- `TerminalEvent(Event)` -- keyboard/mouse input from crossterm
- `PlayerStatus(PlayerState)` -- current player position/state
- `SubtitleLine { text, speaker }` -- appended to the subtitle pane's
  rolling log; `speaker` (ASS `Name` field) optionally prefixes the cue in
  both text modes and colors the line in separate-pane mode
- `PresenceUpdate(Vec<PeerInfo>)` -- presence/role data for the Users pane

**Produces:**
- `UserAction(Action)` -- user intent (send chat, add file, seek, pause, etc.)
- These are translated by the main loop into `LocalOp` for SyncActor or
  commands for PlayerActor as appropriate.

**Rendering:** the UI shell feeds state and local inputs into `Ui`, which maps
them to component props and renders with ratatui. Input events flow through
the synchronous dispatcher and tui-realm component `on` methods, producing
`Msg` values that become `UserAction` outputs.

### PlayerActor

**Owns:** the running player (behind the `Player` trait: mpv over JSON
IPC in production, `MockPlayer` in tests), echo suppression state, the
drift corrector, the seek debouncer, the position broadcast timer, and
crash supervision (relaunch via a `PlayerFactory`). Launches and
relaunches run in a **background task**, never awaited inline in the
run loop's `select!` — mpv can legitimately take its full 30s socket
wait to come up (a slow startup is not a crash), and a parked actor
would stop servicing commands, backing the session's bounded player
channel up into its main loop. Attach mode instead uses a short bounded
re-attach probe with capped backoff.

**Receives** (`PlayerCommand`):
- `Load { file, path }` -- load a video file (opens paused)
- `SetPlaying(bool)` -- the *derived* group playback state, re-asserted
  on every state change; the actor dedups against what the player is
  already doing
- `SyncTo { position, timestamp, playing }` -- the seek authority's
  latest sample; the actor extrapolates it via the shared clock and
  feeds the delta to the `DriftController` (`actors/drift.rs`, pure
  logic: hysteresis, engage/hard-seek debounce, tapered + rate-limited
  slew -- shaped by what is audible, see its module docs), applying the
  returned action (set speed / hard seek) per design.md Playback Rules.
  Suspended while the local user is scrubbing.
- `ClockOffset(i64)` -- shared-clock offset updates (for extrapolation)
- `ShowOsd(text)` -- append a chat message to the rolling OSD log (the
  actor owns retention/expiry and renders it as one `osd-overlay` slot)
- `SetBlockerOverlay(text?)` -- set or clear the persistent "Waiting
  for ..." summary (its own `osd-overlay` slot; re-applied on relaunch)
- `Shutdown`

**Produces** (`PlayerOutput`):
- `UserPaused` / `UserUnpaused` -- a pause flip that was *not* an echo
  of our command
- `UserSeeked { from_millis, to_millis }` -- explicit user seek, debounced
  1500ms (scrubbing coalesces; automatic seeks never emit it)
- `PositionTick { position }` -- current position (100ms playing, 1s
  paused; extrapolated between player reports)
- `DurationKnown { file, duration }` -- probed duration, `NonZeroU64`
  (backfills the playlist entry when the adder didn't supply one; mpv's
  zero-duration report for a file it couldn't open is dropped, not sent)
- `SubtitleLine { text, speaker }` -- observed `sub-text/ass-full` change
  (feeds subtitle pane; `speaker` is the parsed ASS `Name`/actor field)
- `PathObserved { path }` -- the user loaded a file directly into the
  player (observed `path` property), a path we never commanded. Echoes of
  our own `loadfile` (including the placeholder PNG) are filtered out by
  comparing against the last commanded path. The session adopts it when
  the basename matches the now-playing entry (design.md, Manual File
  Mapping: drag-in adoption).
- `Eof { file }` -- file ended, reported once per file (the session
  layer forwards `EofReached` to the server, which owns the transition)
- `FatalCrash` -- the player died twice within 30s (the session layer
  pauses globally and notifies chat)
- `GaveUp` -- the player died three times within 30s; the actor stopped
  relaunching until a different file is loaded (loading a new now-playing
  resets the crash counter and respawns the player). The session layer
  pauses globally and notifies chat, same as `FatalCrash`.

**Echo suppression:** expected-state tracking, entirely on our side —
mpv does *not* flag events as user-vs-programmatic. The actor remembers
what it commanded (a queue of expected pause flips, a counter of
expected seeks, the path of the last `loadfile`); an observation matching
the queue head (or, for `path`, equal to the last commanded path) is an
echo and is swallowed, anything else is the user. Misattribution self-heals:
the actor never enforces locally (observe-and-correct), so the next
`SetPlaying` round trip re-converges the player. The mpv layer
additionally hides two pieces of pure mechanics: the forced pause
around `loadfile`, and the pause mpv performs itself when `keep-open`
hits end of file (which arrives *before* `eof-reached` and is held
briefly for attribution).

**File attribution:** evidence-based, not belief-based. `loadfile` is
asynchronous, so after a `Load` command the player keeps emitting the
*previous* file's observations until the new file actually opens; the
commanded file (which tags positions, seeks, EOF, and duration reports)
already names the new one during that window. The actor therefore tracks
the last *observed* `path` — mpv's own report, ordered with every other
event — and accepts file-attributed observations only while it equals
the commanded path; observations in the gap are dropped as the old
file's (design.md, Events from Player). Drift correction sits behind the
same gate (authority samples are ignored and the `DriftController` reset
while the player is off the commanded file — e.g. a user drag-in). Two
deliberate exceptions: the load-failure report (a file that never opens
may never get a path observation, and suppressing the report is the
worse failure direction), and programmatic-seek echo accounting (an
outstanding echo is consumed even by a gated-out `Seeked` — it is our
own seek, and leaking it would swallow the user's next genuine seek).

### Session policy (`session.rs`)

Not an actor: the synchronous decision core between synced state and
the PlayerActor, mirroring how `ui::app::Ui` is synchronous behind the
UI threads. `PlayerWiring` maps (state view, peer list, player outputs,
matcher results) to directives — player commands, mutations, EOF
reports, matcher runs; `SessionShell` executes those directives against
the real channels and lazily spawns the player actor on the first load.
Player commands that are re-derived on every snapshot (`SetPlaying`,
`SyncTo`, `SetBlockerOverlay`) are sent with `try_send` and dropped when
the actor's bounded channel is full — the next snapshot re-sends them
(the shell re-arms the wiring's dedup for the sparse ones), so a slow
player actor can never wedge the main loop; one-shot commands (`Load`,
`Shutdown`, …) keep the awaited send.
`run_interactive` and the multi-client harness drive the same shell, so
the full pipeline (player → wiring → sync → server → peers → their
players) is covered in tests without a terminal or a real mpv.

**Chat narrator.** `PlayerWiring::narrate` (called at the end of
`on_state`) turns state changes into the chat log's
[system messages](../docs/design.md) (joins, pauses, seeks, not-watching,
new files). It diffs a small captured slice of each (state view, peer
list) against the previous one and emits a `Directive::SystemLine` per
transition; the shell stamps it and forwards it to the UI as a local
`UiInput::System` line (the same path archive/command feedback uses),
*not* through the synced GList. It is pure and snapshot-diffing, so it is
tested like the rest of the wiring: feed it successive snapshots and
assert on the lines. The lone non-derived case is the player crash: the
`FatalCrash` arm already writes an ordinary synced `Mutation::Chat`, which
reaches every client (and late joiners) without special handling. The
09:00 day separator is *not* here -- it is a render-time insertion in the
UI's chat builder, computed from message timestamps.

### The bridge loop (`run::SessionLoop`)

The interactive client's select loop — actions from the UI in, snapshots
and subtitles out, player/matcher/hash completions in between. Extracted
from `run_interactive` into a struct so it runs (and is tested) without
a terminal.

It also owns the **connection-health pipeline** (design.md, Connection
Health Line): a select arm on `ClientHandle.health.changed()` merges each
`LinkHealthReport` with the torrent engine's live speeds
(`TorrentEngine::speeds`, sampled on a retained `Arc`), classifies it
(`props::classify_health`) through an anti-flap hysteresis, and stores the
result for the UI snapshot's `HealthProps` — cleared on disconnect so
stale trouble never outlives the connection that measured it.

**The advisor** (`advisor.rs`) is the seam behind the health row's
suggestion slot. The loop feeds it subtitles (deduped with the shared
`props::subtitle_collapse`, wherever `UiLines` are forwarded to the UI)
and health samples (throttled to 5s inside the advisor); providers —
`AdvisorProvider`, must never block — deliver `SuggestionUpdate`s through
a channel the loop select-arms into the snapshot. The rule-based provider
is the only one today. `SyncEvent::Diverged` (emitted by the sync actor's
divergence alarm) is consumed here and rides the advisor context.

**The commentary engine** (`commentary.rs`; design.md, AI Commentary) is
a sibling of the advisor rather than one of its providers — its output
is a *synced mutation*, not a local suggestion. The loop select-arms its
interval ticker (armed only when a token + interval are configured): a
tick gathers gates (connected, `derive::playback_active`, the client's
own `FileAvailability::Ready` for now-playing, a known series), reuses
the advisor's assembled context (series/episode/subtitle ring, now 100
lines), lets the engine plan (pick vs. reuse the persistent commentator;
seeded-RNG 5% re-roll), requests a player screenshot through the
`SessionShell`, and spawns the blocking Anthropic calls under
`spawn_blocking` — the liveness rule holds, the loop never waits on
HTTP. A second arm receives the finished job and issues
`Mutation::SetMarquee`; the author's own UI sees it through the ordinary
local-apply → snapshot path, identically to every peer. Settings saves
reconfigure the engine live (model swap / ticker re-cadence); an
in-flight call that outlives a disable is discarded on arrival.

**Liveness rule: nothing in the bridge loop may block or run long.**
Every await in an arm body must complete promptly — channel sends to
live actors, oneshot view queries, nothing else. Long work (ed2k
hashing, media-root scans) is started in the background through
`SessionShell` and returns through its completion channels
(`resolutions`, `hashed`) as separate select arms. The 2026-06-12 bug
class: an inline `spawn_blocking(...).await` for playlist-add hashing
starved the loop for the duration of every multi-gigabyte hash — frozen
UI, serialized adds, and a queued Quit that was never read (Ctrl-C
appeared dead). The supervision tests in
`dessplay-rendezvous/tests/interactive_loop.rs` pin this: a hash pointed
at a FIFO (blocks forever) must not stop a quit from landing or other
adds from reaching the playlist.

### FileActor

**Owns (Phase 9A):** the hash cache (ed2k root + per-block hashes,
validated by `(mtime, size)`), manual mappings, download-cache
bookkeeping (retention/eviction), watch-history writes, and the
placeholder renderer. Its own `Storage` connection (WAL handles
concurrency with the sync actor's). Phase 9B adds download coordination
(chunks, bitfields, prefetch).

It also runs the **media-library scan**: at startup and on a timer (about a
minute interactive, a day for a seeder) it walks the media roots, `stat`s each
file, re-hashes only those whose `(mtime, size)` changed, and reports the
indexed hashes so the session can insert `lookup_requests` for any that still
lack metadata (see "Media Library Scanning" in design.md). The hash cache
doubles as the library index, so this also pre-warms playlist-add resolves.
Scan *hashing* defers while transfer traffic (serving or downloading) is
recent and resumes when it goes quiet (design.md, Media Library Scanning
— #21). The actor also watches **mismatched resolutions** for quiescence
(design.md, Content Hash — #26): a 1s `stat` poll per watched file,
re-resolving once a changed file holds still.

The actor owns media-root lifecycle as well. Each hash row records its owning
root and SQLite stores whether that root is wholly vanished or was removed
from the effective configuration. A scan with no surviving recorded file
marks the root vanished and retracts locally advertised availability without
deleting hashes; one surviving record proves the root online and permits
per-file pruning. Removed roots retain hidden rows for seven days. This policy
is local-only and introduces no CRDT or wire state.

Spawned and driven by the `SessionShell` (Phase 7's session policy
layer): the shell sends `FileCommand`s and receives `FileOutput`s on one
channel pair, replacing the ad-hoc `spawn_blocking` resolves and hashes
the shell did before Phase 9. Heavy IO (scans, hashing, PNG rendering)
runs in `spawn_blocking` subtasks whose completions return through an
internal channel, so the inbox stays live — a stuck hash never starves a
resolve or an eviction pass (the bridge loop's liveness rule again).

**Receives (`FileCommand`):**
- `Resolve { file, filename }` -- find a local copy: manual mapping
  first, then a media-root scan with hash-cache-backed verification (a
  candidate whose `(mtime, size)` match a cache row is trusted without
  re-reading; anything else is hashed once and the hash cached)
- `HashAdd { path, after }` -- hash a file for a playlist add (cache hits
  return instantly; misses stream progress for the overlay)
- `SetManualMapping { file, path, series }` -- persist a user-picked file
  (and the series' last-used directory) and resolve it Verified
- `RecordWatched(record)` -- personal watch history (the 85% rule); with
  the auto-archive policy on, also archives the file if it is cache-only
- `CheckSeriesKnown { file, series, key }` -- is the series in watch
  history? (drives the missing-file branch)
- `RenderPlaceholder { file, lines }` -- render the not-watching PNG
- `Archive { file, series_name, filename }` -- move a cached download to
  `<download root>/<series>/<filename>` or directly to
  `<download root>/<filename>` according to the actor's archive policy
- `SetArchivePolicy(policy)` -- subdirectory layout and the auto-on-watched
  trigger (settings save); the same policy drives the manual `A` and the
  automatic archive, and a completed download of an already-watched file
- `RunEviction { protected, group_watched }` -- eviction pass (startup and
  EOF-advance; never evicts now-playing/queued/protected)
- `SetMediaRoots` / `SetRetention` -- settings changes
- `SetTorrentEnabled(bool)` -- the live BitTorrent toggle (design.md,
  BitTorrent Downloads): disabling removes every seeding torrent (files
  deleted) and cancels pending imports; enabling only sticks when the
  engine was constructed at startup
- `SearchNyaa` / `StartNyaaImport` / `CancelNyaaImport` -- playlist-pane
  browse search and local pending imports. Search/metainfo reads and final
  ed2k hashing run off-thread; the actor polls the torrent engine on its normal
  tick and promotes a completed temporary import to its discovered hash.

**Produces (`FileOutput`):**
- `Resolved { file, resolution }` -- `Verified | HashMismatch | NotFound`
- `Hash(HashEvent)` -- playlist-add hash progress/completion
- `SeriesKnown { file, series, known }` -- answer to `CheckSeriesKnown`
- `PlaceholderReady { file, path }` -- the PNG is on disk
- `Archived { file, result }` / `Evicted { files }`
- `NyaaSearchFinished` / `NyaaImportProgress` / `NyaaImportFinished` --
  local-only UI/session events; only successful completion produces the
  ordinary shared playlist mutation.

Phase 9B added the transfer side: `StartDownload` / `PeerMessage` /
`TransferStream` commands and `SendPeer` / `OpenTransfer` /
`Availability` / `DownloadComplete` outputs. The scheduling brain is
`download.rs` (`Downloads` — request window, rarest-first + sequential
window, source snub, urgent set with endgame as its special case;
synchronous and unit-testable); on-disk
assembly + ed2k block verification is `chunkstore.rs`. Chunk traffic
rides per-transfer data streams (network-design.md, protocol v9): the
actor opens one per (source, file) for its downloads, and **serves**
each incoming stream with a dedicated task — reading
`ChunkRequest`/`Cancel` into a per-transfer queue and streaming
`ChunkData` back under a shared upload-rate token bucket, paced by the
stream write itself, so one slow downloader backpressures only its own
task. `BlockHashRequest` and availability stay on the relay stream via
`SendPeer`. The session (`PlayerWiring::plan_download`) drives downloads
for the now-playing file plus a prefetch window — emitted even with no
peer source (the actor treats an empty source set as the ordinary
awaiting-source state and picks sources up from a later refresh).
Seeder auto-fetch (headless, fetch everything) is the remaining 9B
piece.

Two contracts keep the stream bookkeeping honest (2026-08-12 review):

- **Answered requests.** Every `OpenTransfer` the file actor emits is
  answered by the network actor — with a `TransferStream` or an
  explicit `TransferStreamFailed { peer, file }` — never silently
  dropped, because the actor's "already asked" latch is its pending
  message queue for that `(peer, file)`. On failure it drops the
  queued sends and requeues the source's in-flight chunks; the next
  download tick re-plans and re-asks (a paced retry, so a down link is
  not re-asked in a tight loop). A `TransferLinkReset` (bridged from
  the network's Disconnected event — the server tears the transfer
  connection down with the control connection) fails every pending
  open whose answer died with the aborted link task.
- **Generation-stamped lifecycle.** Download streams and serve tasks
  are keyed by `(peer, file)` and a fresh stream replaces a stale
  predecessor, but their end events (`DownloadClosed`/`ServeEnded`)
  arrive on a different channel than the streams, so a predecessor's
  late event can drain after the replacement is installed. Each
  installation carries a monotonic generation; end events name theirs
  and are ignored when stale — otherwise a stale `ServeEnded` aborts
  the fresh serve task and the downloader eats a 30s snub. A
  closed/reset download stream is also the immediate snub signal: the
  `DownloadClosed` arm requeues that source's in-flight chunks and
  re-plans at once (`Downloads::on_source_stream_lost`) instead of
  waiting out the 30s timeout.

**The Nyaa browse import** (design.md, BitTorrent Downloads) is the one
torrent feature: an explicit, user-driven search-and-download,
completely separate from `StartDownload` (missing playlist files are
fetched from peers only). **Interactive clients only**: `run.rs` wires
the engine + nyaa source for them and leaves both `None` for seeders —
a file nyaa can supply makes the seeder redundant, so the seeder
downloads only from peers and skips `StartDownload` entirely when no
peer source exists. Two seams, both inside the file actor:

- `torrent/nyaa.rs` — the browse search: a blocking `NyaaSource` trait
  (`HttpNyaaSource` in production, canned RSS in tests) plus pure
  `parse_rss` and torrent-metainfo inspection
  (`browse_single_file_results`), run on the blocking pool.
- `torrent/engine.rs` — the `TorrentEngine` trait the actor drives
  (adds/removes fire-and-forget, sync `import_status` polled on the
  existing download tick); `FakeTorrentEngine` for actor tests, and
  `torrent/rqbit.rs` (librqbit session, **no persistence** — torrents
  die with the process) in production. `FileConfig` carries both
  handles as `Option`s — `None` disables the torrent path entirely.

A pending import is keyed by a temporary `TorrentImportId` (its ed2k
hash cannot exist before download); no provisional identity enters the
CRDT or wire protocol. The finished payload is ed2k-hashed off-thread
(`Done::NyaaImportHashed`), hardlinked to the hash-addressed cache
path, promoted in the engine to its discovered `Ed2kHash`, and
converges into `on_download_complete`, so everything downstream
(serving, archive, eviction) is identical to a peer download. Eviction,
`ForgetLocalFile`, and the live-disable path drop a promoted import's
torrent (seeding ends when the cached file goes — and always at
shutdown, since nothing is persisted); at startup the actor sweeps
`<cache>/torrents/` wholesale, sparing only a directory that still
hosts a registered cache file.

### IrcActor

A small bridge actor (`actors/irc.rs`) that mirrors the local user's chat
to an IRC channel and surfaces external IRC users back into the chat pane
(see "IRC Bridge" in design.md). It owns one `tokio-rustls` TLS
connection to the IRC server (reusing the project's pinned rustls — no
native-tls), parses the line protocol, and reconnects with capped
exponential backoff. The protocol parsing/formatting (PRIVMSG, PING,
nick derivation, the `*Dess` filter, CTCP ACTION) lives in pure
functions; the loop is driven over an in-memory `tokio::io::duplex` pipe
in tests.

**Interactive-only.** It is spawned from `run_interactive`, *not* the
shared `spawn_client`, so seeders (headless, no chat) never get one. It
talks only to the bridge loop, never to other actors directly.

**Receives (`IrcCommand`):** `SendChat(text)` (forward one of our chat
messages), `Reconfigure(IrcConfig)` (settings changed — reconnect or
idle), `Shutdown` (QUIT and exit).

**Produces (`IrcEvent`):** `Connected` / `Disconnected { reason }`
(mapped to local system lines), `Message { from, text, action }` (an
external user's line, mapped to a local-only `UiInput::Irc` chat line).

**Wiring.** The bridge loop taps `Mutation::Chat` in its `UserAction::
Mutate` arm and forwards the text via a lossy `try_send` (never awaiting
a possibly-reconnecting actor — the liveness rule). A guarded
`irc_events.recv()` select arm drains events without ending the session
when the channel closes. `SaveSettings` sends a `Reconfigure` only when
the derived `IrcConfig` actually changed (an unrelated save, e.g. an F2
subtitle-mode cycle, must not force a reconnect); shutdown sends
`Shutdown` and waits (bounded) for the actor to drop its receiver.

### AniDB worker (server-side, Phase 8)

One background task inside the rendezvous server, not a client actor.
Each pass it refreshes the anime-titles dump when due (daily; one
blocking GET on the blocking pool), drains the replicated
`lookup_requests` GSet and newly-seen series ids into the SQLite
queues, and performs one due lookup — files before series. As it drains
each request it also records the file's identity (filename + size) into
the broadcast `FileCatalogEntry`, so clients can add files they don't
hold. Results land as server-authored LWW writes (`AniDbMetadata`,
`FileCatalogEntry`, `SeriesRelations`) broadcast like any other server
op. The `lookup_requests` set now carries each client's whole indexed
library, not just playlist entries, so the per-hash `anidb_queue`
de-duplication is what keeps the rate-limited lookup load bounded.

It is doubly abstracted for tests: `AniDbApi` (the rate-limited UDP
client; canned tables in tests) and `AniDbHost` (clock + state view +
server writes + storage; the real server or an in-memory mock). No
test ever touches the real API — the only thing that does is the
manual `anidb-probe` binary. Pacing lives in the client (2s floor,
1 per 4s sustained with burst 60, 5s missing-response penalty,
busy/ban backoff); when idle the worker sleeps until the next
scheduled attempt, capped at 60s so fresh lookup requests are noticed
promptly.

---

## Message Flow

### User Sends Chat Message

```
UI subsystem            Main Loop               SyncActor           NetworkActor
  |                        |                       |                     |
  |-- UserAction(Chat)  -->|                       |                     |
  |                        |-- LocalOp(Chat)  ---->|                     |
  |                        |                       |-- OutboundOp  ----->|
  |                        |                       |-- StateChanged  --->|
  |                        |                       |                     |-- send to server
  |<- StateUpdate ---------|<- StateChanged -------|                     |
```

### User Seeks in Player

```
PlayerActor             Main Loop               SyncActor           NetworkActor
  |                        |                       |                     |
  |-- UserSeeked(from,to)->|                       |                     |
  |                        |-- LocalOp(UserSeek) ->|                     |
  |                        |-- LocalOp(Position) ->|                     |
  |                        |                       |-- OutboundOps  ---->|
  |                        |                       |-- StateChanged  --->|
  |                        |                       |                     |-- send to server
```

### Remote User Seeks (We Follow)

```
NetworkActor            Main Loop               SyncActor           PlayerActor
  |                        |                       |                     |
  |-- ServerOp(SeekAuth) ->|                       |                     |
  |-- ServerOp(Position) ->|                       |                     |
  |                        |-- RemoteOp  --------->|                     |
  |                        |                       |-- StateChanged  --->|
  |                        |                       |                     |
  |                        |<- (check authority)   |                     |
  |                        |                       |-- SyncTo(pos)  ---->|
  |                        |                       |   (ignore/slew/seek |
  |                        |                       |    per drift band)  |
```

### File Download

```
FileActor               Main Loop               NetworkActor
  |                        |                       |
  |-- ChunkNeeded -------->|                       |
  |                        |-- SendPeerMessage --->|
  |                        |                       |-- (relay via server)
  |                        |                       |
  |                        |<- PeerMessageRecv ----|
  |<- ChunkReceived -------|                       |
  |                        |                       |
  |-- AvailUpdate -------->|                       |
  |                        |-- LocalOp(FileAvail)->| (to SyncActor)
```

### EOF Transition

```
PlayerActor             Main Loop               NetworkActor          (Server)
  |                        |                       |
  |-- Eof ---------------->|                       |
  |                        |-- ReportEof(hash) --->|
  |                        |                       |-- EofReached ----> server:
  |                        |                       |                    mark watched,
  |                        |                       |                    advance now-playing,
  |                        |                       |                    take seek authority
  |                        |                       |<- StateOp(s) ------|
  |                        |<- ServerOp -----------|
  |                        |   (now-playing changed -> LoadFile next)   |
```

---

## Workspace Structure

Both `dessplay` and `dessplay-rendezvous` are **library crates with a
thin `main.rs`** — required by the composition root, and what lets
cross-crate tests (and the multi-client harness) run real clients
against the real server in one process.

```
Cargo.toml                    (workspace root)
dessplay-core/                (shared library)
  src/
    lib.rs
    types.rs                  (FileId, UserId, ActorId, timestamps)
    lww.rs                    (Lww<V>, LWW resolution)
    state.rs                  (CrdtState, CrdtOp, StateSnapshot)
    playlist.rs               (Identifier-based playlist helpers)
    hash.rs                   (ed2k root + block hashes)
    wire.rs                   (postcard encode/decode)
    test_support.rs           (script + cluster generators; feature-gated)
    net/
      message.rs              (WireMessage, ServerControl, PeerInfo)
      transfer.rs             (PeerMessage, RelayEnvelope, Bitfield,
                               chunk geometry; Phase 9B)
      framing.rs              (length-prefixed stream frames)
      transport.rs            (Transport/Connector/Listener traits)
      timesync.rs             (NTP-style offset estimation)
      tofu.rs                 (TOFU verifier, server cert persistence)
      quic.rs                 (quinn impls, shared transport config)
      sim.rs                  (SimulatedTransport; feature-gated)
  fuzz/                       (cargo-fuzz targets)
dessplay/                     (client: lib + thin binary)
  src/
    lib.rs / main.rs
    client.rs                 (composition: spawns + wires the actors)
    actors/
      network.rs              (NetworkActor)
      sync.rs                 (SyncActor)
      player.rs               (PlayerActor: echo suppression, drift,
                               debounce, crash supervision)
      file.rs                 (FileActor: hash cache, matching, watch
                               history, cache eviction/archive; Phase 9)
    ui/                       (components, dispatcher, threads shell)
    player/                   (Player trait; mpv JSON IPC; MockPlayer)
    session.rs                (PlayerWiring policy + SessionShell glue;
                               drives the FileActor)
    advisor.rs                (the suggestion-slot seam: context
                               collection, provider trait, rule provider)
    commentary.rs             (AI commentary engine: commentator state,
                               Anthropic model seam, marquee writes)
    download.rs               (Downloads: chunk scheduling — request
                               window, rarest-first, snub, urgent set;
                               Phase 9B)
    chunkstore.rs             (on-disk chunk assembly + ed2k block
                               verification + resume; Phase 9B)
    placeholder.rs            (not-watching PNG; image + ab_glyph + the
                               embedded DejaVu Sans in assets/)
    run.rs                    (mode entrypoints: interactive, headless,
                               import, dump)
    import.rs                 (CSV importer; Phase 6)
    storage.rs                (SQLite persistence; hash_cache added in 9A)
    config.rs                 (typed settings)
  assets/                     (DejaVuSans.ttf for the placeholder + license)
dessplay-rendezvous/          (server: lib + thin binary)
  src/
    lib.rs / main.rs
    server.rs                 (accept loop, auth, peer list, time sync,
                               state sync, compaction, and the file-
                               transfer relay — one relay BiStream per
                               peer, forwarded by username; Phase 9B)
    anidb/                    (AniDB integration; Phase 8)
      protocol.rs             (pure UDP codec: commands, masks, parsing)
      schedule.rs             (pure re-validation scheduling rules)
      client.rs               (rate limiter, sessions, AniDbApi trait)
      titles.rs               (anime-titles dump fetch/parse; name search)
      record.rs               (sanitized record/replay of real exchanges)
      worker.rs               (queue drainer; AniDbHost trait to the server)
    storage.rs                (server-side SQLite)
    bin/anidb-probe.rs        (manual real-API probe; never run by tests)
  tests/                      (sim connection tests + real-QUIC smoke)
```

---

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `tokio` | Async runtime, channels, timers |
| `quinn` | QUIC transport |
| `rustls` + `rcgen` | TLS and certificate generation |
| `crdts` | CRDT data types (Map, GList, GSet, Identifier) |
| `postcard` | Compact binary serialization |
| `rusqlite` (bundled) | SQLite persistence |
| `tui-realm` | Elm-architecture TUI framework |
| `ratatui` + `crossterm` | Terminal rendering and input |
| `serde` + `serde_json` | Serialization (general + mpv IPC) |
| `image` | Placeholder PNG generation |
| `strsim` | Edit distance for file matching |
| `ureq` + `flate2` | Server-side anime-titles dump fetch (one GET/day) |
| `proptest` | Property-based testing |
| `insta` | Snapshot testing |
| `cargo-fuzz` / `libfuzzer-sys` | Fuzz testing |


## Local roguelike (2026-09-05)

`roguelike.rs` is a pure deterministic simulation with a serializable RNG.
`roguelike_store.rs` owns versioned local saves and completed-run outbox/history
using a short SQLite transaction. The UI emits `UserAction::Roguelike`; the
session executes it and replies with `UiInput::Roguelike` only after commit.
A full UI input channel retains and retries this response, so game input
cannot remain locked because a snapshot crowded out its acknowledgement.

The session retries pending reports at startup, after game actions, and every
30 seconds. `SyncCommand::PublishLocalReport` preserves the saved timestamp,
deduplicates the exact chat message, and flushes the sync database before
acknowledgement. Ordinary reconnect/merge delivers offline reports to peers.
No new actor or wire message is needed, and seeders never play the game.

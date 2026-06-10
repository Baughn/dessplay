# Architecture

Last updated: 2026-06-10

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

### Testability at Every Level

Each actor can be tested in isolation by sending it messages and asserting on
outputs. The `SyncActor` can be tested without a network. The `UiActor` can
be tested without a terminal. The `PlayerActor` can be tested with a mock
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
   |  SyncActor   | | UiActor   | | PlayerActor    |
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
NetworkActor, and FileActor are spawned -- no UiActor, no PlayerActor. The
main loop is the same; the routing arms for absent actors simply never fire.

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
- `StateChanged(CrdtSnapshot)` -- sent to UiActor when state changes
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

### UiActor

**Owns:** tui-realm `Application`, component state, terminal handle.

**Receives:**
- `StateUpdate(CrdtSnapshot)` -- new CRDT state to display
- `TerminalEvent(Event)` -- keyboard/mouse input from crossterm
- `PlayerStatus(PlayerState)` -- current player position/state
- `SubtitleLine(String)` -- appended to the subtitle pane's rolling log
- `PresenceUpdate(Vec<PeerInfo>)` -- presence/role data for the Users pane

**Produces:**
- `UserAction(Action)` -- user intent (send chat, add file, seek, pause, etc.)
- These are translated by the main loop into `LocalOp` for SyncActor or
  commands for PlayerActor as appropriate.

**Rendering:** The UiActor runs tui-realm's event loop internally. It receives
state updates and maps them to component props. Input events flow through
tui-realm's focus and routing system, producing `Msg` values that become
`UserAction` outputs.

### PlayerActor

**Owns:** mpv process handle, IPC connection, echo suppression filter,
position broadcast timer.

**Receives:**
- `LoadFile(path)` -- load a video file
- `Pause` / `Unpause` -- control playback
- `Seek(position)` -- seek to position
- `SyncTo(position)` -- authority position; the actor picks the drift band
  (ignore / slew via mpv `speed` / hard seek) per design.md Playback Rules
- `ShowOsd(text)` -- display message on video
- `SetSeekAuthority(ActorId)` -- who to sync position to

**Produces:**
- `UserPaused` -- user paused in player (not an echo of our command)
- `UserUnpaused` -- user unpaused in player
- `UserSeeked(position)` -- user seeked in player (not an echo)
- `PositionTick(f64)` -- current position (100ms interval)
- `SubtitleLine(String)` -- observed `sub-text` change (feeds subtitle pane)
- `Eof` -- file ended (main loop forwards as `ReportEof` to NetworkActor;
  the server owns the transition)
- `Crashed` -- player process died

**Echo suppression:** The PlayerActor tags commands it sends to mpv. When
mpv reports events, the actor checks whether they're echoes of its own
commands. Only user-initiated events are forwarded as outputs. mpv
distinguishes user-initiated seeks from programmatic ones, which helps.

### FileActor

**Owns:** File hash cache (including ed2k per-block hashes), media root
index, download state, the download cache (retention/eviction), prefetch
queue.

**Receives:**
- `ScanMediaRoots(paths)` -- index available files
- `HashFile(path)` -- compute ed2k hash (root + block hashes)
- `MatchFile(filename)` -- find local file for a playlist item
- `StartDownload(FileId, peers)` -- begin chunk-based download
- `ChunkReceived(FileId, index, data)` -- store received chunk (block-verified
  as blocks complete)
- `GetAvailability(FileId)` -- query which chunks we have
- `Archive(FileId)` -- move cached file into the download root
  (`<series>/<season>/<filename>`)
- `RunEviction` -- eviction pass (sent at startup and on EOF-advance)

**Produces:**
- `HashResult(path, Ed2kHash)` -- completed hash
- `MatchResult(filename, Option<path>)` -- file match result
- `AvailabilityUpdate(FileId, BitVec)` -- our bitfield changed
- `DownloadComplete(FileId, path)` -- file fully downloaded
- `ChunkNeeded(FileId, Vec<u32>, PeerId)` -- request chunks from peer

Prefetch (queued playlist entries ahead of now-playing; everything, for
seeders) is driven by the main loop from playlist state: it sends
`StartDownload` for entries within the prefetch policy.

---

## Message Flow

### User Sends Chat Message

```
UiActor                 Main Loop               SyncActor           NetworkActor
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
  |-- UserSeeked(pos)  --->|                       |                     |
  |                        |-- LocalOp(SeekAuth) ->|                     |
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
      ui.rs                   (UiActor; Phase 6)
      player.rs               (PlayerActor; Phase 7)
      file.rs                 (FileActor; Phase 9)
    ui/                       (tui-realm components; Phase 6)
    player/                   (mpv, echo filter, mock; Phase 7)
    import.rs                 (CSV importer; Phase 6)
    storage.rs                (SQLite persistence)
    config.rs                 (typed settings)
dessplay-rendezvous/          (server: lib + thin binary)
  src/
    lib.rs / main.rs
    server.rs                 (accept loop, auth, peer list, time sync;
                               state sync + compaction in Phases 4-5)
    anidb.rs                  (AniDB UDP API client; Phase 8)
    relay.rs                  (file transfer relay; Phase 9)
    storage.rs                (server-side SQLite)
  tests/                      (sim connection tests + real-QUIC smoke)
```

---

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `tokio` | Async runtime, channels, timers |
| `quinn` | QUIC transport |
| `rustls` + `rcgen` | TLS and certificate generation |
| `crdts` | CRDT data types (MVReg, Map, GList, GSet, Identifier) |
| `postcard` | Compact binary serialization |
| `rusqlite` (bundled) | SQLite persistence |
| `tui-realm` | Elm-architecture TUI framework |
| `ratatui` + `crossterm` | Terminal rendering and input |
| `serde` + `serde_json` | Serialization (general + mpv IPC) |
| `image` | Placeholder PNG generation |
| `strsim` | Edit distance for file matching |
| `proptest` | Property-based testing |
| `insta` | Snapshot testing |
| `cargo-fuzz` / `libfuzzer-sys` | Fuzz testing |

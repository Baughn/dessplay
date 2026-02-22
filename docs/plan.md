# DessPlay Implementation Plan

Last updated: 2026-02-20

11 phases, bottom-up. Each phase produces testable artifacts. The first
user-facing demo (TUI with chat + shared playlist) arrives at Phase 6;
full watch-party experience at Phase 7.

## Workspace Layout

```
Cargo.toml                    (workspace root)
dessplay-core/                (shared library: types, CRDTs, protocol, sync, network trait)
dessplay/                     (client binary: TUI, player bridge, file management)
dessplay-rendezvous/          (server binary: auth, relay, compaction, AniDB)
```

---

## Phase 1: Foundation & CRDTs

Status: Completed

**Goal**: Workspace, shared types, all CRDT implementations with property tests.
No networking — pure logic.

### What gets built
- Cargo workspace with three crates
- Core types: `FileId` (ed2k hash), `UserId`, `PeerId`, timestamps
- Wire protocol message types (postcard serialization)
- LWW Register
- Playlist Op Log CRDT (Add/Remove/Move)
- Chat append-only log (per-user)
- Version vectors and gap detection
- CrdtState: combined state container with snapshot generation
- ed2k hash computation

### Key crates
`serde`, `postcard`, `ed2k` (or manual impl)

### Testing
- proptest: convergence (same ops in any order → same snapshot) for all CRDTs
- Unit tests: individual op application, edge cases
- Fuzz targets: CrdtOp replay (never panics)

### Milestone
`cargo test` passes with comprehensive CRDT coverage. Types serialize
round-trip correctly.

---

## Phase 2: Storage & Configuration

Status: Completed

**Goal**: SQLite persistence, config management.

### What gets built
- SQLite schema + migrations (rusqlite, bundled)
- Persist/restore CRDT snapshots and op logs (keyed by epoch)
- Local config: username, server, media roots, player choice, password
- Watch history: file hash → watched, last-watched timestamp
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

## Phase 2.5: Debugging

Status: Completed

**Goal**: Debugging tools for network and storage layers.

### What gets built
- `--dump` flag on both client and server binaries
- Reads SQLite database, reconstructs full CRDT state from snapshot + op log
- Pretty-prints all stored data: config, media roots, CRDT state, watch history,
  file mappings, TOFU certs (client); CRDT state, AniDB queue (server)
- `--help` flag on both binaries
- Manual arg parsing (no clap dependency)

### Files added/modified
- `dessplay/src/dump.rs` (new): client dump logic
- `dessplay-rendezvous/src/dump.rs` (new): server dump logic
- `dessplay/src/main.rs`: arg parsing, `--dump`/`--help`
- `dessplay-rendezvous/src/main.rs`: arg parsing, `--dump`/`--help`
- `dessplay/src/storage.rs`: added `get_all_tofu_certs()`, `get_all_file_mappings()`
- `dessplay-rendezvous/src/storage.rs`: added `AniDbQueueEntry`, `get_all_anidb_queue()`

### Testing
- Smoke tests: empty DB and populated DB for both client and server dump modules
- 4 new tests (2 per binary)


---

## Phase 3: Network Layer

**Goal**: QUIC transport, rendezvous protocol, peer connections, time sync.
No state sync yet — transport and connection management only.

### What gets built
- `Network` trait (the testability seam)
- `QuicNetwork`: real QUIC via quinn, self-signed certs, length-prefixed postcard
- `SimulatedNetwork`: in-process test network with configurable loss, latency,
  partitions, reordering, bandwidth limits
- TOFU certificate management
- Client → Rendezvous: auth flow, peer list, time sync
- Client → Client: Hello exchange on control stream
- Connection manager: track peer set, handle join/leave
- NTP-style time synchronization (rolling average, outlier rejection)

### Key crates
`quinn`, `rustls`, `rcgen`, `tokio`

### Testing
- SimulatedNetwork unit tests
- Time sync accuracy tests (simulated latency)
- Integration: two clients connect via localhost rendezvous

### Milestone
Two clients connect through a rendezvous server, discover each other, and
establish direct peer connections with synchronized clocks.

---

## Phase 4: State Sync Engine

Status: Completed

**Goal**: CRDTs sync across peers. Op broadcast, version vectors, gap fill.

### What gets built
- `SyncEngine`: wraps CrdtState + Network
- Eager push via datagrams + reliable send on control stream
- Periodic state summary exchange (1s)
- Gap detection from version vector comparison
- Gap fill over on-demand streams
- Op deduplication
- SQLite persistence integration

### Testing
- SimulatedNetwork: N peers with packet loss → verify convergence
- Partition/heal: ops on both sides of partition → heal → converge
- Reconnection: peer misses ops → reconnects → full state recovery
- Fuzz test: Expanded network sim, with events for new user actions

### Milestone
Multiple clients modify CRDTs and converge to identical state, even with
simulated packet loss.

---

## Phase 5: Application Core & Rendezvous Server

**Goal**: AppState event loop, server compaction, headless client.

### What gets built
- `AppState`: the central state machine (plain struct, no I/O)
- Event enum: network, player, user input events
- Effect enum: commands to network, player, TUI
- Derived playback state (play iff all users Ready/NotWatching, file states permit)
- User state transitions (Ready / Paused / Not Watching)
- File state transitions (Ready / Missing / Downloading)
- Main event loop (tokio select)
- Server compaction (5min after last client disconnects → epoch increment)
- Epoch handling on client reconnection

### Testing
- AppState unit tests: inject events → verify state + effects
- Compaction round-trip: generate ops → compact → reconnect with stale epoch
- Ready state derivation: all user/file state combinations

### Milestone
Headless client connects and syncs. Server compacts correctly.

---

## Phase 6: TUI

**Goal**: Full terminal interface.

### What gets built
- Main layout: chat (left 50%), right column (recent series / users / playlist),
  player status bar, keybinding bar
- Chat pane with input line (cursor movement, word-jump, home/end)
- Users pane with colored ready states (green/red/gray/blue)
- Playlist pane (current highlighted, missing in red, played in muted)
- Recent Series pane (unwatched first → recency → alphabetical)
- Player status bar (progress, now playing)
- Context-sensitive keybinding bar
- Tab cycling: Chat → Recent Series → Playlist
- File browser (add to playlist, manual file mapping)
- Settings screen (first-run + later access)

### Key crates
`ratatui`, `crossterm`

### Testing
- insta snapshot tests: render AppState → buffer → assert snapshot
- Edge cases: empty playlist, no users, long filenames

### Milestone
Interactive TUI client: connect, see peers, chat, manage shared playlist.

---

## Phase 7: Player Integration & Playback Sync

**Goal**: mpv integration, echo suppression, synchronized playback.

### What gets built
- `Player` trait + `MockPlayer` (for tests)
- `MpvPlayer`: JSON IPC over Unix socket
- Echo suppression: tag our commands, filter echoed events
- Play/pause sync (derived from user states)
- Seek broadcast (1500ms debounce)
- Position broadcast (100ms playing, 1s paused)
- Seek-on-receipt when drift > 3s
- Content hash verification (ed2k) before unpause
- OSD messages (chat on video)
- Crash handling (relaunch + seek; second crash within 30s → global pause)

### Key crates
`serde_json` (mpv JSON IPC)

### Testing
- MockPlayer unit tests: correct commands for state transitions
- Echo suppression tests (gated behind `mpv-tests` feature)
- Debounce tests with paused tokio time

### Milestone
**Full working watch party.** Multiple users, shared playlist, synced
video playback in mpv, chat on OSD.

---

## Phase 8: File Management

**Goal**: Media scanning, file matching, watch tracking, manual mapping.

### What gets built
- Recursive media root scanning
- Automatic file matching (by filename) when playlist items added
- Known series vs unknown series detection
- Manual file mapping (file browser, sorted by edit distance)
- Watch tracking (85% duration = watched)
- Recent Series sorting
- Placeholder PNG for "not watching" state
- File mtime tracking for re-hashing

### Key crates
`image` (PNG generation), `strsim` (edit distance)

### Testing
- File matching logic, series detection, sorting
- Integration with test media directory

### Milestone
Missing files detected, shown in red. Manual mapping works. Watch history
drives Recent Series.

---

## Phase 9: Hole Punching, Relay & File Transfer

**Goal**: NAT traversal, relay fallback, peer-to-peer file distribution.

### What gets built
- Hole punching: simultaneous QUIC opens, exponential backoff (100ms–1600ms),
  5s fallback to relay
- TURN relay: application-layer proxy on server (decrypt/re-encrypt)
- Relay envelope wrapping
- Background direct-connection retry (30s) while relayed
- File transfer: 256KiB chunks, availability bitfields
- Rarest-first chunk selection (sequential near playback position)
- Max 4 concurrent streams, 16 chunks pipeline depth
- Temp storage → download root after 50% watched
- Download progress in File State
- Playback gating: speed > bitrate AND ≥20% downloaded

### Testing
- SimulatedNetwork: hole punch, relay fallback
- File transfer integrity, rarest-first distribution
- Bandwidth throttling

### Milestone
Works across firewalls. Missing files downloaded from peers automatically.

---

## Phase 10: AniDB Integration

**Goal**: Server-side metadata lookups.

### What gets built
- AniDB UDP API client (client id: "dessplay")
- Login session management
- Rate limiter (1/2s sustained, 1/4s burst of 60, 5s retry on throttle)
- SQLite-backed validation queue
- ed2k hash → file lookup → series/season/episode
- Results written as server-authoritative LWW Register ops
- Re-validation schedule (30min <1d, 2h <1w, ...; ≥3mo skip; known ≤1/week)

### Testing
- Rate limiter unit tests
- Queue scheduling tests
- Mock AniDB server

### Milestone
Playlist files enriched with series/season/episode from AniDB. Recent
Series shows proper names.

---

## Phase 11: Hardening & Polish

**Goal**: Production readiness.

### What gets built
- Reconnection handling (all scenarios from network-design.md)
- Graceful shutdown, crash recovery
- Error handling (no panics — enforced by clippy lints)
- Fuzz targets: postcard deserialization for all message types
- System tests: tmux end-to-end
- Logging/tracing throughout
- `/exit`, `/quit`, `/q` commands
- VLC support (v2 scope decision)

### Testing
- Fuzz: ≥10min per target
- System tests: full workflow (connect, chat, add, play, seek, disconnect, reconnect)
- Chaos testing: SimulatedNetwork with high loss, partitions, reordering

### Milestone
Stable, production-ready. All documented failure modes handled.

# Testing Strategy

Last updated: 2026-07-12

## Table of Contents

1. [Principles](#principles)
2. [Code Quality Enforcement](#code-quality-enforcement)
3. [Architecture for Testability](#architecture-for-testability)
4. [Test Tiers](#test-tiers)
5. [SimulatedNetwork](#simulatednetwork)
6. [Multi-Client Simulation Harness](#multi-client-simulation-harness)
7. [Player Integration Tests](#player-integration-tests)
8. [CRDT Property Tests](#crdt-property-tests)
9. [TUI Testing](#tui-testing)
10. [Actor Tests](#actor-tests)
11. [Fuzz Testing](#fuzz-testing)
12. [System Tests (tmux)](#system-tests-tmux)
13. [Key Crates](#key-crates)

---

## Principles

- **Deterministic and reproducible**: Seeded RNG, paused tokio time, no flaky
  sleeps. Every test failure should be reproducible from the seed alone.
- **Spec-driven**: Write tests from the specification, not the implementation.
  If the spec is unclear, clarify it before writing the test.
- **Regression tests first**: When fixing a bug, write a test that reproduces
  it *before* writing the fix.
- **High-risk areas get extra coverage**: Echo suppression, CRDT convergence,
  Lww tiebreaking, playlist Identifier ordering, reconnection/epoch handling,
  seek authority transitions, presence transitions (lost/departed pause
  behavior), cache eviction (must never delete the wrong file).

---

## Code Quality Enforcement

### Clippy Lints

The following lints are enforced project-wide via `#![deny(...)]` in `lib.rs`:

```rust
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
```

These are allowed in test code:

```rust
#[cfg(test)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
```

### Other Enforced Lints

```rust
#![deny(clippy::todo)]           // No TODOs in committed code
#![deny(clippy::dbg_macro)]      // No debug prints in committed code
```

---

## Architecture for Testability

The actor model provides natural seams for testing.

### Actor Isolation

Each actor can be tested by:
1. Creating the actor with a test inbox
2. Sending messages to it
3. Asserting on the messages it produces

No network, no terminal, no player process needed for most tests.

### Network Trait

Network I/O goes through a trait for the NetworkActor's internals:

```rust
trait Transport {
    async fn send_reliable(&self, msg: &[u8]) -> Result<()>;
    async fn send_datagram(&self, data: &[u8]) -> Result<()>;
    async fn open_stream(&self) -> Result<BiStream>;
    async fn recv(&self) -> Result<TransportEvent>;
}
```

Production code uses QUIC via quinn. Tests use `SimulatedTransport`.

### Player Trait

Player interaction goes through a `Player` trait:

```rust
trait Player {
    async fn load_file(&self, path: &Path) -> Result<()>;
    async fn pause(&self) -> Result<()>;
    async fn unpause(&self) -> Result<()>;
    async fn seek(&self, position_secs: f64) -> Result<()>;
    async fn get_position(&self) -> Result<f64>;
    async fn show_osd(&self, text: &str) -> Result<()>;
    async fn recv_event(&self) -> Result<PlayerEvent>;
}
```

Production: `MpvPlayer`. Tests: `MockPlayer`.

### Torrent Traits

The torrent-first download path (design.md, BitTorrent Downloads) sits
behind two traits so the whole flow is testable without a network:

- `NyaaSource` (blocking `search(filename) -> RSS`): `HttpNyaaSource` in
  production; tests hand back fixture/canned RSS. The parse/match logic
  (`parse_rss`, `pick_match`) is pure and unit-tested against a
  committed real-shape fixture (entity-escaped titles, `nyaa:` fields).
  The same seam provides category search and `.torrent` bytes for the manual
  browser; pure tests pin the 20-entry cap, feed ordering, single-file
  metainfo filter, and unsafe-name rejection.
- `TorrentEngine` (`add`/`remove`/sync `status` poll): `RqbitEngine`
  (librqbit) in production; `FakeTorrentEngine` lets actor tests script
  progress, completion, and failure. The fetch *policy* (`TorrentFetches`
  — watchdogs, cooldowns, fallback) is a synchronous clock-driven core
  unit-tested like `Downloads`.

Actor-level tests cover the ladder end-to-end with the fakes: no-match →
peer fallback, stall → fallback, completion → ed2k verify → Ready,
mismatch → ban + fallback, local-copy adoption cancelling the torrent,
eviction removing it, and startup reconciliation. One `#[ignore]`d smoke
test starts a real librqbit session.

Manual-import actor tests additionally cover selection -> progress -> ed2k
hash -> cache/promotion -> playlist completion and cancellation cleanup.
Whole-app UI tests cover the disabled-setting notice, `n` routing, the
progress overlay, reopening active imports, and `d` cancellation.

---

## Test Tiers

### Unit Tests (`cargo test`)

Fast, in-process, no external dependencies. Cover:

- CRDT operations via `crdts` crate: apply, merge, snapshot generation
- `Lww<V>` conflict resolution (timestamp + value-based tiebreaking)
- Playlist `Identifier`-based ordering, rebalancing
- CvRDT merge correctness (merge is idempotent, commutative, associative)
- Time sync offset calculation
- File hash computation (ed2k root + per-block hashes)
- Actor message handling (inject message, check outputs)
- Chat GList ordering and deduplication
- Seek authority transitions (user seek, file change, authority departure)
- User state derivation (series preference + manual override -> derived
  state, including Away set/cleared by activity)
- Presence-aware playback gating (every presence x user-state x file-state
  combination; seeders never gate)
- Drift band selection (ignore / slew / hard seek, boundary values)
- Cache eviction policy (retention 0 / finite / infinite; never evicts
  now-playing or queued unwatched entries)
- EOF transition idempotency (duplicate `EofReached` reports are no-ops)
- List importer: CSV section parsing, status heuristics, watcher-initial
  mapping (tested against the real exported sheets as fixtures)

### Integration Tests (`cargo test` with real binaries)

Slower, may spawn external processes. Cover:

- Player bridge with real mpv (`-vo null -ao null`)
- Echo suppression (send command to mpv, verify not re-broadcast)
- Subtitle text observation (`sub-text` property events)
- State sync convergence across multiple SyncActors connected via
  SimulatedTransport
- Multi-client simulation harness scenarios (N full clients + server,
  in-process; see [Multi-Client Simulation Harness](#multi-client-simulation-harness))
- Reconnection and epoch handling (including the daily compaction broadcast)
- Relayed file transfer: chunking, reassembly, block-hash verification,
  corrupted-block re-fetch, resume-after-restart from on-disk chunks

### System Tests (manual tmux smoke)

Full end-to-end in tmux. There is currently no `system-test` Cargo feature or
automated tmux runner; this tier is run manually. Cover:
- The complete user workflow (connect, add file, play, chat, disconnect)
- See [System Tests (tmux)](#system-tests-tmux)

---

## SimulatedNetwork

An in-process transport that simulates network conditions between actors.

### Capabilities

| Feature | Description |
|---------|-------------|
| **Packet loss** | Drop datagrams with configurable probability per link |
| **Latency** | Delay message delivery by a configurable duration per link |
| **Partition** | Completely block traffic between specific endpoints |
| **Reordering** | Shuffle datagram delivery order (configurable window) |
| **Bandwidth limit** | Throttle throughput on specific links |

### Design

```rust
struct SimNetwork { /* shared state + seeded StdRng */ }

struct LinkConfig {
    latency: Duration,        // one-way propagation delay
    datagram_loss: f64,       // drop probability, datagrams only
    datagram_jitter: Duration,// random extra delay -> reordering
    bandwidth: Option<u64>,   // bytes/sec serialization delay
    partitioned: bool,        // holds control frames, drops datagrams
}
```

Implemented in `dessplay_core::net::sim` behind the `test-support`
feature, as `Connector`/`Listener`/`Transport` impls mirroring the QUIC
shapes. Semantics worth knowing:

- Control frames are reliable and ordered: loss never touches them, and
  a partition *delays* them (they flush on heal), like a QUIC stream.
- Reordering is modeled as per-datagram jitter rather than a shuffle
  window — natural in a latency-scheduled system.
- `close()` notifies the peer; *dropping* a transport is silent death,
  which is exactly what presence timeouts must catch.
- Limitation: bytes inside an open `BiStream` bypass the link simulation
  (establishment respects it). Revisit for Phase 9 transfer tests.

### Time Control

Uses `tokio::time::pause()` so that time only advances when explicitly
advanced or when all tasks are idle. Eliminates flaky timing dependencies.

---

## Multi-Client Simulation Harness

The tier between actor tests and tmux: N **complete** clients (every
actor, UI rendered to ratatui's `TestBackend`, `MockPlayer`) plus a real
server actor, in one `current_thread` tokio runtime with paused time,
wired over `SimulatedTransport`. This is where cross-client product
behavior is tested — pause propagation, presence transitions reaching
other users' screens, EOF advancing everyone's playlist — headless and
fast. It exists because of the [composition
root](architecture.md#composition-root): the harness calls the same
wiring function `main()` does.

The model is **Playwright, ported to a TUI**. The borrowed ideas:

- **Client handles as contexts.** A test owns handles to N clients and
  the server; each handle can inject input events, read its rendered
  buffer, and manipulate its network link (partition, loss, latency).
- **Auto-waiting assertions, never sleeps.** Assertions poll a predicate
  while pumping the event loop and advancing *simulated* time, failing
  only when a simulated-time budget expires:
  `eventually(&client_b, 5.secs(), |ui| ui.pane("users").contains("Baughn ▌away"))`.
  Strictly better than Playwright's version — our clock is virtual, so
  "wait up to 5 seconds" costs microseconds and is deterministic.
- **Locators over coordinates.** Behavior tests query semantically —
  "the users pane has a row matching X" — via pane-region helpers over
  the buffer, not cell coordinates and not full-buffer snapshots.
  Full-buffer insta snapshots are reserved for *layout* tests, so layout
  tweaks don't break fifty scenario tests.
- **Failure artifacts.** On failure the harness dumps every client's
  final rendered buffer, the server's op log, and the RNG seed — the
  trace-viewer idea, minus the viewer.

Scenario tests read like screenplays:

```text
spawn server; spawn clients A, B, C
A: add file to playlist
eventually: B and C playlists show the file
B: press pause in player
eventually: A's and C's MockPlayers received Pause
partition C; advance 35s
eventually: A's chat shows "C lost connection"; everyone paused
heal C
eventually: all views converge
```

The harness is built incrementally: sync-only multi-actor form in Phase
4, headless full clients in Phase 5, UI handles in Phase 6, player
handles in Phase 7. Once it exists, the Phase 1 fuzz pattern scales up:
random input events plus network chaos into full clients, asserting no
panics and post-quiesce convergence.

### Determinism Stance

One runtime, paused time, and seeded RNG make scenarios *almost*
deterministic: no wall-clock races are possible. The residual
nondeterminism is tokio's task-polling order, which is stable on
`current_thread` in practice but not contractually guaranteed. Policy:
no test may depend on intra-tick ordering, and an ordering-flake is
treated as a bug in the code under test (it would be a race in
production too). If that policy ever proves insufficient, the
escalation path is a deterministic-simulation runtime (`madsim` /
`turmoil`) — not adopted now because they fight with quinn, and the
`Transport` seam already keeps real QUIC out of this tier. Real-QUIC
coverage comes from a small set of localhost integration tests plus the
tmux smoke layer.

---

## Player Integration Tests

### Layering (as built in Phase 7)

Echo suppression, drift bands, debouncing, position cadence, and crash
supervision are all **PlayerActor logic**, tested deterministically with
`MockPlayer` and paused tokio time (`dessplay/src/actors/player.rs`
tests) — mpv does *not* distinguish user from programmatic events, so
the distinction is our expected-state tracking and is testable without
a player process. Cross-client behavior (pause on A pauses B's player,
seek authority, EOF advance, missing-file gating) lives in the
multi-client harness's player clients (`dessplay-rendezvous/tests/
player.rs`), which run the real session shell around mocks. Those
scenarios touch the real filesystem (tempdir media roots, blocking-pool
matcher), so they are eventually-style rather than perfectly
deterministic.

### Real-mpv smoke test

One end-to-end journey (`dessplay/tests/mpv_real.rs`) proves the JSON
IPC layer speaks actual mpv: load → duration → unpause echo → position
flow → speed slew → exact seek → EOF (asserting keep-open's mechanical
pause does **not** leak as a user pause) → clean shutdown.

- Requires the `mpv` binary; gated behind `--features mpv-tests`.
- The test video is encoded on the fly *by mpv itself* from a lavfi
  source — no committed media, no ffmpeg dependency.
- mpv runs with `--vo=null --ao=null --force-window=no`.
- Processes are spawned with tokio's `kill_on_drop`, so failures don't
  leak mpv instances.

This test earns its keep: it caught mpv emitting the keep-open pause
*before* `eof-reached`, which no mock would have predicted.

---

## AniDB Tests (Phase 8)

**No automated test may ever touch the real AniDB API.** AniDB bans
aggressively and bans stick; the accepted trade is that a parser bug
the spec didn't make obvious surfaces only on a manual probe run.

Layering mirrors the player tests:

- **Codec** (`anidb/protocol.rs`): pure encode/parse tests, including
  the mask constants rebuilt bit-by-bit from named positions, escaping
  (backtick apostrophes, `<br />`, the deliberately-unreversed `/`),
  and special episode numbers.
- **Rate limiter & sessions** (`anidb/client.rs`): the real client over
  a scripted `Wire` mock under paused time — exact gap assertions for
  the 2s floor and the sustained 1-per-4s tail, the 5s timeout penalty,
  re-auth on expired sessions, busy/ban backoff, stale-reply skipping.
- **Scheduling** (`anidb/schedule.rs`): pure-function ladder tests.
- **Worker** (`anidb/worker.rs`): the real worker against an in-memory
  `AniDbHost` (real `CrdtState`, in-memory SQLite) and a canned
  `AniDbApi` — metadata writes, fallback-never-clobbers, relations
  walks, titles refresh cadence, fatal-stops, backoff recovery.
- **Integration** (`dessplay-rendezvous/tests/anidb.rs`): the whole
  flow over the sim transport — client lookup requests through the
  real server and worker into replicated metadata on every client,
  name search over the wire, and the EOF List advance.
- **Replay** (`tests/anidb_replay.rs`): real exchanges recorded by
  `anidb-probe scan <dir>` (manual, rate-limited, single-threaded) live
  in `dessplay-rendezvous/testdata/anidb/` and are re-parsed by the
  real codec on every test run — the parser is pinned to actual server
  output without touching the API. The recorder redacts credentials
  and session keys at write time. The replay test also asserts the
  recorded fmask/amask match the constants we send, so changing the
  masks forces a re-record.
- **Manual**: `anidb-probe` (ping / file / anime / scan) is the only
  real-API contact, run by a human.

---

## CRDT Property Tests

Using proptest to verify convergence properties.

### Core Property: Convergence

For every CRDT type, the fundamental property:

> After all operations are delivered, every replica resolves to the same
> **view** (LWW winners, sorted playlist, chat order).

Two hard-won qualifications, found by Phase 1 property testing:

1. **A plain shuffle is not a valid replay order.** `Map` ops carry
   per-actor sequence numbers; arbitrary permutation silently drops ops,
   and even per-actor-order-preserving permutations can violate causal
   delivery, which `crdts` does not survive (see sync-state.md, crdts API
   notes). The delivery orders the real system produces are: the server's
   total order, with each client's own ops applied early. Convergence is
   therefore tested through a **cluster model**
   (`dessplay_core::test_support::Cluster`): per-client states with local
   echo, a server hub consuming client queues in fuzz-chosen order,
   in-order log delivery (including duplicate delivery of own ops), and
   CvRDT-merge reconnects.
2. **Convergence is view-level.** Replicas that received the same ops in
   different valid orders can hold different internal causal metadata
   while agreeing on the resolved view. Tests compare `CrdtState::view()`;
   raw state equality is only asserted for byte-identical histories.

```rust
proptest! {
    #[test]
    fn cluster_converges(events in vec(arb_cluster_event(), 1..80)) {
        let cluster = run_cluster(&events);   // generate, schedule, flush
        let server_view = cluster.server.view();
        for client in &cluster.clients {
            prop_assert_eq!(client.view(), server_view.clone());
        }
    }
}
```

### Test Scenarios

| CRDT | Property | Notes |
|------|----------|-------|
| `LwwCell<V>` | LWW resolution: highest timestamp wins regardless of apply order | Value-based tiebreaking on equal timestamps |
| Playlist Map | Cluster-delivered ops -> same entries and positions everywhere | Put/tombstone interactions (no `Map::rm`; see sync-state.md) |
| Chat GList | Same inserts, any order -> same message sequence | GList guarantees this |
| Seek Authority | Last timestamp wins | Same as LwwCell |
| CvRDT merge | merge(a, b) == merge(b, a); merge(a, merge(b, c)) == merge(merge(a, b), c) | Commutativity and associativity |

### Identifier Ordering Properties

Specific to the playlist:

- After any sequence of adds/moves, the playlist is always sortable
- `Identifier::between()` always produces a value strictly between its bounds
- Rebalancing preserves ordering

### Multi-Actor Convergence

Test that N SyncActors receiving ops via SimulatedTransport (with loss,
reordering, partitions) eventually converge after the network stabilizes.

---

## TUI Testing

### Snapshot Tests (insta)

tui-realm components are rendered to a buffer and snapshot-tested:

```rust
#[test]
fn test_playlist_rendering() {
    let component = PlaylistPane::new(test_props());
    let buffer = render_component(&component, Rect::new(0, 0, 80, 20));
    insta::assert_snapshot!(buffer_to_string(&buffer));
}
```

### What Snapshot Tests Cover

- Layout proportions (chat, users, playlist, player status)
- Color and style of ready states (green/red/gray/blue)
- Keybinding bar content changes with focused component
- Playlist rendering (current item highlighted, missing items red)
- Chat message display and wrapping
- Edge cases: empty playlist, no connected users, long filenames

### What Snapshot Tests Do NOT Cover

Application logic. Snapshot tests verify rendering only; input routing
is covered by message/update tests; everything above that belongs to
whole-app tests.

### Whole-App TUI Tests

Per-component tests can all pass while the *assembly* misbehaves: focus
cycling, modal open/close stacking, the keybinding bar following focus,
state→props plumbing across event sequences. Whole-app tests close that
gap: instantiate the real synchronous `ui::app::Ui` dispatcher with all
components mounted, feed it synthetic events (the same path production's UI
shell drives), render to `TestBackend`, and assert with the same locator-style queries
the multi-client harness uses.

> Press Tab twice; press `a`; the file browser modal is visible and the
> keybinding bar shows browser bindings.

What stays untestable headless — and is deliberately left to the tmux
tier and manual use: real-terminal resize quirks, whether a particular
emulator advertises its color depth correctly, and mouse protocol variations.
The resulting color-depth behavior itself is headlessly testable because the
capability is injected into `Ui`: limited mode must leave the completed buffer
untouched, while true-color mode must apply the explicit dark background to
every cell, including panes, modals, and passive overlays.

The shared Form additionally tests semantic row identity: selection follows a
row through insertion/reorder, typed controls route only their matching edit
kind, invalid text keeps the masked/plain editor open with an error, and the
fixed Save footer retains all three save paths. Settings has one 100x30 layout
snapshot per category plus model tests for missing-category markers, dormant
control styling, upload-rate parsing, media-root selection, and the persisted
speaker-color overflow default/cycle/round trip.

The subtitle speaker-name preference is tested at the same layers: missing
storage defaults to hidden names, the Playback form toggles and saves the
value, and one shared formatter produces `Name: dialogue` in both Intermixed
and Separate modes without changing unnamed cues. Rendering tests also pin
that the preference applies immediately to buffered cues, Intermixed remains
uniformly dim, and Separate retains its existing speaker-color policy.

Subtitle speaker colors have deterministic policy tests below the snapshot
layer:

- speaker slot assignments stay stable and unique throughout the inclusive
  five-minute activity window, expire immediately after its boundary, and
  recycle holes; repeated cues refresh the lease, backward clock corrections
  cannot resurrect an expired speaker, and passive clock advancement restores
  limited-terminal colors without requiring another cue;
- limited-color rendering preserves deterministic name hashing into the
  ten-color application palette, then exercises both configured overflow
  paths: continued hashing under **Reuse colors** and uniformly dim text under
  **Disable colors**, with colors returning when the active count drops within
  capacity;
- true-color rendering remains distinct past the limited palette, has no
  application-level prefix cap, and uses progressive HSLuv maximin selection.
  Minimum CIEDE2000 distance is pinned at 10.5 through 32 colors, 5.5 through
  128, and 4.25 through 256; the first 256 colors must also remain unique and
  meet at least 4.5:1 contrast against the explicit dark background, as must
  every mapped semantic foreground; and
- turning off the existing speaker-colors master toggle produces uniformly dim
  text regardless of terminal capability or overflow preference.

---

## Actor Tests

New in dessplay2: each actor can be tested in isolation.

### Pattern

```rust
#[tokio::test]
async fn sync_actor_applies_local_op() {
    tokio::time::pause();
    let (tx, rx) = mpsc::channel(16);
    let (out_tx, mut out_rx) = mpsc::channel(16);
    let actor = SyncActor::new(tx, out_tx);

    // Send a local op
    actor.send(SyncMsg::LocalOp(chat_op("hello"))).await;

    // Verify it produces an outbound op
    let msg = out_rx.recv().await.unwrap();
    assert!(matches!(msg, SyncOutput::OutboundOp(_)));
}
```

### What Actor Tests Cover

- SyncActor: op application, CvRDT merge, snapshot generation
- NetworkActor: connection state machine, message routing
- PlayerActor: echo suppression, position broadcasting, crash handling
- FileActor: hash caching, file matching, download state machine
- UI dispatcher: message -> action mapping, state -> props mapping

---

## Fuzz Testing

All fuzz targets use structured `Arbitrary`-based input generation.

Run with `cargo +nightly fuzz run <target>` from the relevant fuzz crate, or
use its convenience script from the repository root:

```bash
dessplay-core/fuzz/run.sh                     # core targets, 300s each
dessplay-core/fuzz/run.sh crdt_op             # one core target
dessplay/fuzz/run.sh --quick                  # client targets, 30s each
```

Fuzz for at least 10 minutes per target before release.

### Generic Targets

#### CRDT Op Replay (`crdt_op`)
Applies arbitrary scripted op sequences to CrdtState, including duplicate
delivery. Asserts no panics; state stays viewable and serializable.

#### CRDT Convergence (`crdt_convergence`)
Arbitrary cluster-event schedules (client ops, server ops, polls, partial
deliveries, reconnects) through the hub-and-spoke `Cluster` model. After
flush, every client's view equals the server's.

#### Snapshot Round-Trip (`snapshot_roundtrip`)
Build state -> postcard snapshot -> decode -> identical state and view.

#### CvRDT Merge Round-Trip (`merge_roundtrip`)
Three independently evolved replicas (one actor each). Merge is
commutative, associative, idempotent, and agrees with op replay.

### Targeted Targets

#### Playlist Identifier (`playlist_identifier`)
Constrained inputs (5 file IDs, 4 actors, small timestamps) force
add/move/remove collisions on the same files. The playlist is always
strictly sorted by `(position, hash)`; rebalancing preserves order and its
ops converge on replicas.

#### LWW Register Convergence (`lww_register_convergence`)
Pairwise-concurrent register writes. Every delivery rotation and merge
order resolves to `max((timestamp, value))`.

#### Chat GList (`chat_glist`)
Concurrent appends from three replicas, delivered in rotated orders with
duplicates. No losses, no duplicates, identical final order.

### Network Targets

#### Postcard Deserialize (`postcard_deserialize`)
Raw bytes -> `wire::decode` for `CrdtOp` and `StateSnapshot`. Must not
panic.

#### Framing Deserialize (`framing_deserialize`) — Phase 3
Raw bytes -> stream/datagram framing layer. Must not panic.

The actual SyncActor/network/server path is covered by the seeded chaos test
in `dessplay-rendezvous/tests/sync.rs`; it is not duplicated as a libFuzzer
target. The `crdt_convergence` target above owns arbitrary schedules at the
pure cluster-model layer.

### Client Targets (`dessplay/fuzz`)

A second, separate cargo-fuzz crate (own `Cargo.toml`/`run.sh`, same
conventions) for logic that lives in the `dessplay` client crate rather
than `dessplay-core` — the mpv player integration and the download
scheduler, both external-input boundaries the core CRDT fuzzing above
doesn't reach.

#### mpv IPC Translate (`mpv_ipc_translate`)
Arbitrary sequences of raw lines (plus a `loading`-flag toggle per step)
fed through `player::mpv::translate` against one running `Translate`
accumulator, mirroring `read_loop`'s per-line parse-or-skip. Targets the
cross-message state (pause/path dedup, the seek-reply request-id match,
EOF edge-triggering), not just single-message parsing. Must not panic.

#### mpv ASS Text (`mpv_ass_text`)
Arbitrary strings through `player::mpv::parse_ass_full` (the ASS
override-tag stripper, including the drawing-mode and unclosed-brace
"eat the rest" paths). Must not panic, and the stripped text must never
be longer than the input.

#### Download Scheduler (`download_scheduler`)
Arbitrary sequences of peer protocol events (honest and adversarial:
wrong-length bitfields, forged block hashes, out-of-range or corrupt
chunk data, churning source sets, clock jumps) against `download::Downloads`
with a small real backing file. Must not panic. Liveness ("does chaos
ever wedge it permanently") is intentionally *not* checked here — see the
companion proptest below — this target is purely a crash-safety net
against malformed peer input.

#### Download Chaos Recovery (`download_props`, proptest)
Not a fuzz target — an in-repo proptest (`dessplay/tests/download_props.rs`)
using the same event vocabulary as `download_scheduler`, plus a forced
honest epilogue: after an arbitrary chaos prefix, every original source
becomes present and truthful again, and the download must reach
completion with the exact original bytes. This generalizes the module's
own history of hand-found wedge regressions (a stalled block-hash source
never re-solicited, a departed driving source, two sources
double-assigned the same chunk, a lying source never dropped) into one
property instead of waiting for the next one to be found by hand. It
already caught two: a source given up on past `MAX_SOLICIT_ATTEMPTS` was
removed permanently, with nothing but an external `set_sources`/`start`
call (never guaranteed for a seeder-only download nobody is watching) able
to re-add it -- fixed by backing off for a long cooldown and retrying
in place instead of dropping the source. And a source that answered
`FileAvailability` without also answering `BlockHashes` (e.g. the two
replies split by a flaky connection) was mistaken for "already answered"
by the stall detector, since it keyed off the bitfield alone -- fixed by
tracking each source's `BlockHashes` reply independently of its
bitfield.

---

## System Tests (tmux)

Full end-to-end **smoke tests** in a tmux server: real binaries, real
QUIC on localhost, real terminals. This tier exists to catch what the
in-process harness *cannot* see — process spawning, real sockets,
terminal reality — and nothing else. Keep it small: product logic is
never tested here (the multi-client harness owns that), and assertions
are poll-for-string with a timeout (`tmux capture-pane` in a retry
loop), never sleep-then-grep.

### Setup

```bash
tmux -L dessplay new-session -d -s test

# Start rendezvous server
tmux -L dessplay send-keys \
  'dessplay-rendezvous --password-file /tmp/test-password' Enter

# Start clients
tmux -L dessplay split-window -h
tmux -L dessplay send-keys \
  'dessplay --server localhost --password-file /tmp/test-password' Enter
```

### What System Tests Verify

- Binaries start, connect, and authenticate over real QUIC on localhost
- One happy-path flow as smoke (a chat message crosses clients)
- Real mpv spawns and is driven (`-vo null -ao null`)
- Kill -9 and restart a client process; it reconnects

Anything subtler than this belongs in the multi-client harness.

### When to Run

- Manually during development
- CI on a dedicated stage (not every commit)
- Required before release

---

## Key Crates

| Crate | Purpose |
|-------|---------|
| `proptest` | Property-based testing for CRDT convergence |
| `insta` | Snapshot testing for TUI rendering |
| `cargo-fuzz` / `libfuzzer-sys` | Fuzz testing |
| `tokio::time::pause()` | Deterministic time control in async tests |
| `tracing-test` | Capture and assert on log output in tests |

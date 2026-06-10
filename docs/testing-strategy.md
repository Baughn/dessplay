# Testing Strategy

Last updated: 2026-03-04

## Table of Contents

1. [Principles](#principles)
2. [Code Quality Enforcement](#code-quality-enforcement)
3. [Architecture for Testability](#architecture-for-testability)
4. [Test Tiers](#test-tiers)
5. [SimulatedNetwork](#simulatednetwork)
6. [Player Integration Tests](#player-integration-tests)
7. [CRDT Property Tests](#crdt-property-tests)
8. [TUI Testing](#tui-testing)
9. [Actor Tests](#actor-tests)
10. [Fuzz Testing](#fuzz-testing)
11. [System Tests (tmux)](#system-tests-tmux)
12. [Key Crates](#key-crates)

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
  seek authority transitions.

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

---

## Test Tiers

### Unit Tests (`cargo test`)

Fast, in-process, no external dependencies. Cover:

- CRDT operations via `crdts` crate: apply, merge, snapshot generation
- `Lww<V>` conflict resolution (timestamp + value-based tiebreaking)
- Playlist `Identifier`-based ordering, rebalancing
- CvRDT merge correctness (merge is idempotent, commutative, associative)
- Time sync offset calculation
- File hash computation
- Actor message handling (inject message, check outputs)
- Chat GList ordering and deduplication
- Seek authority transitions
- User state derivation (series preference + manual override -> derived state)

### Integration Tests (`cargo test` with real binaries)

Slower, may spawn external processes. Cover:

- Player bridge with real mpv (`-vo null -ao null`)
- Echo suppression (send command to mpv, verify not re-broadcast)
- State sync convergence across multiple SyncActors connected via
  SimulatedTransport
- Reconnection and epoch handling
- File transfer chunking and reassembly

### System Tests (`cargo test --features system-test` or manual)

Full end-to-end in tmux. Cover:
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
struct SimulatedTransport {
    endpoints: HashMap<EndpointId, Mailbox>,
    config: SimConfig,
    rng: StdRng,  // seeded for reproducibility
}

struct SimConfig {
    links: HashMap<(EndpointId, EndpointId), LinkConfig>,
    defaults: LinkConfig,
}

struct LinkConfig {
    latency: Duration,
    packet_loss: f64,
    reorder_window: usize,
    bandwidth: Option<u64>,
    partitioned: bool,
}
```

### Time Control

Uses `tokio::time::pause()` so that time only advances when explicitly
advanced or when all tasks are idle. Eliminates flaky timing dependencies.

---

## Player Integration Tests

### Setup

Integration tests that use mpv require the `mpv` binary in `$PATH`. Tests
are gated behind `#[cfg(feature = "mpv-tests")]`.

mpv is launched with `-vo null -ao null` to suppress video/audio output.

### Cleanup

A `Drop` handler on the test fixture ensures all spawned mpv processes are
killed, even on test failure.

### Echo Suppression Tests

Critical integration tests. The pattern:

1. Connect to mpv via IPC
2. Send a seek command
3. Receive the resulting position-change event from mpv
4. Verify the event is tagged as "echo" and not forwarded

Test cases:
- Seek echo (send seek, receive position update)
- Pause echo (send pause, receive pause event)
- Rapid seeks (debouncing interacts with echo detection)
- External pause (user pauses in mpv -- this is *not* an echo)
- User-initiated seek vs programmatic seek (mpv distinguishes these)

---

## CRDT Property Tests

Using proptest to verify convergence properties.

### Core Property: Convergence

For every CRDT type, the fundamental property:

> Given the same set of operations, any application order produces the same
> snapshot.

```rust
proptest! {
    #[test]
    fn playlist_map_converges(
        ops in vec(arb_playlist_op(), 1..50),
        permutation_seed in any::<u64>(),
    ) {
        let snapshot_a = apply_ops(&ops);

        let mut shuffled = ops.clone();
        shuffled.shuffle(&mut StdRng::seed_from_u64(permutation_seed));
        let snapshot_b = apply_ops(&shuffled);

        assert_eq!(snapshot_a, snapshot_b);
    }
}
```

### Test Scenarios

| CRDT | Property | Notes |
|------|----------|-------|
| `MVReg<Lww<V>>` | LWW resolution: highest timestamp wins regardless of apply order | Value-based tiebreaking on equal timestamps |
| Playlist Map | Same ops, any order -> same entries and positions | Test Put/Remove interactions |
| Chat GList | Same inserts, any order -> same message sequence | GList guarantees this |
| Seek Authority | Last timestamp wins | Same as MVReg+Lww |
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

Application logic. UI tests verify rendering and input routing only.

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
- UiActor: message -> action mapping, state -> props mapping

---

## Fuzz Testing

All fuzz targets use structured `Arbitrary`-based input generation.

Run with `cargo +nightly fuzz run <target>`, or the convenience script:

```bash
./fuzz/run.sh                     # all targets, 300s each, parallel
./fuzz/run.sh crdt_op             # one target, 300s
./fuzz/run.sh --quick             # all targets, 30s each
```

Fuzz for at least 10 minutes per target before release.

### Generic Targets

#### CRDT Op Replay (`crdt_op`)
Applies arbitrary sequences of operations to CrdtState. Asserts no panics.

#### CRDT Convergence (`crdt_convergence`)
Same ops in two orders -> identical snapshots.

#### Snapshot Round-Trip (`snapshot_roundtrip`)
Build state -> snapshot -> load into fresh state -> identical snapshots.

#### CvRDT Merge Round-Trip (`merge_roundtrip`)
Two actors with overlapping ops -> CvRDT merge -> convergence.

### Targeted Targets

#### Playlist Identifier (`playlist_identifier`)
Constrained inputs: 5 file IDs, 4 actor IDs, 16 timestamps. Forces meaningful
add/move/remove interactions on the same files. Verifies `Identifier` ordering
consistency.

#### MVReg+Lww Convergence (`mvreg_lww_convergence`)
4 keys, 4 timestamps, 3 actors. Forces concurrent writes and same-timestamp
tiebreaks. Verifies `Lww` resolution produces identical results regardless of
operation order.

#### Chat GList (`chat_glist`)
Constrained inserts. Verifies ordering consistency.

#### CvRDT Merge (`cvmerge`)
Build state on 3 actors via random ops, then merge in random order. Asserts
all converge to identical state. Tests idempotency (merge same state twice).

### Network Targets

#### Postcard Deserialize (`postcard_deserialize`)
Raw bytes -> `postcard::from_bytes` for all wire types. Must not panic.

#### Framing Deserialize (`framing_deserialize`)
Raw bytes -> stream/datagram framing layer. Must not panic.

#### Sync Engine (`sync_engine`)
2-4 SyncActors through random event sequences with partitions and loss.
Mid-run convergence checks.

---

## System Tests (tmux)

Full end-to-end test harness in a tmux server.

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

- End-to-end connectivity via server
- Chat messages appear on all clients
- Playlist changes propagate
- Player sync with real mpv (`-vo null -ao null`)
- Reconnection: kill and restart a client

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

# Testing Strategy

Last updated: 2026-02-20

## Table of Contents

1. [Principles](#principles)
2. [Code Quality Enforcement](#code-quality-enforcement)
3. [Architecture for Testability](#architecture-for-testability)
4. [Test Tiers](#test-tiers)
5. [SimulatedNetwork](#simulatednetwork)
6. [Player Integration Tests](#player-integration-tests)
7. [CRDT Property Tests](#crdt-property-tests)
8. [TUI Testing](#tui-testing)
9. [Fuzz Testing](#fuzz-testing)
10. [System Tests (tmux)](#system-tests-tmux)
11. [Key Crates](#key-crates)

---

## Principles

- **Deterministic and reproducible**: Seeded RNG, paused tokio time, no flaky
  sleeps. Every test failure should be reproducible from the seed alone.
- **Spec-driven**: Write tests from the specification, not the implementation.
  If the spec is unclear, clarify it before writing the test.
- **Regression tests first**: When fixing a bug, write a test that reproduces
  it *before* writing the fix.
- **High-risk areas get extra coverage**: Echo suppression, CRDT convergence,
  playlist conflict resolution, reconnection/epoch handling.

---

## Code Quality Enforcement

### Clippy Lints

The following lints are enforced project-wide via `clippy.toml` /
`#![deny(...)]` in `lib.rs`:

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

Rationale: Rust's type system lets us write code that cannot crash by
construction. `unwrap`/`expect`/`panic` in production code is almost always
a design problem, not a convenience shortcut. This lint is enforced as a
Claude Code stop hook — any code change that introduces a forbidden lint is
caught before commit.

### Other Enforced Lints

```rust
#![deny(clippy::todo)]           // No TODOs in committed code
#![deny(clippy::dbg_macro)]      // No debug prints in committed code
```

---

## Architecture for Testability

The codebase has clear seams so that each layer can be tested in isolation.

### Network Trait

All network I/O goes through a `Network` trait:

```rust
trait Network {
    async fn send_reliable(&self, peer: PeerId, msg: ControlMessage) -> Result<()>;
    async fn send_datagram(&self, peer: PeerId, data: &[u8]) -> Result<()>;
    async fn open_stream(&self, peer: PeerId) -> Result<BiStream>;
    async fn recv(&self) -> Result<NetworkEvent>;
    // ...
}
```

Production code uses `QuicNetwork`. Tests use `SimulatedNetwork` (see below).

### App State / Event Bus

The application is structured as a state machine driven by events:

```
Events (network, player, user input) → AppState → Effects (network, player, TUI)
```

`AppState` is a plain struct with no I/O dependencies. It processes events
and returns effects. This means:

- **Integration tests** drive `AppState` directly by injecting events and
  inspecting the resulting state + effects. No TUI, no terminal, no raw text
  parsing.
- **TUI tests** can render `AppState` to a buffer and snapshot-test the output
  separately.
- **System tests** exercise the full stack end-to-end.

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
    // ...
}
```

Production code uses `MpvPlayer` (real mpv via IPC). Unit tests can use a
`MockPlayer` that records commands and emits scripted events. Integration
tests use real mpv (see [Player Integration Tests](#player-integration-tests)).

---

## Test Tiers

### Unit Tests (`cargo test`)

Fast, in-process, no external dependencies. Cover:

- CRDT operations: apply, merge, snapshot generation
- Playlist op log replay with various operation orders
- Time sync offset calculation
- File hash computation
- Version vector comparison and gap detection
- `AppState` event processing (inject event, check state + effects)
- Chat message ordering and deduplication

### Integration Tests (`cargo test` with real binaries)

Slower, may spawn external processes. Cover:

- Player bridge with real mpv (`-vo null -ao null`)
- Echo suppression (send command to mpv, verify no re-broadcast)
- State sync convergence across multiple `AppState` instances connected
  via `SimulatedNetwork`
- Reconnection and epoch handling
- File transfer chunking and reassembly

### System Tests (`cargo test --features system-test` or manual)

Full end-to-end in tmux. Cover:
- The complete user workflow (connect, add file, play, chat, disconnect)
- See [System Tests (tmux)](#system-tests-tmux)

---

## SimulatedNetwork

An in-process implementation of the `Network` trait that simulates a full
mesh of peers without real sockets.

### Capabilities

| Feature | Description |
|---------|-------------|
| **Packet loss** | Drop datagrams with configurable probability per link |
| **Latency** | Delay message delivery by a configurable duration per link |
| **Partition** | Completely block traffic between specific peers |
| **Reordering** | Shuffle datagram delivery order (configurable window) |
| **Bandwidth limit** | Throttle throughput on specific links |

### Design

```rust
struct SimulatedNetwork {
    peers: HashMap<PeerId, PeerMailbox>,
    config: SimConfig,
    rng: StdRng,  // seeded for reproducibility
    clock: SimClock,  // controlled clock, integrates with tokio::time::pause()
}

struct SimConfig {
    /// Per-link configuration; missing entries use defaults
    links: HashMap<(PeerId, PeerId), LinkConfig>,
    defaults: LinkConfig,
}

struct LinkConfig {
    latency: Duration,
    packet_loss: f64,      // 0.0 - 1.0
    reorder_window: usize, // 0 = in-order
    bandwidth: Option<u64>, // bytes/sec, None = unlimited
    partitioned: bool,
}
```

### Usage Pattern

```rust
#[tokio::test]
async fn test_crdt_convergence_with_packet_loss() {
    tokio::time::pause();
    let mut net = SimulatedNetwork::new(seed(42));
    net.set_default_loss(0.1);  // 10% packet loss

    let peers = net.create_peers(4);
    // ... drive state changes, advance time, assert convergence
}
```

### Time Control

`SimulatedNetwork` uses `tokio::time::pause()` so that time only advances
when explicitly advanced or when all tasks are idle. This eliminates flaky
timing dependencies.

---

## Player Integration Tests

### Setup

Integration tests that use mpv require the `mpv` binary in `$PATH`. Tests
are gated behind `#[cfg(feature = "mpv-tests")]` so CI environments without
mpv can skip them.

mpv is launched with `-vo null -ao null` to suppress video/audio output. It
still processes IPC commands identically to normal operation.

### Cleanup

An atexit handler (via `std::panic::set_hook` + `Drop` on the test fixture)
ensures all spawned mpv processes are killed, even on test failure or panic.

```rust
struct MpvFixture {
    process: Child,
    ipc: MpvIpc,
}

impl Drop for MpvFixture {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
```

### Echo Suppression Tests

These are among the most critical integration tests. The pattern:

1. Connect to mpv via IPC
2. Send a seek command
3. Receive the resulting position-change event from mpv
4. Verify the event is tagged as "echo" and not forwarded to the sync engine

Test cases include:
- Seek echo (send seek, receive position update)
- Pause echo (send pause, receive pause event)
- Rapid seeks (debouncing interacts with echo detection)
- External pause (user pauses in mpv directly — this is *not* an echo)

---

## CRDT Property Tests

Using proptest to verify convergence properties.

### Core Property: Convergence

For every CRDT type, the fundamental property is:

> Given the same set of operations, any application order produces the same
> snapshot.

```rust
proptest! {
    #[test]
    fn playlist_ops_converge(
        ops in vec(arb_playlist_op(), 1..50),
        permutation_seed in any::<u64>(),
    ) {
        // Apply ops in original order
        let snapshot_a = replay_ops(&ops);

        // Apply ops in a random permutation
        let mut shuffled = ops.clone();
        shuffled.shuffle(&mut StdRng::seed_from_u64(permutation_seed));
        let snapshot_b = replay_ops(&shuffled);

        assert_eq!(snapshot_a, snapshot_b);
    }
}
```

### Test Scenarios

| CRDT | Property | Notes |
|------|----------|-------|
| LWW Register | Last timestamp wins regardless of apply order | Straightforward |
| Playlist | Same ops, any order → same final list | Most complex; test Add/Remove/Move interactions |
| Chat | Per-user messages maintain sequence order | Interleaving between users may vary; per-user order is stable |

### Multi-Peer Convergence

Beyond single-replica replay, test that N simulated peers exchanging ops
via `SimulatedNetwork` (with loss, reordering, partitions) eventually
converge to the same state after the network stabilizes.

---

## TUI Testing

### Snapshot Tests (insta)

The TUI rendering function takes `AppState` and produces a `ratatui::Frame`
(or equivalent buffer). Snapshot tests capture the rendered output as text:

```rust
#[test]
fn test_main_screen_layout() {
    let state = AppState::with_test_data();
    let buffer = render_to_buffer(&state, Rect::new(0, 0, 120, 40));
    insta::assert_snapshot!(buffer_to_string(&buffer));
}
```

Snapshots are committed to the repo. `cargo insta review` provides a diff
UI when snapshots change.

### What Snapshot Tests Cover

- Layout proportions (chat, users, playlist, player status)
- Color and style of ready states (green/red/gray/blue)
- Keybinding bar content changes with focused pane
- Playlist rendering (current item highlighted, missing items red)
- Chat message display and wrapping
- Edge cases: empty playlist, no connected users, long filenames

### What Snapshot Tests Do NOT Cover

Application logic. That's the job of `AppState` unit and integration tests,
which operate on the state/event model directly — no terminal rendering, no
text parsing.

---

## Fuzz Testing

### Deserialization Fuzzing

All postcard-encoded messages received from the network are deserialized from
untrusted input. Fuzz targets ensure malformed input cannot crash the process:

```rust
// fuzz/fuzz_targets/peer_control.rs
fuzz_target!(|data: &[u8]| {
    let _ = postcard::from_bytes::<PeerControl>(data);
});

// fuzz/fuzz_targets/peer_datagram.rs
fuzz_target!(|data: &[u8]| {
    let _ = postcard::from_bytes::<PeerDatagram>(data);
});

// fuzz/fuzz_targets/rv_control.rs
fuzz_target!(|data: &[u8]| {
    let _ = postcard::from_bytes::<RvControl>(data);
});
```

Run with `cargo fuzz run <target>`. Fuzz for at least 10 minutes per target
before release.

### CRDT Op Fuzzing

Beyond deserialization, fuzz the CRDT replay engine with random op sequences
to ensure it never panics:

```rust
fuzz_target!(|ops: Vec<CrdtOp>| {
    let mut state = CrdtState::new();
    for op in ops {
        state.apply(op);  // must not panic
    }
    state.snapshot();  // must not panic
});
```

---

## System Tests (tmux)

A full end-to-end test harness that starts the entire system in a tmux server.

### Setup

```bash
# Uses a dedicated tmux server socket to avoid interfering with user sessions
tmux -L dessplay new-session -d -s test

# Start rendezvous server
tmux -L dessplay send-keys \
  'dessplay-rendezvous --password-file /tmp/test-password' Enter

# Start clients in separate panes
tmux -L dessplay split-window -h
tmux -L dessplay send-keys \
  'dessplay --server localhost --password-file /tmp/test-password' Enter
# ... repeat for additional clients
```

### What System Tests Verify

- End-to-end connectivity: clients discover each other via rendezvous
- Chat messages appear on all clients
- Playlist changes propagate
- Player sync: play/pause/seek propagates (with real mpv, `-vo null -ao null`)
- Reconnection: kill and restart a client, verify it rejoins and re-syncs

### Automation

System tests can be driven by `tmux send-keys` to simulate user input and
`tmux capture-pane` to read output. However, these are inherently more
fragile than the `AppState`-level integration tests and serve as a final
confidence check, not the primary test suite.

A test runner script (`tests/system/run.sh`) orchestrates the tmux session,
runs a scenario, captures results, and tears everything down. The tmux server
is killed unconditionally on exit (trap EXIT).

### When to Run

System tests are slow and require a full environment (mpv, tmux, possibly
a test media file). They are:
- Run manually during development (`cargo make system-test` or similar)
- Run in CI on a dedicated stage (not on every commit)
- Required before release

---

## Key Crates

| Crate | Purpose |
|-------|---------|
| `proptest` | Property-based testing for CRDT convergence |
| `insta` | Snapshot testing for TUI rendering |
| `cargo-fuzz` / `libfuzzer-sys` | Fuzz testing for deserialization and CRDT ops |
| `tokio::time::pause()` | Deterministic time control in async tests |
| `tracing-test` | Capture and assert on log output in tests |

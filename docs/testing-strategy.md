# Testing Strategy

Last updated: 2026-02-15

# Part 1: Player Integration

## Why Echo Suppression Exists

DessPlay synchronizes video playback across peers. When one user pauses or seeks, the system sends commands to every other user's player. The problem: mpv reports all state changes — both from IPC commands and from user input — through the same event stream with identical event formats.

Without echo suppression, a programmatic `seek(30)` would produce a `time-pos` property change that the bridge interprets as "the user sought to 30s", which the sync engine would broadcast back to all peers, creating an infinite feedback loop.

The bridge must distinguish:
- **Programmatic actions** (from the sync engine → suppress, don't re-broadcast)
- **User actions** (from keypresses in the mpv window → emit as `UserSeeked`/`UserPauseToggled`)

## The Pending-Command Tracking Algorithm

### Pause/Play

1. Before sending `set_property pause true/false` via IPC, set `pending_pause = Some(target_value)`
2. When a `pause` property-change event arrives:
   - If `pending_pause == Some(received_value)`: clear pending, suppress (programmatic)
   - Otherwise: clear pending, emit `UserPauseToggled` (user action)

### Seek

Seek detection uses mpv's `seek` raw event (not position-jump heuristics). mpv emits `event: "seek"` for ALL seeks, whether from IPC commands or user keybindings.

1. Before sending `seek` via IPC, set `pending_seek = true`
2. On mpv `seek` event:
   - If `pending_seek == true`: clear it, suppress. Set `suppressing_stale_positions = true`
   - If `pending_seek == false`: user-initiated. Set `awaiting_user_seek_pos = true`, `suppressing_stale_positions = true`
3. On next `time-pos` update while `awaiting_user_seek_pos`: emit `UserSeeked { position }`, clear flag
4. On `playback-restart` event: clear `suppressing_stale_positions` (stale position data is gone)

### Why Not Position-Jump Heuristics?

An earlier design considered detecting seeks by watching for discontinuities in `time-pos` (e.g., position jumps > 2s between updates). This approach is fragile:
- Threshold tuning is error-prone (what about slow-motion or frame-stepping?)
- Buffering hiccups can cause legitimate position gaps
- mpv's `seek` event is authoritative and reliable

## Edge Cases

### Seek During Seek

If the user seeks while a programmatic seek is in flight:
1. Programmatic seek sets `pending_seek = true`
2. First `seek` event → matches pending, suppressed, stale positions suppressed
3. User presses RIGHT → mpv emits second `seek` event
4. Second `seek` event → `pending_seek` is already false → treated as user seek
5. Next `time-pos` → emitted as `UserSeeked`

This works correctly because pending state is consumed (cleared) on first match.

### User Overrides Programmatic Command

If a user presses space while a programmatic `pause(true)` is pending:
- The user's space toggles pause to true (same as our command)
- The first `pause` property-change matches `pending_pause = Some(true)` → suppressed
- This is a misattribution: the user's action was suppressed

This is the **fundamental race**: when both actors target the same state at the same instant, the first event consumes the pending token. The impact is bounded:
- The state ends up correct (paused in both cases)
- The sync engine doesn't receive a redundant event (acceptable)
- Detectable via the attribution log in fuzz tests

### Simultaneous Pause From Two Sources

If programmatic `pause(true)` and user space-press both fire within milliseconds:
- mpv receives both, but pause is idempotent (setting `pause=true` twice only fires one event)
- The single event matches the pending → suppressed
- If the user's press arrives second and mpv is already paused, no event fires at all

### Stale Position Data During Seek

Between a seek command and `playback-restart`, mpv may emit `time-pos` updates with the old position (from the pre-seek decode buffer). These are suppressed via `suppressing_stale_positions` to prevent the sync engine from seeing phantom "we're at 5s" updates when we just sought to 30s.

## User Input Simulation

Tests use mpv's `keypress` IPC command: `["keypress", "space"]`, `["keypress", "RIGHT"]`, etc.

This sends the keypress through mpv's full input pipeline — keybindings → input actions → property changes — producing events indistinguishable from real hardware keypresses. The `keypress` command does NOT register pending state in the echo suppression system, so the resulting events are correctly attributed as user actions.

This approach avoids:
- Needing a Wayland/X11 display for testing
- Fragile synthetic X11 events via xdotool
- Testing a different code path than production

All tests run with `--vo=null --ao=null` (headless) to avoid opening windows.

## Separation of Concerns

The bridge reports ALL user seeks and pause toggles to the sync engine. It does not apply the 3-second sync tolerance (that's the sync engine's job). This keeps the bridge layer simple and testable independently.

## Fuzz Test Design

### Setup

5 fuzz tests with fixed seeds (1, 2, 3, 42, 1337), each running 5 seconds of mpv playback time. Marked `#[ignore]` to avoid running in CI by default.

### Two Concurrent Actors

- **Network actor**: randomly calls `pause()`, `play()`, `seek(random_pos)` with 10-200ms delays
- **User actor**: randomly sends `keypress("space")`, `keypress("RIGHT")`, `keypress("LEFT")` with 50-500ms delays

Both run simultaneously using `tokio::spawn`, freely interleaving.

### Invariants Checked

1. **No phantom suppressions**: `suppressed_pause_count <= network_pause_play_count` — every suppression must have a corresponding network command
2. **No phantom seek suppressions**: `suppressed_seek_count <= network_seek_count`
3. **Non-empty attribution**: the log should contain entries (the fuzz run generated events)
4. **Structural consistency**: each attribution entry is exactly one variant (can't be both suppressed and emitted)

### How Violations Map to Bugs

| Violation | Likely Bug |
|-----------|-----------|
| More suppressions than commands | `pending_*` not cleared properly, or set without a command |
| Empty attribution log | Event translator not running, or events not reaching it |
| `SuppressedAsStalePosition` after `playback-restart` | `suppressing_stale_positions` not cleared on restart |

---

# Part 2: Network Testing

## Overview

Network testing uses a `SimulatedNetwork` that implements the `ConnectionManager`
trait entirely in-memory. This enables deterministic, fast tests with configurable
latency, packet loss, reordering, and network partitions — all compatible with
`tokio::time::pause()` for instant-time tests.

## SimulatedNetwork Design

### LinkConfig

Each link between two peers has independent configuration. Links are
asymmetric by default (A→B can differ from B→A).

```rust
pub struct LinkConfig {
    pub latency_ms: u64,       // base one-way latency
    pub jitter_ms: u64,        // uniform jitter ±jitter_ms around base
    pub loss_rate: f64,        // datagram drop probability [0.0, 1.0]
    pub reorder_rate: f64,     // probability of out-of-order delivery
}

impl Default for LinkConfig {
    fn default() -> Self {
        // Perfect network: no latency, no loss
        Self { latency_ms: 0, jitter_ms: 0, loss_rate: 0.0, reorder_rate: 0.0 }
    }
}
```

### SimulatedNetwork

The central coordinator. Creates peers and controls the network topology.

```rust
pub struct SimulatedNetwork {
    seed: u64,  // deterministic RNG for reproducibility
    // Internal: per-link config, partition set, peer channels
}

impl SimulatedNetwork {
    pub fn new(seed: u64) -> Self;

    /// Add a peer and get its ConnectionManager handle.
    pub fn add_peer(&mut self, id: PeerId) -> SimulatedConnectionManager;

    /// Configure link from `from` to `to` (asymmetric).
    pub fn set_link(&self, from: PeerId, to: PeerId, config: LinkConfig);

    /// Configure link in both directions (symmetric).
    pub fn set_link_symmetric(&self, a: PeerId, b: PeerId, config: LinkConfig);

    /// Block all traffic from `from` to `to`.
    pub fn partition(&self, from: PeerId, to: PeerId);

    /// Restore traffic from `from` to `to`.
    pub fn heal(&self, from: PeerId, to: PeerId);
}
```

### SimulatedConnectionManager

Implements `ConnectionManager` for a single peer within the simulation.

**Datagram delivery**: On `send_datagram`, the message is evaluated against the
link config for that (from, to) pair:
1. If partitioned → silently dropped
2. Roll loss_rate → if lost, silently dropped
3. Compute delay = `latency_ms + uniform(-jitter_ms, +jitter_ms)`
4. Roll reorder_rate → if reordering, add extra random delay (0..2×latency)
5. Schedule delivery via `tokio::time::sleep(delay)` + channel push

**Reliable delivery**: On `send_reliable`:
1. If partitioned → return `Err(Partitioned)`
2. Apply latency + jitter (no loss, no reordering — QUIC guarantees these)
3. Deliver via `tokio::time::sleep(delay)` + channel push

This means tests using `tokio::time::pause()` execute near-instantly while
simulating realistic network conditions.

### Seeded RNG

All randomness (loss rolls, jitter values, reorder decisions) uses a seeded
`rand::rngs::StdRng`. Given the same seed and the same sequence of operations,
results are perfectly reproducible. This is critical for debugging — a failing
fuzz seed can be re-run to reproduce the exact failure.

## Test Categories Per Layer

| Category | Count | Simulated? | Description |
|----------|-------|-----------|-------------|
| QUIC integration | ~9 | No (localhost) | Real quinn connections, handshake, streams, datagrams |
| Clock sync | ~7 | Yes | Offset convergence, drift correction, jitter filtering |
| Sync engine | ~10 | Yes | LWW merge, gossip forwarding, gap fill, late join |
| Application channels | ~10 | Yes | Derived pause, playlist merge, chat ordering |
| Fault scenarios | ~12 | Yes | Parameterized adverse conditions (see below) |
| Network fuzz | 5 | Yes | Seeded random workload + random network conditions |

### QUIC Integration (~9 tests, real localhost)

These test the actual quinn-based ConnectionManager against itself on localhost:
1. Two peers connect and exchange datagrams
2. Reliable stream send/recv
3. Multiple concurrent streams
4. Connection event notifications (connect/disconnect)
5. Peer list accuracy
6. Idle timeout triggers disconnect event
7. Reconnection after disconnect
8. Large reliable message (exceeds single packet)
9. Datagram under MTU limit

### Clock Sync (~7 tests, simulated)

1. Two peers converge to same offset (0ms latency)
2. Convergence with symmetric latency (100ms)
3. Convergence with asymmetric latency
4. Jitter filtering (high jitter, verify median filter works)
5. Drift correction (one peer's clock drifts, verify re-sync)
6. Three peers all agree within tolerance
7. Server disconnect — peer-to-peer sync continues

### Sync Engine (~10 tests, simulated)

1. LWW register: newer timestamp wins
2. LWW register: older timestamp ignored
3. Gossip forwarding: A→B→C (A and C not directly connected)
4. Append log: eager push delivers to all peers
5. Gap fill: late joiner receives missing entries
6. Gap fill: origin disconnected, other peer fills gap
7. State vector merge: higher vectors trigger gap fill request
8. Broadcast rate: 100ms during play, 1s during pause
9. Rate transition: burst after state change
10. Three-peer convergence after concurrent updates

### Application Channels (~10 tests, simulated)

1. Derived pause: all Ready → playing
2. Derived pause: one Paused → all paused
3. Derived pause: NotWatching users excluded from check
4. User state: LWW merge on reconnection
5. Playlist: concurrent adds from two users both survive
6. Playlist: Remove of already-removed item is no-op
7. Playlist: Move with stable IDs
8. Chat: messages from all users sorted by timestamp
9. Chat: gap fill recovers missed messages
10. Late join: new peer gets full state (playlist + position + chat)

## Fault Injection Scenarios

12 scenarios, each with 3 peers (A, B, C), a workload (chat messages +
playlist ops + state changes), and convergence verification after settling.

| # | Scenario | Config |
|---|----------|--------|
| 1 | Clean baseline | 0ms latency, 0% loss |
| 2 | Moderate loss | 5% loss, 20ms latency |
| 3 | Heavy loss | 30% loss, 20ms latency |
| 4 | High latency | 500ms latency, 0% loss |
| 5 | Partition: A↔B blocked | A and B both reach C, not each other |
| 6 | Full partition then heal | All links down 2s, then restored |
| 7 | Asymmetric loss | A→B 20% loss, B→A 0% loss |
| 8 | High jitter | 50ms ± 45ms latency |
| 9 | Peer join mid-session | A and B run 1s, then C joins |
| 10 | Peer leave and rejoin | C disconnects for 1s, then reconnects |
| 11 | Clock drift | One peer's clock drifts +50ms/s |
| 12 | Packet reordering | 30% reorder rate |

### Convergence Invariants

After each scenario's workload completes and a settling period elapses:

1. **Position**: all peers agree on playback position (within sync tolerance)
2. **File**: all peers agree on current file
3. **Chat**: all chat messages present on all peers (no gaps)
4. **Playlist**: playlist state converged (same items, same order)
5. **User states**: all peers have consistent view of every user's state

```rust
/// Assert all peers have converged to equivalent state.
pub async fn assert_converged(peers: &[&SyncEngine], tolerance_secs: f64) {
    // Compare position registers (within tolerance)
    // Compare file_id
    // Compare chat logs (all entries present everywhere)
    // Compare playlist (replay produces same result)
    // Compare user state maps
}
```

## Network Fuzz Tests

Same pattern as existing mpv fuzz tests: fixed seeds, `#[ignore]`, random
workloads with invariant checking after settling.

### Setup

5 fuzz tests with fixed seeds (1, 2, 3, 42, 1337). Marked `#[ignore]`.
Each runs for 5 simulated seconds with `tokio::time::pause()`.

### Random Workload

Each peer runs a random workload generator:
- Send chat messages at random intervals (50-500ms)
- Add/remove playlist items at random intervals (200-1000ms)
- Change user state at random intervals (500-2000ms)
- Update position at random intervals (100-500ms)

### Random Network Conditions

The network conditions change during the test:
- Every 500ms, randomly adjust link configs (latency, loss, jitter)
- Occasionally partition random peer pairs (10% chance per interval)
- Heal partitions after random duration (200-1000ms)

### Invariants

Same convergence invariants as fault scenarios, checked after a 2-second
settling period at the end.

## Test Infrastructure

### File Layout

```
tests/
    common/
        mod.rs                  -- re-exports
        simulated_network.rs    -- SimulatedNetwork + SimulatedConnectionManager
        convergence.rs          -- assert_converged() helper
        workload.rs             -- random workload generators
    network_fault_scenarios.rs  -- 12 fault scenario tests
    network_fuzz.rs             -- 5 fuzz tests (#[ignore])
```

### Trace Logging

Following the player integration pattern (EventAttribution), network tests
use structured trace events for debugging failures:

```rust
enum NetworkTraceEvent {
    MessageSent { from: PeerId, to: PeerId, payload_type: &'static str },
    MessageDelivered { from: PeerId, to: PeerId, delay_ms: u64 },
    MessageDropped { from: PeerId, to: PeerId, reason: DropReason },
    GapFillRequested { from: PeerId, to: PeerId, log_type: LogType, range: (u64, u64) },
}

enum DropReason {
    Partitioned,
    RandomLoss,
}
```

These are emitted via `tracing::debug!` and can be captured with
`tracing-subscriber` in tests when debugging specific seeds.

## Full Test Matrix

| Category | Count | `#[ignore]` | Requires mpv | Requires network |
|----------|-------|------------|--------------|------------------|
| Protocol unit tests | 14 | No | No | No |
| Player integration | 21 | No | Yes | No |
| Player fuzz | 5 | Yes | Yes | No |
| QUIC integration | ~9 | No | No | localhost |
| Clock sync | ~7 | No | No | Simulated |
| Sync engine | ~10 | No | No | Simulated |
| Application channels | ~10 | No | No | Simulated |
| Fault scenarios | ~12 | No | No | Simulated |
| Network fuzz | 5 | Yes | No | Simulated |

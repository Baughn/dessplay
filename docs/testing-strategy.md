# Testing Strategy: Player Integration

Last updated: 2026-02-15

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

## Test Matrix

| Category | Count | `#[ignore]` | Requires mpv |
|----------|-------|-------------|--------------|
| Protocol unit tests | 14 | No | No |
| Integration tests | 21 | No | Yes |
| Fuzz tests | 5 | Yes | Yes |

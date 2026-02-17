# Docs

- Overall design: docs/design.md
- Network design: docs/network-design.md
- Sync state registers: docs/sync-state.md
- Implementation plan: docs/plan.md
- Testing strategy: docs/testing-strategy.md

Read the design docs before any planning phase. Update the docs after any design change, and update CLAUDE.md if a document is added.

If anything is unclear, ALWAYS ask the user to clarify.

# Revision Control

This project uses **jujutsu** (`jj`) for revision control, not raw git. Use `jj` commands for commits, branches, and history operations.

When asked to commit, use `jj commit`; not `jj describe`.

# Testing

Full details in docs/testing-strategy.md. This section covers the practical essentials.

## Philosophy

Test comprehensively, especially on high-risk areas (echo suppression, network convergence). Prefer deterministic, reproducible tests — seeded RNG, paused tokio time, no flaky sleeps.

## Running tests

- `cargo test` — all non-ignored tests. Player tests require mpv.
- `cargo test -- --ignored` — fuzz tests (fixed seeds, longer runs)
- `cargo test --test quic_integration` — run a specific test file
- `RUST_LOG=debug cargo test my_test -- --nocapture` — debug a failing test

## Test infrastructure

- **SimulatedNetwork** (`tests/common/simulated_network.rs`): in-memory `ConnectionManager` with configurable latency, loss, jitter, reordering, and partitions. Use with `#[tokio::test(start_paused = true)]` for instant, deterministic execution.
- **Seeded RNG**: all randomness in SimulatedNetwork and fuzz tests uses a fixed seed for reproducibility. A failing seed can be re-run to reproduce exact failures.
- **Headless mpv**: tests use `--vo=null --ao=null`. User input is simulated via mpv's `keypress` IPC command (goes through the full input pipeline, same code path as real keypresses).
- **Attribution log**: echo suppression debugging — `player.attribution_log()` records which events were suppressed and why.

## Writing new tests

- Use `SimulatedNetwork` for anything above the transport layer. It's fast and deterministic.
- Use real QUIC (localhost) only for transport-level concerns (handshake, MTU, stream mechanics).
- Use `#[tokio::test(start_paused = true)]` for any test with timing (delays, timeouts, intervals).
- Fuzz tests: use fixed seeds, mark `#[ignore]`, check invariants not exact output.
- Test at the trait boundary (`ConnectionManager`, `PlayerBridge`) not internal implementation details.

## tmux smoke testing

`scripts/tmux.sh` provides automated manual testing — it launches a full stack in tmux for visual inspection. This is useful for TUI layout checks, end-to-end connectivity verification, and anything that's hard to assert programmatically.

```
scripts/tmux.sh start              # server + 3 clients (alice, bob, charlie)
scripts/tmux.sh capture alice      # print alice's TUI output
scripts/tmux.sh wait-and-capture bob 6  # wait 6s, then capture
scripts/tmux.sh stop               # tear down
```

Use after automated tests pass, not as a substitute. There's a `/tmux-testing` skill for guided walkthroughs.

design.md follows: @docs/design.md


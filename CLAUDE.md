# Docs

- Overall design: docs/design.md
- Architecture (actors, message flow, workspace): docs/architecture.md
- Sync state (CRDTs via `crdts` crate, compaction): docs/sync-state.md
- Network design (QUIC, hub-and-spoke, file transfer): docs/network-design.md
- Testing strategy: docs/testing-strategy.md
- UI architecture (tui-realm, Elm model): docs/ui-architecture.md
- Implementation plan: docs/plan.md

Read the design docs before any planning phase. Update the docs after any design change, and update CLAUDE.md if a document is added.

If anything is unclear, ALWAYS ask the user to clarify.

# Environment

A `.env` file (gitignored) contains `DESSPLAY_PASSWORD` for the default rendezvous server. This is loaded automatically at startup.

# Revision Control

This project uses **jujutsu** (`jj`) for revision control, not raw git. Use `jj` commands for commits, branches, and history operations.

When asked to commit, use `jj commit`, not `jj describe`. Don't bother to check the diff; I don't mix changes.

# Bug fixing

This is important:
- Feel free to add more logging and/or ask the user for assistance.
- Always add a regression test *prior* to fixing the bug. Prefer
  to do this via property or fuzz tests; a property test that happens
  to catch the bug is superior to a unit test that only catches this particular bug.
- Most crucially: Bugs are often an indication that there are architectural issues.
  Every bug is an opportunity to improve the architecture, or clear up a design doc detail,
  refactor some code, write a fuzz test, or in general reduce the chance you'll need to
  fix another bug in the future.
- Refer to docs/testing-strategy.md for details.

You should ALWAYS strive to leave the code *better* off after bug-fixing.

# Testing

Full details in docs/testing-strategy.md. This section covers the practical essentials.

## Philosophy

Test comprehensively, especially on high-risk areas (echo suppression, network convergence). Prefer deterministic, reproducible tests — seeded RNG, paused tokio time, no flaky sleeps. Read docs/testing-strategy.md before writing any plan.

### Principles

- **Deterministic and reproducible**: Seeded RNG, paused tokio time, no flaky
  sleeps. Every test failure should be reproducible from the seed alone.
- **Spec-driven**: Write tests from the specification, not the implementation.
  If the spec is unclear, clarify it before writing the test.
- **Regression tests first**: When fixing a bug, write a test that reproduces
  it *before* writing the fix. The tests MUST be executed and confirmed to fail.
- **High-risk areas get extra coverage**: Echo suppression, CRDT convergence,
  playlist conflict resolution, reconnection/epoch handling.


---

design.md follows: @docs/design.md


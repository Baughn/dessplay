---
name: tmux-testing
description: Manual verification of the DessPlay TUI using tmux. Use for final end-to-end verification after automated unit and integration tests pass. The scripts/tmux.sh helper launches a rendezvous server and 3 test clients (alice, bob, charlie) automatically, with capture commands to inspect output. Tmux-based testing supplements but does not replace automated tests.
model: inherit
# allowed-tools: "Bash(scripts/tmux.sh:*)"
---

# tmux-based TUI Testing

Verify interactive TUI behavior that automated tests cannot cover. **Always run `cargo test` first.**

## Commands

```bash
scripts/tmux.sh start                        # Launch server + alice, bob, charlie
scripts/tmux.sh stop                         # Kill session, clean up temp files
scripts/tmux.sh capture <window>             # Print pane (server|alice|bob|charlie)
scripts/tmux.sh wait-and-capture <window> [s] # Sleep then capture (default 4s)
```

`start` creates a tmux session `dessplay` with 4 windows: a rendezvous server on `[::1]:4433` and 3 clients. Each window runs a hardcoded cargo command — no shell access.

## Typical Workflow

```bash
scripts/tmux.sh start
# Inspect output:
scripts/tmux.sh capture alice    # Verify TUI layout, chat messages, user list
scripts/tmux.sh capture bob      # Verify bob sees alice and charlie
scripts/tmux.sh capture server   # Verify server logs
# Done:
scripts/tmux.sh stop
```

## Key Notes

- **Start waits 8s total** (4s for server, 4s for clients to connect). If binaries need compilation, the first `start` takes longer — use `wait-and-capture` with extra delay.
- **Disconnect detection** takes ~30s (QUIC idle timeout). Use `wait-and-capture <window> 35` if testing disconnects.
- **Windows run fixed commands**, not interactive shells. When the process exits, `read` keeps the window open so output can still be captured.
- Always `scripts/tmux.sh stop` when done to clean up the tmux session.

## In case of missing commands

Stop, and ask the user what to do. DO NOT attempt to work around script limitations.

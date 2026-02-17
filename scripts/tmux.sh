#!/usr/bin/env bash
set -euo pipefail

SESSION="dessplay"
BIND="[::1]:4433"
PASSWORD="dessplay-test"
USERS=("alice" "bob" "charlie")

usage() {
    cat <<'EOF'
Usage: scripts/tmux.sh <command> [args...]

Commands:
  start                  Launch rendezvous server + 3 clients (alice, bob, charlie)
  stop                   Kill the test session and clean up
  capture <window>       Print pane contents (server, alice, bob, charlie)
  wait-and-capture <window> [seconds]  Sleep then capture (default 4s)
  v4                     Connect to v4.brage.info with a bad password
EOF
    exit 1
}

[[ $# -lt 1 ]] && usage

cmd="$1"; shift

case "$cmd" in
    start)
	# Ensure fresh state
	cargo build
        tmux -L dessplay kill-session -t "$SESSION" 2>/dev/null || true

        # Server window
        tmux -L dessplay new-session -d -s "$SESSION" -n server -x 100 -y 25 \
            "cargo run --bin dessplay-rendezvous -- --bind '$BIND' --password '$PASSWORD'; read"
        sleep 1  # wait for server to start

        # Client windows
        for user in "${USERS[@]}"; do
            tmux -L dessplay new-window -t "$SESSION" -n "$user" \
                "cargo run -- --server '$BIND' --password '$PASSWORD' --username '$user'; read"
        done

        sleep 1  # wait for clients to connect
        echo "Test session started: server + ${USERS[*]}"
        ;;
    stop)
        tmux -L dessplay kill-session -t "$SESSION" 2>/dev/null || true
        echo "Test session stopped"
        ;;
    capture)
        [[ $# -lt 1 ]] && usage
        tmux -L dessplay capture-pane -t "$SESSION:$1" -p
        ;;
    wait-and-capture)
        [[ $# -lt 1 ]] && usage
        secs="${2:-4}"
        sleep "$secs"
        tmux -L dessplay capture-pane -t "$SESSION:$1" -p
        ;;
    *)
        echo "Unknown command: $cmd" >&2
        usage
        ;;
esac

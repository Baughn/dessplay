#!/usr/bin/env bash
# Run fuzz targets. Usage:
#   ./fuzz/run.sh                 # all targets, 300s each, in parallel
#   ./fuzz/run.sh crdt_op         # one target, 300s
#   ./fuzz/run.sh --quick         # all targets, 30s each
#   ./fuzz/run.sh --quick crdt_op # one target, 30s
#
# Requires cargo-fuzz and a nightly toolchain:
#   nix run nixpkgs#cargo-fuzz -- --help
set -euo pipefail

cd "$(dirname "$0")/.."

DURATION=300
TARGETS=()
for arg in "$@"; do
    case "$arg" in
        --quick) DURATION=30 ;;
        *) TARGETS+=("$arg") ;;
    esac
done

if [ ${#TARGETS[@]} -eq 0 ]; then
    mapfile -t TARGETS < <(cargo fuzz list)
fi

echo "Fuzzing ${TARGETS[*]} for ${DURATION}s each"

pids=()
for target in "${TARGETS[@]}"; do
    cargo fuzz run "$target" -- -max_total_time="$DURATION" &
    pids+=($!)
done

status=0
for pid in "${pids[@]}"; do
    wait "$pid" || status=1
done

exit $status

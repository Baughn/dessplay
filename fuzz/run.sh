#!/usr/bin/env bash
# Run fuzz targets. Usage:
#   ./fuzz/run.sh             # run all targets, 30s each
#   ./fuzz/run.sh crdt_op     # run one target, 30s
#   ./fuzz/run.sh crdt_op 60  # run one target, 60s
#
# Requires: cargo-fuzz, nightly toolchain

set -euo pipefail
cd "$(dirname "$0")/.."

DURATION="${2:-30}"
TARGETS=("crdt_op" "crdt_convergence" "snapshot_roundtrip" "ops_since")

if [[ $# -ge 1 ]]; then
    TARGETS=("$1")
fi

for target in "${TARGETS[@]}"; do
    echo "=== Fuzzing $target for ${DURATION}s ==="
    cargo fuzz run "$target" -- -max_total_time="$DURATION"
    echo ""
done

echo "All fuzz targets completed."

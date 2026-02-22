#!/usr/bin/env bash
# Fuzz test runner. Suppresses libfuzzer noise, shows clean pass/fail output.
#
# Usage:
#   ./fuzz/run.sh                          # all targets, 300s each
#   ./fuzz/run.sh crdt_op                  # one target, 300s
#   ./fuzz/run.sh crdt_op 60              # one target, 60s
#   ./fuzz/run.sh --quick                  # all targets, 30s each
#   ./fuzz/run.sh --targeted               # only targeted tests, 300s each
#   ./fuzz/run.sh --targeted --quick       # only targeted tests, 30s each
#   ./fuzz/run.sh -j4                      # limit to 4 parallel jobs
#
# Requires: cargo-fuzz, nightly toolchain

cd "$(dirname "$0")/.."

DURATION=300
JOBS=$(( $(lscpu -p=CORE | grep -v '^#' | sort -u | wc -l) - 2 ))
if [[ "$JOBS" -lt 1 ]]; then JOBS=1; fi
TARGET_SET="all"
SINGLE_TARGET=""

ORIGINAL_TARGETS=(crdt_op crdt_convergence snapshot_roundtrip ops_since)
TARGETED_TARGETS=(lww_filestate_convergence chat_gap_fill playlist_targeted postcard_deserialize multi_peer_sync framing_deserialize time_sync_convergence network_sim sync_engine app_state)
ALL_TARGETS=("${ORIGINAL_TARGETS[@]}" "${TARGETED_TARGETS[@]}")

while [[ $# -gt 0 ]]; do
    case "$1" in
        --quick)     DURATION=30; shift ;;
        --targeted)  TARGET_SET="targeted"; shift ;;
        -j*)         JOBS="${1#-j}"; shift ;;
        -h|--help)
            sed -n '2,/^$/{ s/^# //; s/^#$//; p }' "$0"
            exit 0
            ;;
        *)
            if [[ -z "$SINGLE_TARGET" ]]; then
                SINGLE_TARGET="$1"
            else
                DURATION="$1"
            fi
            shift
            ;;
    esac
done

if [[ -n "$SINGLE_TARGET" ]]; then
    TARGETS=("$SINGLE_TARGET")
elif [[ "$TARGET_SET" == "targeted" ]]; then
    TARGETS=("${TARGETED_TARGETS[@]}")
else
    TARGETS=("${ALL_TARGETS[@]}")
fi

WORKDIR=$(mktemp -d)
LOGDIR="$WORKDIR/logs"
RESULTSDIR="$WORKDIR/results"
mkdir -p "$LOGDIR" "$RESULTSDIR"

echo "Fuzzing ${#TARGETS[@]} target(s) for ${DURATION}s each (jobs=$JOBS)"
echo "Logs: $LOGDIR"
echo "---"

# Run a single fuzz target, writing result to a file
run_one() {
    local target=$1
    local logfile="$LOGDIR/${target}.log"
    local resultfile="$RESULTSDIR/${target}"

    if cargo +nightly fuzz run "$target" -- -max_total_time="$DURATION" \
            >"$logfile" 2>&1; then
        echo "PASS" > "$resultfile"
    else
        local artifact
        artifact=$(grep -oP 'Test unit written to \K\S+' "$logfile" 2>/dev/null || true)
        echo "FAIL${artifact:+ $artifact}" > "$resultfile"
    fi
}

# Launch targets, limiting to $JOBS in parallel
running=0
for target in "${TARGETS[@]}"; do
    if [[ $running -ge $JOBS ]]; then
        wait -n 2>/dev/null || true
        ((running--))
    fi
    run_one "$target" &
    ((running++))
done

# Wait for remaining jobs to finish
wait

# Collect and display results
PASSED=0
FAILED=0
FAIL_DETAILS=""

for target in "${TARGETS[@]}"; do
    result=$(cat "$RESULTSDIR/$target" 2>/dev/null || echo "ERROR")
    if [[ "$result" == "PASS" ]]; then
        echo "PASS  $target"
        ((PASSED++))
    else
        artifact="${result#FAIL}"
        artifact="${artifact# }"
        echo "FAIL  $target${artifact:+  artifact=$artifact}"
        FAIL_DETAILS="${FAIL_DETAILS}  $target  log=$LOGDIR/${target}.log${artifact:+  artifact=$artifact}\n"
        ((FAILED++))
    fi
done

TOTAL=${#TARGETS[@]}
echo "---"
echo "$PASSED/$TOTAL passed"

if [[ "$FAILED" -gt 0 ]]; then
    echo ""
    echo "Failed targets:"
    echo -e "$FAIL_DETAILS"
    rm -rf "$WORKDIR"
    exit 1
fi

rm -rf "$WORKDIR"

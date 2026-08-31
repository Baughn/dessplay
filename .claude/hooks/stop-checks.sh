#!/usr/bin/env bash
# Stop hook for dessplay: format the tree, then gate on clippy + tests.
#
# - cargo fmt runs first and rewrites files in place (auto-fix).
# - cargo clippy (warnings-as-errors) and the test suite are quality gates:
#   if either fails, the hook exits 2 and feeds the (tailed) output back to
#   Claude, which then keeps working to fix it instead of stopping.
# - On a green tree the hook exits 0 silently and the turn ends normally.
#
# Tests run under cargo-nextest (parallel across test binaries, per-test
# timeouts, perf tests filtered — see .config/nextest.toml), falling back
# to plain `cargo test` when nextest isn't on PATH (stale dev shell).
# Nextest does not run doctests; the workspace has none (checked
# 2026-08-31) — add a `cargo test --doc` step if that ever changes.
#
# Press Ctrl-C if a genuinely unfixable failure makes this loop.

set -uo pipefail

cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0

out=""
fail=0

if ! fmt_out=$(cargo fmt --all 2>&1); then
  out+="cargo fmt failed:"$'\n'"$fmt_out"$'\n\n'
  fail=1
fi

if ! clippy_out=$(cargo clippy --all-targets --all-features -- -D warnings 2>&1); then
  out+="cargo clippy reported problems (warnings are errors):"$'\n'"$clippy_out"$'\n\n'
  fail=1
fi

# Fast gate: run property tests with a reduced case count (full pinned
# counts run in CI / manual `cargo test`, where PROPTEST_CASES is unset).
# Suites with hardcoded with_cases(N) honor this via
# dessplay_core::test_support::proptest_cases. Pre-set values win.
: "${PROPTEST_CASES:=32}"
export PROPTEST_CASES

if command -v cargo-nextest >/dev/null 2>&1; then
  test_cmd=(cargo nextest run)
else
  test_cmd=(cargo test)
fi

if ! test_out=$("${test_cmd[@]}" 2>&1); then
  out+="${test_cmd[*]} failed:"$'\n'"$test_out"$'\n\n'
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  printf '%s' "$out" | tail -n 200 >&2
  echo "Fix the cargo fmt/clippy/test failures above before finishing." >&2
  exit 2
fi

exit 0

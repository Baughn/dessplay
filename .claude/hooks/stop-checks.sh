#!/usr/bin/env bash
# Stop hook for dessplay: format the tree, then gate on clippy + tests.
#
# - cargo fmt runs first and rewrites files in place (auto-fix).
# - cargo clippy (warnings-as-errors) and cargo test are quality gates:
#   if either fails, the hook exits 2 and feeds the (tailed) output back to
#   Claude, which then keeps working to fix it instead of stopping.
# - On a green tree the hook exits 0 silently and the turn ends normally.
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

if ! test_out=$(cargo test 2>&1); then
  out+="cargo test failed:"$'\n'"$test_out"$'\n\n'
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  printf '%s' "$out" | tail -n 200 >&2
  echo "Fix the cargo fmt/clippy/test failures above before finishing." >&2
  exit 2
fi

exit 0

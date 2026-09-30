#!/usr/bin/env python3
"""Stop hook for dessplay: format the tree, then gate on clippy + tests.

- cargo fmt runs first and rewrites files in place (auto-fix).
- cargo clippy (warnings-as-errors) and the test suite are quality gates:
  if either fails, the hook exits 2 and feeds the (tailed) output back to
  Claude, which then keeps working to fix it instead of stopping.
- On a green tree the hook exits 0 silently and the turn ends normally.
- Loop guard: when Claude is already retrying after a bounce
  (stop_hook_active) and the failures are the same as last bounce, the
  hook lets the stop through and shows the failures to the user instead.
  Failures are compared by signature (step names, compiler diagnostics,
  failing test names) rather than raw text, since timings and nextest's
  progress counters differ on every run.

Tests run under cargo-nextest (parallel across test binaries, per-test
timeouts, perf tests filtered; see .config/nextest.toml), falling back to
plain `cargo test` when nextest isn't on PATH. Nextest does not run
doctests; the workspace has none (checked 2026-08-31). Add a
`cargo test --doc` step if that ever changes.

Cargo runs in the environment `nix develop` provides *now*, not whatever
the Claude session inherited from direnv at startup (a flake.nix change
mid-session would otherwise leave the hook on a stale sysroot). Same
nixpkgs override as .envrc, so the shell derivation matches the
developer's. The dev environment is captured once as a dict and passed
to each cargo run. The flake's shellHook banner never reaches our stdout
(which must stay clean for the JSON reply), and cargo output never lands
in environment variables. (The old bash hook appended output to `$out`,
which stdenv exports, and every exec then failed with E2BIG.)

Standard library only.
"""

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPORT_TAIL = 200
LINE_CAP = 500

# Lines that identify *what* failed, ignoring durations and counters.
NEXTEST_FAIL = re.compile(
    r"^\s+(FAIL|TIMEOUT|SIG[A-Z]+|LEAK(?:-FAIL)?)\s+\[[^\]]*\]\s+(?:\([^)]*\)\s+)?(\S.*)$"
)
LIBTEST_FAIL = re.compile(r"^test (\S+) \.\.\. FAILED")
DIAGNOSTIC = re.compile(r"^(error|warning)(\[[^\]]*\])?: .*|^\s*--> \S+")


def dev_env(project: Path) -> tuple[dict[str, str] | None, str]:
    """The `nix develop` environment, or (None, error output) if it fails.

    Without nix on PATH, the inherited environment is used as-is.
    """
    if shutil.which("nix") is None:
        return dict(os.environ), ""
    args = ["nix", "develop", "."]
    try:
        nixpkgs = subprocess.run(
            ["nix-instantiate", "--find-file", "nixpkgs"],
            capture_output=True, text=True, check=True,
        ).stdout.strip()
        args += ["--override-input", "nixpkgs", f"path:{nixpkgs}"]
    except (OSError, subprocess.CalledProcessError):
        pass
    with tempfile.NamedTemporaryFile(suffix=".json") as dump:
        args += [
            "--command", "python3", "-c",
            "import json, os, sys; json.dump(dict(os.environ), open(sys.argv[1], 'w'))",
            dump.name,
        ]
        result = subprocess.run(
            args, cwd=project, stdin=subprocess.DEVNULL,
            capture_output=True, text=True,
        )
        if result.returncode != 0:
            return None, result.stdout + result.stderr
        return json.load(open(dump.name)), ""


def run_steps(project: Path) -> list[tuple[str, str]]:
    """Run the gate; return (header, output) for each failing step."""
    env, err = dev_env(project)
    if env is None:
        return [("nix develop failed:", err)]

    # Fast gate: run property tests with a reduced case count (full pinned
    # counts run in CI / manual `cargo test`, where PROPTEST_CASES is unset).
    # Suites with hardcoded with_cases(N) honor this via
    # dessplay_core::test_support::proptest_cases. Pre-set values win.
    env.setdefault("PROPTEST_CASES", "32")

    # --no-fail-fast: report every failure in one bounce, and keep the
    # signature stable (fail-fast's "N tests were not run" count varies).
    if shutil.which("cargo-nextest", path=env.get("PATH")):
        test_cmd = ["cargo", "nextest", "run", "--no-fail-fast"]
    else:
        test_cmd = ["cargo", "test", "--no-fail-fast"]

    steps = [
        ("cargo fmt failed:", ["cargo", "fmt", "--all"]),
        (
            "cargo clippy reported problems (warnings are errors):",
            ["cargo", "clippy", "--all-targets", "--all-features", "--", "-D", "warnings"],
        ),
        (f"{' '.join(test_cmd)} failed:", test_cmd),
    ]
    failures = []
    for header, cmd in steps:
        result = subprocess.run(
            cmd, cwd=project, env=env, stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
            errors="replace",
        )
        if result.returncode != 0:
            failures.append((header, result.stdout))
    return failures


def key_lines(failures: list[tuple[str, str]]) -> list[str]:
    """What failed, without run-to-run noise."""
    keys = set()
    for header, output in failures:
        keys.add(header)
        for line in output.splitlines():
            if m := NEXTEST_FAIL.match(line):
                keys.add(f"{m[1]} {m[2].strip()}")
            elif m := LIBTEST_FAIL.match(line):
                keys.add(f"FAIL {m[1]}")
            elif m := DIAGNOSTIC.match(line):
                keys.add(m[0].strip())
    return sorted(keys)


def main() -> int:
    project = Path(os.environ.get("CLAUDE_PROJECT_DIR", "."))
    hook_input = {}
    if not sys.stdin.isatty():
        try:
            hook_input = json.load(sys.stdin)
        except ValueError:
            pass
    retrying = hook_input.get("stop_hook_active") is True
    session = re.sub(r"[^A-Za-z0-9_-]", "", str(hook_input.get("session_id", ""))) or "manual"
    state = Path(tempfile.gettempdir()) / f"dessplay-stop-hook-{session}.keys"

    failures = run_steps(project)
    if not failures:
        state.unlink(missing_ok=True)
        return 0

    keys = key_lines(failures)
    signature = "\n".join(keys) + "\n"
    previous = state.read_text() if state.exists() else None

    if retrying and signature == previous:
        # Same failures as the last bounce: stop looping, tell the user.
        state.unlink(missing_ok=True)
        summary = "\n".join(keys[:20])
        message = (
            "Stop hook: the same failures persisted after a retry; "
            f"letting the turn end.\n{summary}"
        )
        print(json.dumps({"systemMessage": message}))
        return 0

    state.write_text(signature)
    report = "".join(f"{header}\n{output}\n\n" for header, output in failures)
    tail = "\n".join(line[:LINE_CAP] for line in report.splitlines()[-REPORT_TAIL:])
    print(tail, file=sys.stderr)
    print("Fix the cargo fmt/clippy/test failures above before finishing.", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())

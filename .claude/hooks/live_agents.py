#!/usr/bin/env python3
"""Track this project's live subagents, so the Stop gate can stand down.

While subagents (Agent-tool agents, workflow builders and reviewers) are at
work, the working copy is theirs: half-finished edits, test mutants, and
cargo builds racing ours in the shared target/. The Stop gate would report
their failures as the main session's, `cargo fmt` would rewrite files
under them, and its link step could race theirs. So the gate skips while
any is live.

Wired as hooks in .claude/settings.json:
- SubagentStart: `live_agents.py start` records the agent.
- SubagentStop:  `live_agents.py stop` forgets it.

Records are files named by agent id under target/.live-agents, which is
ignored, so jj never snapshots them (`cargo clean` just clears them). A
record older than STALE_SECS is taken for an agent that died without a
SubagentStop and is removed, so a crash can't switch the gate off for good.

Standard library only.
"""

import json
import os
import re
import sys
import time
from pathlib import Path

STALE_SECS = 8 * 60 * 60


def records_dir(project: Path) -> Path:
    """This project's record directory, in the ignored target/ (not a temp
    dir, whose location may differ between the hooks' environments)."""
    return project / "target" / ".live-agents"


def live(project: Path) -> list[str]:
    """Ids of agents recorded as live, after clearing stale records."""
    folder = records_dir(project)
    if not folder.is_dir():
        return []
    now = time.time()
    ids = []
    for record in folder.iterdir():
        try:
            if now - record.stat().st_mtime > STALE_SECS:
                record.unlink(missing_ok=True)
            else:
                ids.append(record.name)
        except OSError:
            pass
    return sorted(ids)


def main() -> int:
    action = sys.argv[1] if len(sys.argv) > 1 else ""
    project = Path(os.environ.get("CLAUDE_PROJECT_DIR", "."))
    try:
        hook_input = json.load(sys.stdin)
    except ValueError:
        hook_input = {}
    agent = re.sub(r"[^A-Za-z0-9_-]", "", str(hook_input.get("agent_id", "")))
    if not agent:
        return 0
    folder = records_dir(project)
    record = folder / agent
    if action == "start":
        folder.mkdir(parents=True, exist_ok=True)
        record.write_text(str(hook_input.get("agent_type", "")) + "\n")
    elif action == "stop":
        record.unlink(missing_ok=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())

import asyncio
from datetime import datetime, timezone
from pathlib import Path

import aiofiles
from git import Repo


async def git_handoff(project_path: str, summary_text: str) -> None:
    # GitPython is fully synchronous, and add(A=True)/commit can take seconds
    # on a large repo, so the blocking work runs in a worker thread to keep the
    # FastAPI event loop responsive (same pattern as DockerRuntime).
    def _commit() -> None:
        repo = Repo(project_path)
        repo.git.add(A=True)
        repo.index.commit(summary_text)

    await asyncio.to_thread(_commit)


def _next_handoff_path(handoffs_dir: Path) -> Path:
    """Path for the next sequential handoff file.

    Scans existing ``handoff_NNN.md`` files and returns ``handoff_<max+1>.md``
    (zero-padded to 3 digits), starting at ``handoff_001.md`` when none exist.
    Gaps are ignored — we always use highest-seen + 1.
    """
    highest = 0
    for entry in handoffs_dir.glob("handoff_*.md"):
        num = entry.stem[len("handoff_"):]
        if num.isdigit():
            highest = max(highest, int(num))
    return handoffs_dir / f"handoff_{highest + 1:03d}.md"


async def summary_handoff(project_path: str, task_description: str) -> None:
    """Append a new numbered handoff file to the project's handoff history.

    Builds a sequential, GitHub-like history under
    ``.handover/handoffs/`` (handoff_001.md, handoff_002.md, ...) so the
    context never grows unbounded, and overwrites ``latest.md`` with the same
    content so the next AI has a fixed path to the most recent state.
    """
    handoffs_dir = Path(project_path) / ".handover" / "handoffs"
    handoffs_dir.mkdir(parents=True, exist_ok=True)

    timestamp = datetime.now(timezone.utc).isoformat()
    content = (
        f"# AI Handoff\n\n"
        f"- Timestamp: {timestamp}\n\n"
        f"## Task\n\n{task_description}\n"
    )

    # The numbered file is the permanent history entry; latest.md is the
    # always-current pointer the AI reads.
    targets = (_next_handoff_path(handoffs_dir), handoffs_dir / "latest.md")
    for path in targets:
        async with aiofiles.open(path, "w", encoding="utf-8") as f:
            await f.write(content)

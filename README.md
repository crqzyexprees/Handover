# Handover

Multi-terminal AI CLI orchestration with Docker sandboxes and project handoffs.

Handover lets you run multiple AI CLI sessions against the same project and
transfer work between them without re-explaining the whole codebase. The desktop
app owns the UI, the Rust backend owns projects, terminals, Docker sandboxes,
and handoff routing.

| Directory | Role |
|-----------|------|
| [`handover_app/`](handover_app/) | Electron + React desktop app |
| [`handover_rust/`](handover_rust/) | Active Rust backend |
| [`handover_python/`](handover_python/) | Legacy Python/FastAPI backend only |

## Workflow

1. Open Handover and add a project folder.
2. Start two or more terminals for that project.
3. Run your AI CLI in each terminal, such as Claude Code, Codex, Gemini, or
   another terminal-based agent.
4. When one AI should hand work to another, click the handoff button.
5. Choose the source terminal, target terminal, and handoff method.

### Handoff Methods

**AI-written Summary File**

Use this when you want the current AI to create a transfer file for the next AI.
You only enter the overall goal or next direction. Handover then:

- Prompts the source terminal's AI to write `.handover/handoffs/latest.md`.
- Also asks it to create the next numbered history file, such as
  `.handover/handoffs/handoff_001.md`.
- Prompts the target terminal's AI to read `.handover/handoffs/latest.md` and
  continue from there.

The source AI writes the useful context because it already has the active
conversation state: current goal, files changed, commands run, test results,
decisions, blockers, and next steps.

**Git Commit**

Use this when the project is a Git repository and you want a checkpoint commit.
Handover stages the current worktree, creates a handoff commit when there are
changes, and prompts the target terminal to inspect the latest commit.

If there are no Git changes, Handover skips the empty commit and tells the next
AI to inspect the current worktree and recent history.

## Current Release

Download the latest AppImage from the GitHub releases page:

```text
https://github.com/crqzyexprees/Handover/releases/latest
```

## Quick Start

### 1. Install app dependencies

```bash
cd handover_app
npm install
```

### 2. Build the Docker sandbox image

Docker mode expects the local sandbox image:

```bash
docker build -f ../handover_python/docker/Dockerfile.base -t handover-base:latest ../handover_python/docker/
```

Run this from `handover_app/`, or adjust the paths if you run it from the repo
root.

### 3. Build the Rust backend

```bash
cd ../handover_rust
cargo build
```

### 4. Run the desktop app

```bash
cd ../handover_app
npm run dev:electron
```

The desktop app starts the Rust backend automatically on a free localhost port.

## Distribution Build

```bash
cd handover_app
npm run build:backend
npm run build
```

The packaged app is written to `handover_app/release/` and bundles
`handover_rust/target/release/handover-backend`.

## Rust Backend Notes

The Rust backend stores project/config state in the user's data directory and
uses Docker containers for sandboxed terminals by default. Native terminals are
also available from project settings.

The backend currently handles:

- Project creation and persisted project config.
- WebSocket PTY terminals with resize support.
- Docker sandbox start/stop/pause/resume and stats.
- RAM governor behavior for suspending inactive Docker projects.
- Git and AI-written summary handoff routing.

## Python Backend Fallback

The old backend is kept in `handover_python/` for reference or fallback:

```bash
cd handover_python
python -m venv venv
./venv/bin/pip install -r requirements.txt

cd ../handover_app
npm run dev:electron:python
```

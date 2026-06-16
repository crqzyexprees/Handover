# Handover App

Electron + React desktop app for Handover.

This package owns the desktop shell, React UI, xterm.js terminals, Vite build,
and Electron packaging. The active backend is the Rust service in
`../handover_rust`.

## Development

```bash
npm install
npm run dev:electron
```

`npm run dev:electron` starts Vite, then launches Electron with
`HANDOVER_BACKEND=rust`. Electron starts the Rust backend on a free localhost
port and passes that port to the renderer.

## Handoff Workflow

Open a project, start at least two terminals, and run your AI CLI sessions in
those terminals. The handoff modal supports two methods:

- `AI-written Summary File`: prompts the source AI terminal to write
  `.handover/handoffs/latest.md` plus the next numbered history file, then
  waits up to 60 seconds before prompting the target terminal to read
  `latest.md` and continue. This is the default for day-to-day AI handoffs.
- `Git Commit`: creates a checkpoint commit when the worktree has changes, then
  prompts the target terminal to inspect the latest commit. Use this when you
  want team-facing history or a review checkpoint.

For summary handoffs, the user only needs to enter the goal or next direction.
The source AI writes the detailed transfer context because it has the active
conversation state. If the file is not created before the timeout, Handover
writes a fallback file with the user-provided goal and does not prompt the
target terminal.

## Python Backend Fallback

The old Python backend lives in `../handover_python`. To run the desktop app
against it:

```bash
cd ../handover_python
python -m venv venv
./venv/bin/pip install -r requirements.txt

cd ../handover_app
npm run dev:electron:python
```

## Build

```bash
npm run build:backend
npm run build
```

`build:backend` builds `../handover_rust/target/release/handover-backend`.
Electron-builder bundles that binary into the packaged app under
`resources/backend/handover-backend`.

Build output lands in `handover_app/release/`.

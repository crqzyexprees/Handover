# Handover

Multi-terminal AI CLI orchestration with Docker sandboxes and project handoffs.

**Current release:** v1.0.8

> **Version policy:** Never bump or rename the version automatically. Change `package.json`, `Cargo.toml`, `tauri.conf.json`, or the line above **only** when the owner explicitly asks. See [`VERSION_POLICY.md`](VERSION_POLICY.md).

| Directory | Role |
|-----------|------|
| [`handover_app/`](handover_app/) | Tauri 2 + React desktop app |
| [`handover_rust/`](handover_rust/) | Rust backend (`handover-backend`: REST API, WebSocket PTY, Docker) |
| [`handover/docker/`](handover/docker/) | Docker sandbox base image (`handover-base:latest`) |

## Terminal (troubleshooting)

The embedded terminal uses the same protocol as the original Python backend:

`xterm onData → WebSocket text → PTY → WebSocket text → xterm write`

On the frontend, `handover_app/src/terminalSession.js` owns this whole path:
one `TerminalSession` per terminal holds the xterm, the WebSocket, paste and
decoding, so every keystroke and paste goes through a single write path.

Shells outlive connections. The backend (`handover_rust/src/pty.rs`) keeps
each terminal's shell running when its window disconnects and buffers recent
output; on reconnect the window receives only what it missed, so a running
Claude / Codex / Cursor session is never restarted by a dropped connection.
The window reconnects automatically (up to 5 tries); ↻ forces a reattach,
or starts a new shell if the old one exited. A shell ends only when you
`exit` it, delete the terminal, or unload the project.

Shells also survive quitting the app. On quit, the app stops the backend only
if no shell is running; otherwise the backend keeps running and the next
launch reconnects to it (its port and token are in `backend.json` in the
app's local data folder, e.g. `~/.local/share/com.crqzyexprees.handover/` on Linux), and
your terminals reappear with their scrollback. A backend left running exits by
itself after 10 minutes with no shells and no app open. Its log is
`backend.log` in the same folder.

If typing fails after code changes, do a **full restart** (not hot reload):

```bash
pkill -f handover-backend; pkill -f tauri   # also ends any shells kept running
cd handover_app && bash scripts/prepare-backend-sidecar.sh debug && npm run dev:tauri
```

Open a **new** terminal tab after restarting. The old Python backend (`handover_python/`) was removed in the Tauri migration; this Rust backend replaces it with the same wire format.

## Architecture

```
┌─────────────────────────────────────────┐
│  handover_app (Tauri + React + xterm)   │
│  ├─ spawns handover-backend as sidecar  │
│  └─ dynamic localhost port → API + WS   │
└──────────────────┬──────────────────────┘
                   │ HTTP / WebSocket
┌──────────────────▼──────────────────────┐
│  handover_rust (Axum backend)           │
│  ├─ REST: projects, instances, handoff│
│  ├─ WS:   /ws/pty/{instance_id}       │
│  └─ Docker sandboxes + resource governor│
└─────────────────────────────────────────┘
```

The desktop app is an **npm workspace** monorepo — run all commands from the repo root.

## Quick Start

### 1. Install dependencies

```bash
npm install
```

### 2. Run the desktop app

```bash
npm run dev:tauri
```

`dev:tauri` builds the Rust backend, copies it into `handover_app/src-tauri/binaries/` as a Tauri sidecar, then launches the window. The backend listens on a free localhost port; the UI discovers it via `get_backend_port`.

**Linux system deps** (Fedora example):

```bash
sudo dnf install webkit2gtk4.1-devel openssl-devel curl wget file librsvg2-devel
```

### 3. Docker sandbox image (optional)

Required for containerized terminals. The image ships Claude Code, Codex, and
Cursor CLI (`claude`, `codex`, `cursor-agent` / `agent`). Host CLI updates do
**not** appear in Docker until you rebuild:

```bash
./handover/docker/build-base.sh
# or: npm run build:docker-base

# Force reinstall of the latest CLIs (skip Docker layer cache):
./handover/docker/build-base.sh --no-cache
```

Then open a **new** Docker terminal in Handover — existing containers keep the
old image.

`ANTHROPIC_API_KEY` / `OPENAI_API_KEY` (from the app's environment) and a
project's custom env vars reach the container through a read-only, owner-only
file that the shell sources at start, not the container environment, so they
don't show up in `docker inspect`. Details: [`handover/docker/README.md`](handover/docker/README.md).

## Features

- **Multi-terminal workspace** — native or Docker-backed PTY sessions per project
- **AI CLIs in Docker** — Claude Code, Codex, and Cursor CLI (`claude`, `codex`, `cursor-agent` / `agent`); rebuild `handover-base` to pick up host CLI updates
- **AI handoffs** — summary file or git-commit transfer between terminals
- **Handoff templates** — `generic`, `nextjs`, `rust-cli` (via `.handover/config.yml`)
- **Handoff history** — browse, compare diffs, export logs
- **Resource dashboard** — per-project CPU/RAM for Docker instances
- **Broadcast prompt** — send a prompt to all terminals in a project
- **Resource governor** — pauses containers when host RAM is high

## Distribution Build

Single command from the repo root:

```bash
npm run build:tauri
```

This builds icons, the release Rust binary, the sidecar bundle, and the Tauri AppImage. (`build:tauri` runs `build:backend` internally — no need to call it separately.)

Output: `handover_app/src-tauri/target/release/bundle/`

## Platform Build Notes

Build release packages on the target OS:

- Linux: build on Linux for AppImage output.
- macOS: build on macOS for `.app`/`.dmg` output. `npm run icons` generates
  a full `icon.icns` (16–1024px, 1x and @2x) from `icon-source.png`.
- Windows: build on Windows with the Rust MSVC toolchain and Visual Studio
  Build Tools installed. See [`WINDOWS_BUILD.md`](WINDOWS_BUILD.md) for the full guide.

Docker mode requires Docker Desktop on macOS and Windows.

For public macOS or Windows distribution, code signing is recommended to avoid
Gatekeeper, SmartScreen, and installer trust warnings.

## Browser-only dev

For UI work without the desktop shell:

```bash
npm run dev
```

Start the standalone backend (below), then open
`http://127.0.0.1:5173/?port=8765&token=<token>` using the token the backend
prints at startup. No native folder picker in the browser.

## Standalone backend

To run the Rust API without Tauri:

```bash
cd handover_rust
cargo run -- --host 127.0.0.1 --port 8765
```

Every API and WebSocket call needs the backend's token (`X-Handover-Token`
header, or `?token=` on WebSockets); only `GET /` is open. The desktop app
generates a fresh token per launch and passes it via `HANDOVER_TOKEN`. Run
standalone without it and the backend generates one and prints it. This stops
other web pages in your browser from reaching your terminals through
localhost; CORS is also limited to the app's own origins.

## Debug logging

```bash
HANDOVER_VERBOSE=1 RUST_LOG=info npm run dev:tauri
```

## Scripts (repo root)

| Script | Description |
|--------|-------------|
| `npm run dev:tauri` | Desktop dev (backend + Tauri + Vite) |
| `npm run build:tauri` | Release AppImage |
| `npm run build:backend` | Rust release binary only |
| `npm run build:docker-base` | Build `handover-base:latest` image |
| `npm run lint` | ESLint on frontend |
| `npm test` | Frontend unit tests (Vitest) |

Backend unit tests: `cd handover_rust && cargo test`. Live end-to-end checks
against a real backend, with native shells and Docker (needs Docker, the
`handover-base:latest` image and Python's `websockets`):
`handover_rust/e2e/run.sh`.

## License

Handover is open source under the [Apache License 2.0](LICENSE).

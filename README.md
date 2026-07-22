# Handover

Multi-terminal AI CLI orchestration with Docker sandboxes and project handoffs.

**Current release:** v1.0.6

> **Version policy:** Do not change version numbers in `package.json`, `Cargo.toml`, `tauri.conf.json`, or here unless explicitly requested. See [`VERSION_POLICY.md`](VERSION_POLICY.md).

| Directory | Role |
|-----------|------|
| [`handover_app/`](handover_app/) | Tauri 2 + React desktop app |
| [`handover_rust/`](handover_rust/) | Rust backend (`handover-backend`: REST API, WebSocket PTY, Docker) |
| [`handover/docker/`](handover/docker/) | Docker sandbox base image (`handover-base:latest`) |

## Terminal (troubleshooting)

The embedded terminal uses the same protocol as the original Python backend:

`xterm onData → WebSocket text → PTY → WebSocket text → xterm write`

If typing fails after code changes, do a **full restart** (not hot reload):

```bash
pkill -f handover-backend; pkill -f tauri
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

Required for containerized terminals:

```bash
./handover/docker/build-base.sh
# or: npm run build:docker-base
```

## Features

- **Multi-terminal workspace** — native or Docker-backed PTY sessions per project
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
- macOS: build on macOS for `.app`/`.dmg` output. The generated `icon.icns`
  is currently a placeholder and should be replaced with a real macOS `.icns`
  before public distribution.
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

Open `http://127.0.0.1:5173`. API calls default to port `8765` unless you pass `?port=` in the URL. No native folder picker in the browser.

## Standalone backend

To run the Rust API without Tauri:

```bash
cd handover_rust
cargo run -- --host 127.0.0.1 --port 8765
```

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

## License

Handover is open source under the [Apache License 2.0](LICENSE).

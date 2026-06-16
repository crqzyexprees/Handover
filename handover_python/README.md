# Handover (Electron UI)

Electron + React frontend with a **Python/FastAPI backend** (`backend/`). The Rust rewrite in [`../handover_rust/`](../handover_rust/) is experimental and optional.

A cross-platform GUI wrapper that orchestrates multiple AI coding CLIs (Claude, Codex, local models) within isolated Docker sandboxes. It enables seamless, sequential project handoffs between different AI agents using custom context preservation methods like git commits or summary files.

---

## Features

- **Multi-Project & Multi-Instance Tab Management** — Open several projects at once, each with multiple terminal instances, organized in a tabbed UI.
- **Docker Sandboxing** — Each terminal runs in an isolated Ubuntu container with strict memory limits, CPU limits, and a per-instance environment.
- **Native Host Mode** — Run a terminal directly on your host shell.
- **Seamless Authentication** — Local credentials auto-mounted into the sandbox.
- **Zero-Context-Loss Handoffs** — Git commits or sequential summary files.
- **Resource Governor** — Auto-pauses idle Docker containers when RAM is high.
- **First-launch Onboarding Wizard** — Docker health-check and global config.

---

## Project structure (this directory)

```
handover_python/
├── src/                 # Electron + React (main.cjs spawns Python backend)
├── backend/             # FastAPI backend (production)
│   ├── main.py
│   ├── requirements.txt
│   └── handover-backend.spec   # PyInstaller spec for distribution
├── index.html
├── package.json
└── vite.config.js
```

---

## Prerequisites

- Node.js 20+ & npm
- Python 3.11+ (for the backend)
- Docker (for sandboxed mode)
- AI CLI accounts on the host

---

## Installation & running (dev)

### 1. Python backend

```bash
cd backend
python -m venv venv
./venv/bin/pip install -r requirements.txt
```

### 2. Sandbox image (Docker mode)

```bash
docker build -f backend/docker/Dockerfile.base -t handover-base:latest backend/docker/
```

### 3. Frontend

```bash
# from handover_python/
npm install
npm run dev:electron
```

Electron waits for the backend health check at `http://127.0.0.1:<port>/` before opening the window.

Other scripts: `npm run dev`, `npm run build`, `npm run lint`.

### Optional: Rust backend (experimental)

```bash
cd ../handover_rust && cargo build
cd ../handover_python
HANDOVER_BACKEND=rust npm run dev:electron
```

---

## Building for distribution

Build the PyInstaller binary first, then the Electron app:

```bash
npm run build:backend   # backend/dist/handover-backend
npm run build           # Vite + electron-builder
```

Output lands in `release/`.

---

## Parent monorepo

See the root [`../README.md`](../README.md) for the full monorepo layout. The Rust rewrite lives in [`../handover_rust/`](../handover_rust/) as an optional future backend.

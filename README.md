# Handover

A cross-platform Electron + Python GUI wrapper that orchestrates multiple AI coding CLIs (Claude, Codex, local models) within isolated Docker sandboxes. It enables seamless, sequential project handoffs between different AI agents using custom context preservation methods like git commits or summary files. Designed for developers who want to leverage multiple CLI subscriptions and local models on a single project without context loss or system interference.

---

## Features

- **Multi-Project & Multi-Instance Tab Management** — Open several projects at once, each with multiple terminal instances, organized in a tabbed UI.
- **Docker Sandboxing** — Each terminal runs in an isolated Ubuntu container with strict memory limits, CPU limits, and a per-instance environment, so AI agents can't interfere with your host.
- **Native Host Mode** — Skip Docker entirely and run a terminal directly on your host shell, giving the AI access to every CLI and tool you already have installed.
- **Seamless Authentication** — Your local credentials (`~/.claude`, `~/.claude.json`, `~/.codex`, `~/.cursor`, …) are auto-mounted into the sandbox, so the CLIs start already logged in — no re-authentication.
- **Zero-Context-Loss Handoffs** — Switch from one agent to another (e.g. Claude → Codex) without losing state, using either **Git commits** or **sequential summary files**.
- **Resource Governor** — A background monitor auto-pauses idle, non-focused Docker containers when system RAM gets too high, protecting your machine from running out of memory.
- **First-launch Onboarding Wizard** — Health-checks Docker and saves global app configuration on first run.

---

## Architecture

```
┌──────────────────────────┐    HTTP + WebSocket     ┌──────────────────────────┐
│  Electron + React (UI)    │ ──────────────────────▶ │  FastAPI Backend          │
│  • tabs / project mgmt    │   REST:  /api/*         │  • REST endpoints         │
│  • xterm.js terminals     │   PTY:   /ws/pty/{id}   │  • WebSocket PTY bridge   │
└──────────────────────────┘ ◀────────────────────── │  • Resource Governor      │
                                  terminal I/O          └────────────┬─────────────┘
                                                                     │ pexpect (PTY)
                                                          ┌──────────┴───────────┐
                                                          │                      │
                                                  docker exec -it           host $SHELL
                                                   (Docker mode)            (Native mode)
                                                          │
                                                   Docker SDK
                                              (pause / resume / lifecycle)
```

- **Frontend** — An Electron desktop shell hosting a React + Vite app. Terminals are rendered with [xterm.js](https://xtermjs.org/); UI actions go through a small REST client (`src/api.js`).
- **Backend** — FastAPI exposes the REST API plus a WebSocket endpoint, `/ws/pty/{instance_id}`, that streams a live pseudo-terminal to the browser.
- **PTY bridge** — `pexpect` allocates a real PTY. In **Docker mode** it attaches via `docker exec -it <container> /bin/bash`; in **Native mode** it spawns your host shell (`$SHELL`) as a login shell so your `~/.bashrc`/`~/.zshrc` and full `PATH` are loaded.
- **Container lifecycle** — Managed through the Docker SDK in `backend/services/docker_runtime.py` (create, stop, pause, resume).
- **Shared `/workspace` volume** — Your project directory is bind-mounted to `/workspace` inside the container (read/write, SELinux-relabeled). The AI terminal and your host edit the **same files**, and containers run as your host UID so files stay owned by you, not `root`.

---

## Project Structure

```
.
├── src/                       # Frontend (Electron + React)
│   ├── main.cjs               # Electron main process — window + native folder-picker IPC
│   ├── preload.cjs            # contextBridge exposing a safe `electronAPI` to the renderer
│   ├── main.jsx               # React entry point
│   ├── App.jsx                # Root component: project/instance state + handoff orchestration
│   ├── Sidebar.jsx            # Project list (add / select / unload)
│   ├── TabBar.jsx             # Per-project instance (terminal) tabs
│   ├── TerminalView.jsx       # xterm.js terminal bound to the /ws/pty WebSocket
│   ├── HandoverModal.jsx      # Dialog to configure & trigger an AI-to-AI handoff
│   ├── ProjectSettings.jsx    # Per-project config (sandbox mode, memory limit, handoff method)
│   ├── useGovernorEvents.js   # Hook: surfaces Resource Governor / high-RAM signals to the UI
│   ├── projectUtils.js        # Shared helpers (ids, names, state/label normalization)
│   ├── api.js                 # Axios REST client for the backend
│   ├── index.css              # Tailwind styles
│   └── assets/                # Static images
│
├── backend/                   # Backend (FastAPI + Python)
│   ├── main.py                # FastAPI app: REST API, PTY WebSocket, Resource Governor, registries
│   ├── requirements.txt       # Python dependencies
│   ├── services/
│   │   ├── docker_runtime.py  # DockerRuntime: container lifecycle (start/stop/pause/resume) + auth mounting
│   │   └── handoff_engine.py  # git_handoff() and summary_handoff() implementations
│   └── docker/
│       └── Dockerfile.base    # The handover-base sandbox image (Ubuntu + Node + AI CLIs)
│
├── index.html                 # Vite HTML entry
├── vite.config.js             # Vite config (dev server on 127.0.0.1:5173)
├── package.json               # Frontend scripts & dependencies
├── eslint.config.js           # ESLint config
└── postcss.config.js          # Tailwind / PostCSS config
```

> Generated/local directories are omitted: `node_modules/` and `dist/` (frontend build output), and `backend/venv/` (Python virtualenv).

---

## Prerequisites

- **Node.js 20+ & npm** — for the Electron/React frontend.
- **Python 3.10+** with `venv` — for the FastAPI backend.
- **Docker** — required for Sandboxed mode (optional if you only use Native mode).
- **AI CLI accounts** — e.g. Claude, Codex, Cursor. Log in to each on your host once; Handover reuses those credentials.

---

## Installation & Running (Dev Mode)

The project is split into a frontend (repo root) and a Python backend (`backend/`).

### 1. Backend (FastAPI)

```bash
cd backend
python3 -m venv venv
source venv/bin/activate          # Windows: venv\Scripts\activate
pip install -r requirements.txt
```

> Backend dependencies (`backend/requirements.txt`): `fastapi`, `uvicorn[standard]`, `websockets`, `docker`, `pexpect`, `GitPython`, `pydantic`, `aiofiles`, `psutil`.

### 2. Build the sandbox image (for Docker mode)

The base sandbox image bundles Ubuntu 22.04 + Node.js and the supported AI CLIs (Claude Code, Codex, Cursor):

```bash
# from the repo root
docker build -f backend/docker/Dockerfile.base -t handover-base:latest backend/docker/
```

### 3. Frontend (Electron + React)

```bash
# from the repo root
npm install
```

### 4. Run it

Start the backend (serves on `http://127.0.0.1:8765`):

```bash
cd backend
source venv/bin/activate
python main.py
```

Then, in a second terminal, start the frontend (launches Vite on `:5173` and opens the Electron window):

```bash
# from the repo root
npm run dev:electron
```

Other frontend scripts (from `package.json`): `npm run dev` (Vite only, in a browser), `npm run build`, `npm run lint`, `npm run preview`.

---

## How Handoffs Work

A handoff captures the current state of a project so a **new** AI instance can pick up exactly where the previous one left off. When you trigger a handoff, Handover saves a checkpoint and then **injects a prompt directly into the new agent's terminal**, telling it where to find the context and what the overall goal is. Two methods are supported.

### Git Commit Method

The backend stages all changes in the project and creates a commit:

```
Handoff checkpoint: <your task description>
```

It then injects a prompt instructing the new AI to inspect the latest commit with `git log -1 -p` to understand exactly what changed, and to continue toward the stated goal. Best for projects that are already Git repositories — the diff itself is the context.

### Sequential Summary File Method

For projects where you don't want to commit (or aren't using Git), Handover maintains a **local "AI GitHub"** of context snapshots under `.handover/handoffs/`:

```
.handover/handoffs/
├── handoff_001.md      ← each handoff appends a new numbered, timestamped file
├── handoff_002.md
├── handoff_003.md
└── latest.md           ← always overwritten with the most recent snapshot
```

Each handoff writes a new numbered file (`handoff_001.md`, `handoff_002.md`, …) containing the task description and a timestamp, and overwrites `latest.md` with the same content. This keeps a full, sequential history without letting any single context file grow unbounded and waste tokens. The new AI is prompted to read `.handover/handoffs/latest.md` to see the current state, while the numbered files remain as an auditable trail of how the project evolved across agents.

---

## Resource Governor

When multiple sandboxed projects are open, a background task checks system RAM every 15 seconds:

- **Above 85%** — pauses (freezes) the containers of the least-recently-used, non-focused project and marks it `suspended`.
- **Above 95%** — emergency-pauses **all** non-focused projects.

The project you're actively working on is never suspended. Paused containers keep their memory but use no CPU, and clicking a suspended project transparently resumes (unpauses) it.

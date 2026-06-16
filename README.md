# Handover (monorepo)

Multi-terminal AI CLI orchestration with Docker sandboxes and project handoffs.

| Directory | Role |
|-----------|------|
| [`handover_python/`](handover_python/) | Electron + React UI (xterm.js terminals) |
| [`handover_rust/`](handover_rust/) | Rust backend used by the desktop app |

## Quick start

### 1. Rust backend

```bash
cd handover_rust
cargo build
```

### 2. Run the desktop app

```bash
cd handover_python
npm install
npm run dev:electron
```

Electron spawns the Rust backend automatically in development.

### 3. Docker sandbox image (optional)

```bash
docker build -f handover_python/backend/docker/Dockerfile.base -t handover-base:latest handover_python/backend/docker/
```

## Distribution build

Build the Rust backend binary, then the Electron app:

```bash
cd handover_python
npm run build:backend   # outputs ../handover_rust/target/release/handover-backend
npm run build           # Vite + electron-builder
```

Installers land in `handover_python/release/`.

## Python backend

The previous Python backend is still present under `handover_python/backend/` for reference. Use `npm run dev:electron:python` or `npm run build:backend:python` from `handover_python/` if you need it.

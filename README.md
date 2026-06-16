# Handover

Multi-terminal AI CLI orchestration with Docker sandboxes and project handoffs.

| Directory | Role |
|-----------|------|
| [`handover_app/`](handover_app/) | Electron + React desktop app |
| [`handover_rust/`](handover_rust/) | Active Rust backend |
| [`handover_python/`](handover_python/) | Legacy Python/FastAPI backend only |

## Quick Start

### 1. Build the Rust backend

```bash
cd handover_rust
cargo build
```

### 2. Run the desktop app

```bash
cd ../handover_app
npm install
npm run dev:electron
```

The desktop app starts the Rust backend automatically on a free localhost port.

### 3. Docker sandbox image

```bash
docker build -f handover_python/docker/Dockerfile.base -t handover-base:latest handover_python/docker/
```

## Distribution Build

```bash
cd handover_app
npm run build:backend
npm run build
```

The packaged app is written to `handover_app/release/` and bundles
`handover_rust/target/release/handover-backend`.

## Python Backend Fallback

The old backend is kept in `handover_python/` for reference or fallback:

```bash
cd handover_python
python -m venv venv
./venv/bin/pip install -r requirements.txt

cd ../handover_app
npm run dev:electron:python
```

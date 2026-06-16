# Handover Python Backend

Legacy Python/FastAPI backend retained for reference and fallback.

The desktop app no longer lives here. Use `../handover_app` for the Electron +
React app and `../handover_rust` for the active Rust backend.

## Setup

```bash
python -m venv venv
./venv/bin/pip install -r requirements.txt
```

## Run

```bash
./venv/bin/python -m uvicorn main:app --host 127.0.0.1 --port 8765
```

## Docker Sandbox Image

```bash
docker build -f docker/Dockerfile.base -t handover-base:latest docker/
```

## Build Old PyInstaller Backend

```bash
./venv/bin/pyinstaller handover-backend.spec
```

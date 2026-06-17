# handover_rust

Rust backend for Handover.

## Features

- REST API matching the desktop UI (`/api/projects`, `/api/instances`, `/api/handoff`, …)
- WebSocket PTY bridge (`/ws/pty/{instance_id}`)
- Docker sandbox lifecycle via [bollard](https://github.com/fussybeaver/bollard) (`handover-base:latest` from `../handover/docker/`)
- Git + summary handoffs
- Resource governor (pauses containers when RAM is high)

## Build

```bash
cd handover_rust
cargo build --release
```

Binary: `target/release/handover-backend`

## Run (standalone)

```bash
cargo run -- --host 127.0.0.1 --port 8765
```

## Tauri integration

`handover_app` spawns this binary as a Tauri **sidecar**:

- **Dev:** `src-tauri/binaries/handover-backend-<target-triple>` (via `scripts/prepare-backend-sidecar.sh`)
- **Release:** bundled as `externalBin` in the AppImage

Build the release binary before `npm run build:tauri`:

```bash
cd handover_rust && cargo build --release
cd ../handover_app && npm run build:tauri
```

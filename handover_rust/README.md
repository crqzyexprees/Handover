# handover_rust

Rust backend for Handover — replaces the Python FastAPI server.

## Features

- REST API matching the Electron UI (`/api/projects`, `/api/instances`, `/api/handoff`, …)
- WebSocket PTY bridge (`/ws/pty/{instance_id}`)
- Docker sandbox lifecycle via [bollard](https://github.com/fussybeaver/bollard)
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

## Electron integration

`handover_python` spawns this binary automatically:

- **Dev:** `target/debug/handover-backend`, or `cargo run` if not built yet
- **Packaged:** `extraResources` bundles `target/release/handover-backend`

Build the release binary before `npm run build` in `handover_python`:

```bash
cd handover_rust && cargo build --release
cd ../handover_python && npm run build
```

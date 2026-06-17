# Handover App

Tauri 2 + React desktop app for Handover.

The UI talks to the Rust backend in `../handover_rust`, which Tauri starts as a **sidecar** process (`handover-backend`) on a dynamic localhost port.

## Development

From the **repo root**:

```bash
npm install
npm run dev:tauri
```

`dev:tauri` (via `scripts/dev-tauri.sh`):

1. Builds `handover-backend` (debug)
2. Copies it to `src-tauri/binaries/handover-backend-<target-triple>`
3. Generates placeholder icons if missing
4. Runs `tauri dev`

## Project layout

| Path | Purpose |
|------|---------|
| `src/` | React UI (terminals, handoffs, settings) |
| `src/platform.js` | Tauri dialog + backend port/URL helpers |
| `src-tauri/` | Tauri shell (sidecar spawn, `get_backend_port`) |
| `scripts/` | Sidecar prep, icons, dev wrapper |

## Handoff workflow

Open a project, start at least two terminals, and run your AI CLI sessions. The handoff modal supports:

- **Summary File** — source AI writes `.handover/handoffs/latest.md`; backend waits up to 60s, then prompts the target.
- **Git Commit** — checkpoint commit, then prompts the target to inspect history.

Configure templates (`generic`, `nextjs`, `rust-cli`) in `.handover/config.yml` via Project Settings.

## Docker sandbox image

```bash
../handover/docker/build-base.sh
# or from repo root: npm run build:docker-base
```

## Build AppImage

From the repo root:

```bash
npm run build:tauri
```

Builds icons → release backend → sidecar bundle → AppImage. Output under `src-tauri/target/release/bundle/`.

## Browser-only dev

```bash
npm run dev
```

Open `http://127.0.0.1:5173`. API defaults to port `8765` unless you pass `?port=`. No native folder picker.

Start the backend separately for full functionality:

```bash
cd ../handover_rust && cargo run -- --host 127.0.0.1 --port 8765
```

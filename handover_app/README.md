# Handover App

Electron + React desktop app for Handover.

This package owns the desktop shell, React UI, xterm.js terminals, Vite build,
and Electron packaging. The active backend is the Rust service in
`../handover_rust`.

## Development

```bash
npm install
npm run dev:electron
```

`npm run dev:electron` starts Vite, then launches Electron with
`HANDOVER_BACKEND=rust`. Electron starts the Rust backend on a free localhost
port and passes that port to the renderer.

## Python Backend Fallback

The old Python backend lives in `../handover_python`. To run the desktop app
against it:

```bash
cd ../handover_python
python -m venv venv
./venv/bin/pip install -r requirements.txt

cd ../handover_app
npm run dev:electron:python
```

## Build

```bash
npm run build:backend
npm run build
```

`build:backend` builds `../handover_rust/target/release/handover-backend`.
Electron-builder bundles that binary into the packaged app under
`resources/backend/handover-backend`.

Build output lands in `handover_app/release/`.

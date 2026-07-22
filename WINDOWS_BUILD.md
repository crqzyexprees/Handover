# Handover — Windows Build Guide

This document explains how to build, run, and distribute **Handover** on Windows.

Handover is a **Tauri 2 + React** desktop app with a **Rust backend sidecar** (`handover-backend`). The Linux AppImage (**v1.0.6**) is tested and released. Windows packaging is supported by Tauri, but **Windows-specific terminal behavior still needs validation and likely small backend changes**.

---

## Table of contents

1. [What you are building](#what-you-are-building)
2. [Requirements](#requirements)
3. [Quick start (dev on Windows)](#quick-start-dev-on-windows)
4. [Release build (installer / exe)](#release-build-installer--exe)
5. [Where files land locally](#where-files-land-locally)
6. [How the Windows app is structured](#how-the-windows-app-is-structured)
7. [Windows-specific behavior to know](#windows-specific-behavior-to-know)
8. [Docker mode on Windows](#docker-mode-on-windows)
9. [Native terminal mode on Windows](#native-terminal-mode-on-windows)
9. [Icons and branding](#icons-and-branding)
10. [Code signing (recommended for public release)](#code-signing-recommended-for-public-release)
11. [GitHub Actions (recommended for releases)](#github-actions-recommended-for-releases)
12. [Testing checklist](#testing-checklist)
13. [Troubleshooting](#troubleshooting)
14. [Suggested next engineering tasks](#suggested-next-engineering-tasks)

---

## What you are building

| Piece | Role on Windows |
|-------|-----------------|
| `handover_app/` | Tauri shell + React UI |
| `handover_rust/` | Backend sidecar: REST API, WebSocket PTY, Docker |
| `handover_app/src-tauri/binaries/` | Bundled `handover-backend-x86_64-pc-windows-msvc.exe` |
| `handover/docker/` | Docker sandbox image (`handover-base:latest`) |

**Current release target:** v1.0.6

**Linux today:** `Handover_1.0.6_amd64.AppImage`

**Windows goal:** `.msi` and/or `.exe` under Tauri’s `bundle/` output directory.

---

## Requirements

Build **on Windows** for Windows. Cross-compiling from Linux is possible in theory, but Tauri + WebView2 + MSVC sidecars are much simpler on a real Windows machine (or a Windows CI runner).

### 1. Windows version

- **Windows 10** (64-bit) or **Windows 11**
- 64-bit (`x86_64`) is the primary target today

### 2. Node.js

Install current **LTS** Node.js (18+ recommended).

```powershell
node -v
npm -v
```

### 3. Rust (MSVC toolchain)

```powershell
winget install Rustlang.Rustup
rustup default stable
rustup target add x86_64-pc-windows-msvc
rustc -V
cargo -V
```

### 4. Visual Studio Build Tools

Install **Visual Studio Build Tools 2022** (or full Visual Studio) with:

- **Desktop development with C++**
- **MSVC v143** (or latest) build tools
- **Windows 10/11 SDK**

Tauri on Windows uses the **MSVC** toolchain, not GNU/mingw.

Open **“x64 Native Tools Command Prompt for VS 2022”** or ensure `cl.exe` is on your `PATH` before building if you hit linker errors.

### 5. WebView2 Runtime

Tauri uses **Microsoft Edge WebView2**.

- Usually preinstalled on Windows 11
- On Windows 10, install if missing: [WebView2 Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)

### 6. Git

```powershell
winget install Git.Git
git --version
```

### 7. Python 3 (icon generation)

Used by `handover_app/scripts/generate-icons.py` during build.

```powershell
python --version
```

### 8. Docker Desktop (optional but important)

Required for **Docker sandbox terminals**.

- Install [Docker Desktop for Windows](https://www.docker.com/products/docker-desktop/)
- Enable WSL2 backend (recommended)
- Build the base image once (see [Docker mode on Windows](#docker-mode-on-windows))

---

## Quick start (dev on Windows)

### 1. Clone the repo

```powershell
git clone https://github.com/crqzyexprees/Handover.git
cd Handover
```

### 2. Install JS dependencies

From the **repo root** (npm workspace):

```powershell
npm install
```

### 3. Build the Rust backend (debug)

```powershell
cd handover_rust
cargo build
cd ..
```

### 4. Prepare the Tauri sidecar

```powershell
cd handover_app
node scripts/prepare-backend-sidecar.js debug
```

Expected output:

```text
Sidecar ready: handover_app\src-tauri\binaries\handover-backend-x86_64-pc-windows-msvc.exe
```

The sidecar script already supports Windows (`win32` → `.exe`, `x86_64-pc-windows-msvc`).

### 5. Generate icons (first time)

```powershell
npm run icons
```

### 6. Run the desktop app in dev mode

The existing dev script is bash-oriented (`scripts/dev-tauri.sh`). On Windows, run the equivalent steps manually:

```powershell
cd handover_app
npx tauri dev
```

Or from repo root after backend + sidecar are ready:

```powershell
npm run dev -w handover
# in another terminal, from handover_app:
npx tauri dev
```

**Tip:** Adding a `dev:tauri:win` PowerShell script (see [Suggested next engineering tasks](#suggested-next-engineering-tasks)) will make this one command.

---

## Release build (installer / exe)

There is currently a Linux-focused script: `handover_app/scripts/build-tauri-appimage.sh`.

On Windows, run the **generic Tauri release pipeline** instead:

### From repo root

```powershell
cd Handover
npm install
```

### Build release backend + sidecar + frontend + Tauri bundle

```powershell
cd handover_app

npm run icons
npm run build:backend
node scripts/prepare-backend-sidecar.js release
npm run build
npx tauri build
```

Or as a one-liner from `handover_app/`:

```powershell
npm run icons; npm run build:backend; node scripts/prepare-backend-sidecar.js release; npm run build; npx tauri build
```

### What Tauri produces

With `"targets": "all"` in `handover_app/src-tauri/tauri.conf.json`, Tauri typically emits:

| Artifact | Purpose |
|----------|---------|
| `.msi` | Windows installer (recommended for most users) |
| `.exe` | NSIS-style setup (if enabled in Tauri bundle config) |
| `handover-desktop.exe` | Raw app binary (under `target/release/`) |

Exact outputs depend on Tauri 2 bundle settings and installed WiX/NSIS tooling.

---

## Where files land locally

After `npx tauri build`, look here:

```text
handover_app\src-tauri\target\release\
  handover-desktop.exe

handover_app\src-tauri\target\release\bundle\
  msi\Handover_1.0.6_x64_en-US.msi          (name may vary)
  nsis\Handover_1.0.6_x64-setup.exe         (if NSIS enabled)
```

Sidecar copied before bundling:

```text
handover_app\src-tauri\binaries\
  handover-backend-x86_64-pc-windows-msvc.exe
```

Rust backend build output:

```text
handover_rust\target\release\
  handover-backend.exe
```

---

## How the Windows app is structured

```text
┌──────────────────────────────────────────────┐
│  Handover.exe (Tauri + WebView2 + React UI)  │
│  ├─ spawns handover-backend sidecar (.exe)  │
│  └─ discovers localhost port dynamically     │
└──────────────────┬───────────────────────────┘
                   │ HTTP + WebSocket
┌──────────────────▼───────────────────────────┐
│  handover-backend.exe (Axum)                 │
│  ├─ REST: projects, instances, handoffs      │
│  ├─ WS:   /ws/pty/{instance_id}            │
│  └─ Docker sandboxes (optional)              │
└──────────────────────────────────────────────┘
```

Relevant config: `handover_app/src-tauri/tauri.conf.json`

```json
"bundle": {
  "active": true,
  "targets": "all",
  "externalBin": ["binaries/handover-backend"]
}
```

Tauri bundles the sidecar next to the app and starts it at runtime.

---

## Windows-specific behavior to know

### What already works without changes

- Tauri app shell (window, tray, file dialogs via `@tauri-apps/plugin-dialog`)
- Sidecar packaging script (`prepare-backend-sidecar.js`) for Windows
- REST API, project persistence, handoffs UI
- Docker mode **if Docker Desktop is installed and the base image is built**
- Terminal input bridge (`handover_app/src/ptyBridge.js`) — frontend is OS-agnostic

### What still needs Windows validation

The Rust PTY layer (`handover_rust/src/pty.rs`) was written and tested primarily on Linux:

```rust
// Native mode today (Linux-oriented)
let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
c.args(["-l", "-i"]);

// Docker mode
docker exec -it <container> /bin/bash
```

On Windows:

| Mode | Expected status |
|------|-----------------|
| **Docker** | Likely works first (Docker Desktop + Linux containers) |
| **Native** | Likely needs code changes (PowerShell/cmd/WSL/Git Bash selection) |

`portable-pty` supports Windows **ConPTY**, but the **spawned shell command** must be Windows-appropriate.

---

## Docker mode on Windows

### Prerequisites

1. Docker Desktop running
2. WSL2 backend enabled (recommended)
3. Base image built

### Build the sandbox image

From Git Bash, WSL, or any shell with Docker available:

```powershell
# From repo root — script is bash; use Git Bash/WSL, or run docker build manually
npm run build:docker-base
```

Or manually:

```powershell
docker build -t handover-base:latest handover/docker
```

### Verify Docker from backend

```powershell
cd handover_rust
cargo run -- --host 127.0.0.1 --port 8765
```

Then open the app and start a **Docker** terminal tab.

---

## Native terminal mode on Windows

Native mode is **not guaranteed to work yet** without backend changes.

Today the backend assumes a Unix shell:

- `$SHELL` or `/bin/bash`
- flags `-l -i`

On Windows you typically want one of:

| Shell | When to use |
|-------|-------------|
| `powershell.exe` | Default Windows experience |
| `cmd.exe` | Minimal fallback |
| Git Bash | If user has Git for Windows |
| WSL `bash` | If WSL is installed |

### Recommended backend change (future)

In `handover_rust/src/pty.rs`, branch on `cfg!(windows)`:

```rust
#[cfg(windows)]
{
    // spawn powershell.exe or detect WSL/Git Bash
}

#[cfg(unix)]
{
    // existing bash -l -i logic
}
```

Test native mode only **after** this work.

---

## Icons and branding

Icons are generated from:

```text
handover_app/src-tauri/icons/icon-source.png
```

Command:

```powershell
cd handover_app
npm run icons
```

`tauri.conf.json` already references:

- `icons/icon.ico` (Windows)
- `icons/icon.icns` (macOS)
- PNG sizes for Linux

Replace `icon-source.png` with a proper brand asset before public Windows release.

---

## Code signing (recommended for public release)

Unsigned Windows builds often trigger **SmartScreen** warnings (“Windows protected your PC”).

For public distribution, plan for:

| Item | Purpose |
|------|---------|
| Authenticode code signing certificate | Sign `.exe` / `.msi` |
| Timestamp server | Signatures remain valid after cert expiry |
| EV certificate (optional) | Faster SmartScreen reputation |

Tauri docs cover signing for v2 bundles. Budget and setup time are non-trivial — fine for personal use unsigned, not ideal for wide public download.

---

## GitHub Actions (recommended for releases)

Today Linux AppImage is built locally. For Windows (and future macOS), add a workflow like:

```yaml
name: Release

on:
  push:
    tags:
      - 'v*'

jobs:
  build-windows:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4

      - uses: actions/setup-node@v4
        with:
          node-version: 20

      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: x86_64-pc-windows-msvc

      - name: Install frontend deps
        run: npm install

      - name: Build Handover (Windows)
        run: |
          cd handover_app
          npm run icons
          npm run build:backend
          node scripts/prepare-backend-sidecar.js release
          npm run build
          npx tauri build

      - name: Upload Windows artifacts
        uses: actions/upload-artifact@v4
        with:
          name: handover-windows
          path: |
            handover_app/src-tauri/target/release/bundle/msi/*.msi
            handover_app/src-tauri/target/release/bundle/nsis/*.exe
```

Attach artifacts to GitHub Releases the same way as the Linux AppImage.

---

## Testing checklist

Before calling Windows “ready”:

### App shell

- [ ] App launches without WebView2 errors
- [ ] Backend sidecar starts (status/connectivity in UI)
- [ ] Folder picker works for adding projects
- [ ] Window controls / custom title bar work

### Terminal — Docker mode

- [ ] Docker Desktop detected
- [ ] Container starts for a project
- [ ] Embedded terminal connects (`Connected`)
- [ ] Typing `abab` shows exactly `abab` (no duplication)
- [ ] Backspace, arrows, resize behave correctly

### Terminal — native mode

- [ ] Shell spawns on Windows (after PTY patch)
- [ ] Same keystroke tests as above

### Handoffs / extras

- [ ] Handoff summary write/read
- [ ] Resource dashboard loads
- [ ] Broadcast prompt sends to all tabs

### Installer

- [ ] `.msi` or setup `.exe` installs cleanly
- [ ] Installed app starts from Start Menu
- [ ] Uninstall removes app cleanly

---

## Troubleshooting

### `link.exe` not found / MSVC errors

Install Visual Studio Build Tools with C++ workload, then reopen the terminal or use **x64 Native Tools Command Prompt**.

### `Missing backend binary`

```powershell
cd handover_rust
cargo build --release
cd ..\handover_app
node scripts/prepare-backend-sidecar.js release
```

### WebView2 missing

Install the WebView2 Runtime and restart.

### Docker mode fails

- Confirm Docker Desktop is running
- Confirm `docker ps` works
- Build `handover-base:latest`
- Check Windows path sharing for project folders mounted into containers

### Native terminal fails immediately

Expected until `pty.rs` gets Windows shell spawning. Use **Docker mode** for now.

### SmartScreen warning

Normal for unsigned builds. Sign the installer for production.

### `npm run build:tauri` fails on Windows

That script is **Linux AppImage-specific** (bash + appimagetool). On Windows use the [Release build](#release-build-installer--exe) steps with `npx tauri build` instead.

---

## Suggested next engineering tasks

To make Windows a first-class platform:

1. **Add `scripts/build-tauri-windows.ps1`**
   - Mirror the Linux script: icons → backend → sidecar → vite build → `tauri build`

2. **Add `scripts/dev-tauri.ps1`**
   - Windows equivalent of `dev-tauri.sh`

3. **Patch `handover_rust/src/pty.rs` for Windows native shells**
   - Use ConPTY via `portable-pty`
   - Spawn PowerShell (or configurable shell)

4. **Add GitHub Actions Windows job**
   - Build and attach `.msi` / `.exe` to releases

5. **Test Docker + terminal on Windows 10 and 11**

6. **Code signing** before public Windows download page

---

## Version policy

Do not bump version numbers unless explicitly requested. Current release line: **v1.0.6**.

See [`VERSION_POLICY.md`](VERSION_POLICY.md).

---

## Quick reference commands (Windows)

```powershell
# Dev
cd Handover
npm install
cd handover_rust && cargo build && cd ..\handover_app
node scripts/prepare-backend-sidecar.js debug
npx tauri dev

# Release
cd handover_app
npm run icons
npm run build:backend
node scripts/prepare-backend-sidecar.js release
npm run build
npx tauri build
```

---

## Related docs

- [`README.md`](README.md) — main project overview
- [`VERSION_POLICY.md`](VERSION_POLICY.md) — version bump policy
- [Tauri v2 — Windows setup](https://v2.tauri.app/start/prerequisites/#windows)
- [Tauri v2 — Windows installer](https://v2.tauri.app/distribute/windows-installer/)

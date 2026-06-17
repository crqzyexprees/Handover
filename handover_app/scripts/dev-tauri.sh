#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT/../handover_rust"
cargo build
cd "$ROOT"
if [[ ! -f src-tauri/icons/icon.png ]]; then
  python3 scripts/generate-icons.py
fi
bash scripts/prepare-backend-sidecar.sh debug
exec npx tauri dev

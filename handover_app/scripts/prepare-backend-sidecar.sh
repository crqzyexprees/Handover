#!/usr/bin/env bash
# Copy handover-backend into src-tauri/binaries/ for Tauri sidecar (dev + release).
set -euo pipefail

APP_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${1:-debug}"
TARGET="$(rustc -vV | sed -n 's/^host: //p')"
SRC="$APP_ROOT/../handover_rust/target/$PROFILE/handover-backend"
DEST_DIR="$APP_ROOT/src-tauri/binaries"
DEST="$DEST_DIR/handover-backend-$TARGET"

if [[ ! -x "$SRC" ]]; then
  echo "Missing backend binary: $SRC" >&2
  echo "Run: cd handover_rust && cargo build${PROFILE/release/ --release}" >&2
  exit 1
fi

mkdir -p "$DEST_DIR"
cp -f "$SRC" "$DEST"
chmod +x "$DEST"
echo "Sidecar ready: $DEST"

#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAURI_DIR="$ROOT_DIR/src-tauri"
APPDIR="$TAURI_DIR/target/release/bundle/appimage/Handover.AppDir"
BUNDLE_DIR="$TAURI_DIR/target/release/bundle/appimage"
VERSION="$(node -p "require('./package.json').version")"
APPIMAGE_NAME="Handover_${VERSION}_amd64.AppImage"
APPIMAGE_PATH="$BUNDLE_DIR/$APPIMAGE_NAME"

npm run icons
npm run build:backend
node scripts/prepare-backend-sidecar.js release

if tauri build; then
  exit 0
fi

if [[ ! -x "$APPDIR/usr/bin/handover-desktop" ]]; then
  echo "Tauri build failed before creating a usable AppDir." >&2
  exit 1
fi

PLUGIN="${HOME}/.cache/tauri/linuxdeploy-plugin-appimage.AppImage"
if [[ ! -x "$PLUGIN" ]]; then
  echo "AppImage plugin not found at $PLUGIN." >&2
  exit 1
fi

mkdir -p "$BUNDLE_DIR"
cp "$APPDIR/Handover.png" "$APPDIR/handover-desktop.png"
(
  cd "$BUNDLE_DIR"
  LINUXDEPLOY_OUTPUT_VERSION="$VERSION" \
    LDAI_OUTPUT="$APPIMAGE_NAME" \
    "$PLUGIN" --appdir="$APPDIR"
)

echo "AppImage ready: $APPIMAGE_PATH"

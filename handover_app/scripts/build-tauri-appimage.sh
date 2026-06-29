#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAURI_DIR="$ROOT_DIR/src-tauri"
# Cursor/sandbox may set CARGO_TARGET_DIR to a cache outside the repo; always build in-tree.
unset CARGO_TARGET_DIR
export CARGO_TARGET_DIR="$TAURI_DIR/target"
export APPIMAGE_EXTRACT_AND_RUN=1
APPDIR="$TAURI_DIR/target/release/bundle/appimage/Handover.AppDir"
BUNDLE_DIR="$TAURI_DIR/target/release/bundle/appimage"
ICON_DIR="$TAURI_DIR/icons"
VERSION="$(node -p "require('./package.json').version")"
APPIMAGE_NAME="Handover_${VERSION}_amd64.AppImage"
APPIMAGE_PATH="$BUNDLE_DIR/$APPIMAGE_NAME"
RUNTIME_FILE="$TAURI_DIR/target/appimage-runtime-x86_64"
RUNTIME_URL="https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64"

sync_appdir_icons() {
  [[ -d "$APPDIR" ]] || return 0
  cp "$ICON_DIR/icon.png" "$APPDIR/Handover.png"
  cp "$ICON_DIR/icon.png" "$APPDIR/handover-desktop.png"
  mkdir -p \
    "$APPDIR/usr/share/icons/hicolor/32x32/apps" \
    "$APPDIR/usr/share/icons/hicolor/128x128/apps" \
    "$APPDIR/usr/share/icons/hicolor/256x256@2/apps" \
    "$APPDIR/usr/share/icons/hicolor/512x512/apps"
  cp "$ICON_DIR/32x32.png" "$APPDIR/usr/share/icons/hicolor/32x32/apps/handover-desktop.png"
  cp "$ICON_DIR/128x128.png" "$APPDIR/usr/share/icons/hicolor/128x128/apps/handover-desktop.png"
  cp "$ICON_DIR/128x128@2x.png" "$APPDIR/usr/share/icons/hicolor/256x256@2/apps/handover-desktop.png"
  cp "$ICON_DIR/icon.png" "$APPDIR/usr/share/icons/hicolor/512x512/apps/handover-desktop.png"
  ln -sf Handover.png "$APPDIR/.DirIcon"
}

npm run icons
npm run build:backend
node scripts/prepare-backend-sidecar.js release

# Force re-link so embedded window icons match icon-source.png
cargo build --release --manifest-path "$TAURI_DIR/Cargo.toml"

set +e
tauri build
TAURI_STATUS=$?
set -e

if [[ "$TAURI_STATUS" -eq 0 ]]; then
  sync_appdir_icons
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
APPIMAGETOOL="$(find /tmp -path '*/usr/bin/appimagetool' -type f -executable 2>/dev/null | head -n 1 || true)"
if [[ -z "$APPIMAGETOOL" ]]; then
  EXTRACT_DIR="$TAURI_DIR/target/appimage-plugin-extract"
  rm -rf "$EXTRACT_DIR"
  mkdir -p "$EXTRACT_DIR"
  (
    cd "$EXTRACT_DIR"
    "$PLUGIN" --appimage-extract >/dev/null
  )
  APPIMAGETOOL="$EXTRACT_DIR/squashfs-root/usr/bin/appimagetool"
fi
if [[ ! -x "$APPIMAGETOOL" ]]; then
  echo "appimagetool not found in extracted plugin." >&2
  exit 1
fi

sync_appdir_icons
if [[ ! -s "$RUNTIME_FILE" ]]; then
  curl -L --fail --output "$RUNTIME_FILE" "$RUNTIME_URL"
fi
mkdir -p "$APPDIR/usr/bin"
cp "$TAURI_DIR/target/release/handover-desktop" "$APPDIR/usr/bin/handover-desktop"
chmod +x "$APPDIR/usr/bin/handover-desktop"
mkdir -p "$BUNDLE_DIR"
ARCH=x86_64 "$APPIMAGETOOL" --runtime-file "$RUNTIME_FILE" "$APPDIR" "$APPIMAGE_PATH"

echo "AppImage ready: $APPIMAGE_PATH"

#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

echo "=== git status ==="
git status -sb

echo "=== git diff --stat ==="
git diff --stat

echo "=== staging ==="
git add -A

if git diff --cached --quiet; then
  echo "Nothing to commit."
else
  git commit -m "$(cat <<'EOF'
Migrate desktop app from Electron to Tauri 2 with Rust sidecar.

Remove Python backend, add Tauri shell with dynamic port injection, handoff templates/history, resource dashboard, and broadcast prompts.
EOF
)"
fi

echo "=== pushing ==="
git push -u origin HEAD

echo "=== done ==="
git status -sb
git log -1 --oneline

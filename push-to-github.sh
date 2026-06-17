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
Remove remaining Electron and Python backend references.

EOF
)"
fi

echo "=== pushing ==="
git push -u origin HEAD

echo "=== done ==="
git status -sb
git log -1 --oneline

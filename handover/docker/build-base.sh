#!/usr/bin/env bash
# Build the Handover sandbox base image. Safe to run from any working directory.
# Pass --no-cache to force reinstall of Claude / Codex / Cursor CLI packages.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

exec docker build \
  "$@" \
  -f "${SCRIPT_DIR}/Dockerfile.base" \
  -t handover-base:latest \
  "${SCRIPT_DIR}"

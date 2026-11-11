#!/usr/bin/env bash
# Live end-to-end checks against a real backend (native shells and Docker).
# Needs: Docker running, the handover-base:latest image, python3 with the
# `websockets` package. Usage: handover_rust/e2e/run.sh
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
cd "$here/.."
cargo build -q || exit 1
bin="$PWD/target/debug/handover-backend"
work="$(mktemp -d)"
export HANDOVER_TOKEN="e2e-$RANDOM$RANDOM"
port=18765
HANDOVER_STATE_PATH="$work/state.json" "$bin" --host 127.0.0.1 --port "$port" \
  >"$work/backend.log" 2>&1 &
backend=$!
trap 'kill $backend 2>/dev/null; wait $backend 2>/dev/null; rm -rf "$work"' EXIT
for _ in $(seq 1 50); do curl -sf "http://127.0.0.1:$port/" >/dev/null && break; sleep 0.1; done

failed=0
python3 "$here/e2e_basic.py" "$port" "$work" || failed=1
for mode in native docker; do
  python3 "$here/smoke_persist.py" "$port" "$work" "$mode" || failed=1
done
python3 "$here/lifecycle.py" "$bin" "$work" || failed=1
[ "$failed" = 0 ] && echo "e2e: all passed" || echo "e2e: FAILURES (backend log: $work/backend.log)"
exit "$failed"

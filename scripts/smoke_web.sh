#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PORT="${AUTOCODING_SMOKE_PORT:-18080}"
BIN="${ROOT_DIR}/target/release/kacf"

if [[ ! -x "$BIN" ]]; then
  echo "[smoke-web] release binary missing, building..."
  cargo build --release
fi

LOG_FILE="$(mktemp -t kacf-smoke.XXXXXX.log)"
cleanup() {
  if [[ -n "${SERVER_PID:-}" ]] && kill -0 "$SERVER_PID" >/dev/null 2>&1; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  rm -f "$LOG_FILE"
}
trap cleanup EXIT

echo "[smoke-web] starting server on port ${PORT}"
AUTOCODING_PORT="$PORT" "$BIN" >"$LOG_FILE" 2>&1 &
SERVER_PID=$!

sleep 0.4
if ! kill -0 "$SERVER_PID" >/dev/null 2>&1; then
  if grep -qi "operation not permitted" "$LOG_FILE"; then
    echo "[smoke-web] skipped: sandbox blocks binding local ports"
    exit 0
  fi
  echo "[smoke-web] server exited early"
  cat "$LOG_FILE"
  exit 1
fi

for _ in $(seq 1 40); do
  if curl -fsS "http://127.0.0.1:${PORT}/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done

echo "[smoke-web] checking /health"
if ! curl -fsS "http://127.0.0.1:${PORT}/health" | grep -q "\"ok\":true"; then
  if grep -qi "operation not permitted" "$LOG_FILE"; then
    echo "[smoke-web] skipped: sandbox blocks binding local ports"
    exit 0
  fi
  echo "[smoke-web] /health failed"
  cat "$LOG_FILE"
  exit 1
fi
echo "[smoke-web] checking /ui_state"
curl -fsS "http://127.0.0.1:${PORT}/ui_state" | grep -q "\"runtime\""
echo "[smoke-web] checking /metrics"
curl -fsS "http://127.0.0.1:${PORT}/metrics" | grep -q "\"total_events\""
curl -fsS "http://127.0.0.1:${PORT}/metrics" | grep -q "\"readiness\""

echo "[smoke-web] success"

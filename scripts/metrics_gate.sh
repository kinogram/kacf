#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PORT="${AUTOCODING_SMOKE_PORT:-18080}"
BIN="${ROOT_DIR}/target/release/kacf"
MIN_SCORE="${AUTOCODING_GATE_MIN_SCORE:-}"
ALLOW_RUNNING="${AUTOCODING_GATE_ALLOW_RUNNING:-0}"
ALLOW_BLOCKERS="${AUTOCODING_GATE_ALLOW_BLOCKERS:-0}"
JSON_MODE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --min-score)
      MIN_SCORE="${2:-}"
      shift 2
      ;;
    --allow-running)
      ALLOW_RUNNING=1
      shift
      ;;
    --allow-blockers)
      ALLOW_BLOCKERS=1
      shift
      ;;
    --json)
      JSON_MODE=1
      shift
      ;;
    *)
      echo "[metrics-gate] unknown argument: $1"
      exit 2
      ;;
  esac
done

if [[ ! -x "$BIN" ]]; then
  echo "[metrics-gate] release binary missing, building..."
  cargo build --release
fi

LOG_FILE="$(mktemp -t kacf-metrics-gate.XXXXXX.log)"
cleanup() {
  if [[ -n "${SERVER_PID:-}" ]] && kill -0 "$SERVER_PID" >/dev/null 2>&1; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  rm -f "$LOG_FILE"
}
trap cleanup EXIT

echo "[metrics-gate] starting server on port ${PORT}"
AUTOCODING_PORT="$PORT" "$BIN" >"$LOG_FILE" 2>&1 &
SERVER_PID=$!

sleep 0.4
if ! kill -0 "$SERVER_PID" >/dev/null 2>&1; then
  if grep -qi "operation not permitted" "$LOG_FILE"; then
    echo "[metrics-gate] skipped: sandbox blocks binding local ports"
    exit 0
  fi
  echo "[metrics-gate] server exited early"
  cat "$LOG_FILE"
  exit 1
fi

METRICS="$(curl -fsS "http://127.0.0.1:${PORT}/metrics" || true)"
if [[ -z "$METRICS" ]]; then
  if grep -qi "operation not permitted" "$LOG_FILE"; then
    echo "[metrics-gate] skipped: sandbox blocks binding local ports"
    exit 0
  fi
  echo "[metrics-gate] failed to fetch /metrics"
  cat "$LOG_FILE"
  exit 1
fi

if [[ "$METRICS" != *"\"gate_passed\":true"* ]]; then
  if [[ "$JSON_MODE" == "1" ]]; then
    printf '{"ok":false,"reason":"backend_gate_failed","metrics":%s}\n' "$METRICS"
  else
    echo "[metrics-gate] gate failed"
    echo "$METRICS"
  fi
  gate_passed=false
else
  gate_passed=true
fi

readiness_score="$(echo "$METRICS" | sed -n 's/.*"readiness_score":\([0-9]\+\).*/\1/p' | head -n1)"
running_val="$(echo "$METRICS" | sed -n 's/.*"running":\([^,}]*\).*/\1/p' | head -n1)"
blockers_raw="$(echo "$METRICS" | sed -n 's/.*"blockers":\[\([^]]*\)\].*/\1/p' | head -n1)"

if [[ -z "$MIN_SCORE" ]]; then
  MIN_SCORE="$(echo "$METRICS" | sed -n 's/.*"gate_threshold":\([0-9]\+\).*/\1/p' | head -n1)"
fi
if [[ -z "$MIN_SCORE" ]]; then
  MIN_SCORE=75
fi

if [[ -n "$readiness_score" ]] && (( readiness_score < MIN_SCORE )); then
  if [[ "$JSON_MODE" == "1" ]]; then
    printf '{"ok":false,"reason":"score_below_min","score":%s,"min_score":%s,"metrics":%s}\n' "$readiness_score" "$MIN_SCORE" "$METRICS"
  else
    echo "[metrics-gate] gate failed: readiness_score=${readiness_score} < min_score=${MIN_SCORE}"
    echo "$METRICS"
  fi
  exit 1
fi

if [[ "${running_val}" == "true" && "${ALLOW_RUNNING}" != "1" ]]; then
  if [[ "$JSON_MODE" == "1" ]]; then
    printf '{"ok":false,"reason":"still_running","metrics":%s}\n' "$METRICS"
  else
    echo "[metrics-gate] gate failed: service still running"
    echo "$METRICS"
  fi
  exit 1
fi

if [[ -n "${blockers_raw}" && "${ALLOW_BLOCKERS}" != "1" ]]; then
  if [[ "$JSON_MODE" == "1" ]]; then
    printf '{"ok":false,"reason":"blockers_present","metrics":%s}\n' "$METRICS"
  else
    echo "[metrics-gate] gate failed: blockers present"
    echo "$METRICS"
  fi
  exit 1
fi

if [[ "${gate_passed}" != "true" && "${ALLOW_BLOCKERS}" != "1" ]]; then
  if [[ "$JSON_MODE" == "1" ]]; then
    printf '{"ok":false,"reason":"backend_gate_failed_final","metrics":%s}\n' "$METRICS"
  else
    echo "[metrics-gate] gate failed by backend decision"
    echo "$METRICS"
  fi
  exit 1
fi

if [[ "$JSON_MODE" == "1" ]]; then
  printf '{"ok":true,"score":"%s","min_score":"%s","metrics":%s}\n' "${readiness_score:-unknown}" "${MIN_SCORE}" "$METRICS"
else
  echo "[metrics-gate] gate passed (score=${readiness_score:-unknown}, min=${MIN_SCORE})"
fi

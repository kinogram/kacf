#!/usr/bin/env bash
set -euo pipefail

# KACF VM ops stress/fault-injection runner.
# Required env:
#   KACF_BASE_URL   e.g. http://127.0.0.1:8080
#   KACF_SESSION    kacf_session cookie value
#
# Usage:
#   bash scripts/vm_ops_stress.sh --vm vm1 --rounds 10 --inject 50 --age 180

BASE_URL="${KACF_BASE_URL:-}"
SESSION="${KACF_SESSION:-}"
VM_NAME=""
ROUNDS=10
INJECT_COUNT=50
AGE_SEC=180
HEAL_EVERY=2
SLEEP_SEC=1

while [[ $# -gt 0 ]]; do
  case "$1" in
    --vm) VM_NAME="${2:-}"; shift 2 ;;
    --rounds) ROUNDS="${2:-10}"; shift 2 ;;
    --inject) INJECT_COUNT="${2:-50}"; shift 2 ;;
    --age) AGE_SEC="${2:-180}"; shift 2 ;;
    --heal-every) HEAL_EVERY="${2:-2}"; shift 2 ;;
    --sleep) SLEEP_SEC="${2:-1}"; shift 2 ;;
    *)
      echo "unknown arg: $1" >&2
      exit 1
      ;;
  esac
done

if [[ -z "$BASE_URL" || -z "$SESSION" || -z "$VM_NAME" ]]; then
  echo "missing required input." >&2
  echo "need: KACF_BASE_URL, KACF_SESSION and --vm <name>" >&2
  exit 1
fi

api_post() {
  local path="$1"
  local json="$2"
  curl -fsS \
    -H "Content-Type: application/json" \
    -H "Cookie: kacf_session=${SESSION}" \
    -X POST \
    "${BASE_URL}${path}" \
    -d "${json}"
}

api_get() {
  local path="$1"
  curl -fsS \
    -H "Cookie: kacf_session=${SESSION}" \
    "${BASE_URL}${path}"
}

echo "[stress] base=${BASE_URL} vm=${VM_NAME} rounds=${ROUNDS} inject=${INJECT_COUNT} age=${AGE_SEC}s"

for ((i=1; i<=ROUNDS; i++)); do
  echo "[round ${i}] inject pending_pressure"
  api_post "/vm/ops/fault_inject" \
    "{\"name\":\"${VM_NAME}\",\"mode\":\"pending_pressure\",\"count\":${INJECT_COUNT},\"age_sec\":${AGE_SEC}}" \
    || true

  echo "[round ${i}] inject stale_running_task"
  api_post "/vm/ops/fault_inject" \
    "{\"name\":\"${VM_NAME}\",\"mode\":\"stale_running_task\",\"count\":1,\"age_sec\":${AGE_SEC}}" \
    || true

  echo "[round ${i}] dispatch + summary"
  api_post "/vm/exec/dispatch" '{}' || true
  api_get "/vm/ops/summary" || true
  echo

  if (( HEAL_EVERY > 0 && i % HEAL_EVERY == 0 )); then
    echo "[round ${i}] health self-heal"
    api_post "/vm/health/scan" '{"self_heal":true}' || true
    echo
  fi

  sleep "${SLEEP_SEC}"
done

echo "[final] ops summary"
api_get "/vm/ops/summary" || true
echo
echo "[final] dispatch trace (latest 10)"
api_get "/vm/exec/dispatch/trace?limit=10" || true
echo
echo "[done]"


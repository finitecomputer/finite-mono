#!/usr/bin/env bash
set -euo pipefail

CANARY_ENV_FILE="${FINITE_PRIVATE_CANARY_ENV_FILE:-secrets/finite-private-canary.env}"
if [ -f "$CANARY_ENV_FILE" ]; then
  # shellcheck disable=SC1090
  source "$CANARY_ENV_FILE"
fi

CONTAINER="${FINITE_PRIVATE_CONTAINER:-kimi-k2-6}"
ENDPOINT="${FINITE_PRIVATE_ENDPOINT:-https://kimi-k2-6.finite.containers.tinfoil.dev}"
MODEL="${FINITE_PRIVATE_MODEL:-glm-5-2}"
TIMEOUT_SECS="${FINITE_PRIVATE_CANARY_TIMEOUT_SECS:-180}"
READY_TIMEOUT_SECS="${FINITE_PRIVATE_READY_TIMEOUT_SECS:-4200}"
LOAD_MAX_FIRST_BYTE_SECS="${FINITE_PRIVATE_LOAD_MAX_FIRST_BYTE_SECS:-90}"

usage() {
  cat >&2 <<'EOF'
usage: infra/runbooks/finite-private-ops.sh COMMAND [ARGS]

Read-only commands:
  status              Print Tinfoil container status JSON.
  live                Check process-only liveness.
  health              Check deep Finite Private readiness.
  canary              Run an authenticated non-streaming chat canary.
  stream-canary       Run chat streaming through the terminal SSE [DONE].
  responses-canary    Run an authenticated non-streaming /v1/responses canary.
  repeated-id-canary  Send two calls with one caller x-request-id.
  load-canary         Run 32 concurrent short calls and enforce first-byte headroom.
  negative-canary     Confirm an invalid Finite key is rejected.
  gate                Run status, live, health, negative-canary, and canary.
  wait-ready          Poll status and deep health until ready.

Mutating command (requires explicit approval and confirmation env):
  relaunch TAG         Relaunch the Tinfoil container from measured TAG.

Environment:
  FINITE_PRIVATE_CONTAINER             default: kimi-k2-6
  FINITE_PRIVATE_ENDPOINT              default: https://kimi-k2-6.finite.containers.tinfoil.dev
  FINITE_PRIVATE_MODEL                 default: glm-5-2
  FINITE_PRIVATE_CANARY_ENV_FILE       default: secrets/finite-private-canary.env
  FINITE_PRIVATE_CANARY_API_KEY        required for canary/gate
  FINITE_PRIVATE_CANARY_TIMEOUT_SECS   default: 180
  FINITE_PRIVATE_READY_TIMEOUT_SECS    default: 4200
  FINITE_PRIVATE_LOAD_MAX_FIRST_BYTE_SECS default: 90
  FINITE_PRIVATE_RELAUNCH_APPROVED     must equal the exact TAG for relaunch
EOF
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 1
  fi
}

require_positive_integer() {
  case "$2" in
    ''|*[!0-9]*|0) echo "$1 must be a positive integer" >&2; exit 64 ;;
  esac
}

status() {
  require_command tinfoil
  tinfoil container get "$CONTAINER" --output json
}

http_status() {
  local path="$1"
  local status_code
  status_code="$(curl -sS --max-time 10 -o /dev/null -w '%{http_code}' "$ENDPOINT$path" 2>/dev/null || true)"
  if [ -z "$status_code" ]; then
    status_code="curl_error"
  fi
  printf '%s' "$status_code"
}

probe_endpoint() {
  local path="$1"
  local expected="${2:-200}"
  local body_file
  local status_code
  body_file="$(mktemp)"
  status_code="$(curl -sS --max-time 10 -o "$body_file" -w '%{http_code}' "$ENDPOINT$path" || true)"
  cat "$body_file"
  printf '\n'
  rm -f "$body_file"
  printf 'HTTP %s %s\n' "$status_code" "$path" >&2
  [ "$status_code" = "$expected" ]
}

live() {
  require_command curl
  probe_endpoint "/live" 200
}

health() {
  require_command curl
  probe_endpoint "/health" 200
}

canary() {
  require_command curl
  if [ -z "${FINITE_PRIVATE_CANARY_API_KEY:-}" ]; then
    echo "FINITE_PRIVATE_CANARY_API_KEY is required for canary" >&2
    exit 1
  fi
  local request_id
  local curl_config
  local payload
  local response_file
  request_id="fp_ops_canary_$(date -u +%Y%m%dT%H%M%SZ)"
  curl_config="$(mktemp)"
  response_file="$(mktemp)"
  chmod 600 "$curl_config"
  trap 'rm -f "$curl_config" "$response_file"' RETURN
  {
    printf '%s\n' 'header = "content-type: application/json"'
    printf 'header = "authorization: Bearer %s"\n' "$FINITE_PRIVATE_CANARY_API_KEY"
    printf 'header = "x-request-id: %s"\n' "$request_id"
  } >"$curl_config"
  payload="$(printf '{"model":"%s","messages":[{"role":"user","content":"Reply with exactly: finite private ok"}],"temperature":0,"max_tokens":128}' "$MODEL")"
  curl -fsS \
    --max-time "$TIMEOUT_SECS" \
    --config "$curl_config" \
    --data "$payload" \
    --output "$response_file" \
    "$ENDPOINT/v1/chat/completions"
  python3 - "$response_file" <<'PY'
import json
import sys

payload = json.load(open(sys.argv[1], encoding="utf-8"))
content = payload.get("choices", [{}])[0].get("message", {}).get("content")
if not isinstance(content, str) or "finite private ok" not in content.lower():
    raise SystemExit("chat canary response did not contain the expected text")
print(json.dumps(payload, sort_keys=True))
PY
  rm -f "$curl_config" "$response_file"
  trap - RETURN
}

stream_canary() {
  require_command curl
  if [ -z "${FINITE_PRIVATE_CANARY_API_KEY:-}" ]; then
    echo "FINITE_PRIVATE_CANARY_API_KEY is required for stream-canary" >&2
    exit 1
  fi
  local curl_config payload response_file request_id
  request_id="fp_ops_stream_$(date -u +%Y%m%dT%H%M%SZ)"
  curl_config="$(mktemp)"
  response_file="$(mktemp)"
  chmod 600 "$curl_config"
  trap 'rm -f "$curl_config" "$response_file"' RETURN
  {
    printf '%s\n' 'header = "content-type: application/json"'
    printf 'header = "authorization: Bearer %s"\n' "$FINITE_PRIVATE_CANARY_API_KEY"
    printf 'header = "x-request-id: %s"\n' "$request_id"
  } >"$curl_config"
  payload="$(printf '{"model":"%s","messages":[{"role":"user","content":"Reply briefly: finite private stream ok"}],"temperature":0,"max_tokens":128,"stream":true}' "$MODEL")"
  curl -fsS --no-buffer --max-time "$TIMEOUT_SECS" --config "$curl_config" \
    --data "$payload" --output "$response_file" "$ENDPOINT/v1/chat/completions"
  grep -Fq 'data: [DONE]' "$response_file"
  cat "$response_file"
  rm -f "$curl_config" "$response_file"
  trap - RETURN
}

responses_canary() {
  require_command curl
  if [ -z "${FINITE_PRIVATE_CANARY_API_KEY:-}" ]; then
    echo "FINITE_PRIVATE_CANARY_API_KEY is required for responses-canary" >&2
    exit 1
  fi
  local curl_config payload response_file request_id
  request_id="fp_ops_responses_$(date -u +%Y%m%dT%H%M%SZ)"
  curl_config="$(mktemp)"
  response_file="$(mktemp)"
  chmod 600 "$curl_config"
  trap 'rm -f "$curl_config" "$response_file"' RETURN
  {
    printf '%s\n' 'header = "content-type: application/json"'
    printf 'header = "authorization: Bearer %s"\n' "$FINITE_PRIVATE_CANARY_API_KEY"
    printf 'header = "x-request-id: %s"\n' "$request_id"
  } >"$curl_config"
  payload="$(printf '{"model":"%s","input":"Reply briefly: finite private responses ok","max_output_tokens":128}' "$MODEL")"
  curl -fsS --max-time "$TIMEOUT_SECS" --config "$curl_config" \
    --data "$payload" --output "$response_file" "$ENDPOINT/v1/responses"
  python3 - "$response_file" <<'PY'
import json
import sys

payload = json.load(open(sys.argv[1], encoding="utf-8"))
if not isinstance(payload.get("id"), str) or not payload["id"]:
    raise SystemExit("responses canary did not return a response id")
print(json.dumps(payload, sort_keys=True))
PY
  rm -f "$curl_config" "$response_file"
  trap - RETURN
}

repeated_id_canary() {
  require_command curl
  if [ -z "${FINITE_PRIVATE_CANARY_API_KEY:-}" ]; then
    echo "FINITE_PRIVATE_CANARY_API_KEY is required for repeated-id-canary" >&2
    exit 1
  fi
  local curl_config payload request_id
  request_id="fp_ops_reused_$(date -u +%Y%m%dT%H%M%SZ)"
  curl_config="$(mktemp)"
  chmod 600 "$curl_config"
  trap 'rm -f "$curl_config"' RETURN
  {
    printf '%s\n' 'header = "content-type: application/json"'
    printf 'header = "authorization: Bearer %s"\n' "$FINITE_PRIVATE_CANARY_API_KEY"
    printf 'header = "x-request-id: %s"\n' "$request_id"
  } >"$curl_config"
  payload="$(printf '{"model":"%s","messages":[{"role":"user","content":"Reply with ok"}],"temperature":0,"max_tokens":8}' "$MODEL")"
  curl -fsS --max-time "$TIMEOUT_SECS" --config "$curl_config" --data "$payload" "$ENDPOINT/v1/chat/completions"
  printf '\n'
  curl -fsS --max-time "$TIMEOUT_SECS" --config "$curl_config" --data "$payload" "$ENDPOINT/v1/chat/completions"
  printf '\ncaller request id reused twice: %s\n' "$request_id"
  rm -f "$curl_config"
  trap - RETURN
}

load_canary() {
  require_command curl
  require_command python3
  require_command xargs
  if [ -z "${FINITE_PRIVATE_CANARY_API_KEY:-}" ]; then
    echo "FINITE_PRIVATE_CANARY_API_KEY is required for load-canary" >&2
    exit 1
  fi
  require_positive_integer FINITE_PRIVATE_CANARY_TIMEOUT_SECS "$TIMEOUT_SECS"
  require_positive_integer FINITE_PRIVATE_LOAD_MAX_FIRST_BYTE_SECS "$LOAD_MAX_FIRST_BYTE_SECS"
  local curl_config payload_file result_dir batch_id
  curl_config="$(mktemp)"
  payload_file="$(mktemp)"
  result_dir="$(mktemp -d)"
  batch_id="$(date -u +%Y%m%dT%H%M%SZ)"
  chmod 600 "$curl_config" "$payload_file"
  trap 'rm -f "$curl_config" "$payload_file"; rm -rf "$result_dir"' RETURN
  {
    printf '%s\n' 'header = "content-type: application/json"'
    printf 'header = "authorization: Bearer %s"\n' "$FINITE_PRIVATE_CANARY_API_KEY"
  } >"$curl_config"
  printf '{"model":"%s","messages":[{"role":"user","content":"Reply with ok"}],"temperature":0,"max_tokens":8}' "$MODEL" >"$payload_file"
  export FP_LOAD_CONFIG="$curl_config" FP_LOAD_PAYLOAD="$payload_file"
  export FP_LOAD_RESULTS="$result_dir" FP_LOAD_ENDPOINT="$ENDPOINT"
  export FP_LOAD_TIMEOUT="$TIMEOUT_SECS" FP_LOAD_BATCH_ID="$batch_id"
  seq 1 32 | xargs -P 32 -I '{}' sh -c '
    n="$1"
    curl -sS --max-time "$FP_LOAD_TIMEOUT" --config "$FP_LOAD_CONFIG" \
      -H "x-request-id: fp_load_${FP_LOAD_BATCH_ID}_${n}" \
      --data-binary "@$FP_LOAD_PAYLOAD" \
      --output "$FP_LOAD_RESULTS/body-${n}.json" \
      --write-out "%{http_code}\t%{time_starttransfer}\n" \
      "$FP_LOAD_ENDPOINT/v1/chat/completions" >"$FP_LOAD_RESULTS/metric-${n}.tsv"
  ' sh '{}'
  python3 - "$result_dir" "$LOAD_MAX_FIRST_BYTE_SECS" <<'PY'
import math
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
limit = float(sys.argv[2])
latencies = []
for path in sorted(root.glob("metric-*.tsv")):
    status, first_byte = path.read_text(encoding="utf-8").strip().split("\t")
    if status != "200":
        raise SystemExit(f"load canary failed: {path.name} returned HTTP {status}")
    latencies.append(float(first_byte))
if len(latencies) != 32:
    raise SystemExit(f"load canary produced {len(latencies)} metrics, expected 32")
latencies.sort()
percentile = lambda p: latencies[max(0, math.ceil(p * len(latencies)) - 1)]
p95, p99 = percentile(0.95), percentile(0.99)
print(f"first-byte seconds: p95={p95:.3f} p99={p99:.3f} max_allowed={limit:.3f}")
if p99 >= limit:
    raise SystemExit("load canary lacks required headroom below limiter first-byte timeout")
PY
  unset FP_LOAD_CONFIG FP_LOAD_PAYLOAD FP_LOAD_RESULTS FP_LOAD_ENDPOINT
  unset FP_LOAD_TIMEOUT FP_LOAD_BATCH_ID
  rm -f "$curl_config" "$payload_file"
  rm -rf "$result_dir"
  trap - RETURN
}

negative_canary() {
  require_command curl
  local payload
  local status_code
  payload="$(printf '{"model":"%s","messages":[{"role":"user","content":"authorization probe"}],"max_tokens":1}' "$MODEL")"
  status_code="$(curl -sS --max-time 15 -o /dev/null -w '%{http_code}' \
    -H 'content-type: application/json' \
    -H 'authorization: Bearer fpk_invalid_rollout_probe' \
    --data "$payload" \
    "$ENDPOINT/v1/chat/completions" || true)"
  printf 'HTTP %s invalid-key canary\n' "$status_code"
  [ "$status_code" = "401" ]
}

gate() {
  status
  live
  health
  negative_canary
  canary
  echo "Finite Private gate passed"
}

relaunch() {
  require_command tinfoil
  local tag="${1:-}"
  if [ -z "$tag" ]; then
    echo "relaunch requires an exact measured release tag" >&2
    exit 1
  fi
  if [ "${FINITE_PRIVATE_RELAUNCH_APPROVED:-}" != "$tag" ]; then
    echo "refusing relaunch: set FINITE_PRIVATE_RELAUNCH_APPROVED to the exact approved tag" >&2
    exit 1
  fi
  tinfoil container relaunch "$CONTAINER" --output json --tag "$tag"
}

wait_ready() {
  require_command tinfoil
  require_command curl
  require_positive_integer FINITE_PRIVATE_READY_TIMEOUT_SECS "$READY_TIMEOUT_SECS"
  local attempt=1
  local deadline=$((SECONDS + READY_TIMEOUT_SECS))
  while true; do
    local live_code
    local health_code
    live_code="$(http_status /live)"
    health_code="$(http_status /health)"
    if status >/dev/null 2>&1 && [ "$live_code" = "200" ] && [ "$health_code" = "200" ]; then
      echo "Finite Private is ready"
      health
      return 0
    fi
    echo "waiting for Finite Private readiness, attempt $attempt, live=$live_code, health=$health_code" >&2
    if [ "$attempt" = "1" ] || [ $((attempt % 10)) = "0" ]; then
      health || true
    fi
    attempt=$((attempt + 1))
    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "Finite Private readiness timed out after ${READY_TIMEOUT_SECS}s" >&2
      return 1
    fi
    sleep 30
  done
}

command="${1:-}"
case "$command" in
  status) status ;;
  live) live ;;
  health) health ;;
  canary) canary ;;
  stream-canary) stream_canary ;;
  responses-canary) responses_canary ;;
  repeated-id-canary) repeated_id_canary ;;
  load-canary) load_canary ;;
  negative-canary) negative_canary ;;
  gate) gate ;;
  relaunch)
    shift
    relaunch "${1:-}"
    ;;
  wait-ready) wait_ready ;;
  -h|--help|help|"") usage ;;
  *)
    usage
    exit 1
    ;;
esac

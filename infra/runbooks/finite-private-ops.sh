#!/usr/bin/env bash
set -euo pipefail

CANARY_ENV_FILE="${FINITE_PRIVATE_CANARY_ENV_FILE:-secrets/finite-private-canary.env}"
if [ -f "$CANARY_ENV_FILE" ]; then
  # shellcheck disable=SC1090
  source "$CANARY_ENV_FILE"
fi

CONTAINER="${FINITE_PRIVATE_CONTAINER:-kimi-k2-6}"
ENDPOINT="${FINITE_PRIVATE_ENDPOINT:-https://kimi-k2-6.finite.containers.tinfoil.dev}"
MODEL="${FINITE_PRIVATE_MODEL:-deepseek-v4-flash-0731}"
TIMEOUT_SECS="${FINITE_PRIVATE_CANARY_TIMEOUT_SECS:-180}"
READY_TIMEOUT_SECS="${FINITE_PRIVATE_READY_TIMEOUT_SECS:-4200}"
LOAD_MAX_FIRST_BYTE_SECS="${FINITE_PRIVATE_LOAD_MAX_FIRST_BYTE_SECS:-90}"
LOAD_CONCURRENCY="${FINITE_PRIVATE_LOAD_CONCURRENCY:-32}"
LOAD_MAX_TOKENS="${FINITE_PRIVATE_LOAD_MAX_TOKENS:-64}"
LOAD_SWEEP_APPROVAL="1,4,8,16,32,64,128,256"

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
  mixed-version-canary
                      Prove older glm-5-2 requests through the current limiter.
  repeated-id-canary  Send two calls with one caller x-request-id.
  load-canary [N]     Run N concurrent streaming calls and report latency/throughput.
  load-sweep          Run the guarded 1,4,8,16,32,64,128,256 maintenance sweep.
  settlement-status SINCE_UTC
                      Prove this canary key has no rollout-era reserved rows.
  negative-canary     Confirm an invalid Finite key is rejected.
  gate                Run status, live, health, negative-canary, and canary.
  wait-ready          Poll status and deep health until ready.

Mutating command (requires explicit approval and confirmation env):
  relaunch TAG         Relaunch the Tinfoil container from measured TAG.

Environment:
  FINITE_PRIVATE_CONTAINER             default: kimi-k2-6
  FINITE_PRIVATE_ENDPOINT              default: https://kimi-k2-6.finite.containers.tinfoil.dev
  FINITE_PRIVATE_MODEL                 default: deepseek-v4-flash-0731
  FINITE_PRIVATE_CANARY_ENV_FILE       default: secrets/finite-private-canary.env
  FINITE_PRIVATE_CANARY_API_KEY        required for canary/gate
  FINITE_PRIVATE_CANARY_TIMEOUT_SECS   default: 180
  FINITE_PRIVATE_READY_TIMEOUT_SECS    default: 4200
  FINITE_PRIVATE_LOAD_MAX_FIRST_BYTE_SECS default: 90
  FINITE_PRIVATE_LOAD_CONCURRENCY        default: 32
  FINITE_PRIVATE_LOAD_MAX_TOKENS         default: 64
  FINITE_PRIVATE_LOAD_SWEEP_APPROVED     must equal 1,4,8,16,32,64,128,256 for N > 32 or load-sweep
  FINITE_PRIVATE_CORE_HOST               default: root@64.34.82.77
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

mixed_version_canary() {
  # DeepSeek is the canonical product model. This local override exercises only
  # the historical request name that already-issued Runtimes may still send.
  local MODEL="glm-5-2"
  canary
  stream_canary
  responses_canary
  echo "Mixed-version Finite Private compatibility passed"
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

require_load_sweep_approval() {
  if [ "${FINITE_PRIVATE_LOAD_SWEEP_APPROVED:-}" != "$LOAD_SWEEP_APPROVAL" ]; then
    echo "refusing high-concurrency load: set FINITE_PRIVATE_LOAD_SWEEP_APPROVED=$LOAD_SWEEP_APPROVAL" >&2
    exit 64
  fi
}

load_canary() {
  require_command curl
  require_command python3
  if [ -z "${FINITE_PRIVATE_CANARY_API_KEY:-}" ]; then
    echo "FINITE_PRIVATE_CANARY_API_KEY is required for load-canary" >&2
    exit 1
  fi
  require_positive_integer FINITE_PRIVATE_CANARY_TIMEOUT_SECS "$TIMEOUT_SECS"
  require_positive_integer FINITE_PRIVATE_LOAD_MAX_FIRST_BYTE_SECS "$LOAD_MAX_FIRST_BYTE_SECS"
  local concurrency="${1:-$LOAD_CONCURRENCY}"
  require_positive_integer FINITE_PRIVATE_LOAD_CONCURRENCY "$concurrency"
  require_positive_integer FINITE_PRIVATE_LOAD_MAX_TOKENS "$LOAD_MAX_TOKENS"
  if [ "$concurrency" -gt 32 ]; then
    require_load_sweep_approval
  fi
  local curl_config payload_file result_dir batch_id batch_elapsed
  curl_config="$(mktemp)"
  payload_file="$(mktemp)"
  result_dir="$(mktemp -d)"
  batch_id="$(date -u +%Y%m%dT%H%M%SZ)_$(python3 -c 'import time; print(time.time_ns())')"
  chmod 600 "$curl_config" "$payload_file"
  trap 'rm -f "$curl_config" "$payload_file"; rm -rf "$result_dir"' RETURN
  {
    printf '%s\n' 'header = "content-type: application/json"'
    printf 'header = "authorization: Bearer %s"\n' "$FINITE_PRIVATE_CANARY_API_KEY"
  } >"$curl_config"
  printf '{"model":"%s","messages":[{"role":"user","content":"Write a compact numbered list from 1 through 50."}],"temperature":0,"max_tokens":%s,"stream":true,"stream_options":{"include_usage":true}}' "$MODEL" "$LOAD_MAX_TOKENS" >"$payload_file"
  export FP_LOAD_CONFIG="$curl_config" FP_LOAD_PAYLOAD="$payload_file"
  export FP_LOAD_RESULTS="$result_dir" FP_LOAD_ENDPOINT="$ENDPOINT"
  export FP_LOAD_TIMEOUT="$TIMEOUT_SECS" FP_LOAD_BATCH_ID="$batch_id"
  batch_elapsed="$(python3 - "$concurrency" <<'PY'
import os
import pathlib
import subprocess
import sys
import time

concurrency = int(sys.argv[1])
root = pathlib.Path(os.environ["FP_LOAD_RESULTS"])
command = [
    "curl",
    "--parallel",
    "--parallel-immediate",
    "--parallel-max",
    str(concurrency),
]
for number in range(1, concurrency + 1):
    if number > 1:
        command.append("--next")
    command.extend(
        [
            "--silent",
            "--show-error",
            "--no-buffer",
            "--http2",
            "--max-time",
            os.environ["FP_LOAD_TIMEOUT"],
            "--config",
            os.environ["FP_LOAD_CONFIG"],
            "--header",
            f"x-request-id: fp_load_{os.environ['FP_LOAD_BATCH_ID']}_{number}",
            "--data-binary",
            f"@{os.environ['FP_LOAD_PAYLOAD']}",
            "--output",
            str(root / f"body-{number}.json"),
            "--write-out",
            f"{number}\t%{{http_code}}\t%{{time_starttransfer}}\t%{{time_total}}\n",
            f"{os.environ['FP_LOAD_ENDPOINT']}/v1/chat/completions",
        ]
    )
started_at = time.monotonic()
result = subprocess.run(
    command,
    capture_output=True,
    text=True,
    check=False,
)
elapsed = time.monotonic() - started_at
(root / "metrics.tsv").write_text(result.stdout, encoding="utf-8")
if result.stderr:
    print(result.stderr, file=sys.stderr, end="")
print(f"{elapsed:.9f}")
raise SystemExit(result.returncode)
PY
)"

  if ! python3 - "$result_dir" "$LOAD_MAX_FIRST_BYTE_SECS" "$concurrency" "$batch_elapsed" <<'PY'
import json
import math
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
limit = float(sys.argv[2])
expected = int(sys.argv[3])
batch_elapsed = float(sys.argv[4])
if not math.isfinite(batch_elapsed) or batch_elapsed <= 0:
    raise SystemExit(f"load canary failed: invalid batch duration {batch_elapsed!r}")
metrics_path = root / "metrics.tsv"
if not metrics_path.is_file():
    raise SystemExit("load canary failed: curl metrics are missing")

first_bytes = []
totals = []
completion_tokens = []
prompt_tokens = []
seen_requests = set()
for line in metrics_path.read_text(encoding="utf-8").splitlines():
    parts = line.strip().split("\t")
    if len(parts) != 4:
        raise SystemExit(f"load canary failed: malformed metric line: {parts!r}")
    request_number_raw, status, first_byte, total = parts
    try:
        request_number = int(request_number_raw)
    except ValueError as error:
        raise SystemExit(
            f"load canary failed: invalid request number {request_number_raw!r}"
        ) from error
    if request_number < 1 or request_number > expected or request_number in seen_requests:
        raise SystemExit(
            f"load canary failed: unexpected request number {request_number}"
        )
    seen_requests.add(request_number)
    if status != "200":
        raise SystemExit(
            f"load canary failed: request {request_number} returned HTTP {status}"
        )
    body_path = root / f"body-{request_number}.json"
    if not body_path.is_file():
        raise SystemExit(
            f"load canary failed: response body {body_path.name} is missing"
        )
    body = body_path.read_text(encoding="utf-8")
    if "data: [DONE]" not in body:
        raise SystemExit(f"load canary failed: {body_path.name} lacks terminal [DONE]")
    usage = None
    for line in body.splitlines():
        if not line.startswith("data: ") or line == "data: [DONE]":
            continue
        try:
            chunk = json.loads(line.removeprefix("data: "))
        except json.JSONDecodeError as error:
            raise SystemExit(f"load canary failed: malformed SSE JSON in {body_path.name}: {error}")
        if isinstance(chunk.get("usage"), dict):
            usage = chunk["usage"]
    if usage is None:
        raise SystemExit(f"load canary failed: {body_path.name} lacks streaming usage")
    first_bytes.append(float(first_byte))
    totals.append(float(total))
    completion_tokens.append(int(usage.get("completion_tokens", 0)))
    prompt_tokens.append(int(usage.get("prompt_tokens", 0)))

if len(first_bytes) != expected:
    raise SystemExit(f"load canary produced {len(first_bytes)} metrics, expected {expected}")
if sum(completion_tokens) <= 0:
    raise SystemExit("load canary reported no completion tokens")

def percentile(values, proportion):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(proportion * len(ordered)) - 1)]

ttft_p50 = percentile(first_bytes, 0.50)
ttft_p95 = percentile(first_bytes, 0.95)
ttft_p99 = percentile(first_bytes, 0.99)
total_p50 = percentile(totals, 0.50)
total_p95 = percentile(totals, 0.95)
total_p99 = percentile(totals, 0.99)
generation_durations = [max(total - first_byte, 0.001) for total, first_byte in zip(totals, first_bytes)]
per_request_rates = [
    tokens / duration for tokens, duration in zip(completion_tokens, generation_durations)
]
aggregate_rate = sum(completion_tokens) / batch_elapsed

print(
    f"requests={expected} prompt_tokens={sum(prompt_tokens)} "
    f"completion_tokens={sum(completion_tokens)} batch_seconds={batch_elapsed:.3f}"
)
print(
    f"time_to_first_byte_seconds p50={ttft_p50:.3f} p95={ttft_p95:.3f} "
    f"p99={ttft_p99:.3f} max_allowed={limit:.3f}"
)
print(
    f"completion_seconds p50={total_p50:.3f} p95={total_p95:.3f} "
    f"p99={total_p99:.3f}"
)
print(
    f"generation_tokens_per_second per_request_p50={percentile(per_request_rates, 0.50):.3f} "
    f"per_request_p95={percentile(per_request_rates, 0.95):.3f} "
    f"aggregate={aggregate_rate:.3f}"
)
if ttft_p99 >= limit:
    raise SystemExit("load canary lacks required headroom below limiter first-byte timeout")
PY
  then
    unset FP_LOAD_CONFIG FP_LOAD_PAYLOAD FP_LOAD_RESULTS FP_LOAD_ENDPOINT
    unset FP_LOAD_TIMEOUT FP_LOAD_BATCH_ID
    return 1
  fi
  unset FP_LOAD_CONFIG FP_LOAD_PAYLOAD FP_LOAD_RESULTS FP_LOAD_ENDPOINT
  unset FP_LOAD_TIMEOUT FP_LOAD_BATCH_ID
  rm -f "$curl_config" "$payload_file"
  rm -rf "$result_dir"
  trap - RETURN
}

wait_load_recovery() {
  local deadline=$((SECONDS + 120))
  while [ "$SECONDS" -lt "$deadline" ]; do
    if health >/dev/null 2>&1; then
      echo "Finite Private recovered after load tier"
      return 0
    fi
    sleep 5
  done
  echo "Finite Private did not recover within 120 seconds" >&2
  return 1
}

load_sweep() {
  require_load_sweep_approval
  local tier
  local failed_tier=""
  for tier in 1 4 8 16 32 64 128 256; do
    echo "=== Finite Private concurrency tier $tier ==="
    if ! load_canary "$tier"; then
      failed_tier="$tier"
      echo "stopping sweep at failed tier $tier" >&2
      break
    fi
    if ! wait_load_recovery || ! load_canary 1 || ! health >/dev/null; then
      failed_tier="$tier"
      echo "stopping sweep because the clean single-request proof failed after tier $tier" >&2
      break
    fi
  done

  if [ -n "$failed_tier" ]; then
    wait_load_recovery || true
    echo "sweep stopped at tier $failed_tier; no further inference requests were issued after failure" >&2
    return 1
  fi
  echo "Finite Private concurrency sweep passed through 256"
}

settlement_status() {
  require_command shasum
  require_command ssh
  if [ -z "${FINITE_PRIVATE_CANARY_API_KEY:-}" ]; then
    echo "FINITE_PRIVATE_CANARY_API_KEY is required for settlement-status" >&2
    exit 1
  fi
  local since="${1:-}"
  if [[ ! "$since" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$ ]]; then
    echo "settlement-status requires SINCE_UTC as YYYY-MM-DDTHH:MM:SSZ" >&2
    exit 64
  fi
  local key_hash
  key_hash="$(printf '%s' "$FINITE_PRIVATE_CANARY_API_KEY" | shasum -a 256 | awk '{print $1}')"
  if [[ ! "$key_hash" =~ ^[0-9a-f]{64}$ ]]; then
    echo "could not compute the canary API key hash" >&2
    exit 1
  fi
  local core_host="${FINITE_PRIVATE_CORE_HOST:-root@64.34.82.77}"
  local result
  result="$(ssh -o BatchMode=yes -o ConnectTimeout=10 "$core_host" \
    "sudo -u postgres psql --no-psqlrc -d finite_core -v ON_ERROR_STOP=1 -P pager=off -Atc \"SELECT r.model, r.status, COALESCE(r.settlement_kind, 'none'), COUNT(*) FROM finite_private_reservations r JOIN finite_private_api_keys k ON k.id = r.api_key_id WHERE k.key_hash = '$key_hash' AND r.created_at >= '$since'::timestamptz GROUP BY 1,2,3 ORDER BY 1,2,3; SELECT 'preexisting_reserved', COUNT(*) FILTER (WHERE r.status = 'reserved' AND r.created_at < '$since'::timestamptz), 'rollout_reserved', COUNT(*) FILTER (WHERE r.status = 'reserved' AND r.created_at >= '$since'::timestamptz) FROM finite_private_reservations r JOIN finite_private_api_keys k ON k.id = r.api_key_id WHERE k.key_hash = '$key_hash';\"")"
  printf '%s\n' "$result"
  local summary preexisting_label preexisting_count rollout_label rollout_count
  summary="$(printf '%s\n' "$result" | tail -n 1)"
  IFS='|' read -r preexisting_label preexisting_count rollout_label rollout_count <<<"$summary"
  if [ "$preexisting_label" != "preexisting_reserved" ] \
    || [ "$rollout_label" != "rollout_reserved" ] \
    || [[ ! "$preexisting_count" =~ ^[0-9]+$ ]] \
    || [[ ! "$rollout_count" =~ ^[0-9]+$ ]]; then
    echo "settlement-status returned an unexpected ledger summary" >&2
    return 1
  fi
  if [ "$rollout_count" != "0" ]; then
    echo "$rollout_count canary reservations created during this rollout remain reserved" >&2
    return 1
  fi
  echo "Finite Private rollout-era canary settlements passed"
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
  mixed-version-canary) mixed_version_canary ;;
  repeated-id-canary) repeated_id_canary ;;
  load-canary)
    shift
    load_canary "${1:-}"
    ;;
  load-sweep) load_sweep ;;
  settlement-status)
    shift
    settlement_status "${1:-}"
    ;;
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

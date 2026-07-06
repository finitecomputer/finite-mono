#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

image_default="finitecomputer-v2-agent-runtime:local"
run_id="${FC_LOCAL_CANARY_RUN_ID:-$(date +%Y%m%d%H%M%S)}"
state_root="${FC_LOCAL_CANARY_STATE_ROOT:-$repo_root/.local-state/create-agent-canary}"
log_dir="$state_root/logs/$run_id"
work_root="$state_root/runner"

core_port="${FC_LOCAL_CANARY_CORE_PORT:-14200}"
dashboard_port="${FC_LOCAL_CANARY_DASHBOARD_PORT:-13002}"
postgres_port="${FC_LOCAL_CANARY_POSTGRES_PORT:-15432}"
agent_port="${FC_LOCAL_CANARY_AGENT_PORT:-18080}"

core_url="http://127.0.0.1:$core_port"
dashboard_url="http://127.0.0.1:$dashboard_port"
core_token="${FC_CORE_API_TOKEN:-local-core-token-$run_id}"
postgres_container="${FC_LOCAL_CANARY_POSTGRES_CONTAINER:-finite-v2-local-postgres}"
postgres_password="${FC_LOCAL_CANARY_POSTGRES_PASSWORD:-finite-local}"
postgres_db="${FC_LOCAL_CANARY_POSTGRES_DB:-finite_saas_core}"
database_url="postgres://postgres:$postgres_password@127.0.0.1:$postgres_port/$postgres_db"

artifact_id="${FC_LOCAL_CANARY_RUNTIME_ARTIFACT_ID:-local-v2-agent-runtime}"
agent_image="${FC_LOCAL_AGENT_IMAGE:-$image_default}"
build_runtime_image="${FC_LOCAL_CANARY_BUILD_RUNTIME_IMAGE:-1}"
runtime_hermes_agent_version="${FC_LOCAL_CANARY_HERMES_AGENT_VERSION:-${FC_RUNTIME_HERMES_AGENT_VERSION:-0.18.0}}"
finitechat_server_url="${FC_RUNNER_FINITECHAT_SERVER_URL:-https://chat.finite.computer}"
finite_private_api_key_override="${FC_LOCAL_CANARY_FINITE_PRIVATE_API_KEY:-${FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE:-}}"
finite_private_upstream_key="${FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY:-}"
require_finite_private_key="${FC_LOCAL_CANARY_REQUIRE_FINITE_PRIVATE_KEY:-1}"
limiter_port="${FC_LOCAL_CANARY_LIMITER_PORT:-18002}"
# How agents inside Docker reach the chained local limiter. host.docker.internal
# works on Docker Desktop; on Linux override with the docker bridge IP.
limiter_agent_base_url="${FC_LOCAL_CANARY_LIMITER_AGENT_BASE_URL:-http://host.docker.internal:$limiter_port/v1}"
source_host_id="${FC_LOCAL_CANARY_SOURCE_HOST_ID:-local-docker}"
runner_id="${FC_LOCAL_CANARY_RUNNER_ID:-local-docker-runner}"
display_name="${FC_LOCAL_CANARY_AGENT_NAME:-Local Hermes Canary}"
launch_code="${FC_LOCAL_CANARY_LAUNCH_CODE:-off2026}"
idempotency_key="${FC_LOCAL_CANARY_IDEMPOTENCY_KEY:-local-canary-$run_id}"
dev_email="${FC_DASHBOARD_DEV_EMAIL:-local-canary@finite.computer}"
dev_workos_user_id="${FC_DASHBOARD_DEV_WORKOS_USER_ID:-user_local_canary}"
keep_services="${FC_LOCAL_CANARY_KEEP_SERVICES:-0}"
replace_runtime="${FC_LOCAL_CANARY_REPLACE_RUNTIME:-1}"

core_pid=""
dashboard_pid=""
limiter_pid=""

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 127
  fi
}

cleanup() {
  if [[ "$keep_services" == "1" ]]; then
    echo "FC_LOCAL_CANARY_KEEP_SERVICES=1; leaving Core, dashboard, Postgres, and runtime running."
    if [[ -n "${dashboard_pid:-}" ]]; then
      disown "$dashboard_pid" 2>/dev/null || true
    fi
    if [[ -n "${core_pid:-}" ]]; then
      disown "$core_pid" 2>/dev/null || true
    fi
    return
  fi
  if [[ -n "${dashboard_pid:-}" ]]; then
    kill "$dashboard_pid" 2>/dev/null || true
  fi
  if [[ -n "${limiter_pid:-}" ]]; then
    kill "$limiter_pid" 2>/dev/null || true
  fi
  if [[ -n "${core_pid:-}" ]]; then
    kill "$core_pid" 2>/dev/null || true
  fi
  docker rm -f "$postgres_container" >/dev/null 2>&1 || true
}
trap cleanup EXIT

wait_http() {
  local name="$1"
  local url="$2"
  local timeout_secs="${3:-90}"
  local started
  started="$(date +%s)"
  local last_error="not attempted"
  while true; do
    if curl -fsS --max-time 2 "$url" >/dev/null 2>"$log_dir/${name//[^a-zA-Z0-9_-]/_}.curl.err"; then
      return 0
    fi
    last_error="$(cat "$log_dir/${name//[^a-zA-Z0-9_-]/_}.curl.err" 2>/dev/null || true)"
    if (( "$(date +%s)" - started >= timeout_secs )); then
      echo "$name did not become ready at $url: $last_error" >&2
      return 1
    fi
    sleep 1
  done
}

core_identity_headers=(
  -H "authorization: Bearer $core_token"
  -H "content-type: application/json"
  -H "x-finite-workos-user-id: $dev_workos_user_id"
  -H "x-finite-workos-email: $dev_email"
  -H "x-finite-workos-email-verified: true"
)

need cargo
need curl
need docker
need npm
need python3

mkdir -p "$log_dir" "$work_root"

echo "finitecomputer-v2 local create-agent canary"
echo "runtime image: $agent_image"
echo "build runtime image: $build_runtime_image"
echo "hermes-agent: $runtime_hermes_agent_version"
echo "finitechat server: $finitechat_server_url"
echo "logs: $log_dir"

if [[ -z "$finite_private_upstream_key" && -z "$finite_private_api_key_override" \
      && "$require_finite_private_key" == "1" ]]; then
  cat >&2 <<'MSG'
A Finite Private credential is required for the chat-capable local canary.

Preferred (real provisioning): set FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY to one
operator-held deployed fpk_... key. The canary then runs the in-tree limiter
chained in front of the deployed limiter (finite-saas-local
finite-private-limiter-up), the runner provisions real runtime-scoped keys from
the throwaway local Core, and agents talk to the local chained limiter.

Fallback (no local limiter): set FC_LOCAL_CANARY_FINITE_PRIVATE_API_KEY to a
valid deployed Finite Private API key; the runner injects it directly into the
agent and bypasses local provisioning.

Or set FC_LOCAL_CANARY_REQUIRE_FINITE_PRIVATE_KEY=0 if you intentionally want a
launch-only check (model calls will fail with 401 invalid_api_key).
MSG
  exit 64
fi

if [[ "$build_runtime_image" == "1" ]]; then
  need rsync
  (
    cd "$repo_root"
    python3 scripts/build_runtime_image.py \
      --image-ref "$agent_image" \
      --hermes-agent-version "$runtime_hermes_agent_version" \
      --report "$log_dir/runtime-image-build.json"
  )
else
  docker image inspect "$agent_image" >/dev/null
fi

docker rm -f "$postgres_container" >/dev/null 2>&1 || true
docker run -d \
  --name "$postgres_container" \
  -e POSTGRES_PASSWORD="$postgres_password" \
  -e POSTGRES_DB="$postgres_db" \
  -p "127.0.0.1:$postgres_port:5432" \
  postgres:16-alpine \
  >"$log_dir/postgres.container"

until docker exec "$postgres_container" pg_isready -U postgres -d "$postgres_db" >/dev/null 2>&1; do
  sleep 1
done

if [[ "$keep_services" == "1" ]]; then
  (
    cd "$repo_root"
    nohup env \
      "FC_CORE_DATABASE_URL=$database_url" \
      "FC_CORE_API_TOKEN=$core_token" \
      "FC_CORE_BIND=127.0.0.1:$core_port" \
      cargo run -p finite-saas-core -- serve \
      >"$log_dir/core.log" 2>&1 < /dev/null &
    echo "$!" >"$log_dir/core.pid"
  )
  core_pid="$(cat "$log_dir/core.pid")"
else
  (
    cd "$repo_root"
    FC_CORE_DATABASE_URL="$database_url" \
      FC_CORE_API_TOKEN="$core_token" \
      FC_CORE_BIND="127.0.0.1:$core_port" \
      cargo run -p finite-saas-core -- serve
  ) >"$log_dir/core.log" 2>&1 &
  core_pid="$!"
fi
wait_http "core" "$core_url/healthz" 90

# Real-provisioning path: chain the in-tree limiter between the agents and the
# deployed limiter so keys minted by the throwaway local Core work for real
# inference. All orchestration logic lives in crates/finite-saas-local.
if [[ -n "$finite_private_upstream_key" ]]; then
  echo "finite private: real provisioning via local chained limiter (port $limiter_port)"
  (
    cd "$repo_root"
    FC_CORE_URL="$core_url" \
      FC_CORE_API_TOKEN="$core_token" \
      FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY="$finite_private_upstream_key" \
      cargo run -p finite-saas-local -- finite-private-limiter-up \
      --listen-addr "0.0.0.0:$limiter_port" \
      --dashboard-url "$dashboard_url/dashboard"
  ) >"$log_dir/local-limiter.log" 2>&1 &
  limiter_pid="$!"
  wait_http "local-limiter" "http://127.0.0.1:$limiter_port/health" 180
elif [[ -n "$finite_private_api_key_override" ]]; then
  echo "finite private: operator key override (local provisioning bypassed)"
fi

curl -fsS \
  -X PUT \
  -H "authorization: Bearer $core_token" \
  -H "content-type: application/json" \
  --data @- \
  "$core_url/api/core/v1/runtime-artifacts/$artifact_id" \
  >"$log_dir/runtime-artifact.json" <<JSON
{
  "kind": "oci_image",
  "reference": "$agent_image",
  "versionLabel": "local-create-agent-canary",
  "sourceGitSha": null,
  "finitecVersion": null,
  "hermesSourceRef": null,
  "finitePlatformPluginRef": null,
  "stateSchemaVersion": "runtime-state-v1",
  "baseImage": null,
  "promoted": true,
  "now": null
}
JSON

if [[ "$replace_runtime" == "1" ]]; then
  old_runtime_ids="$(docker ps -aq \
    --filter "label=computer.finite.v2.runtime=true" \
    --filter "label=computer.finite.v2.source_host_id=$source_host_id")"
  if [[ -n "$old_runtime_ids" ]]; then
    echo "$old_runtime_ids" | xargs docker rm -f >/dev/null
  fi
fi

if [[ "$keep_services" == "1" ]]; then
  (
    cd "$repo_root/apps/dashboard"
    nohup env \
      FC_WORKOS_AUTH_ENABLED=0 \
      FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH=1 \
      "FC_DASHBOARD_DEV_EMAIL=$dev_email" \
      "FC_DASHBOARD_DEV_WORKOS_USER_ID=$dev_workos_user_id" \
      "FC_CORE_BASE_URL=$core_url" \
      "FC_CORE_API_TOKEN=$core_token" \
      "NEXT_PUBLIC_WORKOS_REDIRECT_URI=$dashboard_url/callback" \
      npm run dev -- --hostname 127.0.0.1 --port "$dashboard_port" \
      >"$log_dir/dashboard.log" 2>&1 < /dev/null &
    echo "$!" >"$log_dir/dashboard.pid"
  )
  dashboard_pid="$(cat "$log_dir/dashboard.pid")"
else
  (
    cd "$repo_root/apps/dashboard"
    FC_WORKOS_AUTH_ENABLED=0 \
      FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH=1 \
      FC_DASHBOARD_DEV_EMAIL="$dev_email" \
      FC_DASHBOARD_DEV_WORKOS_USER_ID="$dev_workos_user_id" \
      FC_CORE_BASE_URL="$core_url" \
      FC_CORE_API_TOKEN="$core_token" \
      NEXT_PUBLIC_WORKOS_REDIRECT_URI="$dashboard_url/callback" \
      npm run dev -- --hostname 127.0.0.1 --port "$dashboard_port"
  ) >"$log_dir/dashboard.log" 2>&1 &
  dashboard_pid="$!"
fi
wait_http "dashboard" "$dashboard_url/dashboard" 120

curl -fsS \
  -D "$log_dir/dashboard-create-response.headers" \
  -o "$log_dir/dashboard-create-response.html" \
  -X POST \
  --data-urlencode "displayName=$display_name" \
  --data-urlencode "launchCode=$launch_code" \
  --data-urlencode "idempotencyKey=$idempotency_key" \
  "$dashboard_url/dashboard/agent-creation-requests"

if tr -d '\r' <"$log_dir/dashboard-create-response.headers" \
  | grep -Eiq '^location: .*agentCreationError='; then
  echo "dashboard agent creation failed; see $log_dir/dashboard-create-response.headers" >&2
  tr -d '\r' <"$log_dir/dashboard-create-response.headers" \
    | grep -Ei '^location:' >&2 || true
  exit 1
fi

curl -fsS "${core_identity_headers[@]}" "$core_url/api/core/v1/me" >"$log_dir/me-before-runner.json"

python3 - "$log_dir/me-before-runner.json" <<'PY'
import json
import sys
from pathlib import Path

me = json.loads(Path(sys.argv[1]).read_text())
requests = me.get("agent_creation_requests") or []
pending = [
    request for request in requests
    if request.get("status") in {"requested", "launching"}
]
if not pending:
    raise SystemExit(f"dashboard did not create a launchable agent request: {me}")
print(f"agent_creation_request_status={pending[0].get('status')}")
print(f"agent_creation_request_id={pending[0].get('id')}")
PY

runner_env=(
  "FC_RUNNER_BACKEND=docker"
  "FC_CORE_URL=$core_url"
  "FC_CORE_API_TOKEN=$core_token"
  "FC_RUNNER_RUNTIME_ARTIFACT_ID=$artifact_id"
  "FC_RUNNER_ID=$runner_id"
  "FC_RUNNER_SOURCE_HOST_ID=$source_host_id"
  "FC_RUNNER_WORK_ROOT=$work_root"
  "FC_RUNNER_FINITECHAT_SERVER_URL=$finitechat_server_url"
  "FC_RUNNER_DOCKER_HOST_PORT=$agent_port"
  "FC_RUNNER_RUNTIME_READY_TIMEOUT_SECS=${FC_RUNNER_RUNTIME_READY_TIMEOUT_SECS:-180}"
)
if [[ -n "$finite_private_upstream_key" ]]; then
  # No api_key_override: the runner provisions real runtime-scoped keys from
  # local Core, and agents reach the local chained limiter from inside Docker.
  runner_env+=("FC_RUNNER_FINITE_PRIVATE_BASE_URL=$limiter_agent_base_url")
elif [[ -n "$finite_private_api_key_override" ]]; then
  runner_env+=("FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE=$finite_private_api_key_override")
fi

(
  cd "$repo_root"
  env "${runner_env[@]}" cargo run -p finite-saas-runner -- run-once
) >"$log_dir/runner-outcome.json"

python3 - "$log_dir/runner-outcome.json" <<'PY'
import json
import sys
from pathlib import Path

runner = json.loads(Path(sys.argv[1]).read_text())
if runner.get("status") != "launched":
    raise SystemExit(f"runner did not launch a runtime: {runner}")
PY

curl -fsS "http://127.0.0.1:$agent_port/healthz" >"$log_dir/runtime-healthz.json"
curl -fsS "http://127.0.0.1:$agent_port/invite" >"$log_dir/runtime-invite.json"
curl -fsS "${core_identity_headers[@]}" "$core_url/api/core/v1/me" >"$log_dir/me-after-runner.json"

python3 - "$log_dir/runner-outcome.json" "$log_dir/runtime-invite.json" "$log_dir/me-after-runner.json" <<'PY'
import json
import sys
from pathlib import Path

runner = json.loads(Path(sys.argv[1]).read_text())
invite = json.loads(Path(sys.argv[2]).read_text())
me = json.loads(Path(sys.argv[3]).read_text())

assert runner.get("status") == "launched", runner
assert invite.get("ready") is True, invite
assert isinstance(invite.get("url"), str) and invite["url"].startswith("finite"), invite

projects = me.get("projects") or []
assert projects, me
runtime = projects[0].get("runtime")
assert runtime, projects[0]
assert runtime.get("host_facts", {}).get("runtime_status") == "online", runtime

print("runner_status=launched")
print(f"source_machine_id={runtime.get('source_machine_id')}")
print(f"runtime_invite_endpoint={runtime.get('host_facts', {}).get('published_app_urls', [''])[0]}")
print(f"finitechat_invite_url={invite['url']}")
PY

echo "canary complete"
echo "logs: $log_dir"
echo "runtime health: http://127.0.0.1:$agent_port/healthz"
echo "runtime invite: http://127.0.0.1:$agent_port/invite"

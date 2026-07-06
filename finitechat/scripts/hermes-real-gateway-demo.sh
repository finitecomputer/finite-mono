#!/usr/bin/env bash
# Low-level local runner for manual Hermes gateway debugging.
#
# This is not the hardened physical-phone canary gate. It may use loopback
# server URLs and does not prove the full product flow on a phone. For the
# local phone and remote Docker canary gates, see
# docs/hermes-phone-canary-loop.md. Provider promotion belongs to
# ../finitecomputer-v2/docs/hermes-runtime-test-matrix.md.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
state_root="${FINITECHAT_HERMES_STATE_ROOT:-${repo_root}/.state/hermes-real}"
profile="${FINITECHAT_HERMES_PROFILE:-finitechat-real-demo}"
agent_device_id="${FINITECHAT_HERMES_AGENT_DEVICE_ID:-hermes-real-agent}"
port="${FINITECHAT_HERMES_PORT:-18788}"
server_url="${FINITECHAT_HERMES_SERVER_URL:-http://127.0.0.1:${port}}"
listen_addr="${FINITECHAT_HERMES_LISTEN_ADDR:-127.0.0.1:${port}}"
hermes_repo="${FINITECHAT_HERMES_REPO:-}"
hermes_home="${FINITECHAT_HERMES_HOME:-${state_root}/hermes-home}"
agent_home="${FINITECHAT_HERMES_AGENT_HOME:-${state_root}/agent-home}"
finitechat_bin="${repo_root}/target/debug/finitechat"
server_bin="${repo_root}/target/debug/finitechat-server"
model="${FINITECHAT_HERMES_MODEL:-anthropic/claude-sonnet-4.6}"

if [[ -z "${hermes_repo}" ]]; then
  if [[ -d "${repo_root}/../finitecomputer-v2/.state/hermes-runtime/deps/hermes-agent" ]]; then
    hermes_repo="${repo_root}/../finitecomputer-v2/.state/hermes-runtime/deps/hermes-agent"
  elif [[ -d "${repo_root}/../hermes-agent" ]]; then
    hermes_repo="${repo_root}/../hermes-agent"
  else
    echo "Hermes checkout not found. Set FINITECHAT_HERMES_REPO to a Hermes checkout with a .venv." >&2
    exit 1
  fi
fi

if [[ ! -x "${hermes_repo}/.venv/bin/python" ]]; then
  echo "Hermes venv not found at ${hermes_repo}/.venv/bin/python." >&2
  echo "Use the finitecomputer-v2 Hermes runtime matrix or point FINITECHAT_HERMES_REPO at a prepared Hermes checkout." >&2
  exit 1
fi

cd "${repo_root}"
cargo build -q -p finitechat-cli -p finitechat-server

write_hermes_profile() {
  local target_home="$1"
  mkdir -p "${target_home}/plugins"
  rm -rf "${target_home}/plugins/finitechat"
  cp -R "${repo_root}/integrations/hermes/finitechat" "${target_home}/plugins/finitechat"
  find "${target_home}/plugins/finitechat" -name __pycache__ -type d -prune -exec rm -rf {} +

  cat >"${target_home}/config.yaml" <<EOF
model:
  default: ${model}
  provider: openrouter
  base_url: https://openrouter.ai/api/v1
  api_mode: chat_completions
plugins:
  enabled:
    - finitechat
gateway:
  platforms:
    finitechat:
      enabled: true
      extra:
        home: ${agent_home}
        finitechat_bin: ${finitechat_bin}
        poll_timeout_secs: 1
        poll_limit: 10
terminal:
  backend: local
  cwd: ${repo_root}
  persistent_shell: true
approvals:
  mode: off
display:
  streaming: false
EOF
}

mkdir -p "${state_root}"
write_hermes_profile "${hermes_home}"

if [[ "${profile}" != "default" ]]; then
  profile_home="${hermes_home}/profiles/${profile}"
  if [[ ! -d "${profile_home}" ]]; then
    (
      cd "${hermes_repo}"
      HERMES_HOME="${hermes_home}" .venv/bin/python hermes profile create --no-alias --no-skills "${profile}" >/dev/null
    )
  fi
  write_hermes_profile "${profile_home}"
else
  profile_home="${hermes_home}"
fi

if [[ -n "${FINITECHAT_HERMES_ENV_FILE:-}" ]]; then
  env_files=("${FINITECHAT_HERMES_ENV_FILE}")
else
  env_files=(
    "${repo_root}/.env"
    "${repo_root}/../finitecomputer-v2/secrets/shared-provider-keys.env"
    "${repo_root}/../finitecomputer-v2/.state/hermes-runtime/.env"
  )
fi

for env_file in "${env_files[@]}"; do
  if [[ -f "${env_file}" ]]; then
    set -a
    # shellcheck disable=SC1090
    source "${env_file}"
    set +a
  fi
done

if [[ ! -f "${agent_home}/config.json" ]]; then
  "${finitechat_bin}" hermes --home "${agent_home}" init \
    --server "${server_url}" \
    --device-id "${agent_device_id}" \
    --agent-name "${FINITECHAT_HERMES_ROOM_NAME:-Finite Agent}" \
    >/dev/null
elif command -v jq >/dev/null 2>&1; then
  configured_server="$(jq -r '.server_url // empty' "${agent_home}/config.json")"
  if [[ "${configured_server}" != "${server_url}" ]]; then
    echo "Agent home is initialized for ${configured_server}, not ${server_url}." >&2
    echo "Use a different FINITECHAT_HERMES_STATE_ROOT or intentionally delete ${agent_home}." >&2
    exit 1
  fi
fi

server_pid_file="${state_root}/server.pid"
if [[ -f "${server_pid_file}" ]] && kill -0 "$(cat "${server_pid_file}")" 2>/dev/null; then
  :
else
  "${server_bin}" serve "${listen_addr}" --sqlite "${state_root}/server.sqlite3" >"${state_root}/server.log" 2>&1 &
  echo "$!" >"${server_pid_file}"
fi

for _ in {1..80}; do
  if curl -fsS "${server_url}/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done
curl -fsS "${server_url}/health" >/dev/null

cat >"${state_root}/ready.json" <<EOF
{
  "server_url": "${server_url}",
  "agent_home": "${agent_home}",
  "hermes_home": "${hermes_home}",
  "profile_home": "${profile_home}",
  "hermes_repo": "${hermes_repo}",
  "profile": "${profile}"
}
EOF

echo "Finite Chat server: ${server_url}"
echo "Hermes profile: ${profile}"
echo "Agent home: ${agent_home}"
echo "Running real Hermes gateway. No echo handler is installed by this script."

cd "${hermes_repo}"
HERMES_HOME="${hermes_home}" \
FINITECHAT_HOME="${agent_home}" \
FINITECHAT_BIN="${finitechat_bin}" \
FINITE_GATEWAY_ENABLED=true \
GATEWAY_ALLOW_ALL_USERS=true \
FINITE_ALLOW_ALL_USERS=true \
FINITE_AGENT_ID="agent_${agent_device_id}" \
FINITE_AGENT_NAME="${agent_device_id}" \
  .venv/bin/python hermes -p "${profile}" gateway run

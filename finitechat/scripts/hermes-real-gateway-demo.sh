#!/usr/bin/env bash
# Low-level local runner for manual Hermes gateway debugging.
#
# This is not the hardened physical-phone canary gate. It may use loopback
# server URLs and does not prove the full product flow on a phone. For the
# local phone canary gate, see scripts/hermes-phone-canary.py. Provider
# promotion belongs to
# ../finitecomputer-v2/docs/hermes-runtime-test-matrix.md.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mono_root="$(cd "${repo_root}/.." && pwd)"
state_root="${FINITECHAT_HERMES_STATE_ROOT:-${repo_root}/.state/hermes-real}"
agent_device_id="${FINITECHAT_HERMES_AGENT_DEVICE_ID:-hermes-real-agent}"
port="${FINITECHAT_HERMES_PORT:-18788}"
server_url="${FINITECHAT_HERMES_SERVER_URL:-http://127.0.0.1:${port}}"
listen_addr="${FINITECHAT_HERMES_LISTEN_ADDR:-127.0.0.1:${port}}"
service_port="${FINITECHAT_HERMES_SERVICE_PORT:-$((port + 1))}"
service_addr="127.0.0.1:${service_port}"
service_url="http://127.0.0.1:${service_port}"
hermes_nix_shell="${FINITECHAT_HERMES_NIX_SHELL:-${repo_root}/..#hermes-bridge-ci}"
hermes_home="${FINITECHAT_HERMES_HOME:-${state_root}/hermes-home}"
agent_home="${FINITECHAT_HERMES_AGENT_HOME:-${state_root}/agent-home}"
finite_home="${FINITECHAT_HERMES_FINITE_HOME:-${state_root}/finite-home}"
agent_info="${state_root}/agent-info.json"
model="${FINITECHAT_HERMES_MODEL:-anthropic/claude-sonnet-4.6}"

nix_bin="$(command -v nix || true)"
if [[ -z "${nix_bin}" ]]; then
  echo "nix is required to run the pinned Hermes Agent runtime." >&2
  exit 1
fi

cd "${repo_root}"
finitechat_out="$(nix build --no-link --print-out-paths "${mono_root}#finitechat")"
server_out="$(nix build --no-link --print-out-paths "${mono_root}#finitechat-server")"
finitechat_bin="${finitechat_out}/bin/finitechat"
server_bin="${server_out}/bin/finitechat-server"

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
        inbound_stream: true
        service_url: ${service_url}
        service_addr: ${service_addr}
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
security:
  redact_secrets: true
_config_version: 10
EOF
}

mkdir -p "${state_root}"
write_hermes_profile "${hermes_home}"

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

if [[ -z "${OPENROUTER_API_KEY:-}" ]]; then
  echo "OPENROUTER_API_KEY is required by the local Hermes model profile." >&2
  echo "Put it in finitechat/.env or set FINITECHAT_HERMES_ENV_FILE to a mode-0600 env file." >&2
  exit 1
fi

server_pid_file="${state_root}/server.pid"
if [[ -f "${server_pid_file}" ]] && kill -0 "$(cat "${server_pid_file}")" 2>/dev/null; then
  :
else
  "${server_bin}" serve "${listen_addr}" --sqlite "${state_root}/server.sqlite3" >"${state_root}/server.log" 2>&1 &
  echo "$!" >"${server_pid_file}"
fi

# Replaying a deliberately large local history can take materially longer than
# a fresh server start. Keep the bound finite, but allow the real process to
# finish restoring before declaring the local fixture broken.
for _ in {1..600}; do
  if curl -fsS "${server_url}/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done
curl -fsS "${server_url}/health" >/dev/null

if [[ ! -f "${agent_home}/config.json" ]]; then
  umask 077
  FINITE_HOME="${finite_home}" "${finitechat_bin}" hermes --home "${agent_home}" init \
    --server "${server_url}" \
    --device-id "${agent_device_id}" \
    --agent-name "${FINITECHAT_HERMES_ROOM_NAME:-Finite Agent}" \
    >"${agent_info}"
elif command -v jq >/dev/null 2>&1; then
  configured_server="$(jq -r '.server_url // empty' "${agent_home}/config.json")"
  if [[ "${configured_server}" != "${server_url}" ]]; then
    echo "Agent home is initialized for ${configured_server}, not ${server_url}." >&2
    echo "Use a different FINITECHAT_HERMES_STATE_ROOT or intentionally delete ${agent_home}." >&2
    exit 1
  fi
fi

if [[ ! -s "${agent_info}" ]]; then
  echo "Agent identity metadata is missing at ${agent_info}." >&2
  echo "Use a fresh FINITECHAT_HERMES_STATE_ROOT; local identity migration is intentionally unsupported." >&2
  exit 1
fi

service_pid_file="${state_root}/service.pid"
service_ready_file="${state_root}/service-ready.json"
if [[ -f "${service_pid_file}" ]] && kill -0 "$(cat "${service_pid_file}")" 2>/dev/null; then
  :
else
  FINITE_HOME="${finite_home}" "${finitechat_bin}" hermes --home "${agent_home}" serve \
    --addr "127.0.0.1:${service_port}" \
    --ready-file "${service_ready_file}" \
    --json >"${state_root}/service.log" 2>&1 &
  echo "$!" >"${service_pid_file}"
fi

for _ in {1..600}; do
  if curl -fsS "${service_url}/readyz" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done
curl -fsS "${service_url}/readyz" >/dev/null

cat >"${state_root}/ready.json" <<EOF
{
  "server_url": "${server_url}",
  "service_url": "${service_url}",
  "agent_home": "${agent_home}",
  "hermes_home": "${hermes_home}",
  "hermes_nix_shell": "${hermes_nix_shell}"
}
EOF

echo "Finite Chat server: ${server_url}"
echo "Hermes Nix shell: ${hermes_nix_shell}"
echo "Agent home: ${agent_home}"
echo "Running real Hermes gateway. No echo handler is installed by this script."

exec env HERMES_HOME="${hermes_home}" \
FINITE_HOME="${finite_home}" \
FINITECHAT_HOME="${agent_home}" \
FINITECHAT_BIN="${finitechat_bin}" \
FINITECHAT_HERMES_INBOUND_STREAM=1 \
FINITECHAT_HERMES_SERVICE_ADDR="${service_addr}" \
FINITECHAT_HERMES_SERVICE_URL="${service_url}" \
FINITE_GATEWAY_ENABLED=true \
GATEWAY_ALLOW_ALL_USERS=true \
FINITE_ALLOW_ALL_USERS=true \
FINITE_AGENT_ID="agent_${agent_device_id}" \
FINITE_AGENT_NAME="${agent_device_id}" \
  "${nix_bin}" develop "${hermes_nix_shell}" --command hermes gateway run --replace

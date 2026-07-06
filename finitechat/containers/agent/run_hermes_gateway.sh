#!/usr/bin/env bash
set -euo pipefail

agent_home="${FINITECHAT_HOME:-/data/agent}"
hermes_home="${HERMES_HOME:-${agent_home}/hermes-home}"
server_url="${FINITE_SERVER_URL:-${FINITECHAT_SERVER_URL:-}}"
device_id="${FINITECHAT_HERMES_AGENT_DEVICE_ID:-agent}"
finitechat_bin="${FINITECHAT_BIN:-/usr/local/bin/finitechat}"
plugin_name="${FINITECHAT_HERMES_PLUGIN_NAME:-finitechat}"
agent_name="${FINITECHAT_HERMES_AGENT_NAME:-${FINITE_AGENT_NAME:-${FINITECHAT_HERMES_ROOM_NAME:-Finite Agent}}}"
agent_picture_url="${FINITECHAT_HERMES_AGENT_PICTURE_URL:-https://avatars.githubusercontent.com/u/274919006?v=4}"
if [[ "${FINITE_DEFAULT_INFERENCE_PROFILE:-}" == "finite-private" ]]; then
    model="${FINITECHAT_HERMES_MODEL:-${FINITE_PRIVATE_MODEL:-kimi-k2-6}}"
    provider="${FINITECHAT_HERMES_PROVIDER:-custom}"
    base_url="${FINITECHAT_HERMES_BASE_URL:-${FINITE_PRIVATE_BASE_URL:-https://kimi-k2-6.finite.containers.tinfoil.dev/v1}}"
else
    model="${FINITECHAT_HERMES_MODEL:-anthropic/claude-sonnet-4.6}"
    provider="${FINITECHAT_HERMES_PROVIDER:-openrouter}"
    base_url="${FINITECHAT_HERMES_BASE_URL:-https://openrouter.ai/api/v1}"
fi
api_mode="${FINITECHAT_HERMES_API_MODE:-chat_completions}"
api_key=""
api_key_yaml=""
if [[ -n "${FINITECHAT_HERMES_API_KEY:-}" ]]; then
    api_key="${FINITECHAT_HERMES_API_KEY}"
    api_key_yaml='  api_key: ${FINITECHAT_HERMES_API_KEY}'
elif [[ -n "${FINITE_PRIVATE_API_KEY:-}" ]]; then
    api_key="${FINITE_PRIVATE_API_KEY}"
    api_key_yaml='  api_key: ${FINITE_PRIVATE_API_KEY}'
fi
service_addr="${FINITECHAT_HERMES_SERVICE_ADDR:-127.0.0.1:0}"
poll_timeout_secs="${FINITECHAT_HERMES_POLL_TIMEOUT_SECS:-1}"
poll_limit="${FINITECHAT_HERMES_POLL_LIMIT:-10}"
workspace="${FINITECHAT_WORKSPACE:-/workspace}"

export FINITECHAT_HOME="$agent_home"
# Shared Finite identity on the durable mount (identity/identity.json).
export FINITE_HOME="${FINITE_HOME:-$agent_home}"
export HERMES_HOME="$hermes_home"
export FINITECHAT_BIN="$finitechat_bin"
export FINITECHAT_HERMES_INBOUND_STREAM="${FINITECHAT_HERMES_INBOUND_STREAM:-1}"
export FINITECHAT_HERMES_SERVICE_ADDR="$service_addr"
export FINITECHAT_ALLOW_ALL_USERS="${FINITECHAT_ALLOW_ALL_USERS:-true}"
export FINITE_ALLOW_ALL_USERS="${FINITE_ALLOW_ALL_USERS:-true}"
export GATEWAY_ALLOW_ALL_USERS="${GATEWAY_ALLOW_ALL_USERS:-true}"
export FINITE_AGENT_ID="${FINITE_AGENT_ID:-agent_${device_id}}"
export FINITE_AGENT_NAME="$agent_name"

mkdir -p "$agent_home" "$hermes_home/plugins" "$workspace"

if [[ "${FINITE_DEFAULT_INFERENCE_PROFILE:-}" == "finite-private" && -z "$api_key" ]]; then
    echo "FINITE_DEFAULT_INFERENCE_PROFILE=finite-private requires FINITE_PRIVATE_API_KEY; refusing OpenRouter fallback." >&2
    exit 64
fi

if [[ ! -f "${agent_home}/config.json" ]]; then
    if [[ -z "$server_url" ]]; then
        echo "FINITE_AGENT_START_ERROR missing FINITE_SERVER_URL for first initialization" >&2
        exit 64
    fi
    "$finitechat_bin" hermes --home "$agent_home" init \
        --server "$server_url" \
        --device-id "$device_id" \
        --agent-name "$agent_name" \
        --agent-picture-url "$agent_picture_url" \
        >/dev/null
fi

"$finitechat_bin" hermes --home "$agent_home" install \
    --plugins-dir "${hermes_home}/plugins" \
    --plugin-name "$plugin_name" \
    --finitechat-bin "$finitechat_bin" \
    --force \
    --json \
    >/dev/null

invite_file="${agent_home}/current-invite.json"
# Hosted pairing is no-PIN, so the startup invite must be single-use and
# short-lived. Keep this policy in sync with health_server.py, which owns
# refresh/paired state for the same cache file.
invite_ttl_ms="${FINITE_AGENT_INVITE_TTL_MS:-3600000}"
if [[ -f "$invite_file" ]]; then
    cp "$invite_file" /tmp/finitechat-invite.json
else
    "$finitechat_bin" hermes --home "$agent_home" invite \
        --room-name "$agent_name" \
        --max-joins 1 \
        --ttl-ms "$invite_ttl_ms" \
        --json \
        >"$invite_file"
    cp "$invite_file" /tmp/finitechat-invite.json
fi

cat >"${hermes_home}/config.yaml" <<EOF
model:
  default: ${model}
  provider: ${provider}
  base_url: ${base_url}
  api_mode: ${api_mode}
${api_key_yaml}
plugins:
  enabled:
    - ${plugin_name}
gateway:
  platforms:
    finitechat:
      enabled: true
      extra:
        home: ${agent_home}
        finitechat_bin: ${finitechat_bin}
        inbound_stream: true
        service_addr: ${service_addr}
        poll_timeout_secs: ${poll_timeout_secs}
        poll_limit: ${poll_limit}
terminal:
  backend: local
  cwd: ${workspace}
  persistent_shell: true
approvals:
  mode: off
display:
  streaming: false
security:
  redact_secrets: true
_config_version: 10
EOF

python /opt/health_server.py &
health_pid="$!"
trap 'kill "$health_pid" 2>/dev/null || true' EXIT

echo "FINITE_AGENT_RUNTIME real_hermes_gateway=true hermes_home=${hermes_home} agent_home=${agent_home}"
exec hermes gateway run --replace

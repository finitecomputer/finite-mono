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
api_key_reference=""
if [[ -n "${FINITECHAT_HERMES_API_KEY:-}" ]]; then
    api_key="${FINITECHAT_HERMES_API_KEY}"
    # shellcheck disable=SC2016 # Hermes expands this reference, not the shell.
    api_key_reference='${FINITECHAT_HERMES_API_KEY}'
elif [[ -n "${FINITE_PRIVATE_API_KEY:-}" ]]; then
    api_key="${FINITE_PRIVATE_API_KEY}"
    # shellcheck disable=SC2016 # Hermes expands this reference, not the shell.
    api_key_reference='${FINITE_PRIVATE_API_KEY}'
fi
service_addr="${FINITECHAT_HERMES_SERVICE_ADDR:-127.0.0.1:0}"
poll_timeout_secs="${FINITECHAT_HERMES_POLL_TIMEOUT_SECS:-1}"
poll_limit="${FINITECHAT_HERMES_POLL_LIMIT:-10}"
title_generation_timeout_secs="${FINITECHAT_HERMES_TITLE_TIMEOUT_SECS:-2}"
workspace="${FINITECHAT_WORKSPACE:-/workspace}"
managed_skills_dir="${agent_home}/managed-skills/finite/current"
bundled_skills_dir="${FINITE_BUNDLED_SKILLS_DIR:-/runtime/finite-skills}"
config_reconciler="${FINITE_HERMES_CONFIG_RECONCILER:-/opt/reconcile_hermes_config.py}"

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

if [[ ! -f "${hermes_home}/config.yaml" \
    && "${FINITE_DEFAULT_INFERENCE_PROFILE:-}" == "finite-private" \
    && -z "$api_key" ]]; then
    echo "FINITE_DEFAULT_INFERENCE_PROFILE=finite-private requires FINITE_PRIVATE_API_KEY; refusing OpenRouter fallback." >&2
    exit 64
fi

if [[ ! -f "${agent_home}/config.json" ]]; then
    if [[ -z "$server_url" ]]; then
        echo "FINITE_AGENT_START_ERROR missing FINITE_SERVER_URL for first initialization" >&2
        exit 64
    fi
    # New agents receive the image's Finite Skills baseline exactly once.
    # The durable directory belongs to the agent after this seed: image
    # upgrades and restarts never rewrite it. Existing agents update only via
    # the explicit `finite skills sync` path once that command is available.
    if [[ ! -d "$managed_skills_dir" && -d "$bundled_skills_dir" ]]; then
        mkdir -p "$(dirname "$managed_skills_dir")"
        managed_skills_staging="$(mktemp -d "${managed_skills_dir}.seed.XXXXXX")"
        if ! cp -a "${bundled_skills_dir}/." "$managed_skills_staging/" \
            || [[ ! -f "${managed_skills_staging}/software-development/finitebrain/SKILL.md" ]]; then
            rm -rf "$managed_skills_staging"
            echo "FINITE_AGENT_START_ERROR could not seed bundled Finite Skills" >&2
            exit 64
        fi
        mv "$managed_skills_staging" "$managed_skills_dir"
    elif [[ ! -d "$managed_skills_dir" && "${FINITE_REQUIRE_BUNDLED_SKILLS:-0}" == "1" ]]; then
        echo "FINITE_AGENT_START_ERROR missing bundled Finite Skills at $bundled_skills_dir" >&2
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

# Room admission is Welcome-first: the Hosted Web Device publishes its
# KeyPackage and starts a profile chat with the Agent Principal. The runtime
# must not recreate the deleted invite-session protocol or invent a room before
# a real Device asks to chat. Once a room exists, normal inbound routing and an
# explicit local home-channel choice remain owned by Finite Chat/Hermes state.

managed_skills_config_dir=""
if [[ -d "$managed_skills_dir" ]]; then
    managed_skills_config_dir="$managed_skills_dir"
fi

# Seed product defaults only when config.yaml is absent. Thereafter the image
# repairs only the Finite Chat transport and managed-skills registration. In
# particular, model/provider settings and Telegram/other Hermes platforms are
# Hermes/user-owned and must survive every runtime restart and image upgrade.
FINITE_CONFIG_MODEL="$model" \
FINITE_CONFIG_PROVIDER="$provider" \
FINITE_CONFIG_BASE_URL="$base_url" \
FINITE_CONFIG_API_MODE="$api_mode" \
FINITE_CONFIG_API_KEY_REFERENCE="$api_key_reference" \
FINITE_CONFIG_PLUGIN_NAME="$plugin_name" \
FINITE_CONFIG_TITLE_TIMEOUT_SECS="$title_generation_timeout_secs" \
FINITE_CONFIG_AGENT_HOME="$agent_home" \
FINITE_CONFIG_FINITECHAT_BIN="$finitechat_bin" \
FINITE_CONFIG_SERVICE_ADDR="$service_addr" \
FINITE_CONFIG_POLL_TIMEOUT_SECS="$poll_timeout_secs" \
FINITE_CONFIG_POLL_LIMIT="$poll_limit" \
FINITE_CONFIG_HOME_CHANNEL="${FINITECHAT_HOME_CHANNEL:-}" \
FINITE_CONFIG_MANAGED_SKILLS_DIR="$managed_skills_config_dir" \
FINITE_CONFIG_WORKSPACE="$workspace" \
python "$config_reconciler" --config "${hermes_home}/config.yaml"

if [[ "${1:-}" == "--prepare-only" ]]; then
    echo "FINITE_AGENT_RUNTIME_PREPARED hermes_home=${hermes_home} agent_home=${agent_home}"
    exit 0
fi

if [[ "${FINITE_AGENTD_SUPERVISED:-0}" != "1" ]]; then
    python /opt/health_server.py &
    health_pid="$!"
    trap 'kill "$health_pid" 2>/dev/null || true' EXIT
fi

echo "FINITE_AGENT_RUNTIME real_hermes_gateway=true hermes_home=${hermes_home} agent_home=${agent_home}"
exec hermes gateway run --replace

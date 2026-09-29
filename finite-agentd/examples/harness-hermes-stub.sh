#!/usr/bin/env bash
# E-0 stand-in for finitechat/containers/agent/run_hermes_gateway.sh.
#
# It runs the REAL config reconciler and the REAL pending-disconnect step
# (`python -m hermes_cli.finite_inference_helper apply-pending-disconnect`),
# with the launcher's environment for both, then `exec sleep` in place of the
# Hermes gateway. It never starts a gateway. The launcher itself cannot be
# sourced (it runs at top level), so the variables below repeat its §5.4 and
# §5.6 computations; keep them in step with it.
#
# Harness-only inputs: E0_REPO (repository root), E0_RUN_DIR (scratch run
# directory). Each start appends to "$E0_RUN_DIR/events.log": a `gateway-spawn`
# line on entry, then either `gateway-ready` just before the exec or
# `gateway-stopped` with the stage it was in when a restart stopped it. The
# optional "$E0_RUN_DIR/stub-mode" file selects a behavior: `readd-env-key` (a
# stale writer), `slow-step` (the pending-disconnect step starts 10 s late, as
# a slow start would), or `cut-step-twice` (the first two starts that run the
# step hit the launcher's 20 s limit after clearing the pool entry and before
# the conversation override, and agentd's facts reads hang while they run, as
# on a machine at load 30; later starts run the step as usual). A
# `gateway-ready` line also carries `load` (the 1-minute load average when the
# step stage began), `step_ms` (the step alone, `none` without an intent) and
# `cut` (whether this stub cut the step on purpose).
set -euo pipefail

: "${E0_REPO:?E0_REPO is required}"
: "${E0_RUN_DIR:?E0_RUN_DIR is required}"

readonly CANONICAL_FINITE_PRIVATE_MODEL="glm-5-3-flash"
readonly FINITE_PRIVATE_PRODUCT_BASE_URL="https://finite-private.finite.containers.tinfoil.dev/v1"
readonly HISTORICAL_FINITE_PRIVATE_BASE_URL="https://kimi-k2-6.finite.containers.tinfoil.dev/v1"

is_legacy_finite_private_model() {
    case "$1" in
        glm-5-2|deepseek-v4-flash-0731|glm-5.3-flash) return 0 ;;
        *) return 1 ;;
    esac
}

now_ms() {
    # Bash 5's clock costs no process start; fall back to Python.
    if [[ -n "${EPOCHREALTIME:-}" ]]; then
        local now="${EPOCHREALTIME/./}"
        echo "${now:0:13}"
    else
        python -c 'import time; print(int(time.time() * 1000))'
    fi
}

agent_home="${FINITECHAT_HOME:?FINITECHAT_HOME is required}"
hermes_home="${HERMES_HOME:?HERMES_HOME is required}"
events="$E0_RUN_DIR/events.log"
reconciler="$E0_REPO/finitechat/containers/agent/reconcile_hermes_config.py"

intent_state() {
    local path="${FINITE_AGENTD_INTENT_PATH:-}"
    if [[ -z "$path" || ! -f "$path" ]]; then
        echo none
        return
    fi
    python -c 'import json,sys; r=json.load(open(sys.argv[1])); print(r.get("kind"), r.get("phase"), r.get("state"), sep="/")' "$path" 2>/dev/null || echo unreadable
}

stage="start"
if [[ "${1:-}" != "--prepare-only" ]]; then
    spawned="$(now_ms)"
    trap 'rm -f "$E0_RUN_DIR/facts-slow"; echo "$(now_ms) gateway-stopped pid=$$ stage=$stage" >> "$events"; exit 143' TERM
    # When agentd forked this process, by the kernel's clock: `exec` keeps the
    # pid and its creation time, so this does not depend on how long the
    # process took to get here.
    created="$(python -c 'import psutil, sys; print(int(psutil.Process(int(sys.argv[1])).create_time() * 1000))' "$$" 2>/dev/null || echo 0)"
    echo "$spawned gateway-spawn pid=$$ created=$created intent=$(intent_state)" >> "$events"
fi

# The finite-private default profile, as the launcher computes it.
model="${FINITE_PRIVATE_MODEL:-$CANONICAL_FINITE_PRIVATE_MODEL}"
provider="custom"
base_url="${FINITE_PRIVATE_BASE_URL:-$FINITE_PRIVATE_PRODUCT_BASE_URL}"
if [[ "$base_url" == "$HISTORICAL_FINITE_PRIVATE_BASE_URL" ]]; then
    base_url="$FINITE_PRIVATE_PRODUCT_BASE_URL"
fi
if is_legacy_finite_private_model "$model" && [[ "$base_url" == "$FINITE_PRIVATE_PRODUCT_BASE_URL" ]]; then
    model="$CANONICAL_FINITE_PRIVATE_MODEL"
fi
context_length="${FINITE_PRIVATE_CONTEXT_LENGTH:-393216}"
api_key_reference=""
if [[ -n "${FINITE_PRIVATE_API_KEY:-}" ]]; then
    # shellcheck disable=SC2016 # Hermes expands this reference, not the shell.
    api_key_reference='${FINITE_PRIVATE_API_KEY}'
fi
finite_private_model="${FINITE_PRIVATE_MODEL:-}"
finite_private_base_url="${FINITE_PRIVATE_BASE_URL:-}"
if [[ "$finite_private_base_url" == "$HISTORICAL_FINITE_PRIVATE_BASE_URL" ]]; then
    finite_private_base_url="$FINITE_PRIVATE_PRODUCT_BASE_URL"
fi
if is_legacy_finite_private_model "$finite_private_model" \
    && [[ "$finite_private_base_url" == "$FINITE_PRIVATE_PRODUCT_BASE_URL" ]]; then
    finite_private_model="$CANONICAL_FINITE_PRIVATE_MODEL"
fi
finite_private_key_present=0
if [[ -n "${FINITE_PRIVATE_API_KEY:-}" ]]; then
    finite_private_key_present=1
fi
# §5.6 launch hygiene, before the reconciler and the step.
if [[ -n "${FINITE_PRIVATE_API_KEY:-}" && "${OPENAI_API_KEY:-}" == "$FINITE_PRIVATE_API_KEY" ]]; then
    unset OPENAI_API_KEY
fi
export CODEX_HOME=/dev/null/finite-codex-home-disabled

run_with_config_environment() {
    FINITE_CONFIG_MODEL="$model" \
    FINITE_CONFIG_PROVIDER="$provider" \
    FINITE_CONFIG_BASE_URL="$base_url" \
    FINITE_CONFIG_CONTEXT_LENGTH="$context_length" \
    FINITE_CONFIG_API_MODE="chat_completions" \
    FINITE_CONFIG_API_KEY_REFERENCE="$api_key_reference" \
    FINITE_CONFIG_PLUGIN_NAME="finitechat" \
    FINITE_CONFIG_TITLE_TIMEOUT_SECS="2" \
    FINITE_CONFIG_AGENT_HOME="$agent_home" \
    FINITE_CONFIG_FINITECHAT_BIN="${FINITECHAT_BIN:-finitechat}" \
    FINITE_CONFIG_SERVICE_ADDR="127.0.0.1:0" \
    FINITE_CONFIG_POLL_TIMEOUT_SECS="1" \
    FINITE_CONFIG_POLL_LIMIT="10" \
    FINITE_CONFIG_HOME_CHANNEL="" \
    FINITE_CONFIG_MANAGED_SKILLS_DIR="" \
    FINITE_CONFIG_WORKSPACE="$agent_home/workspace" \
    FINITE_CONFIG_FP_MODEL="$finite_private_model" \
    FINITE_CONFIG_FP_BASE_URL="$finite_private_base_url" \
    FINITE_CONFIG_FP_CONTEXT_LENGTH="${FINITE_PRIVATE_CONTEXT_LENGTH:-}" \
    FINITE_CONFIG_FP_KEY_PRESENT="$finite_private_key_present" \
    FINITE_CONFIG_FP_FALLBACK_MODE="${FINITE_PRIVATE_FALLBACK_MODE:-seed}" \
    "$@"
}

state_digest() {
    local file
    for file in auth.json state.db state.db-wal sessions/sessions.json; do
        if [[ -f "$hermes_home/$file" ]]; then
            cat "$hermes_home/$file"
        fi
    done | shasum -a 256 | cut -c1-16
}

stage="reconciler"
mkdir -p "$agent_home/workspace" "$hermes_home/plugins"
run_with_config_environment python "$reconciler" --config "$hermes_home/config.yaml"

if [[ "${1:-}" == "--prepare-only" ]]; then
    echo "FINITE_AGENT_RUNTIME_PREPARED hermes_home=${hermes_home} agent_home=${agent_home}"
    exit 0
fi

# The launcher's pending-disconnect step, invoked exactly as the launcher does.
stage="step"
phase="none"
step="none"
state_changed="no"
step_ms="none"
cut="no"
load="$(sysctl -n vm.loadavg 2>/dev/null | awk '{print $2}' || true)"
load="${load:-$(cut -d' ' -f1 /proc/loadavg 2>/dev/null || echo none)}"
intent_path="${FINITE_AGENTD_INTENT_PATH:-}"
mode="$(cat "$E0_RUN_DIR/stub-mode" 2>/dev/null || echo none)"
if [[ -n "$intent_path" && -f "$intent_path" ]]; then
    if [[ "$mode" == "slow-step" ]]; then
        sleep 10
    fi
    phase="$(intent_state)"
    before="$(state_digest)"
    step_command=(-m hermes_cli.finite_inference_helper)
    cuts="$(cat "$E0_RUN_DIR/cut-steps" 2>/dev/null || echo 0)"
    if [[ "$mode" == "cut-step-twice" && "$cuts" -lt 2 ]]; then
        echo "$((cuts + 1))" > "$E0_RUN_DIR/cut-steps"
        touch "$E0_RUN_DIR/facts-slow"
        cut="yes"
        # The helper's own step, delayed between its two clears.
        step_command=(-c 'import sys, time
from hermes_cli import finite_inference_helper as helper
clear = helper.clear_session_overrides
helper.clear_session_overrides = lambda providers: (time.sleep(60), clear(providers))[1]
sys.exit(helper.main(sys.argv[1:]))')
    fi
    step_started="$(now_ms)"
    status=0
    step="$(run_with_config_environment \
        timeout -k 5 20 python "${step_command[@]}" \
        apply-pending-disconnect --intent "$intent_path" </dev/null 2>/dev/null)" || status=$?
    step_ms="$(( $(now_ms) - step_started ))"
    rm -f "$E0_RUN_DIR/facts-slow"
    if [[ "$status" -ne 0 ]]; then
        step="failed:$status"
    fi
    if [[ "$(state_digest)" != "$before" ]]; then
        state_changed="yes"
    fi
fi

# A stale writer (§13.3 E-0): puts the removed key back on every start.
stage="stale-writer"
if [[ "$mode" == "readd-env-key" ]]; then
    echo "OPENROUTER_API_KEY=sk-or-v1-e0-fake-stale-writer" >> "$hermes_home/.env"
fi

trap - TERM
echo "$(now_ms) gateway-ready pid=$$ intent=$phase step=$step hermes_state_changed=$state_changed mode=$mode step_ms=$step_ms load=$load cut=$cut" >> "$events"
exec sleep 1000000

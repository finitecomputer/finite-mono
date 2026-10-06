#!/usr/bin/env bash
# E-0 stand-in for finitechat/containers/agent/run_hermes_gateway.sh.
#
# Uses the harness's canonical Finite Private fixture, real reconciler and
# pending-disconnect helper, then sleeps instead of starting a gateway.
# Production launcher migration/first-seed behavior has its own container tests.
# events.log records process creation, completion or interruption. stub-mode
# injects a stale writer, a slow step, or two steps cut between auth/session clears.
set -euo pipefail

: "${E0_REPO:?E0_REPO is required}"
: "${E0_RUN_DIR:?E0_RUN_DIR is required}"

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

# Run::prepare supplies canonical, synthetic settings; do not duplicate the
# production launcher's legacy migration logic in this fixture.
model="${FINITE_PRIVATE_MODEL:?}"
base_url="${FINITE_PRIVATE_BASE_URL:?}"
context_length="${FINITE_PRIVATE_CONTEXT_LENGTH:?}"
: "${FINITE_PRIVATE_API_KEY:?}"
# launch hygiene, before the reconciler and the step.
if [[ -n "${FINITE_PRIVATE_API_KEY:-}" && "${OPENAI_API_KEY:-}" == "$FINITE_PRIVATE_API_KEY" ]]; then
    unset OPENAI_API_KEY
fi
export CODEX_HOME=/dev/null/finite-codex-home-disabled

# shellcheck disable=SC2016 # Hermes expands the key reference.
run_with_config_environment() {
    FINITE_CONFIG_MODEL="$model" \
    FINITE_CONFIG_PROVIDER="custom" \
    FINITE_CONFIG_BASE_URL="$base_url" \
    FINITE_CONFIG_CONTEXT_LENGTH="$context_length" \
    FINITE_CONFIG_API_MODE="chat_completions" \
    FINITE_CONFIG_API_KEY_REFERENCE='${FINITE_PRIVATE_API_KEY}' \
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
    FINITE_CONFIG_FP_MODEL="$model" \
    FINITE_CONFIG_FP_BASE_URL="$base_url" \
    FINITE_CONFIG_FP_CONTEXT_LENGTH="$context_length" \
    FINITE_CONFIG_FP_KEY_PRESENT="1" \
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

# A stale writer (E-0): puts the removed key back on every start.
stage="stale-writer"
if [[ "$mode" == "readd-env-key" ]]; then
    echo "OPENROUTER_API_KEY=sk-or-v1-e0-fake-stale-writer" >> "$hermes_home/.env"
fi

trap - TERM
echo "$(now_ms) gateway-ready pid=$$ intent=$phase step=$step hermes_state_changed=$state_changed mode=$mode step_ms=$step_ms cut=$cut" >> "$events"
exec sleep 1000000

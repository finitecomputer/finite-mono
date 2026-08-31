#!/usr/bin/env bash
set -euo pipefail

truthy() {
    case "${1:-}" in
        1|true|TRUE|yes|YES|on|ON) return 0 ;;
        *) return 1 ;;
    esac
}

agent_home="${FINITECHAT_HOME:-/data/agent}"
workspace="${FINITECHAT_WORKSPACE:-/data/workspace}"
export FBRAIN_WORKING_TREE_ROOT="${FBRAIN_WORKING_TREE_ROOT:-$workspace/finitebrain}"
# The shared Finite identity (identity/identity.json) must live on the same
# durable mount as the rest of the agent state so restarts keep the account key.
export FINITE_HOME="${FINITE_HOME:-$agent_home}"

if ! truthy "${FINITE_AGENT_SUPERVISE:-1}"; then
    exec "$@"
fi

"$@" &
child_pid="$!"
brain_sync_pid=""
if truthy "${FINITE_BRAIN_SYNC_SUPERVISOR:-1}" && command -v fbrain >/dev/null 2>&1; then
    mkdir -p "$FBRAIN_WORKING_TREE_ROOT"
    (
        backoff=1
        while kill -0 "$child_pid" 2>/dev/null; do
            fbrain daemon supervise >>"${FINITE_BRAIN_SYNC_LOG:-/tmp/fbrain-supervisor.log}" 2>&1 || true
            echo "FINITE_BRAIN_SYNC_SUPERVISOR_RESTART backoff_secs=$backoff" >&2
            sleep "$backoff"
            if [[ "$backoff" -lt 30 ]]; then
                backoff=$((backoff * 2))
            fi
        done
    ) &
    brain_sync_pid="$!"
    echo "FINITE_BRAIN_SYNC_SUPERVISOR_START pid=$brain_sync_pid"
fi
child_status=0
terminating=0

shutdown() {
    if [[ "$terminating" -eq 1 ]]; then
        return
    fi
    terminating=1
    if [[ -n "$brain_sync_pid" ]] && kill -0 "$brain_sync_pid" 2>/dev/null; then
        kill -TERM "$brain_sync_pid" 2>/dev/null || true
    fi
    if kill -0 "$child_pid" 2>/dev/null; then
        kill -TERM "$child_pid" 2>/dev/null || true
    fi
}

trap shutdown TERM INT

wait "$child_pid" || child_status="$?"
if [[ "$terminating" -eq 1 ]] && kill -0 "$child_pid" 2>/dev/null; then
    wait "$child_pid" || child_status="$?"
fi
# Post-exit orphan sweep (backported from PR 440's generation quiesce):
# finite-agentd serve leads its own process group (setsid when PID 1 is its
# parent), so its pid is its pgid and everything it spawned — the Finite Chat
# bridge on :37633, the health server, Hermes, gateway-forked stragglers —
# must not outlive it. An orphaned bridge once held :37633 across a restart
# and the next boot's bridge exited at bind. This is a no-op when the group
# is already gone (agentd's own graceful drain emptied it) or when agentd
# never became a group leader (ESRCH); it can never hit this script's own
# group, whose pgid is PID 1, not the child's.
kill -KILL -- "-$child_pid" 2>/dev/null || true
if [[ -n "$brain_sync_pid" ]]; then
    if kill -0 "$brain_sync_pid" 2>/dev/null; then
        kill -TERM "$brain_sync_pid" 2>/dev/null || true
    fi
    wait "$brain_sync_pid" 2>/dev/null || true
fi
exit "$child_status"

#!/usr/bin/env bash
set -euo pipefail

truthy() {
    case "${1:-}" in
        1|true|TRUE|yes|YES|on|ON) return 0 ;;
        *) return 1 ;;
    esac
}

agent_home="${FINITECHAT_HOME:-/data/agent}"
state_root="${FINITE_AGENT_STATE_ROOT:-/data}"
workspace="${FINITECHAT_WORKSPACE:-/data/workspace}"
export FBRAIN_WORKING_TREE_ROOT="${FBRAIN_WORKING_TREE_ROOT:-$workspace/finitebrain}"
# The shared Finite identity (identity/identity.json) must live on the same
# durable mount as the rest of the agent state so restore/backup and
# restarts keep the account key.
export FINITE_HOME="${FINITE_HOME:-$agent_home}"

restic_repository() {
    printf '%s' "${FINITE_AGENT_RESTIC_REPOSITORY:-${FINITE_DOCKER_RESTIC_REPOSITORY:-}}"
}

restic_password() {
    printf '%s' "${FINITE_AGENT_RESTIC_PASSWORD:-${FINITE_DOCKER_RESTIC_PASSWORD:-}}"
}

positive_integer() {
    [[ "${1:-}" =~ ^[0-9]+$ ]] && [[ "$1" -gt 0 ]]
}

export_restic_env() {
    local password="$1"
    export RESTIC_PASSWORD="$password"
    export RESTIC_CACHE_DIR="${RESTIC_CACHE_DIR:-/tmp/restic-cache}"
}

backup_activity_active() {
    local marker="${FINITE_AGENT_BACKUP_ACTIVITY_FILE:-$agent_home/.finitechat-backup-active}"
    if [[ ! -e "$marker" ]]; then
        return 1
    fi

    local stale_secs="${FINITE_AGENT_BACKUP_ACTIVITY_STALE_SECS:-300}"
    local now
    now="$(date +%s)"
    local mtime
    mtime="$(stat -c %Y "$marker" 2>/dev/null || printf '%s' "$now")"
    local age=$((now - mtime))
    if [[ "$age" -lt "$stale_secs" ]]; then
        echo "FINITE_AGENT_BACKUP_SKIPPED activity_active=true age_secs=$age marker=$marker"
        return 0
    fi
    echo "FINITE_AGENT_BACKUP_ACTIVITY_STALE age_secs=$age marker=$marker"
    return 1
}

restore_agent_state() {
    if ! truthy "${FINITE_AGENT_RESTORE_ON_START:-0}"; then
        return 0
    fi

    if [[ -f "$agent_home/config.json" ]] && ! truthy "${FINITE_AGENT_RESTORE_FORCE:-0}"; then
        echo "FINITE_AGENT_RESTORE_SKIPPED existing_state=true home=$agent_home"
        return 0
    fi

    local repository
    repository="$(restic_repository)"
    local password
    password="$(restic_password)"
    local snapshot="${FINITE_AGENT_RESTIC_SNAPSHOT_ID:-}"
    local tag="${FINITE_AGENT_RESTIC_BACKUP_TAG:-${FINITE_DOCKER_RESTIC_SNAPSHOT_TAG:-finite-agent-state}}"
    local target="${FINITE_AGENT_RESTIC_RESTORE_TARGET:-/}"

    if [[ -z "$repository" ]]; then
        echo "FINITE_AGENT_RESTORE_ERROR missing FINITE_AGENT_RESTIC_REPOSITORY" >&2
        return 64
    fi
    if [[ -z "$password" ]]; then
        echo "FINITE_AGENT_RESTORE_ERROR missing FINITE_AGENT_RESTIC_PASSWORD" >&2
        return 64
    fi
    if [[ -z "$snapshot" ]] && ! truthy "${FINITE_AGENT_RESTORE_LATEST:-0}"; then
        echo "FINITE_AGENT_RESTORE_ERROR missing FINITE_AGENT_RESTIC_SNAPSHOT_ID or FINITE_AGENT_RESTORE_LATEST=1" >&2
        return 64
    fi

    mkdir -p "$agent_home"
    export_restic_env "$password"
    if [[ -n "$snapshot" ]]; then
        echo "FINITE_AGENT_RESTORE_START snapshot=$snapshot home=$agent_home"
        restic -r "$repository" restore "$snapshot" --target "$target"
        echo "FINITE_AGENT_RESTORE_COMPLETE snapshot=$snapshot home=$agent_home"
    else
        echo "FINITE_AGENT_RESTORE_START snapshot=latest tag=$tag home=$agent_home"
        restic -r "$repository" restore latest --tag "$tag" --target "$target"
        echo "FINITE_AGENT_RESTORE_COMPLETE snapshot=latest tag=$tag home=$agent_home"
    fi
}

backup_agent_state() {
    if ! truthy "${FINITE_AGENT_BACKUP_ON_EXIT:-0}"; then
        return 0
    fi

    local repository
    repository="$(restic_repository)"
    local password
    password="$(restic_password)"
    local tag="${FINITE_AGENT_RESTIC_BACKUP_TAG:-${FINITE_DOCKER_RESTIC_SNAPSHOT_TAG:-finite-agent-state}}"

    if [[ -z "$repository" ]]; then
        echo "FINITE_AGENT_BACKUP_ERROR missing FINITE_AGENT_RESTIC_REPOSITORY" >&2
        return 64
    fi
    if [[ -z "$password" ]]; then
        echo "FINITE_AGENT_BACKUP_ERROR missing FINITE_AGENT_RESTIC_PASSWORD" >&2
        return 64
    fi
    if [[ ! -d "$state_root" ]]; then
        echo "FINITE_AGENT_BACKUP_SKIPPED missing_state_root=true state_root=$state_root"
        return 0
    fi

    local canonical_state_root
    canonical_state_root="$(realpath -m "$state_root")"
    local canonical_agent_home
    canonical_agent_home="$(realpath -m "$agent_home")"
    local canonical_workspace
    canonical_workspace="$(realpath -m "$workspace")"
    case "$canonical_agent_home" in
        "$canonical_state_root"/*) ;;
        *)
            echo "FINITE_AGENT_BACKUP_ERROR agent home is outside state root" >&2
            return 64
            ;;
    esac
    case "$canonical_workspace" in
        "$canonical_state_root"/*) ;;
        *)
            echo "FINITE_AGENT_BACKUP_ERROR workspace is outside state root" >&2
            return 64
            ;;
    esac
    if backup_activity_active; then
        return 0
    fi

    local lock_dir="${FINITE_AGENT_BACKUP_LOCK_DIR:-/tmp/finite-agent-backup.lock}"
    if ! mkdir "$lock_dir" 2>/dev/null; then
        echo "FINITE_AGENT_BACKUP_SKIPPED backup_running=true state_root=$state_root tag=$tag"
        return 0
    fi

    export_restic_env "$password"
    echo "FINITE_AGENT_BACKUP_START state_root=$state_root tag=$tag"
    local status=0
    restic -r "$repository" backup "$state_root" --tag "$tag" --json || status="$?"
    rmdir "$lock_dir" 2>/dev/null || true
    if [[ "$status" -ne 0 ]]; then
        echo "FINITE_AGENT_BACKUP_ERROR restic_status=$status state_root=$state_root tag=$tag" >&2
        return "$status"
    fi
    echo "FINITE_AGENT_BACKUP_COMPLETE state_root=$state_root tag=$tag"
}

start_periodic_backup() {
    local interval="${FINITE_AGENT_BACKUP_INTERVAL_SECS:-0}"
    if [[ -z "$interval" || "$interval" == "0" ]]; then
        return 0
    fi
    if ! positive_integer "$interval"; then
        echo "FINITE_AGENT_BACKUP_ERROR invalid FINITE_AGENT_BACKUP_INTERVAL_SECS=$interval" >&2
        return 64
    fi
    if ! truthy "${FINITE_AGENT_BACKUP_ON_EXIT:-0}"; then
        echo "FINITE_AGENT_BACKUP_ERROR FINITE_AGENT_BACKUP_INTERVAL_SECS requires FINITE_AGENT_BACKUP_ON_EXIT=1" >&2
        return 64
    fi

    (
        while true; do
            sleep "$interval" || exit 0
            backup_agent_state || true
        done
    ) &
    backup_loop_pid="$!"
    echo "FINITE_AGENT_BACKUP_PERIODIC_START interval_secs=$interval pid=$backup_loop_pid"
}

stop_periodic_backup() {
    if [[ -n "${backup_loop_pid:-}" ]] && kill -0 "$backup_loop_pid" 2>/dev/null; then
        local lock_dir="${FINITE_AGENT_BACKUP_LOCK_DIR:-/tmp/finite-agent-backup.lock}"
        local wait_secs="${FINITE_AGENT_BACKUP_STOP_WAIT_SECS:-30}"
        local waited=0
        while [[ -d "$lock_dir" && "$waited" -lt "$wait_secs" ]]; do
            if [[ "$waited" -eq 0 ]]; then
                echo "FINITE_AGENT_BACKUP_PERIODIC_WAIT backup_running=true wait_secs=$wait_secs"
            fi
            sleep 1
            waited=$((waited + 1))
        done
        kill -TERM "$backup_loop_pid" 2>/dev/null || true
        wait "$backup_loop_pid" 2>/dev/null || true
    fi
}

restore_agent_state

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
backup_loop_pid=""
child_status=0
terminating=0
backup_loop_status=0
start_periodic_backup || backup_loop_status="$?"
if [[ "$backup_loop_status" -ne 0 ]]; then
    kill -TERM "$child_pid" 2>/dev/null || true
    wait "$child_pid" 2>/dev/null || true
    exit "$backup_loop_status"
fi

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
stop_periodic_backup
if [[ -n "$brain_sync_pid" ]]; then
    if kill -0 "$brain_sync_pid" 2>/dev/null; then
        kill -TERM "$brain_sync_pid" 2>/dev/null || true
    fi
    wait "$brain_sync_pid" 2>/dev/null || true
fi
backup_agent_state
exit "$child_status"

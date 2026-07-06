#!/usr/bin/env bash
set -euo pipefail

truthy() {
    case "${1:-}" in
        1|true|TRUE|yes|YES|on|ON) return 0 ;;
        *) return 1 ;;
    esac
}

agent_home="${FINITECHAT_HOME:-/data/agent}"
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
    if [[ ! -d "$agent_home" ]]; then
        echo "FINITE_AGENT_BACKUP_SKIPPED missing_home=true home=$agent_home"
        return 0
    fi
    if backup_activity_active; then
        return 0
    fi

    local lock_dir="${FINITE_AGENT_BACKUP_LOCK_DIR:-/tmp/finite-agent-backup.lock}"
    if ! mkdir "$lock_dir" 2>/dev/null; then
        echo "FINITE_AGENT_BACKUP_SKIPPED backup_running=true home=$agent_home tag=$tag"
        return 0
    fi

    export_restic_env "$password"
    echo "FINITE_AGENT_BACKUP_START home=$agent_home tag=$tag"
    local status=0
    restic -r "$repository" backup "$agent_home" --tag "$tag" --json || status="$?"
    rmdir "$lock_dir" 2>/dev/null || true
    if [[ "$status" -ne 0 ]]; then
        echo "FINITE_AGENT_BACKUP_ERROR restic_status=$status home=$agent_home tag=$tag" >&2
        return "$status"
    fi
    echo "FINITE_AGENT_BACKUP_COMPLETE home=$agent_home tag=$tag"
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
    if kill -0 "$child_pid" 2>/dev/null; then
        kill -TERM "$child_pid" 2>/dev/null || true
    fi
}

trap shutdown TERM INT

wait "$child_pid" || child_status="$?"
if [[ "$terminating" -eq 1 ]] && kill -0 "$child_pid" 2>/dev/null; then
    wait "$child_pid" || child_status="$?"
fi
stop_periodic_backup
backup_agent_state
exit "$child_status"

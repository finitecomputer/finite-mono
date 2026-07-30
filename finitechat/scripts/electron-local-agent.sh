#!/usr/bin/env bash
# Launches an isolated Electron Device against the authority-free local-agent stack.
set -euo pipefail
umask 077

finitechat_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mono_root="$(cd "${finitechat_root}/.." && pwd)"
if [[ -z "${IN_NIX_SHELL:-}" && "${FINITE_MONO_DEV_ENV_ACTIVE:-}" != "1" ]]; then
  exec "${mono_root}/scripts/with-dev-env" "${BASH_SOURCE[0]}" "$@"
fi

dashboard_url="${FINITECHAT_ELECTRON_LOCAL_DASHBOARD_URL:-http://127.0.0.1:23002}"
server_url="${FINITECHAT_ELECTRON_LOCAL_SERVER_URL:-http://127.0.0.1:28788}"
state_root="${FINITECHAT_ELECTRON_LOCAL_STATE_ROOT:-${finitechat_root}/.state/electron-local-agent}"
app_root="${finitechat_root}/apps/electron-chat"
daemon_binary="${mono_root}/target/debug/finitechatd"
remote_debugging_port="${FINITECHAT_ELECTRON_REMOTE_DEBUGGING_PORT:-}"

for endpoint in "${dashboard_url}/api/device-links/account-binding" "${server_url}/health"; do
  if ! curl -fsS "${endpoint}" >/dev/null; then
    echo "The local-agent stack is not ready at ${endpoint}." >&2
    echo "Start it with: just dev ios-local-agent-stress" >&2
    exit 1
  fi
done

echo "Building the local Electron daemon..."
(
  cd "${mono_root}"
  cargo build -q -p finitechat-daemon
)

mkdir -p "${state_root}"
echo "Launching an isolated Electron Device from ${state_root}..."
cd "${app_root}"
electron_args=(.)
if [[ -n "${remote_debugging_port}" ]]; then
  if [[ ! "${remote_debugging_port}" =~ ^[0-9]+$ ]] \
    || ((remote_debugging_port < 1024 || remote_debugging_port > 65535))
  then
    echo "FINITECHAT_ELECTRON_REMOTE_DEBUGGING_PORT must be an unprivileged TCP port." >&2
    exit 1
  fi
  electron_args+=(
    "--remote-debugging-address=127.0.0.1"
    "--remote-debugging-port=${remote_debugging_port}"
  )
fi
FINITECHAT_DASHBOARD_URL="${dashboard_url}" \
FINITECHAT_SERVER_URL="${server_url}" \
FINITECHAT_DAEMON_BINARY="${daemon_binary}" \
FINITECHAT_USER_DATA_DIR="${state_root}/profile" \
FINITECHAT_DISABLE_SINGLE_INSTANCE_LOCK=1 \
  exec "${app_root}/node_modules/.bin/electron" "${electron_args[@]}"

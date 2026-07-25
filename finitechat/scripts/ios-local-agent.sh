#!/usr/bin/env bash
# Real local chat server + hosted human device + dashboard link + Hermes + iOS.
set -euo pipefail
umask 077

finitechat_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mono_root="$(cd "${finitechat_root}/.." && pwd)"
if [[ -z "${IN_NIX_SHELL:-}" && "${FINITE_MONO_DEV_ENV_ACTIVE:-}" != "1" ]]; then
  exec "${mono_root}/scripts/with-dev-env" "${BASH_SOURCE[0]}" "$@"
fi
dashboard_root="${mono_root}/finitecomputer-v2/apps/dashboard"
state_root="${FINITECHAT_IOS_LOCAL_STATE_ROOT:-${finitechat_root}/.state/ios-local-agent}"
hermes_state="${state_root}/hermes"
hosted_state="${state_root}/hosted-device"
chat_port="${FINITECHAT_IOS_LOCAL_CHAT_PORT:-28788}"
hosted_port="${FINITECHAT_IOS_LOCAL_HOSTED_PORT:-48918}"
dashboard_port="${FINITECHAT_IOS_LOCAL_DASHBOARD_PORT:-23002}"
chat_url="http://127.0.0.1:${chat_port}"
hosted_url="http://127.0.0.1:${hosted_port}"
dashboard_url="http://127.0.0.1:${dashboard_port}"
hosted_token="$(openssl rand -hex 32)"
dev_access_token="$(openssl rand -hex 32)"
cookie_password="$(openssl rand -hex 32)"
workos_user_id="user_ios_local"
workos_email="ios-local@finite.invalid"
ios_device_id="${FINITECHAT_IOS_LOCAL_DEVICE_ID:-ios-local-$(openssl rand -hex 6)}"
stress_message_count="${FINITECHAT_IOS_LOCAL_STRESS_MESSAGE_COUNT:-0}"
stress_chat_count="${FINITECHAT_IOS_LOCAL_STRESS_CHAT_COUNT:-81}"
cargo_target_dir="${finitechat_root}/target"
hermes_pid=""
hosted_pid=""
dashboard_pid=""

cleanup() {
  local status=$?
  trap - EXIT INT TERM
  for pid in "${dashboard_pid}" "${hosted_pid}" "${hermes_pid}"; do
    if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then
      kill "${pid}" 2>/dev/null || true
    fi
  done
  if [[ -f "${hermes_state}/server.pid" ]]; then
    local server_pid
    server_pid="$(<"${hermes_state}/server.pid")"
    if [[ "${server_pid}" =~ ^[0-9]+$ ]] && kill -0 "${server_pid}" 2>/dev/null; then
      kill "${server_pid}" 2>/dev/null || true
    fi
  fi
  if [[ -f "${hermes_state}/service.pid" ]]; then
    local service_pid
    service_pid="$(<"${hermes_state}/service.pid")"
    if [[ "${service_pid}" =~ ^[0-9]+$ ]] && kill -0 "${service_pid}" 2>/dev/null; then
      kill "${service_pid}" 2>/dev/null || true
    fi
  fi
  wait 2>/dev/null || true
  exit "${status}"
}
trap cleanup EXIT INT TERM

wait_for_url() {
  local name="$1"
  local url="$2"
  local pid="$3"
  for _ in {1..240}; do
    if curl -fsS "${url}" >/dev/null 2>&1; then
      return 0
    fi
    if [[ -n "${pid}" ]] && ! kill -0 "${pid}" 2>/dev/null; then
      echo "${name} stopped before becoming ready." >&2
      return 1
    fi
    sleep 0.25
  done
  echo "Timed out waiting for ${name} at ${url}." >&2
  return 1
}

require_port_available() {
  local name="$1"
  local port="$2"
  if (exec 3<>"/dev/tcp/127.0.0.1/${port}") 2>/dev/null; then
    exec 3>&-
    exec 3<&-
    echo "${name} port ${port} is already in use." >&2
    echo "Stop the previous local-agent stack or choose a different FINITECHAT_IOS_LOCAL_*_PORT." >&2
    return 1
  fi
}

require_port_available "chat server" "${chat_port}"
require_port_available "hosted device" "${hosted_port}"
require_port_available "dashboard" "${dashboard_port}"

mkdir -p "${state_root}" "${hermes_state}" "${hosted_state}"
# These files describe running processes, not durable chat state. Leaving them
# behind after a previous run can make the launcher observe an old "ready"
# marker before the rebuilt server has actually bound its port.
for marker in \
  "${hermes_state}/ready.json" \
  "${hermes_state}/service-ready.json" \
  "${hermes_state}/server.pid" \
  "${hermes_state}/service.pid"
do
  rm -f "${marker}"
done
printf '%s\n' "${ios_device_id}" >"${state_root}/ios-device-id.txt"

if [[ ! -x "${dashboard_root}/node_modules/.bin/next" ]]; then
  echo "Preparing dashboard dependencies..."
  (cd "${dashboard_root}" && npm ci)
fi

echo "Building local chat services..."
(
  cd "${mono_root}"
  CARGO_TARGET_DIR="${cargo_target_dir}" \
    cargo build -q -p finitechat-cli -p finitechat-server -p finitechat-hosted-device
)

echo "Starting Local Agent and the chat server..."
(
  cd "${finitechat_root}"
  if [[ "${stress_message_count}" != "0" && -z "${OPENROUTER_API_KEY:-}" ]]; then
    export OPENROUTER_API_KEY="local-stress-seeding-pauses-agent-replies"
  fi
  CARGO_TARGET_DIR="${cargo_target_dir}" \
  FINITECHAT_HERMES_STATE_ROOT="${hermes_state}" \
  FINITECHAT_HERMES_PORT="${chat_port}" \
  FINITECHAT_HERMES_ROOM_NAME="Local Agent" \
    scripts/hermes-real-gateway-demo.sh
) >"${state_root}/hermes.log" 2>&1 &
hermes_pid="$!"

for _ in {1..240}; do
  if [[ -s "${hermes_state}/ready.json" && -s "${hermes_state}/agent-info.json" ]]; then
    break
  fi
  if ! kill -0 "${hermes_pid}" 2>/dev/null; then
    echo "Local Agent failed to start. See ${state_root}/hermes.log." >&2
    exit 1
  fi
  sleep 0.25
done
wait_for_url "chat server" "${chat_url}/health" "${hermes_pid}"
agent_npub="$(jq -er '.npub | select(type == "string" and length > 0)' \
  "${hermes_state}/agent-info.json")"

echo "Starting the hosted human device..."
FINITECHAT_HOSTED_API_TOKEN="${hosted_token}" \
FINITECHAT_HOSTED_BIND="127.0.0.1:${hosted_port}" \
FINITECHAT_HOSTED_DATA_ROOT="${hosted_state}" \
FINITECHAT_SERVER_URL="${chat_url}" \
FINITECHAT_PUBLIC_URL="${chat_url}" \
  "${cargo_target_dir}/debug/finitechat-hosted-device" \
  >"${state_root}/hosted-device.log" 2>&1 &
hosted_pid="$!"
wait_for_url "hosted device" "${hosted_url}/healthz" "${hosted_pid}"

auth_headers=(
  -H "Authorization: Bearer ${hosted_token}"
  -H "x-finite-workos-user-id: ${workos_user_id}"
  -H "content-type: application/json"
)
bootstrap_payload="$(jq -cn \
  --arg project_id "local-ios-agent" \
  --arg creation_request_id "local-ios-agent-bootstrap-v1" \
  '{project_id: $project_id, creation_request_id: $creation_request_id}')"
curl -fsS "${auth_headers[@]}" -d "${bootstrap_payload}" \
  "${hosted_url}/v1/app/agent-bindings/authorize-bootstrap" \
  >"${state_root}/agent-bootstrap.json"

binding_payload="$(jq -cn \
  --arg project_id "local-ios-agent" \
  --arg agent_npub "${agent_npub}" \
  '{project_id: $project_id, agent_npub: $agent_npub, display_name: "Local Agent"}')"
binding_ready=0
for _ in {1..120}; do
  status="$(curl -sS -o "${state_root}/agent-binding.json" -w '%{http_code}' \
    "${auth_headers[@]}" -d "${binding_payload}" \
    "${hosted_url}/v1/app/agent-bindings/ensure")"
  if [[ "${status}" == "200" ]]; then
    binding_ready=1
    break
  fi
  if [[ "${status}" != "503" ]]; then
    echo "Could not create the local agent chat (HTTP ${status})." >&2
    cat "${state_root}/agent-binding.json" >&2
    exit 1
  fi
  sleep 0.5
done
if [[ "${binding_ready}" != "1" ]]; then
  echo "Local Agent did not publish its chat key in time." >&2
  echo "See ${state_root}/hermes.log and ${state_root}/agent-binding.json." >&2
  exit 1
fi

canonical_room_id="$(jq -er \
  '.hosted_agent_binding.canonical_room_id | select(type == "string" and length > 0)' \
  "${state_root}/agent-binding.json")"
hermes_service_url="$(jq -er \
  '.service_url | select(type == "string" and length > 0)' \
  "${hermes_state}/ready.json")"
home_channel_payload="$(jq -cn \
  --arg room_id "${canonical_room_id}" \
  '{room_id: $room_id}')"
home_channel_ready=0
persisted_home_channel="${hermes_state}/agent-home/hermes-home-channel.json"
if [[ -s "${persisted_home_channel}" ]] \
  && jq -e --arg room_id "${canonical_room_id}" \
    '.room_id == $room_id' "${persisted_home_channel}" >/dev/null
then
  jq -cn --arg room_id "${canonical_room_id}" \
    '{home_channel: {room_id: $room_id}}' >"${state_root}/home-channel.json"
  home_channel_ready=1
else
  for _ in {1..120}; do
    status="$(curl --max-time 5 -sS -o "${state_root}/home-channel.json" -w '%{http_code}' \
      -H "content-type: application/json" \
      -d "${home_channel_payload}" \
      "${hermes_service_url}/v1/hermes/home-channel-set" || true)"
    if [[ "${status}" == "200" ]] \
      && jq -e --arg room_id "${canonical_room_id}" \
        '.home_channel.room_id == $room_id' \
        "${state_root}/home-channel.json" >/dev/null
    then
      home_channel_ready=1
      break
    fi
    sleep 0.25
  done
fi
if [[ "${home_channel_ready}" != "1" ]]; then
  echo "Local Agent did not accept its canonical chat as the Hermes home channel." >&2
  echo "See ${state_root}/hermes.log and ${state_root}/home-channel.json." >&2
  exit 1
fi

if [[ "${stress_message_count}" != "0" ]]; then
  echo "Pausing Hermes replies while seeding ${stress_message_count} synthetic messages..."
  kill "${hermes_pid}"
  wait "${hermes_pid}" 2>/dev/null || true
  hermes_pid="$(<"${hermes_state}/server.pid")"
  FINITECHAT_STRESS_HOSTED_URL="${hosted_url}" \
  FINITECHAT_STRESS_HOSTED_API_TOKEN="${hosted_token}" \
  FINITECHAT_STRESS_WORKOS_USER_ID="${workos_user_id}" \
  FINITECHAT_STRESS_ROOM_ID="${canonical_room_id}" \
  FINITECHAT_STRESS_MESSAGE_COUNT="${stress_message_count}" \
  FINITECHAT_STRESS_CHAT_COUNT="${stress_chat_count}" \
    node "${finitechat_root}/scripts/seed-local-chat-stress.mjs"
fi

echo "Starting the local dashboard..."
(
  cd "${dashboard_root}"
  FC_WORKOS_AUTH_ENABLED=0 \
  FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH=1 \
  FC_DASHBOARD_DEV_EMAIL="${workos_email}" \
  FC_DASHBOARD_DEV_WORKOS_USER_ID="${workos_user_id}" \
  FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN="${dev_access_token}" \
  FC_HOSTED_WEB_DEVICE_URL="${hosted_url}" \
  FINITECHAT_HOSTED_API_TOKEN="${hosted_token}" \
  FC_DASHBOARD_BASE_URL="${dashboard_url}" \
  NEXT_PUBLIC_APP_URL="${dashboard_url}" \
  NEXT_PUBLIC_WORKOS_REDIRECT_URI="${dashboard_url}/callback" \
  WORKOS_COOKIE_PASSWORD="${cookie_password}" \
  NEXT_DIST_DIR=".next-devfinity" \
    "${dashboard_root}/node_modules/.bin/next" dev \
      --hostname 127.0.0.1 --port "${dashboard_port}"
) >"${state_root}/dashboard.log" 2>&1 &
dashboard_pid="$!"
wait_for_url "dashboard" "${dashboard_url}/api/device-links/account-binding" "${dashboard_pid}"

cargo_bin="$(command -v cargo || true)"
rustc_bin="$(command -v rustc || true)"
if [[ -z "${cargo_bin}" || -z "${rustc_bin}" ]]; then
  echo "The repository Rust toolchain is required to build the iOS bridge." >&2
  echo "Enter the repository development environment and retry." >&2
  exit 1
fi
xcodegen_bin="$(command -v xcodegen || true)"
if [[ -z "${xcodegen_bin}" ]]; then
  echo "xcodegen is required to generate the iOS project." >&2
  echo "No dependency was installed; enter the repository development environment and retry." >&2
  exit 1
fi
rust_target="aarch64-apple-ios-sim"
target_libdir="$("${rustc_bin}" --print target-libdir --target "${rust_target}")"
if [[ ! -d "${target_libdir}" ]]; then
  echo "The repository Rust toolchain is missing ${rust_target}." >&2
  echo "Refresh the pinned development environment and retry." >&2
  exit 1
fi

echo "Building and launching Finite Chat in the iOS Simulator..."
(
  cd "${finitechat_root}"
  CARGO_TARGET_DIR="${cargo_target_dir}" \
  FINITECHAT_SERVER_URL="${chat_url}" \
  FINITECHAT_DASHBOARD_URL="${dashboard_url}" \
  FINITECHAT_DEVICE_ID="${ios_device_id}" \
    "${cargo_bin}" run -q -p finitechat-rmp -- run ios
)

echo
echo "Local Finite is ready."
echo "In the simulator, tap “Continue with Finite” and choose “Local Agent”."
echo "The app uses the same automatic authenticated device-link APIs as Electron."
echo
echo "Logs: ${state_root}"
echo "Press Control-C here to stop the local stack."

while kill -0 "${hermes_pid}" 2>/dev/null \
  && kill -0 "${hosted_pid}" 2>/dev/null \
  && kill -0 "${dashboard_pid}" 2>/dev/null
do
  sleep 2
done

echo "A local service stopped. Inspect ${state_root} for logs." >&2
exit 1

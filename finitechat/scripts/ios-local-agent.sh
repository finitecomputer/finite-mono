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
ios_device_id="ios-local-$(openssl rand -hex 6)"
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

mkdir -p "${state_root}" "${hermes_state}" "${hosted_state}"

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
for _ in {1..120}; do
  status="$(curl -sS -o "${state_root}/home-channel.json" -w '%{http_code}' \
    -H "content-type: application/json" \
    -d "${home_channel_payload}" \
    "${hermes_service_url}/v1/hermes/home-channel-set")"
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
if [[ "${home_channel_ready}" != "1" ]]; then
  echo "Local Agent did not accept its canonical chat as the Hermes home channel." >&2
  echo "See ${state_root}/hermes.log and ${state_root}/home-channel.json." >&2
  exit 1
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

rustup_bin="$(command -v rustup || true)"
if [[ -z "${rustup_bin}" ]]; then
  echo "The existing rustup toolchain is required for the Apple Rust targets." >&2
  echo "No dependency was installed; configure the repository's iOS toolchain and retry." >&2
  exit 1
fi
rustup_bin_dir="$(cd "$(dirname "${rustup_bin}")" && pwd)"
xcodegen_bin="$(command -v xcodegen || true)"
if [[ -z "${xcodegen_bin}" ]]; then
  echo "xcodegen is required to generate the iOS project." >&2
  echo "No dependency was installed; enter the repository development environment and retry." >&2
  exit 1
fi
xcodegen_bin_dir="$(cd "$(dirname "${xcodegen_bin}")" && pwd)"
if ! "${rustup_bin}" target list --installed --toolchain stable \
  | grep -qx "aarch64-apple-ios-sim"
then
  echo "The existing stable Rust toolchain is missing aarch64-apple-ios-sim." >&2
  echo "No dependency was installed; configure the repository's iOS targets and retry." >&2
  exit 1
fi

echo "Building and launching Finite Chat in the iOS Simulator..."
(
  cd "${finitechat_root}"
  /usr/bin/env \
    -u CC -u CXX -u LD -u AR -u AS -u NM -u RANLIB -u STRIP \
    -u OBJCOPY -u OBJDUMP -u SIZE -u SDKROOT -u MACOSX_DEPLOYMENT_TARGET \
    -u NIX_CFLAGS_COMPILE -u NIX_LDFLAGS -u NIX_CC -u NIX_BINTOOLS \
    -u NIX_ENFORCE_NO_NATIVE -u DEVELOPER_DIR -u RUSTC_WRAPPER \
    PATH="${rustup_bin_dir}:${xcodegen_bin_dir}:/usr/bin:/bin:/usr/sbin:/sbin" \
    RUSTUP_TOOLCHAIN=stable \
    CARGO_TARGET_DIR="${cargo_target_dir}" \
    FINITECHAT_SERVER_URL="${chat_url}" \
    FINITECHAT_DASHBOARD_URL="${dashboard_url}" \
    FINITECHAT_DEVICE_ID="${ios_device_id}" \
    "${rustup_bin}" run stable cargo run -q -p finitechat-rmp -- run ios
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

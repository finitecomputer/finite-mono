#!/usr/bin/env bash
set -euo pipefail

# Real iPhone + real Nix-built Hermes Agent runtime + finitechat plugin.
# This test installs an echo set_message_handler callback. It proves adapter
# transport/media wiring through a phone, not real Hermes gateway/model behavior.
#
# Prerequisite: the current FiniteChat build is already installed on the
# target phone. The physical product harness does that as part of its matrix:
#
#   cargo run -p finitechat-rmp -- product-harness ios-device \
#     --scenario text-offline --device codex-phone \
#     --server-url http://<mac-lan-ip>:<port> \
#     --udid <phone-coredevice-id-or-hardware-udid> \
#     --ios-development-team <team-id>
#
# The phone must be unlocked and awake for devicectl launch.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MONOREPO_ROOT="$(cd "$REPO_ROOT/.." && pwd)"

cd "$REPO_ROOT"

finitechat_out="$(nix build --no-link --print-out-paths "$MONOREPO_ROOT#finitechat")"
server_out="$(nix build --no-link --print-out-paths "$MONOREPO_ROOT#finitechat-server")"

env \
    FINITE_IOS_DEVICE_HERMES_AGENT_MEDIA_E2E=1 \
    FINITE_IOS_DEVICE_HERMES_AGENT_MEDIA_E2E_REPORT="$REPO_ROOT/target/ios-device-hermes-agent-media-e2e/report.json" \
    FINITECHAT_BIN="$finitechat_out/bin/finitechat" \
    FINITECHAT_SERVER_BIN="$server_out/bin/finitechat-server" \
    nix develop "$REPO_ROOT/..#hermes-bridge-ci" --command bash -lc \
    'exec "$HERMES_AGENT_RUNTIME_PYTHON" -m unittest tests.hermes.test_live_ios_device_hermes_media_e2e -v'

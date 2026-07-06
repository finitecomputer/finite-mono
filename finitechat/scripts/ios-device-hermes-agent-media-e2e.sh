#!/usr/bin/env bash
set -euo pipefail

# Real iPhone + real pip hermes-agent package + finitechat plugin.
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

cd "$REPO_ROOT"

cargo build -p finitechat-cli -p finitechat-server

env \
    FINITE_IOS_DEVICE_HERMES_AGENT_MEDIA_E2E=1 \
    FINITE_IOS_DEVICE_HERMES_AGENT_MEDIA_E2E_REPORT="$REPO_ROOT/target/ios-device-hermes-agent-media-e2e/report.json" \
    FINITECHAT_BIN="$REPO_ROOT/target/debug/finitechat" \
    FINITECHAT_SERVER_BIN="$REPO_ROOT/target/debug/finitechat-server" \
    uvx --no-config --with hermes-agent python -m unittest \
    tests.hermes.test_live_ios_device_hermes_media_e2e -v

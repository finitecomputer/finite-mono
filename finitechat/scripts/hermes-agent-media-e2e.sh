#!/usr/bin/env bash
# Local Hermes adapter media end-to-end:
#   real pip hermes-agent package + finitechat plugin + finitechat binaries
#   finitechat user joins via invite URL, sends image media, then receives
#   agent text and image media replies.
# This test installs an echo set_message_handler callback. It proves adapter
# transport/media wiring through the sidecar inbound stream, not real Hermes
# gateway/model behavior.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

REPORT="${FINITE_HERMES_AGENT_MEDIA_E2E_REPORT:-$REPO_ROOT/target/hermes-agent-media-e2e/report.json}"

cargo build -p finitechat-cli -p finitechat-server

exec env \
    FINITE_HERMES_AGENT_MEDIA_E2E=1 \
    FINITE_HERMES_AGENT_MEDIA_E2E_REPORT="$REPORT" \
    FINITECHAT_BIN="$REPO_ROOT/target/debug/finitechat" \
    FINITECHAT_SERVER_BIN="$REPO_ROOT/target/debug/finitechat-server" \
    uvx --no-config --with hermes-agent python -m unittest \
    tests.hermes.test_live_hermes_agent_media_e2e -v

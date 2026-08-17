#!/usr/bin/env bash
set -euo pipefail

# Real iOS Simulator + real Nix-built Hermes Agent runtime + finitechat plugin.
# The app joins the agent invite, sends an image attachment with a caption,
# then receives agent text and image replies.
# This test installs an echo set_message_handler callback. It proves adapter
# transport/media wiring through iOS, not real Hermes gateway/model behavior.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MONOREPO_ROOT="$(cd "$REPO_ROOT/.." && pwd)"

cd "$REPO_ROOT"

finitechat_out="$(nix build --no-link --print-out-paths "$MONOREPO_ROOT#finitechat")"
server_out="$(nix build --no-link --print-out-paths "$MONOREPO_ROOT#finitechat-server")"
rmp_out="$(nix build --no-link --print-out-paths "$MONOREPO_ROOT#finitechat-rmp")"

env \
    FINITE_IOS_HERMES_AGENT_MEDIA_E2E=1 \
    FINITE_IOS_HERMES_AGENT_MEDIA_E2E_REPORT="$REPO_ROOT/target/ios-hermes-agent-media-e2e/report.json" \
    FINITECHAT_BIN="$finitechat_out/bin/finitechat" \
    FINITECHAT_SERVER_BIN="$server_out/bin/finitechat-server" \
    FINITECHAT_RMP_BIN="$rmp_out/bin/finitechat-rmp" \
    nix develop "$REPO_ROOT/..#hermes-bridge-ci" --command bash -lc \
    'exec "$HERMES_AGENT_RUNTIME_PYTHON" -m unittest tests.hermes.test_live_ios_simulator_hermes_media_e2e -v'

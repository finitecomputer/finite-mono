#!/usr/bin/env bash
set -euo pipefail

test -x /runtime/bin/finitechat

test -x /runtime/bin/fsite
/runtime/bin/fsite describe workflow publish-static-site --output json >/dev/null

test -x /runtime/hermes-venv/bin/hermes
/runtime/hermes-venv/bin/hermes --version >/dev/null
test -x /runtime/hermes-venv/bin/python
test -f /runtime/hermes-plugin/finite-platform/adapter.py

agent_http_host="${FINITE_AGENT_HTTP_HEALTH_HOST:-127.0.0.1}"
agent_http_port="${FINITE_AGENT_HTTP_PORT:-8080}"
curl -fsS "http://${agent_http_host}:${agent_http_port}/healthz" >/dev/null

agent_home="${FINITECHAT_HOME:-/data/agent}"
test -f "${agent_home}/config.json"
/runtime/bin/finitechat identity --agent-home "${agent_home}" show >/dev/null

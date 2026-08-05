#!/usr/bin/env bash
set -euo pipefail

# finite-agentd supervises this endpoint alongside the Finite Chat bridge and
# Hermes. A 200 response is the authoritative recurring chat/process health
# contract. Runner admission separately reads `admission_ready` from the same
# bounded response; image/package validation belongs to the image build.
agent_http_host="${FINITE_AGENT_HTTP_HEALTH_HOST:-127.0.0.1}"
agent_http_port="${FINITE_AGENT_HTTP_PORT:-8080}"
exec curl -fsS --max-time 4 \
    "http://${agent_http_host}:${agent_http_port}/healthz" >/dev/null

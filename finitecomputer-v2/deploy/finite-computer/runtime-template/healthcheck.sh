#!/usr/bin/env bash
set -euo pipefail

# finite-agentd supervises this endpoint alongside the Finite Chat bridge and
# Hermes. A 200 response is the one authoritative Runtime readiness contract;
# image/package validation belongs to the image build, not this recurring OCI
# health probe.
agent_http_host="${FINITE_AGENT_HTTP_HEALTH_HOST:-127.0.0.1}"
agent_http_port="${FINITE_AGENT_HTTP_PORT:-8080}"
exec curl -fsS --max-time 4 \
    "http://${agent_http_host}:${agent_http_port}/healthz" >/dev/null

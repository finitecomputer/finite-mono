#!/usr/bin/env bash
set -euo pipefail

test -x /runtime/bin/finitechat

test -x /runtime/bin/fsite
/runtime/bin/fsite describe workflow publish-static-site --output json >/dev/null

test -x /runtime/hermes-venv/bin/hermes
test -x /runtime/hermes-venv/bin/python
hermes_version="$(/runtime/hermes-venv/bin/python -c 'import importlib.metadata; print(importlib.metadata.version("hermes-agent"))')"
test "${hermes_version}" = "0.18.2"
/runtime/hermes-venv/bin/python - <<'PY'
import importlib.metadata

import googleapiclient
import google_auth_httplib2
import google_auth_oauthlib

expected = {
    "google-api-python-client": "2.198.0",
    "google-auth-oauthlib": "1.4.0",
    "google-auth-httplib2": "0.4.0",
}
for package, version in expected.items():
    assert importlib.metadata.version(package) == version, package
PY
test -f /runtime/hermes-plugin/finitechat/adapter.py
test -f /runtime/hermes-plugin/finitechat/plugin.yaml

hermes_home="${HERMES_HOME:-/data/agent/hermes-home}"
plugin_name="${FINITECHAT_HERMES_PLUGIN_NAME:-finitechat}"
test -f "${hermes_home}/plugins/${plugin_name}/adapter.py"
test -f "${hermes_home}/plugins/${plugin_name}/plugin.yaml"

bundled_skills=/runtime/finite-skills
test -x /runtime/bin/finite
test -d "${bundled_skills}"
find "${bundled_skills}" -name SKILL.md -type f -print -quit | grep -q .
test -f "${bundled_skills}/software-development/finitebrain/SKILL.md"
test "${FINITE_REQUIRE_BUNDLED_SKILLS:-}" = "1"
test "${FINITECHAT_HERMES_INBOUND_STREAM:-}" = "1"

agent_http_host="${FINITE_AGENT_HTTP_HEALTH_HOST:-127.0.0.1}"
agent_http_port="${FINITE_AGENT_HTTP_PORT:-8080}"
curl -fsS "http://${agent_http_host}:${agent_http_port}/healthz" >/dev/null

agent_home="${FINITECHAT_HOME:-/data/agent}"
test -f "${agent_home}/config.json"
/runtime/bin/finitechat identity --agent-home "${agent_home}" show >/dev/null

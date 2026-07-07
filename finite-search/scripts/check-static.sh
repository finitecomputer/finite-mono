#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

required_files=(
  README.md
  CONTEXT.md
  AGENTS.md
  docs/agents/issue-tracker.md
  docs/agents/triage-labels.md
  docs/agents/domain.md
  docs/adr/0001-self-host-search-extract-boundary.md
  docs/adr/0002-latitude-plain-docker-first.md
  docs/adr/0003-keep-search-and-extract-independent.md
  docs/adr/0004-follow-up-tinfoil-packaging.md
  docs/runbooks/latitude-docker-spike.md
  docs/runbooks/hermes-integration.md
  docs/runbooks/tinfoil-follow-up.md
  docs/runbooks/search-fallback-policy.md
  docs/production-readiness-investigation-2026-07-01.md
  compose/searxng/compose.yml
  compose/searxng/settings.yml
  compose/searxng/.env.example
  compose/firecrawl/README.md
  compose/firecrawl/.env.example
  scripts/doctor.sh
  scripts/smoke-searxng.sh
  scripts/smoke-firecrawl.sh
  scripts/smoke-stack.sh
  scripts/searxng-token-proxy.py
  scripts/probe-stack.sh
  scripts/bootstrap-firecrawl-upstream.sh
)

for file in "${required_files[@]}"; do
  test -f "$file" || {
    echo "missing required file: $file" >&2
    exit 1
  }
done

for script in scripts/*.sh; do
  bash -n "$script"
done

if command -v python3 >/dev/null 2>&1; then
  for script in scripts/*.py; do
    python3 -m py_compile "$script"
  done
fi

if command -v ruby >/dev/null 2>&1; then
  ruby -e 'require "yaml"; YAML.load_file("compose/searxng/compose.yml"); YAML.load_file("compose/searxng/settings.yml")'
fi

if command -v rg >/dev/null 2>&1; then
  if rg -n "TODO|CHANGEME|your\\.example|api-key-here" . \
    --glob '!scripts/check-static.sh'; then
    echo "found placeholder text that should be resolved or intentionally avoided" >&2
    exit 1
  fi
fi

echo "finite-search static checks passed"

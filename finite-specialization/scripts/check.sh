#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

required_files=(
  "README.md"
  "AGENTS.md"
  "config/specializations.example.yaml"
  "config/hermes-capabilities.example.yaml"
  "config/hermes-moa.example.yaml"
  "docs/routing-model.md"
)

for file in "${required_files[@]}"; do
  if [[ ! -f "$file" ]]; then
    echo "missing required file: $file" >&2
    exit 1
  fi
done

if find . -path ./.git -prune -o \( -name ".env" -o -name ".env.*" -o -name "auth.json" -o -name "config.yaml" \) -print | grep -q .; then
  echo "refusing repo with live secret/config-looking files" >&2
  exit 1
fi

if command -v rg >/dev/null 2>&1; then
  if rg -n --hidden --glob '!.git/**' --glob '!scripts/check.sh' \
    '(OPENAI_API_KEY|ANTHROPIC_API_KEY|HERMES_API_KEY|BEGIN [A-Z ]*PRIVATE KEY|sk-[A-Za-z0-9_-]{20,})' .; then
    echo "possible secret found" >&2
    exit 1
  fi
fi

echo "finite-specialization scaffold looks ok"

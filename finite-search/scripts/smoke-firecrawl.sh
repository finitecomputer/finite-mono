#!/usr/bin/env bash
set -euo pipefail

url="${FIRECRAWL_URL:-http://127.0.0.1:3002}"
target="${FIRECRAWL_TARGET_URL:-https://example.com}"

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

curl -fsS --max-time "${FIRECRAWL_TIMEOUT:-60}" \
  -X POST "${url%/}/v1/scrape" \
  -H 'Content-Type: application/json' \
  -d "{\"url\":\"${target}\",\"formats\":[\"markdown\"]}" >"$tmp"

if command -v jq >/dev/null 2>&1; then
  jq -e '
    .success == true
    and ((.data.markdown // .markdown // "") | type == "string")
    and ((.data.markdown // .markdown // "") | length > 0)
  ' "$tmp" >/dev/null
  printf "firecrawl smoke ok: %s markdown chars\n" "$(jq '(.data.markdown // .markdown // "") | length' "$tmp")"
else
  grep -Eq '"success"|"markdown"' "$tmp"
  echo "firecrawl smoke ok"
fi

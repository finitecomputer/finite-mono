#!/usr/bin/env bash
set -euo pipefail

url="${SEARXNG_URL:-http://127.0.0.1:8080}"
query="${SEARXNG_QUERY:-open source}"
engines="${SEARXNG_ENGINES:-}"
token="${SEARXNG_TOKEN:-${FINITE_SEARCH_TOKEN:-}}"
encoded_query="${query// /+}"
request_url="${url%/}/search?q=${encoded_query}&format=json"

if [ -n "$engines" ]; then
  request_url="${request_url}&engines=${engines// /+}"
fi

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

curl_args=(-fsS --max-time "${SEARXNG_TIMEOUT:-20}")
if [ -n "$token" ]; then
  curl_args+=(-H "Authorization: Bearer $token")
fi

curl "${curl_args[@]}" "$request_url" >"$tmp"

if command -v jq >/dev/null 2>&1; then
  jq -e '.results and (.results | length > 0)' "$tmp" >/dev/null
  printf "searxng smoke ok: %s results\n" "$(jq '.results | length' "$tmp")"
else
  grep -Eq '"url"|"content"' "$tmp"
  echo "searxng smoke ok"
fi

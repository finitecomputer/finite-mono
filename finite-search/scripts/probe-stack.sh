#!/usr/bin/env bash
set -euo pipefail

if ! command -v jq >/dev/null 2>&1; then
  echo "probe requires jq" >&2
  exit 1
fi

searxng_url="${SEARXNG_URL:-http://127.0.0.1:8080}"
firecrawl_url="${FIRECRAWL_URL:-http://127.0.0.1:3002}"
search_timeout="${SEARXNG_TIMEOUT:-30}"
extract_timeout="${FIRECRAWL_TIMEOUT:-90}"
strict="${PROBE_STRICT:-false}"

default_search_queries=$'open source\nfirecrawl self hosted docker compose\nsite:github.com firecrawl firecrawl'
default_extract_urls=$'https://example.com\nhttps://en.wikipedia.org/wiki/Open_source\nhttps://news.ycombinator.com/\nhttps://www.reuters.com/technology/'

search_queries="${PROBE_SEARCH_QUERIES:-$default_search_queries}"
extract_urls="${PROBE_EXTRACT_URLS:-$default_extract_urls}"

failures=0
search_count=0
extract_count=0

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

bool_ok() {
  case "$1" in
    true|1|yes) return 0 ;;
    *) return 1 ;;
  esac
}

printf 'type\tstatus\ttime_seconds\tresults_or_chars\tunresponsive\tlabel\tdetail\n'

while IFS= read -r query; do
  [ -n "$query" ] || continue
  search_count=$((search_count + 1))

  encoded="$(jq -rn --arg q "$query" '$q|@uri')"
  body="$tmpdir/search-$search_count.json"
  metrics="$tmpdir/search-$search_count.metrics"

  if curl -fsS --max-time "$search_timeout" \
    -w '%{time_total}' \
    -o "$body" \
    "${searxng_url%/}/search?q=${encoded}&format=json" >"$metrics"; then
    result_count="$(jq '.results | length' "$body")"
    unresponsive="$(jq -r '(.unresponsive_engines // []) | map(.[0] + ":" + .[1]) | join(",")' "$body")"
    first_url="$(jq -r '.results[0].url // ""' "$body")"
    if [ "$result_count" -gt 0 ]; then
      printf 'search\tok\t%s\t%s\t%s\t%s\t%s\n' \
        "$(cat "$metrics")" "$result_count" "$unresponsive" "$query" "$first_url"
    else
      failures=$((failures + 1))
      printf 'search\tzero-results\t%s\t0\t%s\t%s\t%s\n' \
        "$(cat "$metrics")" "$unresponsive" "$query" ""
    fi
  else
    failures=$((failures + 1))
    printf 'search\trequest-failed\t0\t0\t\t%s\t%s\n' "$query" ""
  fi
done <<<"$search_queries"

while IFS= read -r target; do
  [ -n "$target" ] || continue
  extract_count=$((extract_count + 1))

  body="$tmpdir/extract-$extract_count.json"
  metrics="$tmpdir/extract-$extract_count.metrics"
  payload="$(jq -cn --arg url "$target" '{url:$url,formats:["markdown"]}')"

  if curl -fsS --max-time "$extract_timeout" \
    -w '%{time_total}' \
    -X POST "${firecrawl_url%/}/v1/scrape" \
    -H 'Content-Type: application/json' \
    -d "$payload" \
    -o "$body" >"$metrics"; then
    success="$(jq -r '.success // false' "$body")"
    chars="$(jq '(.data.markdown // .markdown // "") | length' "$body")"
    title="$(jq -r '.data.metadata.title // .metadata.title // .data.title // .title // ""' "$body")"
    if [ "$success" = "true" ] && [ "$chars" -gt 0 ]; then
      printf 'extract\tok\t%s\t%s\t\t%s\t%s\n' \
        "$(cat "$metrics")" "$chars" "$target" "$title"
    else
      failures=$((failures + 1))
      printf 'extract\tempty-or-failed\t%s\t%s\t\t%s\t%s\n' \
        "$(cat "$metrics")" "$chars" "$target" "$title"
    fi
  else
    failures=$((failures + 1))
    printf 'extract\trequest-failed\t0\t0\t\t%s\t%s\n' "$target" ""
  fi
done <<<"$extract_urls"

printf 'summary\t%s\t0\t%s\t\tsearches=%s extracts=%s\tstrict=%s\n' \
  "$([ "$failures" -eq 0 ] && printf ok || printf failures)" \
  "$failures" \
  "$search_count" \
  "$extract_count" \
  "$strict"

if bool_ok "$strict" && [ "$failures" -gt 0 ]; then
  exit 1
fi

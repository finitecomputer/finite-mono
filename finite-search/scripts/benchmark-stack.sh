#!/usr/bin/env bash
set -euo pipefail

if ! command -v jq >/dev/null 2>&1; then
  echo "benchmark requires jq" >&2
  exit 1
fi

iterations="${BENCHMARK_ITERATIONS:-5}"
searxng_url="${SEARXNG_URL:-http://127.0.0.1:8080}"
firecrawl_url="${FIRECRAWL_URL:-http://127.0.0.1:3002}"
search_query="${BENCHMARK_SEARCH_QUERY:-open source}"
extract_target="${BENCHMARK_EXTRACT_URL:-https://example.com}"

if ! [[ "$iterations" =~ ^[0-9]+$ ]] || [ "$iterations" -lt 1 ]; then
  echo "BENCHMARK_ITERATIONS must be a positive integer" >&2
  exit 1
fi

encoded_query="${search_query// /+}"

search_times=()
extract_times=()
search_ok=0
extract_ok=0
last_results=0
last_markdown_chars=0

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

summarize_times() {
  if [ "$#" -eq 0 ]; then
    printf "avg=0 min=0 max=0"
    return
  fi

  printf '%s\n' "$@" | awk '
    NR == 1 { min = max = sum = $1; next }
    { if ($1 < min) min = $1; if ($1 > max) max = $1; sum += $1 }
    END { printf "avg=%.3fs min=%.3fs max=%.3fs", sum / NR, min, max }
  '
}

for i in $(seq 1 "$iterations"); do
  body="$tmp/search-$i.json"
  metrics="$tmp/search-$i.metrics"
  if curl -fsS --max-time "${SEARXNG_TIMEOUT:-30}" \
    -w '%{time_total}' \
    -o "$body" \
    "${searxng_url%/}/search?q=${encoded_query}&format=json" >"$metrics"; then
    count="$(jq '.results | length' "$body")"
    if [ "$count" -gt 0 ]; then
      search_ok=$((search_ok + 1))
      last_results="$count"
      search_times+=("$(cat "$metrics")")
    fi
  fi

  body="$tmp/extract-$i.json"
  metrics="$tmp/extract-$i.metrics"
  if curl -fsS --max-time "${FIRECRAWL_TIMEOUT:-90}" \
    -w '%{time_total}' \
    -X POST "${firecrawl_url%/}/v1/scrape" \
    -H 'Content-Type: application/json' \
    -d "{\"url\":\"${extract_target}\",\"formats\":[\"markdown\"]}" \
    -o "$body" >"$metrics"; then
    chars="$(jq '(.data.markdown // .markdown // "") | length' "$body")"
    if jq -e '.success == true' "$body" >/dev/null && [ "$chars" -gt 0 ]; then
      extract_ok=$((extract_ok + 1))
      last_markdown_chars="$chars"
      extract_times+=("$(cat "$metrics")")
    fi
  fi
done

cat <<REPORT
finite-search benchmark
iterations: ${iterations}
search_url: ${searxng_url}
search_query: ${search_query}
search_success: ${search_ok}/${iterations}
search_latency: $(summarize_times "${search_times[@]}")
search_last_results: ${last_results}
extract_url: ${firecrawl_url}
extract_target: ${extract_target}
extract_success: ${extract_ok}/${iterations}
extract_latency: $(summarize_times "${extract_times[@]}")
extract_last_markdown_chars: ${last_markdown_chars}
REPORT

if [ "$search_ok" -ne "$iterations" ] || [ "$extract_ok" -ne "$iterations" ]; then
  exit 1
fi

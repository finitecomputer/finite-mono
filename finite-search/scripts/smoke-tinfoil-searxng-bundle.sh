#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bundle_dir="${repo_root}/tinfoil/searxng-public"
image="${TINFOIL_SEARXNG_IMAGE:-finite-search/searxng-tinfoil-smoke:local}"
port="${TINFOIL_SEARXNG_SMOKE_PORT:-18088}"
container="finite-search-tinfoil-searxng-smoke-$$"
secret="${SEARXNG_SECRET:-}"
token="${FINITE_SEARCH_TOKEN:-${SEARXNG_TOKEN:-}}"

if [ -z "$secret" ]; then
  if command -v openssl >/dev/null 2>&1; then
    secret="$(openssl rand -hex 32)"
  else
    secret="finite-search-local-smoke-secret"
  fi
fi

if [ -z "$token" ]; then
  if command -v openssl >/dev/null 2>&1; then
    token="$(openssl rand -hex 32)"
  else
    token="finite-search-local-smoke-token"
  fi
fi

cleanup() {
  docker rm -f "$container" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker build -t "$image" "$bundle_dir"
docker run -d \
  --name "$container" \
  -e "SEARXNG_SECRET=${secret}" \
  -e "SEARXNG_LIMITER=false" \
  -e "FINITE_SEARCH_TOKEN=${token}" \
  -p "127.0.0.1:${port}:8081" \
  "$image" >/dev/null

health_url="http://127.0.0.1:${port}/healthz"
url="http://127.0.0.1:${port}/search?q=open+source&format=json"
tmp="$(mktemp)"
trap 'rm -f "$tmp"; cleanup' EXIT

for _ in $(seq 1 30); do
  unauth_status="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 10 "$url" 2>/dev/null || true)"
  if curl -fsS --max-time 10 "$health_url" >/dev/null 2>&1 &&
    [ "$unauth_status" = "401" ] &&
    curl -fsS --max-time 10 -H "Authorization: Bearer ${token}" "$url" >"$tmp" 2>/dev/null; then
    if command -v jq >/dev/null 2>&1; then
      if jq -e '.results and (.results | length > 0)' "$tmp" >/dev/null; then
        printf "tinfoil searxng bundle smoke ok: auth gate enforced, %s results\n" "$(jq '.results | length' "$tmp")"
        exit 0
      fi
    elif grep -Eq '"url"|"content"' "$tmp"; then
      echo "tinfoil searxng bundle smoke ok: auth gate enforced"
      exit 0
    fi
  fi
  sleep 2
done

docker logs "$container" >&2 || true
echo "tinfoil searxng bundle smoke failed" >&2
exit 1

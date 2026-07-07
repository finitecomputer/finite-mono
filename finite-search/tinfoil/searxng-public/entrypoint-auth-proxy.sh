#!/bin/sh
set -eu

export SEARXNG_PORT="${SEARXNG_PORT:-8080}"
export FINITE_SEARCH_PROXY_PORT="${FINITE_SEARCH_PROXY_PORT:-8081}"
export FINITE_SEARCH_UPSTREAM="${FINITE_SEARCH_UPSTREAM:-http://127.0.0.1:${SEARXNG_PORT}}"

/usr/local/searxng/entrypoint.sh &
searxng_pid="$!"

/usr/sbin/python3 /usr/local/bin/finite_search_auth_proxy.py &
proxy_pid="$!"

shutdown() {
  kill "$searxng_pid" "$proxy_pid" >/dev/null 2>&1 || true
  wait "$searxng_pid" "$proxy_pid" >/dev/null 2>&1 || true
}

trap shutdown INT TERM

while :; do
  if ! kill -0 "$searxng_pid" >/dev/null 2>&1; then
    wait "$searxng_pid"
    exit "$?"
  fi
  if ! kill -0 "$proxy_pid" >/dev/null 2>&1; then
    wait "$proxy_pid"
    exit "$?"
  fi
  sleep 1
done

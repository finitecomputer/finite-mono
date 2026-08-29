#!/usr/bin/env bash
# Minimal local repro for the finitechat-hermes sidecar wedge.
#
# Boots the freshly built `finitechat hermes serve` sidecar against a fake
# chat server, lets it reach steady state, then flips the fake server into
# stall mode (established SSE stream goes silent; new requests never answered).
# On an unfixed binary the sidecar never recovers — no reconnect, no error, no
# progress; on a fixed binary each stalled call errors within its HTTP bound
# and healthy traffic resumes when the stall lifts.
#
# Usage: run.sh [phase2-wait-seconds [phase3-watch-seconds]]
#   phase 1 (healthy) runs ~15s, then stall is engaged and we watch for
#   PHASE2_SECS (default 60) more seconds; phase 3 then watches the healed
#   server for PHASE3_SECS (default 15 — raise to ~90 when running the fixed
#   binary at its default 60s bounds, so an in-flight stalled request can
#   time out and recover within the window).

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
WORKTREE="$(cd "$HERE/../.." && pwd)"
BIN="$WORKTREE/target/debug/finitechat"
PORT="${PORT:-18180}"
PHASE2_SECS="${1:-60}"
SCRATCH="${SCRATCH:-$(mktemp -d /tmp/hermes-wedge.XXXXXX)}"

mkdir -p "$SCRATCH"
MODE_FILE="$SCRATCH/mode"
echo "healthy" > "$MODE_FILE"

echo "== scratch: $SCRATCH"

cleanup() {
  local exit_code=$?
  for pid in "${SERVE_PID:-}" "${SERVER_PID:-}"; do
    [ -n "${pid:-}" ] && kill "$pid" 2>/dev/null || true
  done
  exit $exit_code
}
trap cleanup EXIT INT TERM

echo "== starting fake chat server on 127.0.0.1:$PORT"
python3 "$HERE/fake_chat_server.py" --port "$PORT" --mode-file "$MODE_FILE" \
  > "$SCRATCH/server.log" 2>&1 &
SERVER_PID=$!

for _ in $(seq 1 50); do
  curl -sf "http://127.0.0.1:$PORT/health" > /dev/null 2>&1 && break
  sleep 0.1
done

echo "== hermes init (scratch identity + agent home)"
FINITE_HOME="$SCRATCH/finite-home" \
  "$BIN" hermes init \
    --agent-home "$SCRATCH/agent-home" \
    --server "http://127.0.0.1:$PORT" \
    --device-id repro-agent \
    --skip-agent-profile \
    --json > "$SCRATCH/init.json"
cat "$SCRATCH/init.json"

echo "== hermes serve (ready file: $SCRATCH/ready.json)"
FINITE_HOME="$SCRATCH/finite-home" \
  "$BIN" hermes serve \
    --agent-home "$SCRATCH/agent-home" \
    --addr "127.0.0.1:18443" \
    --ready-file "$SCRATCH/ready.json" \
    > "$SCRATCH/serve.out" 2> "$SCRATCH/serve.err" &
SERVE_PID=$!

for _ in $(seq 1 100); do
  [ -f "$SCRATCH/ready.json" ] && break
  sleep 0.1
done
if [ ! -f "$SCRATCH/ready.json" ]; then
  echo "SIDECAR NEVER BECAME READY (see $SCRATCH/serve.err)"; exit 1
fi
SERVICE_URL="http://127.0.0.1:18443"
echo "== sidecar ready (pid $SERVE_PID): $(cat "$SCRATCH/ready.json")"

echo "== phase 1: healthy operation for 15s"
sleep 15
echo "-- last 8 server requests:"
tail -8 "$SCRATCH/server.log"
echo "-- store wal mtime (phase 1): $(stat -f '%Sm' "$SCRATCH/agent-home/client.sqlite3-wal" 2>/dev/null || echo 'no wal')"

echo
echo "== phase 2: STALL — established stream goes silent, new requests hang"
echo "stall" > "$MODE_FILE"
STALL_AT=$(date +%s)

sleep "$PHASE2_SECS"

echo "-- server requests during stall (should be none from the sidecar):"
awk -v t="$(date -r $((STALL_AT)) +%H:%M:%S)" 'BEGIN{p=0} {if (substr($1,1,8)>=t && substr($1,1,8)!=t) p=1} p' "$SCRATCH/server.log" | tail -5 || true
echo "-- sidecar still alive? $(kill -0 $SERVE_PID 2>/dev/null && echo YES || echo NO)"
echo "-- store wal mtime (phase 2): $(stat -f '%Sm' "$SCRATCH/agent-home/client.sqlite3-wal" 2>/dev/null || echo 'no wal')"

echo
echo "== probe: does the sidecar answer /readyz while wedged?"
curl -sf --max-time 5 "$SERVICE_URL/readyz" | head -c 400 && echo " <- answered" \
  || echo "readyz TIMED OUT (5s)"

echo "== probe: does a hermes action (ack) answer while wedged?"
curl -s --max-time 5 -X POST "$SERVICE_URL/v1/hermes/ack" \
  -H 'content-type: application/json' \
  -d '{"room_id":"r","seq":1,"message_id":"m"}' | head -c 400 && echo " <- answered" \
  || echo "ack TIMED OUT (5s)"

echo
echo "== thread sample of the wedged sidecar -> $SCRATCH/sample.txt"
sample "$SERVE_PID" 3 -file "$SCRATCH/sample.txt" >/dev/null 2>&1 || true
grep -nE "finitechat-resident-sync|psynch_cvwait|kevent|recvfrom|read" "$SCRATCH/sample.txt" \
  | grep -A2 -B2 "finitechat-resident-sync" | head -30 || true

echo
echo "== phase 3: UNSTALL — does the sidecar recover?"
echo "healthy" > "$MODE_FILE"
UNSTALL_AT=$(date +%s)
sleep "${2:-15}"
echo "-- requests after unstall (recovery proof; empty = still wedged):"
awk -v t="$(date -r $UNSTALL_AT +%H:%M:%S)" 'substr($1,1,8)>t' "$SCRATCH/server.log" | grep -v STALL | tail -6 || true

echo
echo "Repro complete. Artifacts in $SCRATCH (server.log, serve.err, sample.txt)."
echo "On the fixed binary the default 60s bounds apply: give phase 2 at least"
echo "~75s to watch error-and-retry through the stall instead of the park."

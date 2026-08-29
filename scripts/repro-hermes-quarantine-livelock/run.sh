#!/usr/bin/env bash
# Repro for the finitechat-hermes sidecar quarantine LIVELOCK (sibling of
# repro-hermes-wedge, which covers the stalled-stream wedge).
#
# Production shape (verified 2026-08-28): a room sync tick fetches a page the
# device cannot apply (e.g. an MLS application-ciphertext entry this device
# cannot decrypt). The failure is QUARANTINED — Ok, room marked, durable
# cursor frozen — and because the room's server-side last_seq is now
# permanently ahead of the cursor, every freshly opened /sync/stream emits
# RoomAdvanced instantly (~40 ms). The hint dispatch re-fetches the SAME
# rejected page: one agent at 25.3 fetches/s (~50 GB egress), five agents
# live-looping. Every individual request SUCCEEDS, so no per-request bound
# (#765/#768) sees it; no log line fires; /healthz stays static.
#
# This script runs the in-repo regression test against the real protocol
# stack: a live finitechat server, a real MLS welcome join, and a poison
# application entry the victim cannot decrypt (accepted by /events, which
# never inspects ciphertext). A request-counting middleware counts room-page
# fetches (/sync/group) exactly.
#
#   unfixed tree: the test FAILS — every RoomAdvanced hint re-fetches the
#                 rejected page (fetches == hints; the livelock).
#   fixed tree:   the test passes — the quarantined room backs off
#                 exponentially on the hint path, healthy rooms keep
#                 immediate hint sync, and the bounded-rate quarantine line
#                 ("finitechat room sync quarantined: room=...") is visible
#                 in the output below, proving the sidecar stderr signal an
#                 operator would see in `nerdctl logs`.
#
# Usage: run.sh [extra cargo test args...]

set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT"

OUT="$(mktemp /tmp/hermes-quarantine-livelock.XXXXXX)"
echo "== output also captured at $OUT"

./scripts/with-dev-env \
  cargo test -p finitechat-core --lib quarantined_room_hints -- --nocapture \
  "$@" 2>&1 | tee "$OUT"

echo
if grep -q "finitechat room sync quarantined: room=" "$OUT"; then
  echo "== quarantine stderr line captured:"
  grep "finitechat room sync quarantined: room=" "$OUT" | head -2
else
  echo "== no quarantine stderr line captured (unexpected on a fixed tree)"
fi

if grep -q "test result: ok" "$OUT"; then
  echo "== FIXED shape: hint fetch rate is bounded under quarantine."
  exit 0
fi
echo "== UNFIXED shape: every RoomAdvanced hint re-fetches the rejected page."
exit 1

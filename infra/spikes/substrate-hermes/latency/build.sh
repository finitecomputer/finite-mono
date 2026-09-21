#!/usr/bin/env bash
set -euo pipefail
spike="$(cd "$(dirname "$0")/.." && pwd)"
context="${SPIKE_STATE_DIR:?}/latency-build"
mkdir -p "$context/latency"
pin=29112bef099274229cadff79cdff7bf7b99c4b77
source_dir="$(mktemp -d "$SPIKE_STATE_DIR/latency-source.XXXXXX")"
trap 'rm -rf "$source_dir"' EXIT
for file in plugins/platforms/simplex/adapter.py gateway/run.py run_agent.py hermes_cli/web_server.py; do
  mkdir -p "$source_dir/$(dirname "$file")"
  git -C "${SPIKE_HERMES_SOURCE:?}" show "$pin:$file" > "$source_dir/$file"
done
(cd "$source_dir" && git apply "$spike/hermes-simplex-relay.patch")
python3 "$spike/latency/patch-hermes.py" "$source_dir" "$context/patched"
cp -R "$spike/notifications" "$context/"
cp "$spike/latency/latency_trace.py" "$spike/latency/wake_control.py" "$spike/latency/stream_delivery.py" "$context/latency/"
cp "$spike/runtime.py" "$context/runtime.py"
cp "$spike/latency/Dockerfile" "$context/Dockerfile"
docker build --build-arg "BASE_IMAGE=${SPIKE_BASE_IMAGE:?}" -t "${SPIKE_IMAGE:?}" "$context"
docker push "$SPIKE_IMAGE"

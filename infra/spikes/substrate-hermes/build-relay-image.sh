#!/usr/bin/env bash
set -euo pipefail
spike_dir="$(cd "$(dirname "$0")" && pwd)"
state="${SPIKE_STATE_DIR:?Set private scratch space outside git}"
source="${SPIKE_HERMES_SOURCE:?Set the pinned Hermes source checkout}"
image="${SPIKE_IMAGE:?Set local image tag}"
base="${SPIKE_BASE_IMAGE:?Set the baseline image digest from build-image.sh}"
pin=29112bef099274229cadff79cdff7bf7b99c4b77
context="$state/relay-build-context"
# A fresh source directory avoids accidentally applying the patch twice.
patch_dir="$(mktemp -d "$state/hermes-patch.XXXXXX")"
trap 'rm -rf "$patch_dir"' EXIT
mkdir -p "$patch_dir/plugins/platforms/simplex" "$context"
git -C "$source" show "$pin:plugins/platforms/simplex/adapter.py" > "$patch_dir/plugins/platforms/simplex/adapter.py"
(cd "$patch_dir" && git apply "$spike_dir/hermes-simplex-relay.patch")
cp "$patch_dir/plugins/platforms/simplex/adapter.py" "$context/simplex-adapter.py"
cp "$spike_dir/runtime.py" "$spike_dir/gateway-start.py" "$spike_dir/simplex-relay.py" "$context/"
cp "$spike_dir/Dockerfile.relay" "$context/Dockerfile"
docker build --build-arg "BASE_IMAGE=$base" -t "$image" "$context"
docker push "$image"

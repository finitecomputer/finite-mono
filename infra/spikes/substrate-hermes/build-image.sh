#!/usr/bin/env bash
set -euo pipefail
# Run within tools.nix. Existing Linux builder must mount this worktree read-only
# at the same absolute path, as Finite's local nix-aarch64-builder does.
spike_dir="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$spike_dir/../../.." && pwd)"
builder="${SPIKE_NIX_BUILDER:-nix-aarch64-builder}"
state="${SPIKE_STATE_DIR:?Set SPIKE_STATE_DIR to private scratch space outside git}"
image="${SPIKE_IMAGE:?Set SPIKE_IMAGE to a local or authorized registry image}"
mkdir -p "$state/image"
docker exec -w "$repo" "$builder" nix --extra-experimental-features 'nix-command flakes' \
  build --accept-flake-config --no-link --json \
  .#packages.aarch64-linux.hermes-agent-minimal \
  .#packages.aarch64-linux.hermes-agent-minimal-runtime \
  .#packages.aarch64-linux.simplex-chat > "$state/nix-build.json"
hermes_path="$(jq -r '.[] | select(.drvPath | test("hermes-agent-0")) | .outputs.out' "$state/nix-build.json")"
python_path="$(jq -r '.[] | select(.drvPath | test("hermes-agent-env")) | .outputs.out' "$state/nix-build.json")"
simplex_path="$(jq -r '.[] | select(.drvPath | test("simplex-chat-7")) | .outputs.out' "$state/nix-build.json")"
for path in "$hermes_path" "$python_path" "$simplex_path"; do
  [[ "$path" == /nix/store/* ]] || { echo 'Missing Nix output' >&2; exit 1; }
done
docker exec "$builder" nix-store -qR "$hermes_path" "$python_path" "$simplex_path" > "$state/closure-paths"
# Paths are Nix store paths, with no spaces or secret values.
mapfile_compat=()
while IFS= read -r path; do mapfile_compat+=("$path"); done < "$state/closure-paths"
docker exec "$builder" tar -c "${mapfile_compat[@]}" | tar -x -C "$state/image"
cp "$spike_dir/Dockerfile" "$spike_dir/runtime.py" "$state/image/"
docker build -t "$image" --build-arg "HERMES_PATH=$hermes_path" \
  --build-arg "PYTHON_PATH=$python_path" --build-arg "SIMPLEX_PATH=$simplex_path" "$state/image"
docker push "$image"

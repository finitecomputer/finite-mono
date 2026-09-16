#!/usr/bin/env bash
# RUNNER_PROOF_BINARY=target/debug/finite-saas-runner bash "$0"
# macOS uses the existing OrbStack nixos VM; Linux requires namespace permission.
# All dependencies derive from the repo flake. No system trust or production changes.
set -euo pipefail
proof_script="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
proof_repo="$(cd "$(dirname "$proof_script")/../.." && pwd)"
proof_flake="git+file://$proof_repo"
cd "$proof_repo"

if [[ "${1:-}" != --linux ]]; then
  : "${RUNNER_PROOF_BINARY:?Build finite-saas-runner and set RUNNER_PROOF_BINARY to its executable}"
  proof_system="$(nix eval --impure --raw --expr builtins.currentSystem)"
  proof_node="$(nix build --no-link --print-out-paths --impure --expr "let f = builtins.getFlake \"$proof_flake\"; in f.inputs.nixpkgs.legacyPackages.${proof_system}.nodejs_24")/bin/node"
  proof_rendered="$(mktemp -d "${TMPDIR:-/tmp}/finite-hermes-rendered.XXXXXX")"
  # /private/tmp is shared into the local VM; macOS TMPDIR is not guaranteed to be.
  if [[ "$(uname -s)" != Linux ]]; then
    rmdir "$proof_rendered"
    proof_rendered="$(mktemp -d /private/tmp/finite-hermes-rendered.XXXXXX)"
  fi
  trap 'rm -rf -- "$proof_rendered"' EXIT
  "$proof_node" --input-type=module - "$proof_rendered" <<'JS'
import { writeFileSync } from 'node:fs';
const dir = process.argv[2];
writeFileSync(`${dir}/manifest.json`, JSON.stringify({ public_origin: 'https://localhost:55443',
  listen: '127.0.0.1:55443', admin_socket: `${dir}/admin.sock`, allowed_origins: ['https://finite.computer'],
  routes: [{ runtime_id: 'runtime_native_proof', host_port: 30000 }] }));
JS
  "$RUNNER_PROOF_BINARY" render-hosted-hermes-caddy --manifest "$proof_rendered/manifest.json" > "$proof_rendered/caddy.json"
  if [[ "$(uname -s)" != Linux ]]; then
    orb -m nixos -u root bash "$proof_script" --linux "$proof_rendered"
  else
    bash "$proof_script" --linux "$proof_rendered"
  fi
  exit
fi

[[ "$(id -u)" == 0 ]] || { echo 'Run the Linux namespace launcher as root.' >&2; exit 2; }
proof_rendered="$2"
proof_system="$(nix eval --impure --raw --expr builtins.currentSystem)"
proof_output_text="$(nix build --no-link --print-out-paths --impure --expr "
  let f = builtins.getFlake \"$proof_flake\"; p = f.inputs.nixpkgs.legacyPackages.${proof_system};
  in [ f.packages.${proof_system}.hermes-agent-minimal-runtime p.nodejs_24 p.caddy p.iproute2 p.util-linux.bin ]")"
mapfile -t proof_outputs <<< "$proof_output_text"
for proof_output in "${proof_outputs[@]}"; do
  [[ ! -x "$proof_output/bin/hermes" ]] || export HERMES_PROOF_BINARY="$proof_output/bin/hermes"
  [[ ! -x "$proof_output/bin/caddy" ]] || export CADDY_BIN="$proof_output/bin/caddy"
  [[ ! -x "$proof_output/bin/node" ]] || proof_node="$proof_output/bin/node"
  [[ ! -x "$proof_output/bin/unshare" ]] || proof_unshare="$proof_output/bin/unshare"
  [[ ! -x "$proof_output/bin/ip" ]] || proof_ip="$proof_output/bin/ip"
done
: "${HERMES_PROOF_BINARY:?}" "${CADDY_BIN:?}" "${proof_node:?}" "${proof_unshare:?}" "${proof_ip:?}"
proof_source="$(nix eval --impure --raw --expr "let f = builtins.getFlake \"$proof_flake\"; in f.inputs.hermes-agent.outPath")"
export HERMES_PROOF_SOURCE="$proof_source"
export PATH="$(dirname "$proof_node"):$PATH"
"$proof_unshare" --mount --ipc --uts --net --pid --fork --kill-child --mount-proc \
  bash -c '"$1" link set lo up; exec "$2" "$3" "$4/manifest.json" "$4/caddy.json"' \
  proof "$proof_ip" "$proof_node" "${proof_script%.sh}.mjs" "$proof_rendered"

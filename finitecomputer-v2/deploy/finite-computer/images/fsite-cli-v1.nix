# The canonical v1 fsite CLI, pinned to the fsite/v0.5.3 release binary.
# main's fsite-cli source is the static-only v2 client (it speaks only
# /api/v2/*), while canonical Sites serves the public v1 contract until the
# deliberate cutover — an image built from main would ship agents a CLI that
# 404s against the canonical API (and the image smoke cannot catch that,
# because `describe workflow` never touches the network). Pin the released
# v1 binary instead, mirroring finitesitesd-legacy-canonical in
# infra/nixos/packages.nix (same release lineage: deploy-2 revision
# 7c04a681 plus the 0.5.3 version bump). Remove this pin only together with
# the Sites v2 cutover.
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  glibc,
  libgcc,
}:
let
  assets = {
    x86_64-linux.asset = "fsite-linux-x86_64";
    x86_64-linux.hash = "sha256-dqLEzE/lGSKTPKv7ZTw7cFjjJxIswupbOnHEfTXIJes=";
    aarch64-darwin.asset = "fsite-macos-aarch64";
    aarch64-darwin.hash = "sha256-9j+lSXRWghRjcS6SKQxMnDoMxAw2miXDLPm+G1E/v9c=";
    x86_64-darwin.asset = "fsite-macos-x86_64";
    x86_64-darwin.hash = "sha256-lUH68aReYa+JXZDElbLGSV2v9brsOwpt3FdhOpN3OCI=";
  };
  asset = assets.${stdenv.hostPlatform.system} or (throw "fsite-cli-v1: unsupported system ${stdenv.hostPlatform.system}");
in
stdenv.mkDerivation rec {
  pname = "fsite-cli-v1";
  version = "0.5.3";
  src = fetchurl {
    url = "https://github.com/finitecomputer/finite-releases/releases/download/fsite/v${version}/${asset.asset}.tar.gz";
    inherit (asset) hash;
  };
  dontUnpack = true;
  nativeBuildInputs = lib.optionals stdenv.isLinux [ autoPatchelfHook ];
  buildInputs = lib.optionals stdenv.isLinux [ glibc libgcc ];
  installPhase = ''
    mkdir -p "$out/bin"
    tar -xzf "$src" -C "$out/bin" fsite
    chmod 0555 "$out/bin/fsite"
  '';
  # Execute the binary at build time so a loader/linkage regression fails
  # the build instead of a production agent.
  doInstallCheck = true;
  installCheckPhase = "$out/bin/fsite --version";
  passthru = {
    sourceTag = "fsite/v0.5.3";
    sourceSha = "662f837ed5875de4cbf55acd1d1adfe288e762a1";
  };
  meta = {
    mainProgram = "fsite";
    platforms = [
      "x86_64-linux"
      "aarch64-darwin"
      "x86_64-darwin"
    ];
  };
}

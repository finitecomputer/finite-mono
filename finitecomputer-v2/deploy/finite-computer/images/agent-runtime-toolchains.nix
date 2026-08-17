# Baseline CLIs staged into the canonical Agent Runtime image alongside the
# Nix-built Hermes closure. Pin through hermes-nixpkgs (callPackage from the
# flake) so the image carries one glibc family. Node is Hermes's wrapped
# Node 26 — do not add a second nixpkgs nodejs. Versions live on this
# derivation's passthru and in flake.lock; do not copy sha256s into Dockerfiles.
#
# `bins` is the single authority for which CLI symlinks the image exposes:
# the build passes it as the AGENT_RUNTIME_TOOLCHAIN_BINS build-arg, and the
# Dockerfile loops and workflow probes render from that list. Do not
# enumerate these bin names anywhere else.
{
  lib,
  symlinkJoin,
  bun,
  deno,
  uv,
  playwright-driver,
  playwright-test,
  hermesAgent,
}:
let
  nodejs = hermesAgent.hermesNpmLib.nodejs;
  browsers = playwright-driver.browsers;
in
symlinkJoin {
  name = "agent-runtime-toolchains";
  paths = [
    nodejs
    bun
    deno
    uv
    playwright-test
    browsers
  ];
  passthru = {
    inherit nodejs browsers;
    browsersPath = "${browsers}";
    bins = [
      "node"
      "npm"
      "npx"
      "bun"
      "bunx"
      "deno"
      "uv"
      "uvx"
      "playwright"
    ];
    versions = {
      bun = bun.version;
      deno = deno.version;
      playwright = playwright-driver.version;
      uv = uv.version;
    };
  };
  meta = {
    description = "Finite Agent Runtime baseline toolchains";
    longDescription = ''
      node/npm/npx (Hermes Node 26), bun, deno, uv, and the Playwright CLI
      plus browser blobs. Exposed on the container PATH so agents do not
      re-download toolchains into ephemeral writable layers.
    '';
    license = with lib.licenses; [
      mit
      asl20
    ];
  };
}

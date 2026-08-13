# Baseline CLIs staged into the canonical Agent Runtime image alongside the
# Nix-built Hermes closure. Pin through hermes-nixpkgs (callPackage from the
# flake) so the image carries one glibc family. Node is Hermes's wrapped
# Node 26 — do not add a second nixpkgs nodejs. Versions live on this
# derivation's passthru and in flake.lock; do not copy sha256s into Dockerfiles.
{
  lib,
  symlinkJoin,
  bun,
  deno,
  uv,
  ffmpeg,
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
    ffmpeg
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
      "ffmpeg"
      "ffprobe"
      "playwright"
    ];
    versions = {
      bun = bun.version;
      deno = deno.version;
      ffmpeg = ffmpeg.version;
      playwright = playwright-driver.version;
      uv = uv.version;
    };
  };
  meta = {
    description = "Finite Agent Runtime baseline toolchains";
    longDescription = ''
      node/npm/npx (Hermes Node 26), bun, deno, uv, ffmpeg/ffprobe, and the
      Playwright CLI plus browser blobs. Exposed on the container PATH so
      agents do not re-download toolchains into ephemeral writable layers.
    '';
    license = lib.licenses.mit;
  };
}

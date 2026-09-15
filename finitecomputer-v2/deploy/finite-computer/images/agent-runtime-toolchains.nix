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
  writeShellScriptBin,
  makeFontsConf,
  python3,
  dejavu_fonts,
  liberation_ttf,
  bun,
  deno,
  uv,
  playwright-driver,
  playwright-test,
  hermesAgent,
  simplexChat,
  fsiteCliV1,
}:
let
  nodejs = hermesAgent.hermesNpmLib.nodejs;
  browsers = playwright-driver.browsers;

  # HTML→PDF without pip: the pin's WeasyPrint brings its own pango/gobject
  # stack, so the CLI renders wherever the image runs (workarounds log
  # 2026-09-12: a pip-installed WeasyPrint cannot load libgobject under the
  # Nix interpreter). The withPackages env also exposes its own python3;
  # ship only the CLI so it cannot shadow the Hermes venv interpreter.
  weasyprint = python3.pkgs.weasyprint;
  weasyprintEnv = python3.withPackages (ps: [ ps.weasyprint ]);
  pdfFontsConf = makeFontsConf {
    fontDirectories = [
      dejavu_fonts
      liberation_ttf
    ];
  };
  weasyprintCli = writeShellScriptBin "weasyprint" ''
    export FONTCONFIG_FILE="${pdfFontsConf}"
    exec "${weasyprintEnv}/bin/weasyprint" "$@"
  '';
  # The pinned Chromium headless shell has no font wrapper of its own.
  # Supply the document fonts explicitly in the slim Runtime image too.
  playwrightCli = writeShellScriptBin "playwright" ''
    export FONTCONFIG_FILE="${pdfFontsConf}"
    exec "${playwright-test}/bin/playwright" "$@"
  '';
in
symlinkJoin {
  name = "agent-runtime-toolchains";
  paths = [
    nodejs
    bun
    deno
    uv
    playwrightCli
    browsers
    weasyprintCli
    simplexChat
    fsiteCliV1
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
      "weasyprint"
      "simplex-chat"
      "fsite"
    ];
    versions = {
      bun = bun.version;
      deno = deno.version;
      playwright = playwright-driver.version;
      uv = uv.version;
      weasyprint = weasyprint.version;
      simplex = simplexChat.version;
      fsiteV1 = fsiteCliV1.version;
    };
  };
  meta = {
    description = "Finite Agent Runtime baseline toolchains";
    longDescription = ''
      node/npm/npx (Hermes Node 26), bun, deno, uv, the Playwright CLI plus
      browser blobs, and the WeasyPrint HTML→PDF CLI. Exposed on the
      container PATH so agents do not re-download toolchains into ephemeral
      writable layers.
    '';
    license = with lib.licenses; [
      mit
      asl20
      bsd3
    ];
  };
}

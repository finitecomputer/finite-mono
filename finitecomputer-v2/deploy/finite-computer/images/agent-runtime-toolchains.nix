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
  gcc,
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
  weasyprintFontsConf = makeFontsConf {
    fontDirectories = [
      dejavu_fonts
      liberation_ttf
    ];
  };
  weasyprintCli = writeShellScriptBin "weasyprint" ''
    export FONTCONFIG_FILE="${weasyprintFontsConf}"
    exec "${weasyprintEnv}/bin/weasyprint" "$@"
  '';

  # The Hermes venv interpreter is the image's python3, but binary wheels
  # pip-installed into it expect a FHS libstdc++ the Nix build never put on
  # its loader path (workarounds log 2026-09-12: `import playwright.sync_api`
  # dies on greenlet's missing libstdc++.so.6; hand-pointing LD_LIBRARY_PATH
  # at system or foreign store paths segfaults). Expose python3 as a shim
  # that prepends the pin's own gcc runtime — same glibc family as every
  # other ELF staged here, and baked per image build, never by hand.
  hermesPython = writeShellScriptBin "python3" ''
    export LD_LIBRARY_PATH="${gcc.cc.lib}/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    exec "${hermesAgent.hermesVenv}/bin/python3" "$@"
  '';
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
    weasyprintCli
    hermesPython
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
      "python3"
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
      browser blobs, the WeasyPrint HTML→PDF CLI, and the Hermes venv
      python3 behind a libstdc++-loading shim. Exposed on the container PATH
      so agents do not re-download toolchains into ephemeral writable layers.
    '';
    license = with lib.licenses; [
      mit
      asl20
      bsd3
    ];
  };
}

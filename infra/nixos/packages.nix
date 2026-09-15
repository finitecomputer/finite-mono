# Rust package outputs shared by the root flake and NixOS hosts.
# Cargo.nix is upstream-generated from the root manifests and lockfile.
{ pkgs, sourceRoot }:
let
  inherit (pkgs) lib;

  # These CLIs embed files outside their crate directory. Keep their relative
  # layout without making every crate depend on the entire repository.
  embeddedSource = member: paths: {
    src = lib.fileset.toSource {
      root = sourceRoot;
      fileset = lib.fileset.unions (map (path: sourceRoot + "/${path}") paths);
    };
    workspace_member = member;
  };
  workspace = import (sourceRoot + "/Cargo.nix") {
    inherit pkgs;
    # Match Cargo's default release codegen parallelism.
    buildRustCrateForPkgs = p: p.buildRustCrate.override { defaultCodegenUnits = 16; };
    defaultCrateOverrides = pkgs.defaultCrateOverrides // {
      fsite-cli =
        _:
        embeddedSource "finite-sites/crates/fsite-cli" [
          "finite-sites/crates/fsite-cli"
          "finite-sites/examples"
        ];
      finitechat-cli =
        _:
        embeddedSource "finitechat/crates/finitechat-cli" [
          "finitechat/crates/finitechat-cli"
          "finitechat/integrations/hermes/finitechat/__init__.py"
          "finitechat/integrations/hermes/finitechat/adapter.py"
          "finitechat/integrations/hermes/finitechat/plugin.yaml"
        ];
      finitechat-server = attrs: {
        # Includes the resolved dependency graph as well as the server source.
        # It changes when any compiled input changes, never with unrelated code.
        FINITECHAT_BUILD_FINGERPRINT = "nix-${
          builtins.substring 0 32 (
            builtins.hashString "sha256" (
              toString attrs.src + lib.concatMapStrings (dep: dep.drvPath) attrs.dependencies
            )
          )
        }";
        FINITECHAT_BUILD_DIRTY = "false";
      };
    };
  };
  package =
    crate: mainProgram:
    let
      built = workspace.workspaceMembers.${crate}.build;
    in
    built.overrideAttrs (old: {
      meta = (old.meta or { }) // {
        inherit mainProgram;
      };
      passthru =
        (old.passthru or { })
        // lib.optionalAttrs (crate == "finitechat-server") {
          sourceFingerprint = built.FINITECHAT_BUILD_FINGERPRINT;
        };
    });
in
rec {
  devfinity-unwrapped = package "devfinity" "devfinity";
  devfinity =
    let
      runtimeInputs = [
        devfinity-unwrapped
        finite-saas-core
        finite-saas-local
        finite-saas-runner
        finitechat-server
        finitechat-hosted-device
        finitesitesd
        finite-identity
        finite-brain
        fsite
        fbrain
        pkgs.curl
        pkgs.git
        pkgs.jq
        pkgs.nodejs_24
        pkgs.pnpm
        pkgs.postgresql_16
        pkgs.process-compose
        pkgs.python3
        pkgs.sqlite
      ];
    in
    pkgs.writeShellApplication {
      name = "devfinity";
      inherit runtimeInputs;
      text = ''
        exec ${devfinity-unwrapped}/bin/devfinity "$@"
      '';
      meta.mainProgram = "devfinity";
      passthru = {
        inherit runtimeInputs;
        unwrapped = devfinity-unwrapped;
      };
    };

  finite-saas-core = package "finite-saas-core" "finite-saas-core";
  finite-saas-runner = package "finite-saas-runner" "finite-saas-runner";
  finite-saas-local = package "finite-saas-local" "finite-saas-local";
  finitechat-server = package "finitechat-server" "finitechat-server";
  finitechat-hosted-device = package "finitechat-hosted-device" "finitechat-hosted-device";
  finite-agentd = package "finite-agentd" "finite-agentd";
  finitesitesd = package "finitesitesd" "finitesitesd";
  finite-brain = package "finite-brain-app" "finite-brain";
  finite-identity = package "finite-identity" "finite-identityd";
  fsite = package "fsite-cli" "fsite";
  fbrain = package "finite-brain-cli" "fbrain";
  finitechat = package "finitechat-cli" "finitechat";

  # Static Rust libraries are build-time inputs, absent from runtime closures.
  # Retain and publish them explicitly so a fresh runner can reuse each crate.
  rust-build-cache =
    let
      node = value: {
        key = toString value;
        inherit value;
      };
      closure = lib.genericClosure {
        startSet = map node [
          devfinity-unwrapped
          finite-saas-core
          finite-saas-runner
          finite-saas-local
          finitechat-server
          finitechat-hosted-device
          finite-agentd
          finitesitesd
          finite-brain
          finite-identity
          fsite
          fbrain
          finitechat
        ];
        operator =
          item: map node ((item.value.dependencies or [ ]) ++ (item.value.buildDependencies or [ ]));
      };
    in
    pkgs.linkFarm "rust-build-cache" (
      lib.imap0 (index: item: {
        name = toString index;
        path = item.value;
      }) closure
    );
  finitesitesd-legacy-canonical = pkgs.stdenvNoCC.mkDerivation {
    pname = "finitesitesd-legacy-canonical";
    version = "0.5.3";
    src = pkgs.fetchurl {
      url = "https://github.com/finitecomputer/finite-releases/releases/download/fsite/v0.5.3/finitesitesd-linux-x86_64.tar.gz";
      hash = "sha256-uw2RFWLzGRRJuVFGoMWlrc0wUt0AorQ4vTfS2tcbbx8=";
    };
    # The release tarball is built on an Ubuntu runner and links against its
    # glibc (interpreter /lib64/ld-linux-x86-64.so.2, libgcc_s/libc/libm),
    # none of which exist on NixOS: the daemon exited 127 at first launch
    # (2026-09-07). autoPatchelf rebinds the interpreter and library rpath
    # into the nix store.
    nativeBuildInputs = [ pkgs.autoPatchelfHook ];
    buildInputs = [
      pkgs.glibc
      pkgs.libgcc
    ];
    dontUnpack = true;
    installPhase = ''
      mkdir -p "$out/bin"
      tar -xzf "$src" -C "$out/bin" finitesitesd
      chmod 0555 "$out/bin/finitesitesd"
    '';
    # Execute the binary at build time so a loader/linkage regression fails
    # the build instead of the production unit.
    doInstallCheck = true;
    installCheckPhase = "$out/bin/finitesitesd --version";
    passthru = {
      sourceTag = "fsite/v0.5.3";
      # Release-prep commit on release/fsite-0.5.3: the deploy-2 revision
      # 7c04a681 plus the 0.5.3 version bump. v0.5.2 predates the Aug-31
      # daemon-local email-proof change (dfa765f3) that canonical lat2 runs.
      sourceSha = "662f837ed5875de4cbf55acd1d1adfe288e762a1";
    };
    meta = {
      mainProgram = "finitesitesd";
      platforms = [ "x86_64-linux" ];
    };
  };
}

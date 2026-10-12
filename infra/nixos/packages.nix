# Rust package outputs shared by the root flake and NixOS hosts.
# Cargo.nix is upstream-generated from the root manifests and lockfile.
{ pkgs, sourceRoot }:
let
  inherit (pkgs) lib;

  workspaceManifest = builtins.fromTOML (builtins.readFile (sourceRoot + "/Cargo.toml"));
  # Preserve the release-only source boundary: docs and test fixtures must not
  # invalidate a service derivation and cause a NixOS restart. Cargo.nix owns
  # dependency resolution; copying the root lockfile into each source would
  # defeat per-crate caching on unrelated dependency changes.
  compileInputs = {
    "finite-saas-core" = [ "finitecomputer-v2/crates/finite-saas-core/migrations" ];
    "finitechat-cli" = [
      "finitechat/integrations/hermes/finitechat/__init__.py"
      "finitechat/integrations/hermes/finitechat/adapter.py"
      "finitechat/integrations/hermes/finitechat/plugin.yaml"
      "finitechat/integrations/hermes/finitechat/simplex_topics.py"
    ];
  };
  localSources = builtins.listToAttrs (
    map (
      member:
      let
        root = sourceRoot + "/${member}";
        manifest = builtins.fromTOML (builtins.readFile (root + "/Cargo.toml"));
        name = manifest.package.name;
        fileset = lib.fileset.unions (
          [
            (root + "/Cargo.toml")
            (root + "/src")
          ]
          ++ lib.optional (builtins.pathExists (root + "/build.rs")) (root + "/build.rs")
          ++ map (path: sourceRoot + "/${path}") (compileInputs.${name} or [ ])
        );
      in
      {
        inherit name;
        value = {
          src = lib.fileset.toSource {
            inherit fileset;
            root = sourceRoot;
          };
          workspace_member = member;
          files = map (file: lib.path.removePrefix sourceRoot file) (lib.fileset.toList fileset);
        };
      }
    ) workspaceManifest.workspace.members
  );
  localOverrides = lib.mapAttrs (
    name: source: attrs:
    (pkgs.defaultCrateOverrides.${name} or (_: { })) attrs // { inherit (source) src workspace_member; }
  ) localSources;
  crateClosure =
    roots:
    lib.genericClosure {
      startSet = map (value: {
        key = toString value;
        inherit value;
      }) roots;
      operator =
        item:
        map (value: {
          key = toString value;
          inherit value;
        }) ((item.value.dependencies or [ ]) ++ (item.value.buildDependencies or [ ]));
    };
  sourceFiles =
    built:
    lib.unique (
      lib.concatMap (
        item: (localSources.${item.value.crateName} or { files = [ ]; }).files
      ) (crateClosure [ built ])
    );
  workspace = import (sourceRoot + "/Cargo.nix") {
    inherit pkgs;
    # Match Cargo's default release codegen parallelism.
    buildRustCrateForPkgs = p: p.buildRustCrate.override { defaultCodegenUnits = 16; };
    defaultCrateOverrides =
      pkgs.defaultCrateOverrides
      // localOverrides
      // {
        finitechat-server =
          attrs:
          localOverrides.finitechat-server attrs
          // {
            # Includes the resolved dependency graph as well as the server source.
            # It changes when any compiled input changes, never with unrelated code.
            FINITECHAT_BUILD_FINGERPRINT = "nix-${
              builtins.substring 0 32 (
                builtins.hashString "sha256" (
                  toString localSources.finitechat-server.src
                  + lib.concatMapStrings (dep: dep.drvPath) attrs.dependencies
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
        // {
          sourceFiles = sourceFiles built;
        }
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
        sourceFiles = devfinity-unwrapped.sourceFiles;
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
      closure = crateClosure [
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
    in
    pkgs.linkFarm "rust-build-cache" (
      lib.imap0 (index: item: {
        name = toString index;
        path = item.value;
      }) closure
    );
}

# Nix builds of the workspace server binaries + CLIs, shared by flake.nix.
# Each package receives a generated workspace manifest plus only its transitive
# local crate closure. Crane first compiles that scoped package's dependencies
# from dummy sources, then reuses those artifacts for the real application build.
# Keep these path lists aligned with Cargo path dependencies; the Nix package-build
# CI lane catches omissions.
# A missing path dependency usually surfaces there as Cargo's "failed to load
# manifest" or "no targets specified" error; update that package's sourcePaths.
# doCheck = false: tests run in CI via cargo; nix builds stay fast/reliable.
{
  craneLib,
  pkgs,
  sourceRoot,
}:
let
  inherit (pkgs) lib;

  workspaceManifest = builtins.fromTOML (builtins.readFile (sourceRoot + "/Cargo.toml"));
  workspaceMembers = workspaceManifest.workspace.members;

  scopedSources =
    paths:
    let
      members = builtins.filter (member: builtins.elem member paths) workspaceMembers;
      manifest = (pkgs.formats.toml { }).generate "Cargo.toml" (
        workspaceManifest
        // {
          workspace = workspaceManifest.workspace // {
            inherit members;
          };
        }
      );
      files = lib.fileset.toSource {
        root = sourceRoot;
        fileset = lib.fileset.unions (
          [
            (sourceRoot + "/Cargo.lock")
            (sourceRoot + "/Cargo.toml")
          ]
          ++ map (path: sourceRoot + "/${path}") paths
        );
      };
      app = pkgs.runCommand "source" { } ''
        cp -R ${files} "$out"
        chmod u+w "$out" "$out/Cargo.toml"
        cp ${manifest} "$out/Cargo.toml"
      '';
    in
    {
      inherit app files manifest;
    };

  crateVersion =
    dir: (builtins.fromTOML (builtins.readFile (sourceRoot + "/${dir}/Cargo.toml"))).package.version;

  mkWorkspaceCrate =
    {
      pname,
      crate ? pname,
      dir,
      sourcePaths,
      cargoExtraArgs ? "--offline -p ${crate}",
      exposeSourceFingerprint ? false,
      mainProgram ? pname,
      dummySourceAttrs ? { },
      extraAttrs ? { },
    }:
    let
      sources = scopedSources sourcePaths;
      src = sources.app;
      version = crateVersion dir;
      cargoVendorDir = craneLib.vendorCargoDeps {
        cargoLock = sourceRoot + "/Cargo.lock";
      };
      commonArgs = {
        inherit
          cargoVendorDir
          pname
          src
          version
          ;
        # The scoped workspace has fewer members than the root lock records,
        # so Cargo must be allowed to normalize its build-directory copy. The
        # vendored root lock remains the only available dependency universe.
        inherit cargoExtraArgs;
        strictDeps = true;
        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = [ pkgs.openssl ];
      };
      dummySrc = craneLib.mkDummySrc (
        {
          src = sources.files;
          cargoLock = sourceRoot + "/Cargo.lock";
          # mkDummySrc reads the real root manifest to discover Cargo targets,
          # then this restores the same scoped workspace used by the app build.
          extraDummyScript = ''
            chmod u+w "$out/Cargo.toml"
            cp ${sources.manifest} "$out/Cargo.toml"
          '';
        }
        // dummySourceAttrs
      );
      cargoArtifacts = craneLib.buildDepsOnly (
        (builtins.removeAttrs commonArgs [ "src" ])
        // {
          inherit dummySrc;
          # CI consumes release build artifacts; workspace checks/tests run in
          # their own lanes, so skip buildDepsOnly's additional cargo check.
          cargoCheckCommand = ":";
          doCheck = false;
        }
      );
      sourceFingerprint = "nix-${builtins.substring 0 32 (builtins.baseNameOf (toString src))}";
      fingerprintBuildAttrs = lib.optionalAttrs exposeSourceFingerprint {
        FINITECHAT_BUILD_FINGERPRINT = sourceFingerprint;
        # Nix builds an immutable scoped snapshot. This describes that build
        # input; it does not inspect the caller's Git working-tree status.
        FINITECHAT_BUILD_DIRTY = "false";
      };
      fingerprintPassthru = lib.optionalAttrs exposeSourceFingerprint {
        inherit sourceFingerprint;
      };
      appAttrs = builtins.removeAttrs extraAttrs [ "passthru" ];
    in
    craneLib.buildPackage (
      commonArgs
      // {
        inherit cargoArtifacts;
        doCheck = false;
        meta.mainProgram = mainProgram;
      }
      // fingerprintBuildAttrs
      // appAttrs
      // {
        passthru = (extraAttrs.passthru or { }) // fingerprintPassthru // { inherit cargoArtifacts; };
      }
    );
in
rec {
  # CI/local harness
  devfinity-unwrapped = mkWorkspaceCrate {
    pname = "devfinity";
    dir = "devfinity";
    sourcePaths = [ "devfinity" ];
    dummySourceAttrs.cleanCargoTomlFilter =
      path:
      craneLib.filters.cargoTomlDefault path
      &&
        path != [
          "dev-dependencies"
          "finitechat-core"
        ];
    extraAttrs.postPatch = ''
      # The binary package does not run devfinity tests; avoid pulling the
      # test-only finitechat-core path dependency into this Nix source closure.
      substituteInPlace devfinity/Cargo.toml \
        --replace-fail 'finitechat-core = { path = "../finitechat/crates/finitechat-core" }' ""
    '';
  };

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

  # Servers
  finite-saas-core = mkWorkspaceCrate {
    pname = "finite-saas-core";
    dir = "finitecomputer-v2/crates/finite-saas-core";
    sourcePaths = [
      "finitecomputer-v2/crates/finite-saas-core"
    ];
  };
  finite-saas-runner = mkWorkspaceCrate {
    pname = "finite-saas-runner";
    dir = "finitecomputer-v2/crates/finite-saas-runner";
    sourcePaths = [
      "finitecomputer-v2/crates/finite-saas-core"
      "finitecomputer-v2/crates/finite-saas-runner"
    ];
  };
  finite-saas-local = mkWorkspaceCrate {
    pname = "finite-saas-local";
    dir = "finitecomputer-v2/crates/finite-saas-local";
    sourcePaths = [
      "finitecomputer-v2/crates/finite-private-limiter"
      "finitecomputer-v2/crates/finite-saas-core"
      "finitecomputer-v2/crates/finite-saas-local"
    ];
  };
  finitechat-server = mkWorkspaceCrate {
    pname = "finitechat-server";
    dir = "finitechat/crates/finitechat-server";
    sourcePaths = [
      "finitechat/crates/finitechat-blob"
      "finitechat/crates/finitechat-delivery"
      "finitechat/crates/finitechat-http"
      "finitechat/crates/finitechat-proto"
      "finitechat/crates/finitechat-server"
      "finitechat/crates/finitechat-transport"
    ];
    exposeSourceFingerprint = true;
  };
  finitechat-hosted-device = mkWorkspaceCrate {
    pname = "finitechat-hosted-device";
    dir = "finitechat/crates/finitechat-hosted-device";
    sourcePaths = [
      "finite-brain/crates/finite-brain-core"
      "finite-brain/crates/finite-brain-server"
      "finite-brain/crates/finite-brain-store"
      "finite-identity"
      "finite-nostr"
      "finitechat/crates/finitechat-blob"
      "finitechat/crates/finitechat-client"
      "finitechat/crates/finitechat-core"
      "finitechat/crates/finitechat-delivery"
      "finitechat/crates/finitechat-hermes"
      "finitechat/crates/finitechat-hosted-device"
      "finitechat/crates/finitechat-http"
      "finitechat/crates/finitechat-mls"
      "finitechat/crates/finitechat-proto"
      "finitechat/crates/finitechat-server"
      "finitechat/crates/finitechat-transport"
    ];
  };
  finite-agentd = mkWorkspaceCrate {
    pname = "finite-agentd";
    dir = "finite-agentd";
    sourcePaths = [
      "finite-agentd"
      "finitechat/crates/finitechat-proto"
    ];
  };
  finite-specialization-worker = mkWorkspaceCrate {
    pname = "finite-specialization-worker";
    dir = "finitecomputer-v2/crates/finite-specialization-worker";
    sourcePaths = [ "finitecomputer-v2/crates/finite-specialization-worker" ];
  };
  finitesitesd = mkWorkspaceCrate {
    pname = "finitesitesd";
    dir = "finite-sites/crates/finitesitesd";
    sourcePaths = [
      "finite-sites/crates/finitesites-blob"
      "finite-sites/crates/finitesites-engine"
      "finite-sites/crates/finitesites-proto"
      "finite-sites/crates/finitesites-store"
      "finite-sites/crates/finitesitesd"
    ];
  };
  # Crate finite-brain-app; the installed bin is named finite-brain.
  finite-brain = mkWorkspaceCrate {
    pname = "finite-brain";
    crate = "finite-brain-app";
    dir = "finite-brain/crates/finite-brain-app";
    sourcePaths = [
      "finite-brain/crates/finite-brain-app"
      "finite-brain/crates/finite-brain-core"
      "finite-brain/crates/finite-brain-server"
      "finite-brain/crates/finite-brain-store"
      "finite-nostr"
    ];
  };
  finite-identity = mkWorkspaceCrate {
    pname = "finite-identity";
    dir = "finite-identity";
    sourcePaths = [ "finite-identity" ];
    mainProgram = "finite-identityd";
  };

  # CLIs (same mechanism, trivial to carry along)
  fsite = mkWorkspaceCrate {
    pname = "fsite";
    crate = "fsite-cli";
    dir = "finite-sites/crates/fsite-cli";
    sourcePaths = [
      "finite-identity"
      "finite-sites/crates/finitesites-proto"
      "finite-sites/crates/fsite-cli"
      "finite-sites/examples"
    ];
  };
  fbrain = mkWorkspaceCrate {
    pname = "fbrain";
    crate = "finite-brain-cli";
    dir = "finite-brain/crates/finite-brain-cli";
    sourcePaths = [
      "finite-brain/crates/finite-brain-cli"
      "finite-brain/crates/finite-brain-core"
      "finite-brain/crates/finite-brain-server"
      "finite-brain/crates/finite-brain-store"
      "finite-identity"
      "finite-nostr"
    ];
  };
  finitechat = mkWorkspaceCrate {
    pname = "finitechat";
    crate = "finitechat-cli";
    dir = "finitechat/crates/finitechat-cli";
    sourcePaths = [
      "finite-identity"
      "finite-nostr"
      "finitechat/crates/finitechat-blob"
      "finitechat/crates/finitechat-cli"
      "finitechat/crates/finitechat-client"
      "finitechat/crates/finitechat-core"
      "finitechat/crates/finitechat-delivery"
      "finitechat/crates/finitechat-hermes"
      "finitechat/crates/finitechat-http"
      "finitechat/crates/finitechat-mls"
      "finitechat/crates/finitechat-proto"
      "finitechat/crates/finitechat-server"
      "finitechat/crates/finitechat-transport"
      "finitechat/integrations/hermes/finitechat/__init__.py"
      "finitechat/integrations/hermes/finitechat/adapter.py"
      "finitechat/integrations/hermes/finitechat/plugin.yaml"
    ];
  };
  finitechat-rmp = mkWorkspaceCrate {
    pname = "finitechat-rmp";
    dir = "finitechat/crates/finitechat-rmp";
    sourcePaths = [
      "finite-identity"
      "finite-nostr"
      "finitechat/crates/finitechat-blob"
      "finitechat/crates/finitechat-client"
      "finitechat/crates/finitechat-core"
      "finitechat/crates/finitechat-delivery"
      "finitechat/crates/finitechat-hermes"
      "finitechat/crates/finitechat-http"
      "finitechat/crates/finitechat-mls"
      "finitechat/crates/finitechat-proto"
      "finitechat/crates/finitechat-rmp"
      "finitechat/crates/finitechat-server"
      "finitechat/crates/finitechat-transport"
    ];
  };
}

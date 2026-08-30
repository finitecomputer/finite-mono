# Nix builds of the workspace server binaries + CLIs, shared by flake.nix.
# Each package receives a generated workspace manifest plus only its transitive
# local crate closure. Related binaries share product-family dependency artifacts;
# every real application build retains its own narrower source closure.
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

  cargoVendorDir = craneLib.vendorCargoDeps {
    cargoLock = sourceRoot + "/Cargo.lock";
  };

  mkCargoArtifacts =
    {
      pname,
      version,
      sourcePaths,
      cargoExtraArgs,
      cargoBuildArgs ? [ cargoExtraArgs ],
      dummySourceAttrs ? { },
    }:
    let
      sources = scopedSources sourcePaths;
      dummySrc = craneLib.mkDummySrc (
        {
          src = sources.files;
          cargoLock = sourceRoot + "/Cargo.lock";
          # mkDummySrc reads the real root manifest to discover Cargo targets,
          # then this restores the scoped workspace used by the app builds.
          extraDummyScript = ''
            chmod u+w "$out/Cargo.toml"
            cp ${sources.manifest} "$out/Cargo.toml"
          '';
        }
        // dummySourceAttrs
      );
    in
    craneLib.buildDepsOnly {
      inherit
        cargoExtraArgs
        cargoVendorDir
        dummySrc
        pname
        version
        ;
      # Run grouped members separately so the archive contains each package's
      # exact Cargo feature resolution. A single multi-package invocation
      # unifies features and makes the narrower final builds recompile deps.
      buildPhaseCargoCommand = lib.concatMapStringsSep "\n" (
        args: "cargoWithProfile build ${args}"
      ) cargoBuildArgs;
      strictDeps = true;
      nativeBuildInputs = [ pkgs.pkg-config ];
      buildInputs = [ pkgs.openssl ];
      # CI consumes release build artifacts; workspace checks/tests run in
      # their own lanes, so skip buildDepsOnly's additional cargo check.
      cargoCheckCommand = ":";
      doCheck = false;
    };

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
      sharedCargoArtifacts ? null,
      cargoArtifactGroup ? pname,
      extraAttrs ? { },
    }:
    let
      sources = scopedSources sourcePaths;
      src = sources.app;
      version = crateVersion dir;
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
      cargoArtifacts =
        if sharedCargoArtifacts != null then
          sharedCargoArtifacts
        else
          mkCargoArtifacts {
            inherit
              cargoExtraArgs
              dummySourceAttrs
              pname
              sourcePaths
              version
              ;
          };
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
        passthru =
          (extraAttrs.passthru or { })
          // fingerprintPassthru
          // {
            inherit cargoArtifactGroup cargoArtifacts;
          };
      }
    );

  finiteSaasCoreSourcePaths = [
    "finitecomputer-v2/crates/finite-saas-core"
  ];
  finiteSaasRunnerSourcePaths = [
    "finitecomputer-v2/crates/finite-saas-core"
    "finitecomputer-v2/crates/finite-saas-runner"
  ];
  finiteSaasLocalSourcePaths = [
    "finitecomputer-v2/crates/finite-private-limiter"
    "finitecomputer-v2/crates/finite-saas-core"
    "finitecomputer-v2/crates/finite-saas-local"
  ];
  finiteSaasCargoArtifacts = mkCargoArtifacts {
    pname = "finite-saas-group";
    version = crateVersion "finitecomputer-v2/crates/finite-saas-core";
    sourcePaths = lib.unique (
      finiteSaasCoreSourcePaths ++ finiteSaasRunnerSourcePaths ++ finiteSaasLocalSourcePaths
    );
    cargoExtraArgs = "--offline -p finite-saas-core";
    cargoBuildArgs = [
      "--offline -p finite-saas-core"
      "--offline -p finite-saas-runner"
      "--offline -p finite-saas-local"
    ];
  };

  finitechatServerSourcePaths = [
    "finite-nostr"
    "finitechat/crates/finitechat-blob"
    "finitechat/crates/finitechat-delivery"
    "finitechat/crates/finitechat-http"
    "finitechat/crates/finitechat-proto"
    "finitechat/crates/finitechat-server"
    "finitechat/crates/finitechat-transport"
  ];
  finitechatHostedDeviceSourcePaths = [
    "finite-brain/crates/finite-brain-core"
    "finite-brain/crates/finite-brain-server"
    "finite-brain/crates/finite-brain-store"
    "finite-identity"
    "finite-mail"
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
  finitechatCliSourcePaths = [
    "finite-identity"
    "finite-mail"
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
  finitechatRmpSourcePaths = [
    "finite-identity"
    "finite-mail"
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
  finitechatCargoArtifacts = mkCargoArtifacts {
    pname = "finitechat-group";
    version = crateVersion "finitechat/crates/finitechat-hosted-device";
    sourcePaths = lib.unique (
      finitechatServerSourcePaths
      ++ finitechatHostedDeviceSourcePaths
      ++ finitechatCliSourcePaths
      ++ finitechatRmpSourcePaths
    );
    cargoExtraArgs = "--offline -p finitechat-server";
    cargoBuildArgs = [
      "--offline -p finitechat-server"
      "--offline -p finitechat-hosted-device"
      "--offline -p finitechat-cli"
      "--offline -p finitechat-rmp"
    ];
  };

  finiteBrainAppSourcePaths = [
    "finite-brain/crates/finite-brain-app"
    "finite-brain/crates/finite-brain-core"
    "finite-brain/crates/finite-brain-server"
    "finite-brain/crates/finite-brain-store"
    "finite-mail"
    "finite-nostr"
  ];
  finiteBrainCliSourcePaths = [
    "finite-brain/crates/finite-brain-cli"
    "finite-brain/crates/finite-brain-core"
    "finite-brain/crates/finite-brain-server"
    "finite-brain/crates/finite-brain-store"
    "finite-identity"
    "finite-mail"
    "finite-nostr"
  ];
  finiteBrainCargoArtifacts = mkCargoArtifacts {
    pname = "finite-brain-group";
    version = crateVersion "finite-brain/crates/finite-brain-app";
    sourcePaths = lib.unique (finiteBrainAppSourcePaths ++ finiteBrainCliSourcePaths);
    cargoExtraArgs = "--offline -p finite-brain-app";
    cargoBuildArgs = [
      "--offline -p finite-brain-app"
      "--offline -p finite-brain-cli"
    ];
  };
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
        cargoArtifactGroup = devfinity-unwrapped.cargoArtifactGroup;
        cargoArtifacts = devfinity-unwrapped.cargoArtifacts;
        unwrapped = devfinity-unwrapped;
      };
    };

  # Servers
  finite-saas-core = mkWorkspaceCrate {
    pname = "finite-saas-core";
    dir = "finitecomputer-v2/crates/finite-saas-core";
    sourcePaths = finiteSaasCoreSourcePaths;
    sharedCargoArtifacts = finiteSaasCargoArtifacts;
    cargoArtifactGroup = "finite-saas";
  };
  finite-saas-runner = mkWorkspaceCrate {
    pname = "finite-saas-runner";
    dir = "finitecomputer-v2/crates/finite-saas-runner";
    sourcePaths = finiteSaasRunnerSourcePaths;
    sharedCargoArtifacts = finiteSaasCargoArtifacts;
    cargoArtifactGroup = "finite-saas";
  };
  finite-saas-local = mkWorkspaceCrate {
    pname = "finite-saas-local";
    dir = "finitecomputer-v2/crates/finite-saas-local";
    sourcePaths = finiteSaasLocalSourcePaths;
    sharedCargoArtifacts = finiteSaasCargoArtifacts;
    cargoArtifactGroup = "finite-saas";
  };
  finitechat-server = mkWorkspaceCrate {
    pname = "finitechat-server";
    dir = "finitechat/crates/finitechat-server";
    sourcePaths = finitechatServerSourcePaths;
    sharedCargoArtifacts = finitechatCargoArtifacts;
    cargoArtifactGroup = "finitechat";
    exposeSourceFingerprint = true;
  };
  finitechat-hosted-device = mkWorkspaceCrate {
    pname = "finitechat-hosted-device";
    dir = "finitechat/crates/finitechat-hosted-device";
    sourcePaths = finitechatHostedDeviceSourcePaths;
    sharedCargoArtifacts = finitechatCargoArtifacts;
    cargoArtifactGroup = "finitechat";
  };
  finite-agentd = mkWorkspaceCrate {
    pname = "finite-agentd";
    dir = "finite-agentd";
    sourcePaths = [
      "finite-agentd"
      "finitechat/crates/finitechat-proto"
    ];
  };
  finitesitesd = mkWorkspaceCrate {
    pname = "finitesitesd";
    dir = "finite-sites/crates/finitesitesd";
    sourcePaths = [
      "finite-mail"
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
    sourcePaths = finiteBrainAppSourcePaths;
    sharedCargoArtifacts = finiteBrainCargoArtifacts;
    cargoArtifactGroup = "finite-brain";
  };
  finite-identity = mkWorkspaceCrate {
    pname = "finite-identity";
    dir = "finite-identity";
    sourcePaths = [
      "finite-identity"
      "finite-mail"
    ];
    mainProgram = "finite-identityd";
  };

  # CLIs (same mechanism, trivial to carry along)
  fsite = mkWorkspaceCrate {
    pname = "fsite";
    crate = "fsite-cli";
    dir = "finite-sites/crates/fsite-cli";
    sourcePaths = [
      "finite-identity"
      "finite-mail"
      "finite-sites/crates/finitesites-proto"
      "finite-sites/crates/fsite-cli"
      "finite-sites/examples"
    ];
  };
  fbrain = mkWorkspaceCrate {
    pname = "fbrain";
    crate = "finite-brain-cli";
    dir = "finite-brain/crates/finite-brain-cli";
    sourcePaths = finiteBrainCliSourcePaths;
    sharedCargoArtifacts = finiteBrainCargoArtifacts;
    cargoArtifactGroup = "finite-brain";
  };
  finitechat = mkWorkspaceCrate {
    pname = "finitechat";
    crate = "finitechat-cli";
    dir = "finitechat/crates/finitechat-cli";
    sourcePaths = finitechatCliSourcePaths;
    sharedCargoArtifacts = finitechatCargoArtifacts;
    cargoArtifactGroup = "finitechat";
  };
  finitechat-rmp = mkWorkspaceCrate {
    pname = "finitechat-rmp";
    dir = "finitechat/crates/finitechat-rmp";
    sourcePaths = finitechatRmpSourcePaths;
    sharedCargoArtifacts = finitechatCargoArtifacts;
    cargoArtifactGroup = "finitechat";
  };
}

# Nix builds of the workspace server binaries + CLIs, shared by flake.nix.
# Each package receives a generated workspace manifest plus only its transitive
# local crate closure. Keep these path lists aligned with Cargo path dependencies;
# the Nix package-build CI lane catches omissions. The root Cargo.lock has git deps
# (hypernote-mdx, pinned finitechat crates), hence allowBuiltinFetchGit.
# A missing path dependency usually surfaces there as Cargo's "failed to load
# manifest" or "no targets specified" error; update that package's sourcePaths.
# doCheck = false: tests run in CI via cargo; nix builds stay fast/reliable.
{
  pkgs,
  sourceRoot,
}:
let
  inherit (pkgs) lib;

  workspaceManifest = builtins.fromTOML (builtins.readFile (sourceRoot + "/Cargo.toml"));
  workspaceMembers = workspaceManifest.workspace.members;

  scopedSource =
    name: paths:
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
          [ (sourceRoot + "/Cargo.lock") ] ++ map (path: sourceRoot + "/${path}") paths
        );
      };
    in
    pkgs.runCommand "${name}-source" { } ''
      cp -R ${files} "$out"
      chmod u+w "$out"
      cp ${manifest} "$out/Cargo.toml"
    '';

  crateVersion =
    dir: (builtins.fromTOML (builtins.readFile (sourceRoot + "/${dir}/Cargo.toml"))).package.version;

  mkWorkspaceCrate =
    {
      pname,
      crate ? pname,
      dir,
      sourcePaths,
      exposeSourceFingerprint ? false,
      mainProgram ? pname,
      extraAttrs ? { },
    }:
    let
      src = scopedSource pname sourcePaths;
      sourceFingerprint = "nix-${builtins.substring 0 32 (builtins.baseNameOf (toString src))}";
      fingerprintAttrs = lib.optionalAttrs exposeSourceFingerprint {
        FINITECHAT_BUILD_FINGERPRINT = sourceFingerprint;
        # Nix builds an immutable scoped snapshot. This describes that build
        # input; it does not inspect the caller's Git working-tree status.
        FINITECHAT_BUILD_DIRTY = "false";
        passthru.sourceFingerprint = sourceFingerprint;
      };
    in
    pkgs.rustPlatform.buildRustPackage (
      {
        inherit pname src;
        version = crateVersion dir;
        cargoLock = {
          lockFile = sourceRoot + "/Cargo.lock";
          allowBuiltinFetchGit = true;
        };
        cargoBuildFlags = [
          "-p"
          crate
        ];
        doCheck = false;
        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = [ pkgs.openssl ];
        meta.mainProgram = mainProgram;
      }
      // fingerprintAttrs
      // extraAttrs
    );
in
{
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
}

# Nix builds of the workspace server binaries + CLIs, shared by flake.nix.
# One mechanism for every crate: buildRustPackage over the whole workspace
# with `cargoBuildFlags = -p <crate>`. The root Cargo.lock has git deps
# (hypernote-mdx, pinned finitechat crates), hence allowBuiltinFetchGit.
# doCheck = false: tests run in CI via cargo; nix builds stay fast/reliable.
{ pkgs, src }:
let
  crateVersion =
    dir: (builtins.fromTOML (builtins.readFile (src + "/${dir}/Cargo.toml"))).package.version;

  # A flake fetched by exact Git revision has no .git directory for build.rs
  # to inspect. Carry the immutable source revision into the server compiler
  # environment instead; dirty local flakes intentionally omit provenance.
  finitechatProvenance =
    if src ? rev then
      {
        FINITECHAT_BUILD_COMMIT = builtins.substring 0 12 src.rev;
        FINITECHAT_BUILD_DIRTY = "false";
      }
    else
      { };

  mkWorkspaceCrate =
    {
      pname,
      crate ? pname,
      dir,
      mainProgram ? pname,
      extraAttrs ? { },
    }:
    pkgs.rustPlatform.buildRustPackage (
      {
        inherit pname src;
        version = crateVersion dir;
        cargoLock = {
          lockFile = src + "/Cargo.lock";
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
      // extraAttrs
    );
in
{
  # Servers
  finite-saas-core = mkWorkspaceCrate {
    pname = "finite-saas-core";
    dir = "finitecomputer-v2/crates/finite-saas-core";
  };
  finite-saas-runner = mkWorkspaceCrate {
    pname = "finite-saas-runner";
    dir = "finitecomputer-v2/crates/finite-saas-runner";
  };
  finitechat-server = mkWorkspaceCrate {
    pname = "finitechat-server";
    dir = "finitechat/crates/finitechat-server";
    extraAttrs = finitechatProvenance;
  };
  finitechat-hosted-device = mkWorkspaceCrate {
    pname = "finitechat-hosted-device";
    dir = "finitechat/crates/finitechat-hosted-device";
  };
  finite-agentd = mkWorkspaceCrate {
    pname = "finite-agentd";
    dir = "finite-agentd";
  };
  finite-specialization-worker = mkWorkspaceCrate {
    pname = "finite-specialization-worker";
    dir = "finitecomputer-v2/crates/finite-specialization-worker";
  };
  finitesitesd = mkWorkspaceCrate {
    pname = "finitesitesd";
    dir = "finite-sites/crates/finitesitesd";
  };
  # Crate finite-brain-app; the installed bin is named finite-brain.
  finite-brain = mkWorkspaceCrate {
    pname = "finite-brain";
    crate = "finite-brain-app";
    dir = "finite-brain/crates/finite-brain-app";
  };

  # CLIs (same mechanism, trivial to carry along)
  fsite = mkWorkspaceCrate {
    pname = "fsite";
    crate = "fsite-cli";
    dir = "finite-sites/crates/fsite-cli";
  };
  fbrain = mkWorkspaceCrate {
    pname = "fbrain";
    crate = "finite-brain-cli";
    dir = "finite-brain/crates/finite-brain-cli";
  };
  finitechat = mkWorkspaceCrate {
    pname = "finitechat";
    crate = "finitechat-cli";
    dir = "finitechat/crates/finitechat-cli";
  };
}

# Evaluation only: compare cache identities on the production Linux target.
{ source, system ? "x86_64-linux" }:
let
  flake = builtins.getFlake (toString ../../..);
  pkgs = import flake.inputs.nixpkgs { inherit system; };
  root = /. + source;
  crane = import (root + "/infra/nixos/packages.nix") {
    inherit pkgs;
    sourceRoot = root;
    craneLib = flake.inputs.crane.mkLib pkgs;
  };
  candidate = import (root + "/Cargo.nix") { inherit pkgs; };
  members = {
    devfinity = "devfinity";
    finite-saas-core = "finite-saas-core";
    finite-saas-local = "finite-saas-local";
    finite-saas-runner = "finite-saas-runner";
    finitechat-server = "finitechat-server";
    finitechat-hosted-device = "finitechat-hosted-device";
    finite-agentd = "finite-agentd";
    finitesitesd = "finitesitesd";
    finite-brain = "finite-brain-app";
    finite-identity = "finite-identity";
    fsite = "fsite-cli";
    fbrain = "finite-brain-cli";
    finitechat = "finitechat-cli";
  };
  node = drv: { key = drv.drvPath; inherit drv; };
  graph = drv: map (n: {
    drv = n.key;
    name = n.drv.crateName;
    version = n.drv.version;
    features = n.drv.crateFeatures;
  }) (builtins.genericClosure {
    startSet = [ (node drv) ];
    operator = n: map node (n.drv.dependencies ++ n.drv.buildDependencies);
  });
in
{
  inherit system;
  crate2nixVersion = pkgs.crate2nix.version;
  packagingRustVersion = pkgs.rustc.version;
  packages = pkgs.lib.mapAttrs (name: member: {
    # Compare the Rust executable, not devfinity's service/runtime wrapper.
    crane = (crane.${name}.unwrapped or crane.${name}).drvPath;
    craneDeps = crane.${name}.cargoArtifacts.drvPath;
    candidate = candidate.workspaceMembers.${member}.build.drvPath;
    crates = graph candidate.workspaceMembers.${member}.build;
  }) members;
}

# Match the production compiler and compare two service binaries.
{ source, engine }:
let
  flake = builtins.getFlake (toString ../../..);
  pkgs = import flake.inputs.nixpkgs { system = builtins.currentSystem; };
  root = /. + source;
  crane = import (root + "/infra/nixos/packages.nix") {
    inherit pkgs;
    sourceRoot = root;
    craneLib = flake.inputs.crane.mkLib pkgs;
  };
  candidate = import (root + "/Cargo.nix") { inherit pkgs; };
  packages =
    if engine == "crane" then
      [ crane.finitesitesd crane.finite-saas-core ]
    else if engine == "crate2nix" then
      [ candidate.workspaceMembers.finitesitesd.build candidate.workspaceMembers.finite-saas-core.build ]
    else throw "Unknown benchmark engine: ${engine}";
in
packages

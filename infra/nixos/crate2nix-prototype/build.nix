# Optional compilation probe. This is not production packaging.
{ source, member ? "finitesites-engine", system ? builtins.currentSystem }:
let
  flake = builtins.getFlake (toString ../../..);
  pkgs = import flake.inputs.nixpkgs { inherit system; };
  candidate = import ((/. + source) + "/Cargo.nix") { inherit pkgs; };
in
candidate.workspaceMembers.${member}.build

# Throwaway tooling, pinned by the existing root flake.lock.
let
  root = ../../..;
  flake = builtins.getFlake (toString root);
  pkgs = import flake.inputs.nixpkgs {
    system = builtins.currentSystem;
    overlays = [ (import flake.inputs.rust-overlay) ];
  };
  rust = pkgs.rust-bin.fromRustupToolchainFile (root + "/rust-toolchain.toml");
in
[ pkgs.crate2nix pkgs.git rust ]

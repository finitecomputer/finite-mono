# Local Linux fixture tools; uses the same package inputs as the Runner hosts.
# nix build --impure --file scripts/proofs/hosted-hermes-lifecycle-tools.nix
let
  flake = builtins.getFlake (toString ../..);
  pkgs = import flake.inputs.nixpkgs { system = builtins.currentSystem; };
  hostPkgs = import flake.inputs.nixpkgs-lat3 { system = builtins.currentSystem; };
  kataPkgs = import flake.inputs.nixpkgs-kata { system = builtins.currentSystem; };
  image = pkgs.dockerTools.buildLayeredImage {
    name = "finite-hosted-lifecycle-proof";
    tag = "fixture";
    contents = [ pkgs.busybox ];
    config.Cmd = [ "${pkgs.busybox}/bin/sleep" "3600" ];
  };
in
pkgs.buildEnv {
  name = "hosted-hermes-lifecycle-proof-tools";
  paths = [
    kataPkgs.nerdctl
    hostPkgs.containerd
    hostPkgs.runc
    hostPkgs.erofs-utils
    pkgs.caddy
    pkgs.python3
    pkgs.iproute2
    pkgs.iptables
  ];
  postBuild = ''
    mkdir -p "$out/lib"
    ln -s ${kataPkgs.cni-plugins}/bin "$out/lib/cni"
    mkdir -p "$out/share"
    ln -s ${image} "$out/share/proof-image.tar.gz"
  '';
}

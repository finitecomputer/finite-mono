# FIN-162 qualification candidate: nixpkgs-kata's nerdctl 2.3.4 with the
# carried log-collector patch. Only the flake's kata-nerdctl-candidate output
# imports this file, so CI can build it and run the patch's tests. The Runner
# hosts and the proof fixture run nixpkgs-kata's nerdctl unchanged, and
# `just nixos kata-nerdctl-selection-contract` fails if either selects this
# package. It is not accepted for adoption; see infra/nixos/README.md.
{ nixpkgs, system }:
let
  pkgs = import nixpkgs { inherit system; };
in
assert pkgs.lib.assertMsg (pkgs.nerdctl.version == "2.3.4")
  "nixpkgs-kata moved nerdctl to ${pkgs.nerdctl.version}: re-qualify or drop patches/nerdctl-logger-lifetime.patch";
pkgs.nerdctl.overrideAttrs (old: {
  # A container records the nerdctl store path that created it as its log
  # collector, so key the candidate to the patch content, never to the whole
  # flake source.
  patches = (old.patches or [ ]) ++ [
    (builtins.path {
      name = "nerdctl-logger-lifetime.patch";
      path = ./patches/nerdctl-logger-lifetime.patch;
    })
  ];
  # Upstream skips tests because most need a live containerd. The logger
  # package's tests, including the patch's, need none.
  doCheck = true;
  checkFlags = [ "-v" ];
  checkPhase = ''
    runHook preCheck
    export GOFLAGS=''${GOFLAGS//-trimpath/}
    buildGoDir test ./pkg/logging
    runHook postCheck
  '';
})

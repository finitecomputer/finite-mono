# Task-local tools; no system installation. Go is newer than the root nixpkgs pin.
let
  root = builtins.getFlake (toString ../../..);
  pkgs = root.inputs.nixpkgs.legacyPackages.${builtins.currentSystem};
  go = pkgs.stdenvNoCC.mkDerivation {
    pname = "go-substrate";
    version = "1.27.1";
    src = pkgs.fetchurl {
      url = "https://go.dev/dl/go1.27.1.darwin-arm64.tar.gz";
      sha256 = "ee215d57e0ec269c60cc9ceca68e6bda321ba9ee5afe24f4b0988703c2d87d12";
    };
    dontBuild = true;
    installPhase = "mkdir -p $out; cp -R . $out/";
  };
in pkgs.mkShell {
  packages = [ go pkgs.gettext pkgs.jq (pkgs.python3.withPackages (ps: [ ps.requests ps.websockets ])) pkgs.git pkgs.curl ];
  GOTOOLCHAIN = "local";
}

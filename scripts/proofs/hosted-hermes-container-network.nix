# Local Linux qualification fixtures; not an Agent Runtime image or deployment.
let
  repo = builtins.getFlake (toString ../..);
  system = builtins.currentSystem;
  base = import repo.inputs.nixpkgs { inherit system; };
  host = import repo.inputs.nixpkgs-lat3 { inherit system; };
  kata = import repo.inputs.nixpkgs-kata { inherit system; };
in
{
  tools = base.buildEnv {
    name = "finite-hermes-container-network-proof-tools";
    paths = [
      kata.nerdctl
      kata.cni-plugins
      host.containerd
      host.runc
      host.iptables
      host.iproute2
      host.conntrack-tools
      host.util-linux
      host.erofs-utils
      base.caddy
      base.nodejs_24
      base.jq
      base.busybox
    ];
    ignoreCollisions = true;
  };
  image = base.dockerTools.buildLayeredImage {
    name = "finite-hermes-network-proof";
    tag = "local";
    contents = [ base.busybox base.nodejs_24 ];
    config = {
      Cmd = [ "${base.nodejs_24}/bin/node" "/proof/server.mjs" ];
      ExposedPorts."8080/tcp" = { };
    };
  };
}

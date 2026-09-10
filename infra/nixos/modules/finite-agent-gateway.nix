# Opt-in direct runner ingress. This module is not imported by any host.
{
  config,
  lib,
  pkgs,
  kataPackages,
  ...
}:
let
  cfg = config.finite.agentGateway;
  runner = config.finite.kataRunnerHost;
  python = pkgs.python3.withPackages (ps: [ ps.aiohttp ]);
  routes = "/run/finite-agent-gateway/routes.json";
in
{
  options.finite.agentGateway.domain = lib.mkOption {
    type = lib.types.nullOr lib.types.str;
    default = null;
    description = "Runner gateway DNS suffix; wildcard DNS must point directly to this runner.";
  };
  config = lib.mkIf (cfg.domain != null) {
    networking.firewall.allowedTCPPorts = [
      80
      443
    ];
    systemd.services.finite-agent-gateway-inventory = {
      description = "Project local Kata ownership into gateway routes";
      wantedBy = [ "multi-user.target" ];
      after = [ "containerd.service" ];
      environment = {
        NERDCTL = "${kataPackages.nerdctl}/bin/nerdctl";
        FINITE_RUNNER_WORK_ROOT = runner.workRoot;
        FINITE_SOURCE_HOST_ID = runner.sourceHostId;
        FINITE_GATEWAY_ROUTES = routes;
      };
      serviceConfig = {
        ExecStart = "${python}/bin/python ${../../../finite-agentd/gateway_inventory.py}";
        Restart = "always";
        RestartSec = 2;
        RuntimeDirectory = "finite-agent-gateway";
        RuntimeDirectoryMode = "0755";
        UMask = "0077";
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        NoNewPrivileges = true;
      };
    };
    systemd.services.finite-agent-gateway = {
      description = "Authenticated Hermes gateway ingress";
      wantedBy = [ "multi-user.target" ];
      after = [ "finite-agent-gateway-inventory.service" ];
      environment = {
        FINITE_GATEWAY_DOMAIN = cfg.domain;
        FINITE_GATEWAY_ROUTES = routes;
      };
      serviceConfig = {
        ExecStart = "${python}/bin/python ${../../../finite-agentd/gateway_proxy.py}";
        DynamicUser = true;
        Restart = "always";
        RestartSec = 2;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        NoNewPrivileges = true;
        InaccessiblePaths = [
          runner.workRoot
          "/run/containerd"
        ];
      };
    };
    services.caddy = {
      enable = true;
      email = "paul@finite.vip";
      # Disable both NixOS's per-vhost access log and the default error log:
      # either may otherwise persist token-bearing request URIs.
      logFormat = "output discard";
      virtualHosts."https://".logFormat = null;
      globalConfig = ''
        on_demand_tls {
          ask http://127.0.0.1:8793/certificate-permission
        }
      '';
      virtualHosts."https://".extraConfig = ''
        tls {
          on_demand
        }
        reverse_proxy 127.0.0.1:8792
      '';
    };
  };
}

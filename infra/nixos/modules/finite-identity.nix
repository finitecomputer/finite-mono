# Finite Identity Authority — the shared source of truth for public Finite VIP
# Email/NIP-05 bindings. It owns public identity state only; Local Identity Key
# secret material remains inside each user or Agent Runtime.
#
# The public signing origin is identity.finite.vip. Trusted services on this
# host use loopback so managed-agent creation does not depend on public DNS.
{
  config,
  finitePackages,
  lib,
  pkgs,
  ...
}:
let
  serviceName = "finite-identity";
  loopbackAuthority = "http://127.0.0.1:8790";
  operatorEnvironmentFile = "/etc/finite/identity-operator.env";
in
{
  systemd.services.${serviceName} = {
    description = "Finite Identity Authority";
    wants = [ "network-online.target" ];
    after = [ "network-online.target" ];
    wantedBy = [ "multi-user.target" ];

    serviceConfig = {
      Type = "simple";
      ExecStart = ''
        ${finitePackages.finite-identity}/bin/finite-identityd serve \
          --data /var/lib/finite-identity \
          --listen 127.0.0.1:8790 \
          --external-base-url https://identity.finite.vip \
          --finite-vip-domain finite.vip \
          --mailer resend \
          --mail-from "Finite Identity <identity@finite.chat>"
      '';
      ExecStartPost = ''
        ${pkgs.curl}/bin/curl \
          --fail --silent --show-error \
          --retry 10 --retry-connrefused --retry-delay 1 \
          ${loopbackAuthority}/health
      '';

      # The operator token is shared only with trusted provisioning services.
      # The existing Resend send-only credential remains owned by Sites and is
      # read here by systemd without copying its value into the Nix store.
      EnvironmentFile = [
        operatorEnvironmentFile
        "/etc/finite-saas/sites.env"
      ];

      DynamicUser = true;
      User = serviceName;
      Group = serviceName;
      UMask = "0077";
      StateDirectory = serviceName;
      StateDirectoryMode = "0700";
      WorkingDirectory = "/var/lib/${serviceName}";

      CapabilityBoundingSet = "";
      AmbientCapabilities = "";
      DevicePolicy = "closed";
      LockPersonality = true;
      MemoryDenyWriteExecute = true;
      NoNewPrivileges = true;
      PrivateDevices = true;
      PrivateMounts = true;
      PrivateTmp = true;
      ProtectClock = true;
      ProtectControlGroups = true;
      ProtectHome = true;
      ProtectHostname = true;
      ProtectKernelLogs = true;
      ProtectKernelModules = true;
      ProtectKernelTunables = true;
      ProtectProc = "invisible";
      ProtectSystem = "strict";
      ProcSubset = "pid";
      RemoveIPC = true;
      RestrictAddressFamilies = [
        "AF_UNIX"
        "AF_INET"
        "AF_INET6"
      ];
      RestrictNamespaces = true;
      RestrictRealtime = true;
      RestrictSUIDSGID = true;
      SystemCallArchitectures = "native";
      SystemCallFilter = [
        "@system-service"
        "~@privileged"
        "~@resources"
      ];

      Restart = "on-failure";
      RestartSec = "3s";
      TimeoutStartSec = "30s";
      TimeoutStopSec = "30s";
    };
  };

  # Managed Agent Email registration is part of creation completion for both
  # Standard (Kata) and Confidential (Phala) runtimes. Make the shared
  # authority and Core hard startup dependencies, and inject only the
  # loopback URL plus the root-owned operator environment.
  systemd.services.finite-saas-runner = {
    requires = [
      "${serviceName}.service"
      "finite-saas-core.service"
    ];
    after = [
      "${serviceName}.service"
      "finite-saas-core.service"
    ];
    environment.FINITE_IDENTITY_AUTHORITY = loopbackAuthority;
    serviceConfig.EnvironmentFile = lib.mkAfter [ operatorEnvironmentFile ];
  };

  systemd.services.finite-saas-runner-phala = {
    requires = [
      "${serviceName}.service"
      "finite-saas-core.service"
    ];
    after = [
      "${serviceName}.service"
      "finite-saas-core.service"
    ];
    environment.FINITE_IDENTITY_AUTHORITY = loopbackAuthority;
    serviceConfig.EnvironmentFile = lib.mkAfter [ operatorEnvironmentFile ];
  };

  assertions = [
    {
      assertion =
        config.systemd.services.finite-saas-runner.environment.FINITE_IDENTITY_AUTHORITY
        == loopbackAuthority;
      message = "the Kata worker must use the loopback Identity Authority";
    }
    {
      assertion =
        config.systemd.services.finite-saas-runner-phala.environment.FINITE_IDENTITY_AUTHORITY
        == loopbackAuthority;
      message = "the Phala worker must use the loopback Identity Authority";
    }
    {
      assertion =
        builtins.elem operatorEnvironmentFile config.systemd.services.finite-saas-runner.serviceConfig.EnvironmentFile
        && builtins.elem operatorEnvironmentFile config.systemd.services.finite-saas-runner-phala.serviceConfig.EnvironmentFile;
      message = "both managed-agent workers must load the shared Identity Authority operator credential";
    }
  ];
}

# Finite Identity Authority — the shared source of truth for public Finite VIP
# Email/NIP-05 bindings. It owns public identity state only; Local Identity Key
# secret material remains inside each user or Agent Runtime.
#
# The public signing origin is identity.finite.vip. Trusted services on this
# host use loopback so managed-agent creation does not depend on public DNS.
#
# The daemon binds two loopback listeners. 8790 serves the full router
# (the public surface plus the loopback-only operator routes such as
# operator/agent-email-bindings, which the managed-agent Runner still calls)
# and stays reachable only from this host. 8791 serves
# only the service-owned public surface (`public_router` in
# finite-identity/src/authority.rs); the Caddy edge proxies 8791 verbatim and
# keeps no route list of its own.
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
  loopbackPublic = "http://127.0.0.1:8791";
  operatorEnvironmentFile = "/etc/finite/identity-operator.env";
in
{
  systemd.services.${serviceName} = {
    description = "Finite Identity Authority";
    wants = [ "network-online.target" ];
    after = [ "network-online.target" ];
    wantedBy = [ "multi-user.target" ];

    serviceConfig = {
      # systemd's default soft fd limit (1024) starved the hosted-device daemon
      # of sockets during the 2026-08-12 sync burst (reqwest Client::new EMFILE
      # -> "Chat is unavailable"). Raise it for every long-running platform
      # service; the hard limit already allows it.
      LimitNOFILE = 65536;
      Type = "simple";
      ExecStart = ''
        ${finitePackages.finite-identity}/bin/finite-identityd serve \
          --data /var/lib/finite-identity \
          --listen 127.0.0.1:8790 \
          --public-listen 127.0.0.1:8791 \
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
        ${pkgs.curl}/bin/curl \
          --fail --silent --show-error \
          --retry 10 --retry-connrefused --retry-delay 1 \
          ${loopbackPublic}/health
      '';

      # The operator token is shared only with trusted provisioning services.
      # The existing Resend send-only credential remains owned by Sites and is
      # read here by systemd without copying its value into the Nix store.
      # The retired identity-sites-notification.env load is gone: the
      # directory shrink removed the Sites notification relay, and Sites now
      # sends its own mail.
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
  #
  # Runner hosts only: these stanzas must not exist where the runner module
  # isn't imported — defining `systemd.services.finite-saas-runner`
  # unconditionally creates ExecStart-less husk units on runnerless hosts
  # (the lat2 app plane), tripping the no-runner fence in
  # deploy-lat2-closure-cache by unit-file existence. `or false` keeps
  # hosts that don't define the option evaluating.
  systemd.services.finite-saas-runner = lib.mkIf (config.finite.saasRunner or false) {
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

  systemd.services.finite-saas-runner-phala = lib.mkIf (config.finite.saasRunner or false) {
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

  # Sites no longer resolves anything through the Directory at request time
  # (daemon-local email proofs, ADR 0027): no FINITE_IDENTITY_AUTHORITY, no
  # shared notification credential, no boot ordering against this service.

  assertions = [
    {
      # Runner hosts only (see the mkIf gates above); the short-circuit keeps
      # runnerless hosts from forcing the nonexistent unit attributes.
      assertion =
        !(config.finite.saasRunner or false)
        ||
          config.systemd.services.finite-saas-runner.environment.FINITE_IDENTITY_AUTHORITY
          == loopbackAuthority;
      message = "the Kata worker must use the loopback Identity Authority";
    }
    {
      assertion =
        !(config.finite.saasRunner or false)
        ||
          config.systemd.services.finite-saas-runner-phala.environment.FINITE_IDENTITY_AUTHORITY
          == loopbackAuthority;
      message = "the Phala worker must use the loopback Identity Authority";
    }
    {
      assertion =
        !(config.finite.saasRunner or false)
        || (
          builtins.elem operatorEnvironmentFile config.systemd.services.finite-saas-runner.serviceConfig.EnvironmentFile
          && builtins.elem operatorEnvironmentFile config.systemd.services.finite-saas-runner-phala.serviceConfig.EnvironmentFile
        );
      message = "both managed-agent workers must load the shared Identity Authority operator credential";
    }
    {
      assertion =
    {
      assertion =
        !(builtins.elem operatorEnvironmentFile config.systemd.services.finite-saas-sites.serviceConfig.EnvironmentFile);
      message = "Sites must not receive the Identity Authority operator credential";
    }
  ];
}

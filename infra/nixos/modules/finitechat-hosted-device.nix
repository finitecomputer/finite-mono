# finitechat-hosted-device — one durable, isolated Finite Chat Device per
# verified SaaS account. This service owns chat client state only; it does not
# provision, restart, inspect, or otherwise control Agent Runtimes.
{ finitePackages, ... }:
{
  systemd.services.finitechat-hosted-device = {
    description = "Finite Chat Hosted Web Devices";
    wants = [ "network-online.target" ];
    after = [
      "network-online.target"
      "finitechat-server.service"
      "finite-identity.service"
    ];
    requires = [
      "finitechat-server.service"
      "finite-identity.service"
    ];
    wantedBy = [ "multi-user.target" ];

    environment = {
      FINITECHAT_HOSTED_BIND = "127.0.0.1:38918";
      FINITECHAT_HOSTED_DATA_ROOT = "/var/lib/finitechat-hosted-device";
      # Keep HTTP transport on loopback while encrypted Device Link payloads
      # bind the canonical URL that the joining Device is configured to trust.
      FINITECHAT_SERVER_URL = "http://127.0.0.1:8788";
      FINITECHAT_PUBLIC_URL = "https://chat.finite.computer";
      FINITE_IDENTITY_AUTHORITY = "http://127.0.0.1:8790";
    };

    serviceConfig = {
      ExecStart = "${finitePackages.finitechat-hosted-device}/bin/finitechat-hosted-device";
      DynamicUser = true;
      StateDirectory = "finitechat-hosted-device";
      WorkingDirectory = "/var/lib/finitechat-hosted-device";
      # Operator-created, root:root 0600. It is shared with the dashboard
      # container and contains the same random value under both names:
      #   FINITECHAT_HOSTED_API_TOKEN
      EnvironmentFile = [
        "/etc/finite/hosted-web-device.env"
        "/etc/finite/identity-operator.env"
      ];
      Restart = "always";
      RestartSec = 2;
      NoNewPrivileges = true;
      PrivateTmp = true;
      ProtectSystem = "strict";
      ProtectHome = true;
      ProtectKernelTunables = true;
      ProtectControlGroups = true;
      RestrictSUIDSGID = true;
    };
  };
}

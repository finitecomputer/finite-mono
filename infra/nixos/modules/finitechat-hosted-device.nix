# finitechat-hosted-device — one durable, isolated Finite Chat Device per
# verified SaaS account. This service owns chat client state only; it does not
# provision, restart, inspect, or otherwise control Agent Runtimes.
{ config, finitePackages, ... }:
{
  systemd.services.finitechat-hosted-device = {
    description = "Finite Chat Hosted Web Devices";
    wants = [ "network-online.target" ];
    after = [
      "network-online.target"
      "finitechat-server.service"
    ];
    requires = [ "finitechat-server.service" ];
    # Aug 11 fence left hosted-device down: Requires= already stopped it with
    # finitechat-server, but starting the server did not pull it back. Mirror
    # finite-litestream.nix: PartOf= the owner so stop/restart of chat-server
    # takes this unit with it.
    partOf = [ "finitechat-server.service" ];
    wantedBy = [ "multi-user.target" ];

    environment = {
      FINITECHAT_HOSTED_BIND = "127.0.0.1:38918";
      FINITECHAT_HOSTED_DATA_ROOT = "/var/lib/finitechat-hosted-device";
      # Keep HTTP transport on loopback while encrypted Device Link payloads
      # bind the canonical URL that the joining Device is configured to trust.
      FINITECHAT_SERVER_URL = "http://127.0.0.1:8788";
      FINITECHAT_PUBLIC_URL = "https://chat.finite.computer";
    };

    serviceConfig = {
      # systemd's default soft fd limit (1024) starved the hosted-device daemon
      # of sockets during the 2026-08-12 sync burst (reqwest Client::new EMFILE
      # -> "Chat is unavailable"). Raise it for every long-running platform
      # service; the hard limit already allows it.
      LimitNOFILE = 65536;
      ExecStart = "${finitePackages.finitechat-hosted-device}/bin/finitechat-hosted-device";
      DynamicUser = true;
      StateDirectory = "finitechat-hosted-device";
      WorkingDirectory = "/var/lib/finitechat-hosted-device";
      # Operator-created, root:root 0600. It is shared with the dashboard
      # container and contains the same random value under both names:
      #   FINITECHAT_HOSTED_API_TOKEN
      # The retired identity-operator.env load is gone: the daemon no longer
      # reads FINITE_IDENTITY_OPERATOR_TOKEN.
      EnvironmentFile = [ "/etc/finite/hosted-web-device.env" ];
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

  # Loading this module creates the reverse want so finitechat-server can
  # exist without hosted-device. Start/restart of the owner then pulls
  # hosted-device back — same pairing as finite-litestream.nix.
  systemd.services.finitechat-server.wants = [ "finitechat-hosted-device.service" ];

  assertions = [
    {
      assertion = builtins.elem "finitechat-server.service" config.systemd.services.finitechat-hosted-device.partOf;
      message = "hosted-device must be PartOf finitechat-server so a chat-server stop takes it down";
    }
    {
      assertion = builtins.elem "finitechat-hosted-device.service" config.systemd.services.finitechat-server.wants;
      message = "finitechat-server must Wants hosted-device so a chat-server start pulls it back";
    }
  ];
}

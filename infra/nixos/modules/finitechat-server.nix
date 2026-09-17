# Chat server: the app-plane listener for chat.finite.computer.
# Deployment and single-writer rules: infra/runbooks/deploy-finitechat-server.md.
{ finitePackages, ... }:
{
  systemd.services.finitechat-server = {
    description = "Finitechat server (chat.finite.computer)";
    wants = [ "network-online.target" ];
    after = [ "network-online.target" ];
    wantedBy = [ "multi-user.target" ];

    environment.FINITECHAT_PUBLIC_URL = "https://chat.finite.computer";

    serviceConfig = {
      # systemd's default soft fd limit (1024) starved the hosted-device daemon
      # of sockets during the 2026-08-12 sync burst (reqwest Client::new EMFILE
      # -> "Chat is unavailable"). Raise it for every long-running platform
      # service; the hard limit already allows it.
      LimitNOFILE = 65536;
      ExecStart = "${finitePackages.finitechat-server}/bin/finitechat-server serve 127.0.0.1:8788 --sqlite /var/lib/finite-chat/data/server.sqlite3";
      DynamicUser = true;
      # DynamicUser stores this under /var/lib/private/finite-chat/data/.
      StateDirectory = "finite-chat/data";
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

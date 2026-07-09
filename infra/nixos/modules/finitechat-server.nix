# finitechat-server — chat.finite.computer (moving here from clawland,
# infra/hosts/clawland/finitechat-server.md).
#
# ## PORT REASSIGNMENT: 8787 -> 8788 ##
# The chat server historically listened on 8787 (clawland bound
# 10.42.0.1:8787). On this consolidated box finitesitesd owns its own
# historical 8787, so the chat server moves to 127.0.0.1:8788. The PUBLIC URL
# is unchanged: Caddy routes chat.finite.computer -> 127.0.0.1:8788
# (modules/caddy.nix).
{ finitePackages, ... }:
{
  systemd.services.finitechat-server = {
    description = "Finitechat server (chat.finite.computer)";
    wants = [ "network-online.target" ];
    after = [ "network-online.target" ];
    wantedBy = [ "multi-user.target" ];

    serviceConfig = {
      ExecStart = "${finitePackages.finitechat-server}/bin/finitechat-server serve 127.0.0.1:8788 --sqlite /var/lib/finite-chat/data/server.sqlite3";
      DynamicUser = true;
      # Nested StateDirectory creates finite-chat/ and finite-chat/data/;
      # the clawland SQLite is restored into data/ at cutover (real path under
      # DynamicUser: /var/lib/private/finite-chat/data/server.sqlite3).
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

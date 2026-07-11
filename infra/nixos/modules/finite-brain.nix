# finite-brain — first-party server behind the finite.computer dashboard.
# It binds loopback only. Next.js proxies /health, /client, and /_admin. WorkOS
# protects the browser Product Client at /client; Brain owns route-level auth
# for /_admin, normally through signed Nostr request proofs.
{ finitePackages, ... }:
{
  systemd.services.finite-brain-app = {
    description = "FiniteBrain Rust application server";
    wants = [ "network-online.target" ];
    after = [ "network-online.target" ];
    wantedBy = [ "multi-user.target" ];

    environment = {
      FBRAIN_CONFIG_DIR = "/var/lib/finitebrain/fbrain";
      FINITE_BRAIN_ADDR = "127.0.0.1:3015";
      FINITE_BRAIN_DB = "/var/lib/finitebrain/finite-brain.sqlite3";
      FINITE_BRAIN_PUBLIC_BASE_URL = "https://finite.computer";
      FINITE_BRAIN_SERVER_URL = "https://finite.computer";
    };

    serviceConfig = {
      ExecStart = "${finitePackages.finite-brain}/bin/finite-brain";
      DynamicUser = true;
      # SQLite restored from smoke at cutover; real path under DynamicUser:
      # /var/lib/private/finitebrain/finite-brain.sqlite3.
      StateDirectory = "finitebrain";
      WorkingDirectory = "/var/lib/finitebrain";
      Restart = "always";
      RestartSec = 3;
      NoNewPrivileges = true;
      PrivateTmp = true;
      ProtectSystem = "full";
      ReadWritePaths = [ "/var/lib/finitebrain" ];
    };
  };
}

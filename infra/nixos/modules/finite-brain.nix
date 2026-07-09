# finite-brain — brain.smoke.finite.computer (moving here from ovh-vps-smoke).
# Mirrors the captured finite-brain-app.service (infra/hosts/smoke/) except:
# FINITE_BRAIN_ADDR binds 127.0.0.1 instead of 0.0.0.0 — smoke exposed :3015
# on its public IP (flagged risk in the smoke README); here only Caddy +
# oauth2-proxy front it.
# Config is Environment= only — no EnvironmentFile, no secrets (per capture).
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
      FINITE_BRAIN_PUBLIC_BASE_URL = "https://brain.smoke.finite.computer";
      FINITE_BRAIN_SERVER_URL = "https://brain.smoke.finite.computer";
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

# Postgres 16 for finite-saas-core (was k3s StatefulSet finite-core-postgres,
# postgres:16-alpine, db finite_core / user finite). Loopback TCP only — core
# connects via FC_CORE_DATABASE_URL at 127.0.0.1:5432.
#
# The db keeps its captured name `finite_core` (NOT renamed): the cutover
# restores old lat1's pg_dump into it and FC_CORE_DATABASE_URL must match.
#
# Bootstrap note: ensureUsers cannot set passwords, and ensureDBOwnership
# only works when db name == role name (ours differ). At cutover, before
# restoring the dump (infra/runbooks/postgres-backup-restore.md):
#   sudo -u postgres psql -c "ALTER ROLE finite WITH PASSWORD '<POSTGRES_PASSWORD>';"
#   sudo -u postgres psql -c "ALTER DATABASE finite_core OWNER TO finite;"
# using the POSTGRES_PASSWORD from old lat1's k8s Secret finite-computer-secrets.
{
  config,
  lib,
  pkgs,
  ...
}:
{
  services.postgresql = {
    enable = true;
    package = pkgs.postgresql_16;
    settings.listen_addresses = lib.mkForce "127.0.0.1";
    ensureDatabases = [ "finite_core" ];
    ensureUsers = [
      { name = "finite"; }
    ];
    authentication = ''
      host finite_core finite 127.0.0.1/32 scram-sha-256
    '';
  };

  # Timestamped 6-hourly dumps + retention to /data/backups/postgres,
  # replacing the old CronJob that overwrote finite_core_latest.dump in place
  # (the single-snapshot flaw called out in the runbook). Deliberately NOT
  # services.postgresqlBackup: that module also overwrites one file per db.
  # Off-box copies: modules/backups.nix borgs this directory.
  systemd.services.finite-postgres-backup = {
    description = "Timestamped pg_dump of finite_core to /data/backups/postgres";
    after = [ "postgresql.service" ];
    requires = [ "postgresql.service" ];
    path = [
      config.services.postgresql.package
      pkgs.coreutils
      pkgs.findutils
    ];
    serviceConfig = {
      Type = "oneshot";
      User = "postgres";
      Group = "postgres";
    };
    script = ''
      set -euo pipefail
      dir=/data/backups/postgres
      ts=$(date -u +%Y%m%dT%H%M%SZ)
      pg_dump --format=custom --file="$dir/finite_core-$ts.dump" finite_core
      # Retention: 28 days of 6-hourly dumps (~1MB each at capture).
      find "$dir" -name 'finite_core-*.dump' -type f -mtime +28 -delete
    '';
  };
  systemd.timers.finite-postgres-backup = {
    wantedBy = [ "timers.target" ];
    timerConfig = {
      # Mirrors the old CronJob cadence (17 */6 * * *).
      OnCalendar = "*-*-* 00/6:17:00";
      Persistent = true;
    };
  };

  systemd.tmpfiles.rules = [
    "d /data/backups 0755 root root -"
    "d /data/backups/postgres 0750 postgres postgres -"
  ];
}

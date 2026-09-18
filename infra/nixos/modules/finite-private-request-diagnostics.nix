# Local, bounded reporting observer. Never starts/restarts Core or inference.
{ pkgs, ... }:
let
  psql = "${pkgs.postgresql}/bin/psql --no-psqlrc --quiet --set=ON_ERROR_STOP=1 --dbname=finite_core";

in
{
  services.postgresql.ensureUsers = [ { name = "finite_private_diagnostics"; } ];
  services.postgresql.authentication = lib.mkAfter ''
    local finite_core finite_private_diagnostics peer
  '';
  users.groups.finite-private-diagnostics = { };
  users.users.finite_private_diagnostics = {
    isSystemUser = true;
    group = "finite-private-diagnostics";
  };
  # May fail closed until Core's additive migration exists. There is deliberately
  # no Wants/Requires relationship that starts Core to satisfy monitoring.
  systemd.services.finite-private-request-diagnostics-grants = {
    description = "Grant narrow local request reporting access";
    after = [ "postgresql.service" "finite-saas-core.service" ];
    serviceConfig = {
      Type = "oneshot";
      User = "postgres";
      ExecStart = pkgs.writeShellScript "grant-finite-private-diagnostics" ''
        set -euo pipefail
        ${psql} <<'SQL'
        SET statement_timeout = '5s';
        SET lock_timeout = '1s';
        GRANT USAGE ON SCHEMA public TO finite_private_diagnostics;
        GRANT DELETE ON finite_private_request_diagnostics TO finite_private_diagnostics;
        SQL
      '';
      TimeoutStartSec = "15s";
    };
  };

  systemd.services.finite-private-request-diagnostics-prune = {
    description = "Expire request diagnostics without changing accounting";
    after = [ "finite-private-request-diagnostics-grants.service" ];
    requires = [ "finite-private-request-diagnostics-grants.service" ];
    serviceConfig = {
      Type = "oneshot";
      User = "finite_private_diagnostics";
      ExecStart = pkgs.writeShellScript "prune-finite-private-diagnostics" ''
        set -euo pipefail
        ${psql} <<'SQL'
        SET statement_timeout = '10s';
        SET lock_timeout = '1s';
        DELETE FROM finite_private_request_diagnostics
          WHERE observed_at < CURRENT_TIMESTAMP - INTERVAL '7 days';
        SQL
      '';
      NoNewPrivileges = true;
      PrivateTmp = true;
      ProtectHome = true;
      ProtectSystem = "strict";
      TimeoutStartSec = "15s";
    };
  };
  systemd.timers.finite-private-request-diagnostics-prune = {
    wantedBy = [ "timers.target" ];
    timerConfig = {
      OnBootSec = "1min";
      OnUnitActiveSec = "10min";
      AccuracySec = "5s";
      Persistent = true;
    };
  };

}

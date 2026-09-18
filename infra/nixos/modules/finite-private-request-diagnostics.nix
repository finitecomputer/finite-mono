# Local, bounded reporting observer. Never starts/restarts Core or inference.
{ config, lib, pkgs, ... }:
let
  stateDirectory = "/var/lib/finite-private-request-diagnostics";
  metricsPath = "/run/finite-monitoring/finite-private-request-diagnostics.prom";
  source = ../../monitoring/private_requests;
  psql = "${pkgs.postgresql}/bin/psql --no-psqlrc --quiet --set=ON_ERROR_STOP=1 --dbname=finite_core";
  exporter = pkgs.writeShellScript "finite-private-request-diagnostics-exporter" ''
    set -euo pipefail
    export PATH=${lib.makeBinPath [ pkgs.postgresql pkgs.python3 ]}
    exec ${pkgs.util-linux}/bin/flock -n ${stateDirectory}/exporter.lock \
      ${pkgs.python3}/bin/python3 ${source}/export.py \
      ${stateDirectory}/counters.json ${metricsPath}
  '';
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
  systemd.tmpfiles.rules = [
    "d ${stateDirectory} 0750 finite_private_diagnostics finite-private-diagnostics - -"
  ];

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
        GRANT SELECT (reservation_id, request_id, api_key_id, grant_id, project_id, agent_runtime_id, endpoint, model, prompt_tokens, completion_tokens, first_output_ms, first_answer_ms, duration_ms, termination_reason, measurement_quality, accounting_status, settlement_kind, settled_usage_units, upstream_status, upstream_error_class, observed_at, exported_at) ON finite_private_request_diagnostics TO finite_private_diagnostics;
        GRANT UPDATE (exported_at), DELETE ON finite_private_request_diagnostics TO finite_private_diagnostics;
        SQL
      '';
      TimeoutStartSec = "15s";
    };
  };

  systemd.services.finite-private-request-diagnostics-exporter = {
    description = "Export redacted Finite Private diagnostics to Loki";
    after = [ "finite-private-request-diagnostics-grants.service" ];
    requires = [ "finite-private-request-diagnostics-grants.service" ];
    serviceConfig = {
      Type = "oneshot";
      User = "finite_private_diagnostics";
      Group = "finite-monitoring";
      EnvironmentFile = config.finite.metrics.logsWriteEnvironmentFile;
      ExecStart = exporter;
      NoNewPrivileges = true;
      PrivateTmp = true;
      ProtectHome = true;
      ProtectSystem = "strict";
      ReadWritePaths = [ stateDirectory "/run/finite-monitoring" ];
      UMask = "0027";
      TimeoutStartSec = "55s";
    };
  };
  systemd.timers.finite-private-request-diagnostics-exporter = {
    wantedBy = [ "timers.target" ];
    timerConfig = {
      OnBootSec = "30s";
      OnUnitActiveSec = "15s";
      AccuracySec = "1s";
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

  finite.metrics.allowedMetricNames = map (suffix: "finite_private_request_diagnostics_${suffix}") [
    "collection_started_timestamp_seconds"
    "last_export_timestamp_seconds"
    "exported_total"
    "export_failures_total"
    "exporter_query_success"
    "pending_requests"
    "oldest_pending_age_seconds"
  ];
}

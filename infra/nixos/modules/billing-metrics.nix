# Aggregate-only, read-only Core collection on the authoritative app host.
{ config, pkgs, ... }:
let
  directory = "/run/finite-monitoring";
  collector =
    pkgs.runCommand "finite-billing-metrics" { nativeBuildInputs = [ pkgs.makeWrapper ]; }
      ''
        mkdir -p "$out/bin" "$out/lib/scripts"
        cp ${../scripts/finite_billing_metrics.py} "$out/lib/finite_billing_metrics.py"
        cp ${../scripts/finite_billing_metrics.sql} "$out/lib/finite_billing_metrics.sql"
        cp ${../scripts/finite_runtime_metrics.py} "$out/lib/finite_runtime_metrics.py"
        cp ${../../../scripts/finite_status.py} "$out/lib/scripts/finite_status.py"
        touch "$out/lib/scripts/__init__.py"
        makeWrapper ${pkgs.python3}/bin/python "$out/bin/finite-billing-metrics" \
          --add-flags "$out/lib/finite_billing_metrics.py" --set PYTHONPATH "$out/lib"
      '';
in
{
  finite.metrics.allowedMetricNames = [
    "finite_billing_accounts"
    "finite_billing_collected_at_seconds"
    "finite_billing_collection_success"
  ];
  systemd.services.finite-billing-metrics = {
    description = "Publish aggregate Core billing account metrics";
    after = [ "postgresql.service" ];
    wants = [ "postgresql.service" ];
    path = [ config.services.postgresql.package ];
    serviceConfig = {
      Type = "oneshot";
      User = "root";
      Group = "finite-monitoring";
      UMask = "0027";
      ExecStart = "${collector}/bin/finite-billing-metrics ${directory}/finite-billing.prom";
      TimeoutStartSec = "20s";
      NoNewPrivileges = true;
      PrivateTmp = true;
      ProtectHome = true;
      ProtectSystem = "strict";
      ReadWritePaths = [ directory ];
    };
  };
  systemd.timers.finite-billing-metrics = {
    wantedBy = [ "timers.target" ];
    timerConfig = {
      OnBootSec = "3min";
      OnUnitActiveSec = "5min";
      AccuracySec = "15s";
    };
  };
}

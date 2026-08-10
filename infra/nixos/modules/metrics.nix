# Shared, loopback-only Prometheus transport for the monitoring MVP.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.finite.metrics;
  metricsDirectory = "/run/finite-monitoring";
  staticMetrics = pkgs.writeText "finite-version-static.prom" cfg.staticVersionMetrics;
  runtimeMetrics =
    pkgs.runCommand "finite-runtime-metrics" { nativeBuildInputs = [ pkgs.makeWrapper ]; }
      ''
        mkdir -p "$out/bin" "$out/lib/finite-runtime-metrics/scripts"
        cp ${../../../scripts/finite_runtime_metrics.py} "$out/lib/finite-runtime-metrics/finite_runtime_metrics.py"
        cp ${../../../scripts/finite_status.py} "$out/lib/finite-runtime-metrics/scripts/finite_status.py"
        touch "$out/lib/finite-runtime-metrics/scripts/__init__.py"
        makeWrapper ${pkgs.python3}/bin/python "$out/bin/finite-runtime-metrics" \
          --add-flags "$out/lib/finite-runtime-metrics/finite_runtime_metrics.py" \
          --set PYTHONPATH "$out/lib/finite-runtime-metrics"
      '';
in
{
  options.finite.metrics = {
    enable = lib.mkEnableOption "Finite's narrow Prometheus metrics transport";
    staticVersionMetrics = lib.mkOption {
      type = lib.types.lines;
      description = "Closure-derived Prometheus version metrics for this host.";
    };
    collectRuntimeArtifacts = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Collect Core-recorded Runtime artifact metrics on this host.";
    };
  };

  config = lib.mkIf cfg.enable (
    lib.mkMerge [
      {
        users.groups.finite-monitoring = { };
        systemd.tmpfiles.rules = [
          "d ${metricsDirectory} 2770 root finite-monitoring - -"
          "L+ ${metricsDirectory}/finite-version-static.prom - - - - ${staticMetrics}"
        ];

        services.prometheus.exporters.node = {
          enable = true;
          listenAddress = "127.0.0.1";
          port = 9100;
          enabledCollectors = [ "textfile" ];
          extraFlags = [ "--collector.textfile.directory=${metricsDirectory}" ];
        };
        systemd.services.prometheus-node-exporter.serviceConfig.SupplementaryGroups = [
          "finite-monitoring"
        ];

        services.alloy = {
          enable = true;
          environmentFile = "/etc/finite/grafana-cloud-metrics.env";
          extraFlags = [ "--disable-reporting" ];
        };
        environment.etc."alloy/config.alloy".text = ''
          prometheus.scrape "finite_internal_health" {
            targets = [{
              "__address__" = "127.0.0.1:9100",
              "instance"    = "${config.networking.hostName}",
              "job"         = "finite-internal-health",
            }]

            scrape_interval = "60s"
            scrape_timeout  = "10s"
            forward_to      = [prometheus.relabel.finite_mvp.receiver]
          }

          prometheus.relabel "finite_mvp" {
            forward_to = [prometheus.remote_write.grafana_cloud.receiver]

            rule {
              action        = "keep"
              source_labels = ["__name__"]
              regex         = "finite_component_build_info|finite_component_version_mismatch|finite_healthcheck_success|finite_runtime_artifact_info|finite_service_health_status|node_textfile_mtime_seconds|node_textfile_scrape_error|up"
            }
          }

          prometheus.remote_write "grafana_cloud" {
            endpoint {
              url = sys.env("GRAFANA_CLOUD_PROMETHEUS_URL")

              basic_auth {
                username = sys.env("GRAFANA_CLOUD_PROMETHEUS_USERNAME")
                password = sys.env("GRAFANA_CLOUD_PROMETHEUS_PASSWORD")
              }
            }
          }
        '';
        systemd.services.alloy = {
          after = [ "prometheus-node-exporter.service" ];
          wants = [ "prometheus-node-exporter.service" ];
        };
      }
      (lib.mkIf cfg.collectRuntimeArtifacts {
        systemd.services.finite-runtime-metrics = {
          description = "Publish Core Runtime artifact metrics";
          after = [ "postgresql.service" ];
          wants = [ "postgresql.service" ];
          path = [ config.services.postgresql.package ];
          serviceConfig = {
            Type = "oneshot";
            User = "root";
            Group = "finite-monitoring";
            UMask = "0027";
            ExecStart = "${runtimeMetrics}/bin/finite-runtime-metrics ${metricsDirectory}/finite-runtime.prom";
            NoNewPrivileges = true;
            PrivateTmp = true;
            ProtectHome = true;
            ProtectSystem = "strict";
            ReadWritePaths = [ metricsDirectory ];
          };
        };
        systemd.timers.finite-runtime-metrics = {
          wantedBy = [ "timers.target" ];
          timerConfig = {
            OnBootSec = "3min";
            OnUnitActiveSec = "5min";
            Persistent = true;
          };
        };
      })
    ]
  );
}

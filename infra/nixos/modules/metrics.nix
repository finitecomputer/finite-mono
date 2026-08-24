# Shared, loopback-only Prometheus/Loki transport for the monitoring MVP.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.finite.metrics;
  metricsDirectory = "/run/finite-monitoring";
  metricsRemoteWriteEnvironmentFile =
    if builtins.hasAttr "metrics-remote-write" config.finite.secrets.files then
      config.finite.secrets.files."metrics-remote-write".path
    else
      "/etc/finite/metrics-remote-write.env";
  logsWriteEnvironmentFile =
    if builtins.hasAttr "logs-write" config.finite.secrets.files then
      config.finite.secrets.files."logs-write".path
    else
      "/etc/finite/logs-write.env";
  allowedMetricNamesRegex = lib.concatStringsSep "|" [
    "finite_component_build_info"
    "finite_component_version_mismatch"
    "finite_component_version_mismatched_active_agents"
    "finite_healthcheck_success"
    "finite_runtime_artifact_active_agents"
    "finite_runtime_artifact_info"
    "finite_service_health_status"
    "node_cpu_seconds_total"
    "node_disk_io_time_seconds_total"
    "node_disk_read_bytes_total"
    "node_disk_written_bytes_total"
    "node_filesystem_avail_bytes"
    "node_filesystem_readonly"
    "node_filesystem_size_bytes"
    "node_load1"
    "node_load5"
    "node_load15"
    "node_memory_MemAvailable_bytes"
    "node_memory_MemTotal_bytes"
    "node_memory_SwapFree_bytes"
    "node_memory_SwapTotal_bytes"
    "node_network_receive_bytes_total"
    "node_network_receive_errs_total"
    "node_network_transmit_bytes_total"
    "node_network_transmit_errs_total"
    "node_textfile_mtime_seconds"
    "node_textfile_scrape_error"
    "up"
  ];
  journalSourceFor = index: unit: ''
    loki.source.journal "finite_unit_${toString index}" {
      forward_to    = [loki.write.finite_monitoring_logs.receiver]
      matches       = ${builtins.toJSON "_SYSTEMD_UNIT=${unit}"}
      max_age       = "10m"
      relabel_rules = loki.relabel.finite_journal.rules
      labels        = {
        host = ${builtins.toJSON config.networking.hostName},
        role = ${builtins.toJSON cfg.logRole},
      }
    }
  '';
  journalSources = lib.concatStringsSep "\n" (
    builtins.genList (index: journalSourceFor index (builtins.elemAt cfg.journalLogUnits index)) (
      builtins.length cfg.journalLogUnits
    )
  );
  logPipeline = lib.optionalString (cfg.journalLogUnits != [ ]) ''
    loki.relabel "finite_journal" {
      forward_to = []

      rule {
        source_labels = ["__journal__systemd_unit"]
        target_label  = "unit"
      }

      rule {
        source_labels = ["__journal_priority_keyword"]
        target_label  = "priority"
      }
    }

    ${journalSources}

    loki.write "finite_monitoring_logs" {
      endpoint {
        url = "https://metrics-ingest.finite.computer/loki/api/v1/push"

        basic_auth {
          username = sys.env("FINITE_LOGS_WRITE_USERNAME")
          password = sys.env("FINITE_LOGS_WRITE_PASSWORD")
        }
      }
    }
  '';
  staticMetrics = pkgs.writeText "finite-version-static.prom" cfg.staticVersionMetrics;
  latMonitoringSecretsCheck = pkgs.writeShellApplication {
    name = "check-lat-monitoring-secrets";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.gawk
      pkgs.gnugrep
    ];
    text = builtins.readFile ../scripts/check-lat-monitoring-secrets;
  };
  runtimeMetrics =
    pkgs.runCommand "finite-runtime-metrics" { nativeBuildInputs = [ pkgs.makeWrapper ]; }
      ''
        mkdir -p "$out/bin" "$out/lib/finite-runtime-metrics/scripts"
        cp ${../scripts/finite_runtime_metrics.py} "$out/lib/finite-runtime-metrics/finite_runtime_metrics.py"
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
    logsWriteEnvironmentFile = lib.mkOption {
      type = lib.types.str;
      default = logsWriteEnvironmentFile;
      readOnly = true;
      description = "Resolved environment file holding the Loki write credential.";
    };
    logRole = lib.mkOption {
      type = lib.types.str;
      default = config.networking.hostName;
      description = "Low-cardinality role label applied to forwarded journald logs.";
    };
    journalLogUnits = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      description = "Explicit systemd unit allowlist for journald log shipping.";
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
            forward_to = [prometheus.remote_write.finite_monitoring.receiver]

            rule {
              action        = "keep"
              source_labels = ["__name__"]
              regex         = "${allowedMetricNamesRegex}"
            }
          }

          prometheus.remote_write "finite_monitoring" {
            endpoint {
              url = "https://metrics-ingest.finite.computer/api/v1/write"

              basic_auth {
                username = sys.env("FINITE_METRICS_REMOTE_WRITE_USERNAME")
                password = sys.env("FINITE_METRICS_REMOTE_WRITE_PASSWORD")
              }
            }
          }
          ${logPipeline}
        '';
        systemd.services.alloy = {
          after = [ "prometheus-node-exporter.service" ];
          wants = [ "prometheus-node-exporter.service" ];
          serviceConfig = {
            EnvironmentFile = [
              metricsRemoteWriteEnvironmentFile
            ]
            ++ lib.optional (cfg.journalLogUnits != [ ]) cfg.logsWriteEnvironmentFile;
            SupplementaryGroups = [
              "adm"
              "systemd-journal"
            ];
          };
        };

        assertions = [
          {
            assertion = cfg.journalLogUnits == lib.unique cfg.journalLogUnits;
            message = "finite.metrics.journalLogUnits must not contain duplicates";
          }
          {
            assertion =
              cfg.journalLogUnits == [ ] || cfg.logsWriteEnvironmentFile != metricsRemoteWriteEnvironmentFile;
            message = "finite.metrics logs must use a separate Loki credential file";
          }
        ];
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
      (lib.mkIf (cfg.journalLogUnits != [ ]) {
        system.activationScripts.finite-lat-monitoring-secrets.text = ''
          ${latMonitoringSecretsCheck}/bin/check-lat-monitoring-secrets
        '';
      })
    ]
  );
}

# Dedicated production monitoring receiver.
#
# This replaces the historical Docker Compose stack with native NixOS services:
# Grafana for dashboards, Prometheus for remote-write + public probes, Loki for
# log ingestion, blackbox exporter for external uptime probes, and Caddy as the
# only public edge.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  grafanaDomain = "monitoring.finite.computer";
  ingestDomain = "metrics-ingest.finite.computer";

  grafanaAdminPasswordFile = "/etc/finite/monitoring/grafana-admin-password";
  grafanaSecretKeyFile = "/etc/finite/monitoring/grafana-secret-key";
  caddyEnvironmentFile = "/etc/finite/monitoring/caddy.env";

  dashboards = pkgs.runCommand "finite-grafana-dashboards" { } ''
    mkdir -p "$out"
    cp ${../../monitoring/grafana/dashboards/finite-production-mvp.json} "$out/finite-production-mvp.json"
  '';

  blackboxRelabelConfigs = [
    {
      source_labels = [ "__address__" ];
      target_label = "__param_target";
    }
    {
      source_labels = [ "__param_target" ];
      target_label = "instance";
    }
    {
      target_label = "__address__";
      replacement = "127.0.0.1:9115";
    }
  ];

  probeMetricRelabelConfigs = [
    {
      source_labels = [ "__name__" ];
      regex = "probe_success|probe_duration_seconds|probe_http_status_code";
      action = "keep";
    }
  ];

  publicProbe =
    {
      name,
      target,
      module ? "http_200",
      scrapeInterval ? null,
    }:
    {
      job_name = name;
      metrics_path = "/probe";
      params.module = [ module ];
      static_configs = [
        {
          targets = [ target ];
          labels.probe = "finite-monitoring";
        }
      ];
      relabel_configs = blackboxRelabelConfigs;
      metric_relabel_configs = probeMetricRelabelConfigs;
    }
    // lib.optionalAttrs (scrapeInterval != null) {
      scrape_interval = scrapeInterval;
    };

  blackboxConfig = (pkgs.formats.yaml { }).generate "blackbox.yml" {
    modules = {
      http_200 = {
        prober = "http";
        timeout = "3s";
        http = {
          method = "GET";
          valid_status_codes = [ 200 ];
          follow_redirects = true;
          preferred_ip_protocol = "ip4";
        };
      };

      http_404 = {
        prober = "http";
        timeout = "3s";
        http = {
          method = "GET";
          valid_status_codes = [ 404 ];
          follow_redirects = true;
          preferred_ip_protocol = "ip4";
        };
      };

      # /readyz performs a committed delivery-store probe with a 1s server
      # budget. Treat a slow edge-to-store result as unavailable too.
      chat_ready = {
        prober = "http";
        timeout = "1500ms";
        http = {
          method = "GET";
          valid_status_codes = [ 200 ];
          follow_redirects = true;
          preferred_ip_protocol = "ip4";
        };
      };
    };
  };
in
{
  services.prometheus.exporters.blackbox = {
    enable = true;
    listenAddress = "127.0.0.1";
    port = 9115;
    configFile = blackboxConfig;
  };

  services.prometheus = {
    enable = true;
    listenAddress = "127.0.0.1";
    port = 9090;
    retentionTime = "15d";
    extraFlags = [
      "--storage.tsdb.retention.size=20GB"
      "--web.enable-remote-write-receiver"
    ];
    globalConfig = {
      scrape_interval = "5m";
      scrape_timeout = "3s";
      evaluation_interval = "1m";
    };
    scrapeConfigs = [
      (publicProbe {
        name = "finite.computer";
        target = "https://finite.computer";
      })
      (publicProbe {
        name = "chat.finite.computer";
        target = "https://chat.finite.computer/readyz";
        module = "chat_ready";
        scrapeInterval = "1m";
      })
      (publicProbe {
        name = "brain.finite.computer";
        target = "https://brain.finite.computer/health";
      })
      (publicProbe {
        name = "finitechat-native-mockup.finite.chat";
        target = "https://finitechat-native-mockup.finite.chat/";
      })
      (publicProbe {
        name = "uptime-probe.docs.finite.chat";
        target = "https://uptime-probe.docs.finite.chat/";
        module = "http_404";
      })
    ];
  };

  services.loki = {
    enable = true;
    configuration = {
      auth_enabled = false;

      server = {
        http_listen_address = "127.0.0.1";
        http_listen_port = 3100;
        grpc_listen_address = "127.0.0.1";
        grpc_listen_port = 9096;
        log_level = "info";
      };

      common = {
        instance_addr = "127.0.0.1";
        path_prefix = "/var/lib/loki";
        storage.filesystem = {
          chunks_directory = "/var/lib/loki/chunks";
          rules_directory = "/var/lib/loki/rules";
        };
        replication_factor = 1;
        ring.kvstore.store = "inmemory";
      };

      query_range.results_cache.cache.embedded_cache = {
        enabled = true;
        max_size_mb = 100;
      };

      schema_config.configs = [
        {
          from = "2026-08-01";
          store = "tsdb";
          object_store = "filesystem";
          schema = "v13";
          index = {
            prefix = "index_";
            period = "24h";
          };
        }
      ];

      limits_config = {
        allow_structured_metadata = false;
        retention_period = "336h";
      };

      compactor = {
        working_directory = "/var/lib/loki/compactor";
        compaction_interval = "10m";
        retention_enabled = true;
        retention_delete_delay = "2h";
        retention_delete_worker_count = 2;
        delete_request_store = "filesystem";
      };

      analytics.reporting_enabled = false;
    };
  };

  services.grafana = {
    enable = true;
    settings = {
      server = {
        domain = grafanaDomain;
        http_addr = "127.0.0.1";
        http_port = 3000;
        root_url = "https://${grafanaDomain}/";
      };
      analytics.reporting_enabled = false;
      security = {
        admin_password = "$__file{${grafanaAdminPasswordFile}}";
        secret_key = "$__file{${grafanaSecretKeyFile}}";
      };
      users.allow_sign_up = false;
    };
    provision = {
      enable = true;
      datasources.settings = {
        apiVersion = 1;
        prune = true;
        datasources = [
          {
            name = "Finite Prometheus";
            type = "prometheus";
            uid = "finite-prometheus";
            access = "proxy";
            url = "http://127.0.0.1:9090";
            isDefault = true;
            editable = false;
            jsonData = {
              httpMethod = "POST";
              prometheusType = "Prometheus";
              prometheusVersion = "3.12.0";
              timeInterval = "1m";
            };
          }
          {
            name = "Finite Loki";
            type = "loki";
            uid = "finite-loki";
            access = "proxy";
            url = "http://127.0.0.1:3100";
            editable = false;
            jsonData.maxLines = 1000;
          }
        ];
      };
      dashboards.settings = {
        apiVersion = 1;
        providers = [
          {
            name = "finite";
            orgId = 1;
            folder = "";
            type = "file";
            disableDeletion = true;
            allowUiUpdates = false;
            options.path = dashboards;
          }
        ];
      };
    };
  };

  services.caddy = {
    enable = true;
    globalConfig = ''
      admin unix//run/caddy/admin.sock
      email ops@finite.computer
    '';
    virtualHosts = {
      ${grafanaDomain}.extraConfig = ''
        encode zstd gzip
        reverse_proxy 127.0.0.1:3000
      '';

      ${ingestDomain}.extraConfig = ''
        @remote_write {
          method POST
          path /api/v1/write
        }

        @loki_push {
          method POST
          path /loki/api/v1/push
        }

        handle @remote_write {
          basic_auth argon2id {
            {$METRICS_USERNAME} {$METRICS_PASSWORD_HASH}
          }
          reverse_proxy 127.0.0.1:9090
        }

        handle @loki_push {
          basic_auth argon2id {
            {$LOGS_USERNAME} {$LOGS_PASSWORD_HASH}
          }
          reverse_proxy 127.0.0.1:3100
        }

        handle {
          respond "Not found" 404
        }
      '';
    };
  };

  systemd.services.caddy.serviceConfig = {
    EnvironmentFile = [ caddyEnvironmentFile ];
    RuntimeDirectory = lib.mkDefault "caddy";
    RuntimeDirectoryMode = lib.mkDefault "0750";
  };

  systemd.tmpfiles.rules = [
    "d /etc/finite/monitoring 0700 root root - -"
    "z ${grafanaAdminPasswordFile} 0640 root grafana - -"
    "z ${grafanaSecretKeyFile} 0640 root grafana - -"
    "z ${caddyEnvironmentFile} 0600 root root - -"
  ];

  assertions = [
    {
      assertion = config.services.prometheus.listenAddress == "127.0.0.1";
      message = "monitoring Prometheus must stay loopback-only behind Caddy";
    }
    {
      assertion = config.services.loki.configuration.server.http_listen_address == "127.0.0.1";
      message = "monitoring Loki must stay loopback-only behind Caddy";
    }
    {
      assertion = config.services.grafana.settings.server.http_addr == "127.0.0.1";
      message = "monitoring Grafana must stay loopback-only behind Caddy";
    }
  ];
}

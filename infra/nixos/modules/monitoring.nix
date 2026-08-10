# Born observed (single-server-plan.md watch-list item 5): node-exporter on
# loopback, plus a health-check timer that curls every service's health
# endpoint, fails LOUDLY into the journal, and publishes machine-scoped
# Prometheus metrics through node-exporter's textfile collector.
#   journalctl -u finite-healthcheck   # is the box healthy?
{
  config,
  lib,
  pkgs,
  ...
}:
let
  grafanaCloudEnvironmentFile = "/etc/finite/grafana-cloud-metrics.env";
  healthMetricsDirectory = "/run/finite-monitoring";
  probedServiceUnits = [
    "finite-saas-core.service"
    "podman-finite-saas-dashboard.service"
    "finitechat-server.service"
    "finitechat-hosted-device.service"
    "finite-brain-app.service"
    "finite-saas-sites.service"
    "podman-searxng.service"
    "podman-firecrawl-api.service"
    "prometheus-node-exporter.service"
  ];
in
{
  services.prometheus.exporters.node = {
    enable = true;
    listenAddress = "127.0.0.1";
    port = 9100;
    enabledCollectors = [ "textfile" ];
    extraFlags = [ "--collector.textfile.directory=${healthMetricsDirectory}" ];
  };

  services.alloy = {
    enable = true;
    environmentFile = grafanaCloudEnvironmentFile;
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
        regex         = "finite_healthcheck_success|finite_service_health_status|node_textfile_mtime_seconds|node_textfile_scrape_error|up"
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

  systemd.services.finite-healthcheck = {
    description = "Curl every service health endpoint; fail loudly on any miss";
    # Ordering only: the observer must not start or retain the services it
    # checks. If a timer tick lands during a NixOS activation, systemd queues
    # the check behind any in-flight starts for these units.
    after = probedServiceUnits;
    path = [
      pkgs.coreutils
      pkgs.curl
    ];
    serviceConfig = {
      Type = "oneshot";
      DynamicUser = true;
      RuntimeDirectory = "finite-monitoring";
      RuntimeDirectoryMode = "0755";
      RuntimeDirectoryPreserve = "yes";
      # Keep slow or wedged local endpoints from extending an activation
      # indefinitely even though each probe has its own curl timeout.
      TimeoutStartSec = "2min";
    };
    script = ''
      set -eu
      max_attempts=13
      retry_delay_seconds=5
      attempt=1
      deadline=$((SECONDS + 60))
      host=${lib.escapeShellArg config.networking.hostName}
      metrics_dir="''${FINITE_HEALTHCHECK_METRICS_DIRECTORY:-${healthMetricsDirectory}}"
      metrics_file="$metrics_dir/finite-healthcheck.prom"
      metrics_tmp=
      trap 'rm -f "$metrics_tmp"' EXIT

      begin_metrics() {
        metrics_tmp=$(mktemp "$metrics_file.tmp.XXXXXX")
        {
          echo '# HELP finite_healthcheck_success Whether every internal health endpoint passed the latest check.'
          echo '# TYPE finite_healthcheck_success gauge'
          echo '# HELP finite_service_health_status Whether an internal service endpoint passed the latest check.'
          echo '# TYPE finite_service_health_status gauge'
        } > "$metrics_tmp"
      }

      check() {
        name=$1; shift
        if curl -fsS --max-time 10 -o /dev/null "$@"; then
          echo "OK   $name"
          status=1
        else
          echo "$failure_prefix $name ($*)" >&2
          fail=1
          status=0
        fi
        printf 'finite_service_health_status{host="%s",service="%s"} %s\n' \
          "$host" "$name" "$status" >> "$metrics_tmp"
      }

      publish_metrics() {
        if [ "$fail" -eq 0 ]; then
          aggregate=1
        else
          aggregate=0
        fi
        printf 'finite_healthcheck_success{host="%s"} %s\n' \
          "$host" "$aggregate" >> "$metrics_tmp"
        chmod 0644 "$metrics_tmp"
        mv -f "$metrics_tmp" "$metrics_file"
        metrics_tmp=
      }

      while :; do
        fail=0
        begin_metrics
        if [ "$attempt" -ge "$max_attempts" ] || [ "$SECONDS" -ge "$deadline" ]; then
          failure_prefix=FAIL
        else
          failure_prefix=WAIT
        fi

        check finite-saas-core    http://127.0.0.1:4200/healthz
        check dashboard           http://127.0.0.1:3000/healthz
        check finitechat-server   http://127.0.0.1:8788/health
        check hosted-web-device   http://127.0.0.1:38918/healthz
        check finite-brain        http://127.0.0.1:3015/health
        check finitesitesd        -H "Host: api.finite.chat" http://127.0.0.1:8787/api/v1/healthz
        check searxng             http://127.0.0.1:8080/healthz
        check firecrawl           http://127.0.0.1:3002/v0/health/readiness
        check node-exporter       http://127.0.0.1:9100/metrics

        publish_metrics
        if [ "$fail" -eq 0 ]; then
          exit 0
        fi
        if [ "$attempt" -ge "$max_attempts" ] || [ "$SECONDS" -ge "$deadline" ]; then
          echo "FAIL health endpoints remained unavailable after the bounded startup grace" >&2
          exit 1
        fi

        echo "WAIT health endpoints are still starting; retrying in $retry_delay_seconds seconds" >&2
        sleep "$retry_delay_seconds"
        attempt=$((attempt + 1))
      done
    '';
  };
  systemd.timers.finite-healthcheck = {
    wantedBy = [ "timers.target" ];
    timerConfig = {
      OnBootSec = "2min";
      OnUnitActiveSec = "1min";
    };
  };
}

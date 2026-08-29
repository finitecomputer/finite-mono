# Born observed (single-server-plan.md watch-list item 5): node-exporter on
# loopback, plus a health-check timer that curls every service's health
# endpoint, fails LOUDLY into the journal, and publishes machine-scoped
# Prometheus metrics through node-exporter's textfile collector.
#   journalctl -u finite-healthcheck   # is the box healthy?
{
  config,
  finitePackages,
  lib,
  pkgs,
  ...
}:
let
  healthMetricsDirectory = "/run/finite-monitoring";
  revision =
    if config.system.configurationRevision == null then "" else config.system.configurationRevision;
  prometheusEscape = lib.replaceStrings [ "\\" "\"" "\n" ] [ "\\\\" "\\\"" "\\n" ];
  versionMetric =
    {
      component,
      version,
      source,
      gitSha ? "",
      imageDigest ? "",
    }:
    ''
      finite_component_build_info{host="${prometheusEscape config.networking.hostName}",component="${prometheusEscape component}",version="${prometheusEscape version}",git_sha="${prometheusEscape gitSha}",image_digest="${prometheusEscape imageDigest}",source="${source}"} 1
      finite_component_version_mismatch{host="${prometheusEscape config.networking.hostName}",component="${prometheusEscape component}"} 0
    '';
  last = values: builtins.elemAt values (builtins.length values - 1);
  imageDigest =
    containerImage:
    if lib.hasInfix "@" containerImage then last (lib.splitString "@" containerImage) else "";
  imageVersion =
    containerImage:
    let
      digest = imageDigest containerImage;
    in
    if digest != "" then digest else last (lib.splitString ":" containerImage);
  dashboardImage = config.virtualisation.oci-containers.containers.finite-saas-dashboard.image;
  searxngImage = config.virtualisation.oci-containers.containers.searxng.image;
  firecrawlImage = config.virtualisation.oci-containers.containers.firecrawl-api.image;
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
  imports = [ ./metrics.nix ];

  finite.metrics = {
    enable = true;
    collectRuntimeArtifacts = true;
    logRole = "app";
    journalLogUnits = lib.unique (
      probedServiceUnits
      ++ [
        "alloy.service"
        "borgbackup-job-finite-hosted-web-chat-offsite.service"
        "caddy.service"
        "finite-core-private-proxy.service"
        "finite-healthcheck.service"
        "finite-hosted-web-chat-offsite-health.service"
        "finite-hosted-web-chat-snapshot-health.service"
        "finite-identity-backup-health.service"
        "finite-identity-backup.service"
        "finite-identity-private-proxy.service"
        "finite-identity.service"
        "finite-litestream-finite-brain.service"
        "finite-litestream-finite-chat-server.service"
        "finite-litestream-health.service"
        "finite-postgres-backup.service"
        "finite-runtime-metrics.service"
        "finite-saas-runner-phala.service"
        "finite-saas-runner.service"
      ]
    );
    hostIncidentLogSources = [
      {
        source = "kernel";
        matches = "_TRANSPORT=kernel PRIORITY=0";
      }
      {
        source = "kernel";
        matches = "_TRANSPORT=kernel PRIORITY=1";
      }
      {
        source = "kernel";
        matches = "_TRANSPORT=kernel PRIORITY=2";
      }
      {
        source = "kernel";
        matches = "_TRANSPORT=kernel PRIORITY=3";
      }
      {
        source = "kernel";
        matches = "_TRANSPORT=kernel PRIORITY=4";
      }
      {
        source = "systemd";
        matches = "SYSLOG_IDENTIFIER=systemd";
      }
      {
        source = "nixos-activation";
        matches = "SYSLOG_IDENTIFIER=nixos";
      }
      {
        source = "auth";
        matches = "SYSLOG_IDENTIFIER=sshd";
      }
      {
        source = "auth";
        matches = "SYSLOG_IDENTIFIER=sudo";
      }
    ];
    staticVersionMetrics = lib.concatMapStrings versionMetric [
      {
        component = "finite-saas-core";
        version = finitePackages.finite-saas-core.version;
        gitSha = revision;
        source = "nix";
      }
      {
        component = "finite-saas-dashboard";
        version = imageVersion dashboardImage;
        imageDigest = imageDigest dashboardImage;
        source = "image";
      }
      {
        component = "finitechat-server";
        version = finitePackages.finitechat-server.version;
        gitSha = revision;
        source = "nix";
      }
      {
        component = "finitechat-hosted-device";
        version = finitePackages.finitechat-hosted-device.version;
        gitSha = revision;
        source = "nix";
      }
      {
        component = "finite-brain";
        version = finitePackages.finite-brain.version;
        gitSha = revision;
        source = "nix";
      }
      {
        component = "finite-saas-sites";
        version = finitePackages.finitesitesd.version;
        gitSha = revision;
        source = "nix";
      }
      {
        component = "searxng";
        version = imageVersion searxngImage;
        imageDigest = imageDigest searxngImage;
        source = "image";
      }
      {
        component = "firecrawl";
        version = imageVersion firecrawlImage;
        imageDigest = imageDigest firecrawlImage;
        source = "image";
      }
      {
        component = "postgres";
        version = config.services.postgresql.package.version;
        source = "nix";
      }
      {
        component = "finite-saas-runner";
        version = finitePackages.finite-saas-runner.version;
        gitSha = revision;
        source = "nix";
      }
      {
        component = "nixos-system-profile";
        version = config.system.nixos.version;
        gitSha = revision;
        source = "nix";
      }
    ];
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
      SupplementaryGroups = [ "finite-monitoring" ];
      # DynamicUser implies ProtectSystem=strict, which mounts /run read-only
      # for the unit; the group grant alone cannot open the textfile directory.
      ReadWritePaths = [ healthMetricsDirectory ];
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
        chmod 0640 "$metrics_tmp"
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
        check finitechat-server   http://127.0.0.1:8788/readyz
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

# Born observed (single-server-plan.md watch-list item 5): node-exporter on
# loopback for a future scraper, plus a health-check timer that curls every
# service's health endpoint and fails LOUDLY into the journal.
#   journalctl -u finite-healthcheck   # is the box healthy?
#
# TODO: dead-man's-switch ping URL — pick a provider (e.g. healthchecks.io),
# put the URL in /etc/finite/monitoring.env as DEADMAN_PING_URL, and curl it
# at the end of the health check on success, so silence pages someone.
{ pkgs, ... }:
let
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
      pkgs.jq
    ];
    serviceConfig = {
      Type = "oneshot";
      DynamicUser = true;
      # Keep slow or wedged local endpoints from extending an activation
      # indefinitely even though each probe has its own curl timeout.
      TimeoutStartSec = "2min";
    };
    script = ''
      set -u
      max_attempts=13
      retry_delay_seconds=5
      attempt=1
      deadline=$((SECONDS + 60))

      check() {
        name=$1; shift
        if curl -fsS --max-time 10 -o /dev/null "$@"; then
          echo "OK   $name"
        else
          echo "$failure_prefix $name ($*)" >&2
          fail=1
        fi
      }

      check_finite_brain_cohort_writes() {
        endpoint=http://127.0.0.1:3015/health
        if body="$(curl -fsS --max-time 10 "$endpoint")" \
          && printf '%s' "$body" \
            | jq -e '(.capabilities // []) | index("account_cohort_writes_v1") != null' \
              >/dev/null; then
          echo "OK   finite-brain-cohort-writes"
        else
          echo "$failure_prefix finite-brain-cohort-writes ($endpoint missing account_cohort_writes_v1)" >&2
          fail=1
        fi
      }

      while :; do
        fail=0
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
        check_finite_brain_cohort_writes
        check finitesitesd        -H "Host: api.finite.chat" http://127.0.0.1:8787/api/v1/healthz
        check searxng             http://127.0.0.1:8080/healthz
        check firecrawl           http://127.0.0.1:3002/v0/health/readiness
        check node-exporter       http://127.0.0.1:9100/metrics

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

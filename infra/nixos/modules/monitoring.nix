# Born observed (single-server-plan.md watch-list item 5): node-exporter on
# loopback for a future scraper, plus a health-check timer that curls every
# service's health endpoint and fails LOUDLY into the journal.
#   journalctl -u finite-healthcheck   # is the box healthy?
#
# TODO: dead-man's-switch ping URL — pick a provider (e.g. healthchecks.io),
# put the URL in /etc/finite/monitoring.env as DEADMAN_PING_URL, and curl it
# at the end of the health check on success, so silence pages someone.
{ pkgs, ... }:
{
  services.prometheus.exporters.node = {
    enable = true;
    listenAddress = "127.0.0.1";
    port = 9100;
  };

  systemd.services.finite-healthcheck = {
    description = "Curl every service health endpoint; fail loudly on any miss";
    path = [ pkgs.curl ];
    serviceConfig = {
      Type = "oneshot";
      DynamicUser = true;
    };
    script = ''
      set -u
      fail=0
      check() {
        name=$1; shift
        if curl -fsS --max-time 10 -o /dev/null "$@"; then
          echo "OK   $name"
        else
          echo "FAIL $name ($*)" >&2
          fail=1
        fi
      }
      check finite-saas-core    http://127.0.0.1:4200/healthz
      check dashboard           http://127.0.0.1:3000/healthz
      check finitechat-server   http://127.0.0.1:8788/health
      check hosted-web-device   http://127.0.0.1:38918/healthz
      check finite-brain        http://127.0.0.1:3015/health
      check finitesitesd        -H "Host: api.finite.chat" http://127.0.0.1:8787/api/v1/healthz
      check searxng             http://127.0.0.1:8080/healthz
      check firecrawl           http://127.0.0.1:3002/v0/health/readiness
      check node-exporter       http://127.0.0.1:9100/metrics
      exit $fail
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

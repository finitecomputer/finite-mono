#!/usr/bin/env python3
"""Values-free contract for the NixOS production monitoring host."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DASHBOARD = ROOT / "infra/monitoring/grafana/dashboards/finite-production-mvp.json"

HOST_METRIC_NAMES = [
    "node_cpu_seconds_total",
    "node_load1",
    "node_load5",
    "node_load15",
    "node_memory_MemTotal_bytes",
    "node_memory_MemAvailable_bytes",
    "node_memory_SwapTotal_bytes",
    "node_memory_SwapFree_bytes",
    "node_filesystem_size_bytes",
    "node_filesystem_avail_bytes",
    "node_filesystem_readonly",
    "node_disk_read_bytes_total",
    "node_disk_written_bytes_total",
    "node_disk_io_time_seconds_total",
    "node_network_receive_bytes_total",
    "node_network_transmit_bytes_total",
    "node_network_receive_errs_total",
    "node_network_transmit_errs_total",
]

HOST_PANEL_TITLES = [
    "LAT Host Scrape Health",
    "LAT CPU Busy",
    "LAT Load Average",
    "LAT Memory Used",
    "LAT Swap Used",
    "LAT Filesystem Used",
    "LAT Disk Throughput",
    "LAT Disk I/O Time",
    "LAT Network Throughput",
    "LAT Network Errors",
    "LAT Filesystem Read-only",
]


def nix_eval() -> dict[str, Any]:
    expression = r'''
      let
        flake = builtins.getFlake (toString ./.);
        cfg = flake.nixosConfigurations.finite-monitoring.config;
        datasourceNames =
          map (datasource: datasource.name)
            cfg.services.grafana.provision.datasources.settings.datasources;
        datasourceUids =
          map (datasource: datasource.uid)
            cfg.services.grafana.provision.datasources.settings.datasources;
      in {
        hostName = cfg.networking.hostName;
        release = cfg.system.nixos.release;
        firewallTcp = cfg.networking.firewall.allowedTCPPorts;
        grafana = {
          enable = cfg.services.grafana.enable;
          address = cfg.services.grafana.settings.server.http_addr;
          domain = cfg.services.grafana.settings.server.domain;
          datasourceNames = datasourceNames;
          datasourceUids = datasourceUids;
          dashboardProviderCount =
            builtins.length cfg.services.grafana.provision.dashboards.settings.providers;
        };
        prometheus = {
          enable = cfg.services.prometheus.enable;
          address = cfg.services.prometheus.listenAddress;
          port = cfg.services.prometheus.port;
          retentionTime = cfg.services.prometheus.retentionTime;
          extraFlags = cfg.services.prometheus.extraFlags;
          scrapeJobs = map (job: job.job_name) cfg.services.prometheus.scrapeConfigs;
        };
        blackbox = {
          enable = cfg.services.prometheus.exporters.blackbox.enable;
          address = cfg.services.prometheus.exporters.blackbox.listenAddress;
          port = cfg.services.prometheus.exporters.blackbox.port;
        };
        loki = {
          enable = cfg.services.loki.enable;
          address = cfg.services.loki.configuration.server.http_listen_address;
          port = cfg.services.loki.configuration.server.http_listen_port;
          retention = cfg.services.loki.configuration.limits_config.retention_period;
          authEnabled = cfg.services.loki.configuration.auth_enabled;
        };
        caddy = {
          enable = cfg.services.caddy.enable;
          envFiles = cfg.systemd.services.caddy.serviceConfig.EnvironmentFile;
          grafanaVhost = cfg.services.caddy.virtualHosts."monitoring.finite.computer".extraConfig;
          ingestVhost = cfg.services.caddy.virtualHosts."metrics-ingest.finite.computer".extraConfig;
        };
        latMetrics = {
          finite-lat-1 = flake.nixosConfigurations.finite-lat-1.config.environment.etc."alloy/config.alloy".text;
          finite-lat-3 = flake.nixosConfigurations.finite-lat-3.config.environment.etc."alloy/config.alloy".text;
        };
      }
    '''
    completed = subprocess.run(
        [
            "nix",
            "eval",
            "--impure",
            "--json",
            "--expr",
            expression,
        ],
        cwd=ROOT,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    )
    return json.loads(completed.stdout)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def require_contains(haystack: str, needle: str, subject: str) -> None:
    require(needle in haystack, f"{subject} missing {needle!r}")


def panel_targets(panel: dict[str, Any]) -> list[dict[str, Any]]:
    return panel.get("targets", [])


def panel_rect(panel: dict[str, Any]) -> tuple[int, int, int, int]:
    grid = panel["gridPos"]
    return (grid["x"], grid["y"], grid["x"] + grid["w"], grid["y"] + grid["h"])


def overlaps(left: dict[str, Any], right: dict[str, Any]) -> bool:
    left_x1, left_y1, left_x2, left_y2 = panel_rect(left)
    right_x1, right_y1, right_x2, right_y2 = panel_rect(right)
    return left_x1 < right_x2 and right_x1 < left_x2 and left_y1 < right_y2 and right_y1 < left_y2


def check_dashboard_contract() -> None:
    dashboard = json.loads(DASHBOARD.read_text(encoding="utf-8"))
    panels = dashboard["panels"]
    panel_ids = [panel["id"] for panel in panels]
    require(len(panel_ids) == len(set(panel_ids)), "Grafana panel IDs must be unique")

    for index, left in enumerate(panels):
        for right in panels[index + 1 :]:
            require(
                not overlaps(left, right),
                f"Grafana panels overlap: {left['title']!r} and {right['title']!r}",
            )

    panels_by_title = {panel["title"]: panel for panel in panels}
    for title in HOST_PANEL_TITLES:
        require(title in panels_by_title, f"missing Grafana host panel {title!r}")

    rendered_dashboard = json.dumps(dashboard)
    for metric_name in HOST_METRIC_NAMES:
        require_contains(rendered_dashboard, metric_name, "Grafana host panels")

    for title in HOST_PANEL_TITLES:
        for target in panel_targets(panels_by_title[title]):
            expression = target["expr"]
            require_contains(expression, 'instance=~"finite-lat-1|finite-lat-3"', title)
            require(
                "finite-lat-2" not in expression,
                f"{title} must not include finite-lat-2 in production host panels",
            )

    require_contains(
        panels_by_title["LAT Filesystem Used"]["targets"][0]["expr"],
        'mountpoint=~"/|/data"',
        "LAT Filesystem Used",
    )
    require_contains(
        panels_by_title["LAT Network Throughput"]["targets"][0]["expr"],
        'device=~"en.*|eth.*|wg-finite"',
        "LAT Network Throughput",
    )


def main() -> int:
    contract = nix_eval()

    require(contract["hostName"] == "finite-monitoring", "unexpected monitoring hostname")
    require(contract["release"] == "26.05", "monitoring host must use NixOS 26.05")
    require(contract["firewallTcp"] == [22, 80, 443], "unexpected public TCP port set")

    grafana = contract["grafana"]
    require(grafana["enable"], "Grafana must be enabled")
    require(grafana["address"] == "127.0.0.1", "Grafana must bind loopback")
    require(grafana["domain"] == "monitoring.finite.computer", "Grafana domain drifted")
    require("finite-prometheus" in grafana["datasourceUids"], "Prometheus datasource uid missing")
    require("finite-loki" in grafana["datasourceUids"], "Loki datasource uid missing")
    require(grafana["dashboardProviderCount"] == 1, "expected exactly one dashboard provider")

    prometheus = contract["prometheus"]
    require(prometheus["enable"], "Prometheus must be enabled")
    require(prometheus["address"] == "127.0.0.1", "Prometheus must bind loopback")
    require(prometheus["port"] == 9090, "Prometheus port drifted")
    require(prometheus["retentionTime"] == "15d", "Prometheus retention time drifted")
    require(
        "--storage.tsdb.retention.size=20GB" in prometheus["extraFlags"],
        "Prometheus retention size missing",
    )
    require(
        "--web.enable-remote-write-receiver" in prometheus["extraFlags"],
        "Prometheus remote-write receiver must be enabled",
    )
    require(
        prometheus["scrapeJobs"]
        == [
            "finite.computer",
            "chat.finite.computer",
            "brain.finite.computer",
            "finitechat-native-mockup.finite.chat",
            "uptime-probe.docs.finite.chat",
        ],
        "public probe job set drifted",
    )

    blackbox = contract["blackbox"]
    require(blackbox["enable"], "blackbox exporter must be enabled")
    require(blackbox["address"] == "127.0.0.1", "blackbox exporter must bind loopback")
    require(blackbox["port"] == 9115, "blackbox exporter port drifted")

    loki = contract["loki"]
    require(loki["enable"], "Loki must be enabled")
    require(loki["address"] == "127.0.0.1", "Loki must bind loopback")
    require(loki["port"] == 3100, "Loki port drifted")
    require(loki["retention"] == "336h", "Loki retention drifted")
    require(loki["authEnabled"] is False, "Loki auth belongs at Caddy, not Loki")

    caddy = contract["caddy"]
    require(caddy["enable"], "Caddy must be enabled")
    require(
        caddy["envFiles"] == ["/etc/finite/monitoring/caddy.env"],
        "Caddy must load the monitoring credential hash env file",
    )
    require_contains(caddy["grafanaVhost"], "reverse_proxy 127.0.0.1:3000", "Grafana vhost")
    require_contains(caddy["ingestVhost"], "path /api/v1/write", "ingest vhost")
    require_contains(caddy["ingestVhost"], "path /loki/api/v1/push", "ingest vhost")
    require_contains(caddy["ingestVhost"], "{$METRICS_USERNAME}", "ingest vhost")
    require_contains(caddy["ingestVhost"], "{$LOGS_USERNAME}", "ingest vhost")
    require_contains(caddy["ingestVhost"], "reverse_proxy 127.0.0.1:9090", "ingest vhost")
    require_contains(caddy["ingestVhost"], "reverse_proxy 127.0.0.1:3100", "ingest vhost")

    for host_name, alloy_config in contract["latMetrics"].items():
        for metric_name in HOST_METRIC_NAMES:
            require_contains(alloy_config, metric_name, f"{host_name} Alloy config")

    check_dashboard_contract()

    print("monitoring NixOS contract OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

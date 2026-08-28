#!/usr/bin/env python3
"""Values-free contract for the NixOS production monitoring host."""

from __future__ import annotations

import json
import subprocess
import sys
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

LOG_PANEL_TITLES = [
    "LAT Recent Warning Logs",
]

LAT_LOG_UNITS = {
    "finite-lat-1": [
        "alloy.service",
        "caddy.service",
        "finite-healthcheck.service",
        "finite-saas-core.service",
        "finitechat-server.service",
        "finitechat-hosted-device.service",
        "finite-brain-app.service",
        "finite-saas-sites.service",
        "finite-identity.service",
        "finite-saas-runner.service",
        "prometheus-node-exporter.service",
        "finite-litestream-health.service",
        "borgbackup-job-finite-hosted-web-chat-offsite.service",
    ],
    "finite-lat-3": [
        "alloy.service",
        "finite-md-check.service",
        "finite-saas-runner.service",
        "finite-storage-health.service",
        "prometheus-node-exporter.service",
        "systemd-networkd.service",
        "wireguard-wg-finite.service",
    ],
}

LAT_ROLES = {
    "finite-lat-1": "app",
    "finite-lat-3": "runner",
}


def nix_eval() -> dict[str, Any]:
    expression = r"""
      let
        flake = builtins.getFlake (toString ./.);
        cfg = flake.nixosConfigurations.finite-monitoring.config;
        datasourceNames =
          map (datasource: datasource.name)
            cfg.services.grafana.provision.datasources.settings.datasources;
        datasourceUids =
          map (datasource: datasource.uid)
            cfg.services.grafana.provision.datasources.settings.datasources;
        chatProbe = builtins.head (
          builtins.filter
            (job: job.job_name == "chat.finite.computer")
            cfg.services.prometheus.scrapeConfigs
        );
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
          chatProbe = {
            interval = chatProbe.scrape_interval;
            module = builtins.head chatProbe.params.module;
            target = builtins.head (builtins.head chatProbe.static_configs).targets;
          };
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
          globalConfig = cfg.services.caddy.globalConfig;
          envFiles = cfg.systemd.services.caddy.serviceConfig.EnvironmentFile;
          runtimeDirectory = cfg.systemd.services.caddy.serviceConfig.RuntimeDirectory;
          runtimeDirectoryMode = cfg.systemd.services.caddy.serviceConfig.RuntimeDirectoryMode;
          grafanaVhost = cfg.services.caddy.virtualHosts."monitoring.finite.computer".extraConfig;
          ingestVhost = cfg.services.caddy.virtualHosts."metrics-ingest.finite.computer".extraConfig;
        };
        latAlloy = {
          finite-lat-1 = {
            config = flake.nixosConfigurations.finite-lat-1.config.environment.etc."alloy/config.alloy".text;
            envFiles = flake.nixosConfigurations.finite-lat-1.config.systemd.services.alloy.serviceConfig.EnvironmentFile;
            supplementaryGroups = flake.nixosConfigurations.finite-lat-1.config.systemd.services.alloy.serviceConfig.SupplementaryGroups;
            activation = flake.nixosConfigurations.finite-lat-1.config.system.activationScripts.finite-lat-monitoring-secrets.text;
          };
          finite-lat-3 = {
            config = flake.nixosConfigurations.finite-lat-3.config.environment.etc."alloy/config.alloy".text;
            envFiles = flake.nixosConfigurations.finite-lat-3.config.systemd.services.alloy.serviceConfig.EnvironmentFile;
            supplementaryGroups = flake.nixosConfigurations.finite-lat-3.config.systemd.services.alloy.serviceConfig.SupplementaryGroups;
            activation = flake.nixosConfigurations.finite-lat-3.config.system.activationScripts.finite-lat-monitoring-secrets.text;
          };
        };
      }
    """
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
    return (
        left_x1 < right_x2
        and right_x1 < left_x2
        and left_y1 < right_y2
        and right_y1 < left_y2
    )


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
    for title in LOG_PANEL_TITLES:
        require(title in panels_by_title, f"missing Grafana log panel {title!r}")

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

    for title in LOG_PANEL_TITLES:
        panel = panels_by_title[title]
        require(panel["datasource"]["uid"] == "finite-loki", f"{title} must use Loki")
        for target in panel_targets(panel):
            expression = target["expr"]
            require_contains(expression, 'host=~"finite-lat-1|finite-lat-3"', title)
            require_contains(
                expression, 'priority=~"warning|error|crit|alert|emerg"', title
            )
            require(
                "finite-lat-2" not in expression,
                f"{title} must not include finite-lat-2 in production log panels",
            )


def check_ubuntu_contract() -> None:
    subprocess.run(
        [sys.executable, "infra/monitoring/ubuntu/check_contract.py"],
        cwd=ROOT,
        check=True,
    )


def main() -> int:
    contract = nix_eval()

    require(
        contract["hostName"] == "finite-monitoring", "unexpected monitoring hostname"
    )
    require(contract["release"] == "26.05", "monitoring host must use NixOS 26.05")
    require(contract["firewallTcp"] == [22, 80, 443], "unexpected public TCP port set")

    grafana = contract["grafana"]
    require(grafana["enable"], "Grafana must be enabled")
    require(grafana["address"] == "127.0.0.1", "Grafana must bind loopback")
    require(grafana["domain"] == "monitoring.finite.computer", "Grafana domain drifted")
    require(
        "finite-prometheus" in grafana["datasourceUids"],
        "Prometheus datasource uid missing",
    )
    require("finite-loki" in grafana["datasourceUids"], "Loki datasource uid missing")
    require(
        grafana["dashboardProviderCount"] == 1,
        "expected exactly one dashboard provider",
    )

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
    require(
        prometheus["chatProbe"]
        == {
            "interval": "1m",
            "module": "chat_ready",
            "target": "https://chat.finite.computer/readyz",
        },
        "Chat probe must exercise semantic readiness every minute",
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
    require_contains(
        caddy["globalConfig"], "admin unix//run/caddy/admin.sock", "Caddy global config"
    )
    require(
        caddy["runtimeDirectory"] == "caddy",
        "Caddy must own /run/caddy for the Unix admin socket",
    )
    require(
        caddy["runtimeDirectoryMode"] == "0750", "Caddy runtime directory mode drifted"
    )
    require_contains(
        caddy["grafanaVhost"], "reverse_proxy 127.0.0.1:3000", "Grafana vhost"
    )
    require_contains(caddy["ingestVhost"], "path /api/v1/write", "ingest vhost")
    require_contains(caddy["ingestVhost"], "path /loki/api/v1/push", "ingest vhost")
    require_contains(caddy["ingestVhost"], "{$METRICS_USERNAME}", "ingest vhost")
    require_contains(caddy["ingestVhost"], "{$LOGS_USERNAME}", "ingest vhost")
    require_contains(
        caddy["ingestVhost"], "reverse_proxy 127.0.0.1:9090", "ingest vhost"
    )
    require_contains(
        caddy["ingestVhost"], "reverse_proxy 127.0.0.1:3100", "ingest vhost"
    )

    for host_name, alloy in contract["latAlloy"].items():
        alloy_config = alloy["config"]
        require(
            "/etc/finite/metrics-remote-write.env" in alloy["envFiles"],
            f"{host_name} Alloy must load the metrics write credential",
        )
        require(
            "/etc/finite/logs-write.env" in alloy["envFiles"],
            f"{host_name} Alloy must load the logs write credential",
        )
        require(
            "adm" in alloy["supplementaryGroups"]
            and "systemd-journal" in alloy["supplementaryGroups"],
            f"{host_name} Alloy must be able to read journald",
        )
        require_contains(
            alloy["activation"],
            "check-lat-monitoring-secrets",
            f"{host_name} monitoring secret activation preflight",
        )

        for metric_name in HOST_METRIC_NAMES:
            require_contains(alloy_config, metric_name, f"{host_name} Alloy config")

        for expected in (
            'loki.relabel "finite_journal"',
            'loki.write "finite_monitoring_logs"',
            "https://metrics-ingest.finite.computer/loki/api/v1/push",
            'sys.env("FINITE_LOGS_WRITE_USERNAME")',
            'sys.env("FINITE_LOGS_WRITE_PASSWORD")',
            'source_labels = ["__journal__systemd_unit"]',
            'target_label  = "unit"',
            'source_labels = ["__journal_priority_keyword"]',
            'target_label  = "priority"',
            'max_age       = "10m"',
            f'host = "{host_name}"',
            f'role = "{LAT_ROLES[host_name]}"',
        ):
            require_contains(alloy_config, expected, f"{host_name} Alloy log config")

        for unit in LAT_LOG_UNITS[host_name]:
            require_contains(
                alloy_config,
                f'matches       = "_SYSTEMD_UNIT={unit}"',
                f"{host_name} journald allowlist",
            )

    check_dashboard_contract()
    check_ubuntu_contract()

    print("monitoring NixOS contract OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

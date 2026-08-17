#!/usr/bin/env python3
"""Values-free contract for the NixOS production monitoring host."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]


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

    print("monitoring NixOS contract OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Values-free contract for the NixOS production monitoring host."""

from __future__ import annotations

import importlib.util
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
MVP_DASHBOARD = ROOT / "infra/monitoring/grafana/dashboards/finite-production-mvp.json"
SLOTS_DASHBOARD = (
    ROOT / "infra/monitoring/grafana/dashboards/finite-agent-runtime-slots.json"
)
SLOTS_HOSTS = ("finite-lat-3", "finite-lat-4")
TINFOIL_DASHBOARD = ROOT / "infra/monitoring/grafana/dashboards/finite-tinfoil-gpu.json"
TINFOIL_COLLECTOR = ROOT / "infra/monitoring/tinfoil/tinfoil-usage-collector"
TINFOIL_CONTAINER_NAME = "finite-private"

# Frozen binding of dashboard panel titles to the `finite_tinfoil_*` metric
# names. Shared by the dashboard contract check and the collector contract
# check so collector and dashboard cannot drift apart independently.
TINFOIL_PANEL_METRIC_BINDINGS = {
    "Data Freshness": "finite_tinfoil_source_sample_timestamp_seconds",
    "Sample Age": "finite_tinfoil_source_sample_timestamp_seconds",
    "Container Ready": "finite_tinfoil_container_ready",
    "GPU Allocation": "finite_tinfoil_container_gpus",
    "Model Upstream": "finite_tinfoil_component_ready",
    "Accounting API": "finite_tinfoil_component_ready",
    "GPU Utilization": "finite_tinfoil_gpu_utilization_percent",
    "GPU Memory Utilization": "finite_tinfoil_gpu_memory_utilization_percent",
    "CPU Utilization": "finite_tinfoil_cpu_utilization_percent",
    "Host Memory Utilization": "finite_tinfoil_host_memory_utilization_percent",
    "Current Dependency Probe Latency": "finite_tinfoil_component_probe_duration_seconds",
}

LAT_DASHBOARD_HOSTS = (
    "finite-lat-1",
    "finite-lat-2",
    "finite-lat-3",
    "finite-lat-4",
)
LAT_DASHBOARD_HOST_REGEX = "finite-lat-[1-4]"

HOST_METRIC_NAMES = [
    "node_cpu_seconds_total",
    "node_cpu_scaling_frequency_hertz",
    "node_cpu_scaling_frequency_max_hertz",
    "node_hwmon_sensor_label",
    "node_hwmon_temp_celsius",
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
    "LAT CPU Temperature",
    "LAT CPU Clock",
    "LAT Thermal Throttling Heuristic",
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
    "LAT Host Incident Logs",
    "LAT Kernel Warning Logs",
    "LAT Activation And Unit Lifecycle",
    "LAT SSH And Sudo Logs",
]

HOST_INCIDENT_LOG_SOURCES = [
    ("kernel", "_TRANSPORT=kernel PRIORITY=0"),
    ("kernel", "_TRANSPORT=kernel PRIORITY=1"),
    ("kernel", "_TRANSPORT=kernel PRIORITY=2"),
    ("kernel", "_TRANSPORT=kernel PRIORITY=3"),
    ("kernel", "_TRANSPORT=kernel PRIORITY=4"),
    ("systemd", "SYSLOG_IDENTIFIER=systemd"),
    ("nixos-activation", "SYSLOG_IDENTIFIER=nixos"),
    ("auth", "SYSLOG_IDENTIFIER=sshd"),
    ("auth", "SYSLOG_IDENTIFIER=sudo"),
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
    "finite-lat-2": [
        "alloy.service",
        "borgbackup-job-finite-hosted-web-chat-offsite.service",
        "caddy.service",
        "finite-core-private-proxy.service",
        "finite-healthcheck.service",
        "finite-hosted-web-chat-offsite-health.service",
        "finite-hosted-web-chat-snapshot-health.service",
        "finite-identity-backup-health.service",
        "finite-identity-backup.service",
        "finite-identity-private-proxy.service",
        "finite-identity.service",
        "finite-litestream-finite-brain.service",
        "finite-litestream-finite-chat-server.service",
        "finite-litestream-health.service",
        "finite-postgres-backup.service",
        "finite-runtime-metrics.service",
        "finite-saas-core.service",
        "finite-saas-sites.service",
        "finitechat-hosted-device.service",
        "finitechat-server.service",
        "finite-brain-app.service",
        "prometheus-node-exporter.service",
        "podman-finite-saas-dashboard.service",
    ],
    "finite-lat-4": [
        "alloy.service",
        "finite-md-check.service",
        "finite-saas-runner.service",
        "finite-storage-health.service",
        "prometheus-node-exporter.service",
        "systemd-networkd-persistent-storage.service",
        "systemd-networkd.service",
        "wireguard-wg-finite.service",
    ],
}

LAT_ROLES = {
    "finite-lat-1": "app",
    "finite-lat-3": "runner",
    # Emergency replacement app-plane host (lat1's stack, no runner).
    "finite-lat-2": "app",
    "finite-lat-4": "runner",
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
          finite-lat-2 = {
            config = flake.nixosConfigurations.finite-lat-2.config.environment.etc."alloy/config.alloy".text;
            envFiles = flake.nixosConfigurations.finite-lat-2.config.systemd.services.alloy.serviceConfig.EnvironmentFile;
            supplementaryGroups = flake.nixosConfigurations.finite-lat-2.config.systemd.services.alloy.serviceConfig.SupplementaryGroups;
            activation = flake.nixosConfigurations.finite-lat-2.config.system.activationScripts.finite-lat-monitoring-secrets.text;
          };
          finite-lat-4 = {
            config = flake.nixosConfigurations.finite-lat-4.config.environment.etc."alloy/config.alloy".text;
            envFiles = flake.nixosConfigurations.finite-lat-4.config.systemd.services.alloy.serviceConfig.EnvironmentFile;
            supplementaryGroups = flake.nixosConfigurations.finite-lat-4.config.systemd.services.alloy.serviceConfig.SupplementaryGroups;
            activation = flake.nixosConfigurations.finite-lat-4.config.system.activationScripts.finite-lat-monitoring-secrets.text;
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


def check_dashboard_layout(dashboard: dict[str, Any], subject: str) -> None:
    panels = dashboard["panels"]
    panel_ids = [panel["id"] for panel in panels]
    require(
        len(panel_ids) == len(set(panel_ids)), f"{subject} panel IDs must be unique"
    )

    for panel in panels:
        x1, y1, x2, y2 = panel_rect(panel)
        require(
            x1 >= 0 and y1 >= 0,
            f"{subject} panel {panel['title']!r} starts outside the grid",
        )
        require(
            x2 <= 24,
            f"{subject} panel {panel['title']!r} exceeds the 24-column grid",
        )
        require(
            x1 < x2 and y1 < y2,
            f"{subject} panel {panel['title']!r} has an empty grid area",
        )

    for index, left in enumerate(panels):
        for right in panels[index + 1 :]:
            require(
                not overlaps(left, right),
                f"{subject} panels overlap: {left['title']!r} and {right['title']!r}",
            )


def check_mvp_dashboard_contract() -> None:
    dashboard = json.loads(MVP_DASHBOARD.read_text(encoding="utf-8"))
    require(
        dashboard["refresh"] == "30s", "MVP Grafana dashboard must refresh every 30s"
    )
    check_dashboard_layout(dashboard, "MVP Grafana dashboard")
    panels = dashboard["panels"]
    panels_by_title = {panel["title"]: panel for panel in panels}
    for title in HOST_PANEL_TITLES:
        require(title in panels_by_title, f"missing Grafana host panel {title!r}")
    for title in LOG_PANEL_TITLES:
        require(title in panels_by_title, f"missing Grafana log panel {title!r}")

    rendered_dashboard = json.dumps(dashboard)
    require(
        "finite-lat-1|finite-lat-3" not in rendered_dashboard,
        "Grafana dashboard must not keep the pre-migration lat1/lat3-only selector",
    )
    for metric_name in HOST_METRIC_NAMES:
        require_contains(rendered_dashboard, metric_name, "Grafana host panels")

    for title in HOST_PANEL_TITLES:
        for target in panel_targets(panels_by_title[title]):
            expression = target["expr"]
            require_contains(
                expression,
                f'instance=~"{LAT_DASHBOARD_HOST_REGEX}"',
                title,
            )

    health_expression = panels_by_title["LAT Host Scrape Health"]["targets"][0]["expr"]
    for host_name in LAT_DASHBOARD_HOSTS:
        require_contains(
            health_expression,
            f'label_replace(vector(0), "instance", "{host_name}"',
            "LAT Host Scrape Health retired-host fallback",
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
    require_contains(
        panels_by_title["LAT CPU Temperature"]["targets"][0]["expr"],
        'label=~"Tctl|Tccd[0-9]+"',
        "LAT CPU Temperature",
    )
    throttle_expression = panels_by_title["LAT Thermal Throttling Heuristic"][
        "targets"
    ][0]["expr"]
    for threshold in (">= bool 95", ">= bool 70", "<= bool 0.70"):
        require_contains(
            throttle_expression,
            threshold,
            "LAT Thermal Throttling Heuristic",
        )

    for title in LOG_PANEL_TITLES:
        panel = panels_by_title[title]
        require(panel["datasource"]["uid"] == "finite-loki", f"{title} must use Loki")
        for target in panel_targets(panel):
            expression = target["expr"]
            require_contains(
                expression,
                f'host=~"{LAT_DASHBOARD_HOST_REGEX}"',
                title,
            )
            require_contains(
                expression, 'priority=~"warning|error|crit|alert|emerg"', title
            )

    require_contains(
        panels_by_title["LAT Recent Warning Logs"]["targets"][0]["expr"],
        'priority=~"warning|error|crit|alert|emerg"',
        "LAT Recent Warning Logs",
    )
    require_contains(
        panels_by_title["LAT Host Incident Logs"]["targets"][0]["expr"],
        'source=~"kernel|systemd|nixos-activation|auth"',
        "LAT Host Incident Logs",
    )
    require_contains(
        panels_by_title["LAT Kernel Warning Logs"]["targets"][0]["expr"],
        'source="kernel"',
        "LAT Kernel Warning Logs",
    )
    require_contains(
        panels_by_title["LAT Kernel Warning Logs"]["targets"][0]["expr"],
        'priority=~"warning|error|crit|alert|emerg"',
        "LAT Kernel Warning Logs",
    )
    require_contains(
        panels_by_title["LAT Activation And Unit Lifecycle"]["targets"][0]["expr"],
        'source=~"systemd|nixos-activation"',
        "LAT Activation And Unit Lifecycle",
    )
    require_contains(
        panels_by_title["LAT SSH And Sudo Logs"]["targets"][0]["expr"],
        'source="auth"',
        "LAT SSH And Sudo Logs",
    )


def runner_slot_capacity() -> dict[str, str]:
    """Import the operator-pinned slot ceiling from the runner host contract.

    The Agent Runtime slots dashboard renders the per-host
    FC_RUNNER_MAX_SANDBOXES ceiling as a query constant because no live
    metric carries it. Importing the same mapping that
    scripts/check_runner_host_contract.py enforces against the rendered
    runner env keeps the dashboard and the hosts on one pinned value; a
    ceiling change must update both or CI fails.
    """
    path = ROOT / "scripts/check_runner_host_contract.py"
    spec = importlib.util.spec_from_file_location("check_runner_host_contract", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.EXPECTED_MAX_SANDBOXES


def check_agent_runtime_slots_dashboard_contract() -> None:
    dashboard = json.loads(SLOTS_DASHBOARD.read_text(encoding="utf-8"))
    require(
        dashboard["uid"] == "finite-agent-runtime-slots", "unexpected dashboard uid"
    )
    require(dashboard["refresh"] == "2m", "slots dashboard must refresh every 2m")
    panels = dashboard["panels"]
    panel_ids = [panel["id"] for panel in panels]
    require(
        len(panel_ids) == len(set(panel_ids)),
        "slots dashboard panel IDs must be unique",
    )
    for index, left in enumerate(panels):
        for right in panels[index + 1 :]:
            require(
                not overlaps(left, right),
                f"slots dashboard panels overlap: {left['title']!r} and {right['title']!r}",
            )

    panels_by_title = {panel["title"]: panel for panel in panels}
    required_titles = (
        "Draft data contract",
        "lat3 Slots Used",
        "lat3 Slots Free",
        "lat4 Slots Used",
        "lat4 Slots Free",
        "Active Agent Runtimes",
        "Free Slots",
    )
    for title in required_titles:
        require(title in panels_by_title, f"slots dashboard missing panel {title!r}")

    notice = panels_by_title["Draft data contract"]
    require(notice["type"] == "text", "draft notice must stay a text panel")
    rendered_notice = notice["options"]["content"]
    for fragment in (
        "not yet provisioned",
        "finite_runtime_artifact_active_agents",
        "FC_RUNNER_MAX_SANDBOXES",
        "check_runner_host_contract.py",
        "No data",
    ):
        require_contains(rendered_notice, fragment, "slots dashboard draft notice")

    capacity = runner_slot_capacity()
    require(
        capacity["finite-lat-3"] == capacity["finite-lat-4"],
        "the combined slots chart assumes equal ceilings on both Runner hosts",
    )
    expected_exprs: set[str] = set()
    for host in SLOTS_HOSTS:
        ceiling = capacity[host]
        expected_exprs.add(
            f'sum(finite_runtime_artifact_active_agents{{source_host_id="{host}"}})'
        )
        expected_exprs.add(
            f'{ceiling} - sum(finite_runtime_artifact_active_agents{{source_host_id="{host}"}})'
        )
    expected_exprs.add(
        "sum by (source_host_id) (finite_runtime_artifact_active_agents"
        '{source_host_id=~"finite-lat-3|finite-lat-4"})'
    )
    expected_exprs.add(
        f"{capacity['finite-lat-3']} - "
        "sum by (source_host_id) (finite_runtime_artifact_active_agents"
        '{source_host_id=~"finite-lat-3|finite-lat-4"})'
    )

    actual_exprs: set[str] = set()
    for title in required_titles:
        panel = panels_by_title[title]
        if panel["type"] == "text":
            continue
        require(
            panel["datasource"]["uid"] == "finite-prometheus",
            f"{title} must use the finite-prometheus datasource",
        )
        for target in panel_targets(panel):
            expression = target["expr"]
            actual_exprs.add(expression)
            require(
                "or vector(" not in expression,
                f"{title} must fail closed on missing data, not substitute a constant",
            )
            require(
                "up{" not in expression,
                f"{title} must not gate on scrape health",
            )

    require(
        actual_exprs == expected_exprs,
        "slots dashboard query expressions drifted from the pinned contract",
    )

    rendered_dashboard = json.dumps(dashboard)
    metric_tokens = set(re.findall(r"finite_[a-z_]+", rendered_dashboard))
    unexpected = metric_tokens - {"finite_runtime_artifact_active_agents"}
    require(
        not unexpected,
        f"slots dashboard references metrics outside its contract: {sorted(unexpected)}",
    )


def check_tinfoil_dashboard_contract() -> None:
    dashboard = json.loads(TINFOIL_DASHBOARD.read_text(encoding="utf-8"))
    require(
        dashboard["refresh"] == "2m",
        "Tinfoil Grafana dashboard must refresh every 2m (Tinfoil publishes 48m buckets)",
    )
    check_dashboard_layout(dashboard, "Tinfoil Grafana dashboard")
    panels = dashboard["panels"]
    panels_by_title = {panel["title"]: panel for panel in panels}
    required_titles = {
        "Draft data contract",
        "Data Freshness",
        "Container Ready",
        "Model Upstream",
        "Accounting API",
        "GPU Allocation",
        "Sample Age",
        "GPU Utilization",
        "GPU Memory Utilization",
        "CPU Utilization",
        "Host Memory Utilization",
        "Current Dependency Probe Latency",
    }
    missing_titles = sorted(required_titles - panels_by_title.keys())
    require(
        not missing_titles,
        f"Tinfoil Grafana dashboard is missing panels: {missing_titles}",
    )

    for panel in panels:
        if panel["type"] != "text":
            require(
                panel["datasource"]["uid"] == "finite-prometheus",
                f"{panel['title']} must use finite-prometheus",
            )
            for target in panel_targets(panel):
                require(
                    'container="' in target["expr"],
                    f"{panel['title']} query must select a container",
                )

    rendered_dashboard = json.dumps(dashboard)
    container_selectors = set(
        re.findall(r'container=\\"([^"\\]+)\\"', rendered_dashboard)
    )
    require(
        len(container_selectors) == 1,
        f"Tinfoil dashboard container selectors must agree: {sorted(container_selectors)}",
    )
    (container_name,) = container_selectors
    require(
        container_name in panels_by_title["Draft data contract"]["options"]["content"],
        f"Tinfoil draft notice must name the selected container {container_name!r}",
    )
    panel_metric_bindings = TINFOIL_PANEL_METRIC_BINDINGS
    for title, metric_name in panel_metric_bindings.items():
        expressions = [
            target["expr"] for target in panel_targets(panels_by_title[title])
        ]
        require(
            expressions,
            f"{title} must have at least one query target",
        )
        for expression in expressions:
            require_contains(expression, metric_name, title)

    require(
        'component="upstream"'
        in panels_by_title["Model Upstream"]["targets"][0]["expr"],
        "Model Upstream must probe the upstream component",
    )
    require(
        'component="usage_api"'
        in panels_by_title["Accounting API"]["targets"][0]["expr"],
        "Accounting API must probe the usage_api component",
    )
    require(
        panels_by_title["Current Dependency Probe Latency"]["targets"][0][
            "legendFormat"
        ]
        == "{{component}}",
        "Current Dependency Probe Latency must label series by component",
    )

    freshness_expression = panels_by_title["Data Freshness"]["targets"][0]["expr"]
    require_contains(
        freshness_expression,
        "clamp(floor((time() - finite_tinfoil_source_sample_timestamp_seconds",
        "Data Freshness",
    )
    require_contains(freshness_expression, "/ 300), 0, 2)", "Data Freshness")
    require_contains(freshness_expression, "or on() vector(2)", "Data Freshness")
    sample_age_thresholds = [
        step["value"]
        for step in panels_by_title["Sample Age"]["fieldConfig"]["defaults"][
            "thresholds"
        ]["steps"]
    ]
    require(
        sample_age_thresholds == [0, 300, 600],
        "Sample Age thresholds must stay aligned with the freshness buckets",
    )
    require(
        "up{" not in rendered_dashboard,
        "Tinfoil collector health must fail closed on sample age",
    )
    freshness_defaults = panels_by_title["Data Freshness"]["fieldConfig"]["defaults"]
    freshness_mappings = freshness_defaults["mappings"][0]
    require(
        {
            value: mapping["text"]
            for value, mapping in freshness_mappings["options"].items()
        }
        == {"0": "FRESH", "1": "AGING", "2": "STALE"},
        "Data Freshness mappings drifted",
    )
    notice = panels_by_title["Draft data contract"]["options"]["content"]
    require_contains(
        notice,
        "It is not yet provisioned to production Grafana and no production Tinfoil metrics source is wired yet",
        "Tinfoil draft notice",
    )


def check_tinfoil_collector_contract() -> None:
    """Pin the textfile collector to the dashboard's frozen metric bindings.

    The dashboard's metric names are the collector contract: the collector
    script must emit exactly those names, with the same container selector,
    the same component label values, and fail-closed rendering (NaN until a
    successful sample, frozen sample timestamp). Drift fails CI, not Grafana.
    """
    collector = TINFOIL_COLLECTOR.read_text()
    # The collector writes utilization metrics as `finite_tinfoil_${key}`
    # over its USAGE_KEYS list; expand that template before comparing with
    # the frozen bindings.
    usage_keys_match = re.search(r"USAGE_KEYS=\(([^)]*)\)", collector)
    require(usage_keys_match, "Tinfoil collector must define USAGE_KEYS")
    emitted = set(re.findall(r"finite_tinfoil_[a-z_]+", collector))
    for key in re.findall(r"[a-z_]+", usage_keys_match.group(1)):
        emitted.add(f"finite_tinfoil_{key}")
    contract = set(TINFOIL_PANEL_METRIC_BINDINGS.values())
    require(
        emitted == contract,
        "Tinfoil collector metrics must equal the dashboard panel bindings: "
        f"collector-only={sorted(emitted - contract)} "
        f"dashboard-only={sorted(contract - emitted)}",
    )
    require(
        f'CONTAINER="${{FINITE_TINFOIL_CONTAINER:-{TINFOIL_CONTAINER_NAME}}}"'
        in collector,
        "Tinfoil collector default container must match the dashboard selector",
    )
    require(
        "COMPONENT_NAMES=(upstream usage_api)" in collector,
        "Tinfoil collector components must stay upstream and usage_api",
    )
    require_contains(
        collector, 'aggregation=\\"mean\\"', "collector utilization aggregation label"
    )
    require_contains(
        collector,
        "value_or_nan",
        "collector sample timestamp must render NaN until a good sample",
    )
    require_contains(
        collector,
        'LAST_GOOD_TS_FILE="${STATE_DIR}/last-good-sample-ts"',
        "collector must persist the last good sample timestamp (fail closed)",
    )
    require_contains(
        collector,
        'TEXTFILE="${STATE_DIR}/textfile/tinfoil.prom"',
        "collector must write the node_exporter textfile",
    )
    require_contains(
        collector,
        'container get "$CONTAINER" --output json',
        "collector must use the same tinfoil CLI flags as finite-private-ops",
    )
    require_contains(
        collector,
        "timeout -s TERM",
        "collector must bound the tinfoil CLI and usage command",
    )
    require(
        "up{" not in collector,
        "Tinfoil collector health must fail closed on sample age, not up{}",
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
            "v2.finite.chat",
            "uptime-probe.v2.finite.chat",
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

        require_contains(
            alloy_config,
            'scrape_interval = "15s"',
            f"{host_name} Alloy metrics cadence",
        )

        for expected in (
            'loki.relabel "finite_journal"',
            'loki.write "finite_monitoring_logs"',
            "https://metrics-ingest.finite.computer/loki/api/v1/push",
            'sys.env("FINITE_LOGS_WRITE_USERNAME")',
            'sys.env("FINITE_LOGS_WRITE_PASSWORD")',
            'source_labels = ["__journal__systemd_unit"]',
            'source_labels = ["__journal_unit"]',
            'regex         = "(.+)"',
            'target_label  = "unit"',
            'source_labels = ["__journal_priority_keyword"]',
            'target_label  = "priority"',
            'max_age       = "10m"',
            f'host = "{host_name}"',
            f'role = "{LAT_ROLES[host_name]}"',
            'source = "service"',
        ):
            require_contains(alloy_config, expected, f"{host_name} Alloy log config")

        for unit in LAT_LOG_UNITS[host_name]:
            require_contains(
                alloy_config,
                f'matches       = "_SYSTEMD_UNIT={unit}"',
                f"{host_name} journald allowlist",
            )

        for index, (source, matches) in enumerate(HOST_INCIDENT_LOG_SOURCES):
            require_contains(
                alloy_config,
                f'loki.source.journal "finite_host_incident_{index}"',
                f"{host_name} host incident journald source",
            )
            require_contains(
                alloy_config,
                f'matches       = "{matches}"',
                f"{host_name} host incident journald source",
            )
            require_contains(
                alloy_config,
                f'source = "{source}"',
                f"{host_name} host incident source label",
            )

    check_mvp_dashboard_contract()
    check_agent_runtime_slots_dashboard_contract()
    check_tinfoil_dashboard_contract()
    check_tinfoil_collector_contract()
    check_ubuntu_contract()

    print("monitoring NixOS contract OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

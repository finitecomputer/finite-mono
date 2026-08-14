#!/usr/bin/env python3
"""Verify the lean self-hosted monitoring deployment contract."""

from __future__ import annotations

import json
import re
from pathlib import Path

STACK = Path(__file__).resolve().parents[1]

EXPECTED_IMAGES = {
    "grafana/grafana-oss:13.0.2@sha256:5dad0df181cb644a14e13617b913b261a54f7d4fd4510721dba420929f35bea2",
    "prom/prometheus:v3.13.2@sha256:508729e0e2d18e11fd742a5a5ca70e557b940a93948c3c95fd0123a6fd538b69",
    "prom/blackbox-exporter:v0.28.0@sha256:e753ff9f3fc458d02cca5eddab5a77e1c175eee484a8925ac7d524f04366c2fc",
    "caddy:2.11.4-alpine@sha256:5f5c8640aae01df9654968d946d8f1a56c497f1dd5c5cda4cf95ab7c14d58648",
}
EXPECTED_TARGETS = {
    "finite.computer": "https://finite.computer",
    "chat.finite.computer": "https://chat.finite.computer/health",
    "brain.finite.computer": "https://brain.finite.computer/health",
    "finitechat-native-mockup.finite.chat": "https://finitechat-native-mockup.finite.chat/",
    "uptime-probe.docs.finite.chat": "https://uptime-probe.docs.finite.chat/",
}


def require(text: str, expected: str, source: str) -> None:
    if expected not in text:
        raise SystemExit(f"{source} is missing {expected!r}")


def main() -> None:
    compose = (STACK / "compose.yaml").read_text(encoding="utf-8")
    images = set(re.findall(r"^\s+image: (\S+)$", compose, re.MULTILINE))
    if images != EXPECTED_IMAGES:
        raise SystemExit(f"container images differ:\nexpected={EXPECTED_IMAGES}\nactual={images}")
    if compose.count("ports:") != 1:
        raise SystemExit("only Caddy may publish host ports")
    for expected in (
        '- "80:80"',
        '- "443:443"',
        "--web.enable-remote-write-receiver",
        "${CADDY_CONFIG_FILE:?set CADDY_CONFIG_FILE}",
        "--envfile",
        "/run/secrets/caddy_env",
    ):
        require(compose, expected, "compose.yaml")

    prometheus = (STACK / "prometheus.yml").read_text(encoding="utf-8")
    for job, target in EXPECTED_TARGETS.items():
        require(prometheus, f"job_name: {job}", "prometheus.yml")
        require(prometheus, target, "prometheus.yml")
    for expected in (
        "scrape_interval: 5m",
        "scrape_timeout: 3s",
        "time: 15d",
        "size: 20GB",
        "probe: self-hosted-monitoring",
        "module: [http_404]",
        "probe_success|probe_duration_seconds|probe_http_status_code",
    ):
        require(prometheus, expected, "prometheus.yml")

    blackbox = (STACK / "blackbox.yml").read_text(encoding="utf-8")
    for expected in ("http_200:", "valid_status_codes: [200]", "http_404:", "valid_status_codes: [404]"):
        require(blackbox, expected, "blackbox.yml")

    caddy = (STACK / "Caddyfile").read_text(encoding="utf-8")
    for expected in (
        "method POST",
        "path /api/v1/write",
        "basic_auth argon2id",
        "respond \"Not found\" 404",
    ):
        require(caddy, expected, "Caddyfile")

    ip_caddy = (STACK / "Caddyfile.ip").read_text(encoding="utf-8")
    for expected in (
        "http://{$MONITORING_IP}",
        "path /api/v1/write",
        'respond "TLS required" 426',
        "reverse_proxy grafana:3000",
    ):
        require(ip_caddy, expected, "Caddyfile.ip")

    datasource = (STACK / "grafana/provisioning/datasources/prometheus.yml").read_text(encoding="utf-8")
    require(datasource, "uid: finite-prometheus", "Grafana datasource")
    require(datasource, "url: http://prometheus:9090", "Grafana datasource")

    installer = (STACK / "install-ubuntu").read_text(encoding="utf-8")
    require(installer, 'chown root:472 "${GRAFANA_PASSWORD_FILE}"', "installer")
    require(installer, 'chmod 0640 "${GRAFANA_PASSWORD_FILE}"', "installer")
    require(installer, "caddy validate", "installer")
    require(installer, "admin reset-admin-password", "installer")
    require(installer, 'MONITORING_MODE must be \'ip\' or \'dns\'', "installer")
    require(installer, 'REMOTE_WRITE_UNAUTHENTICATED_STATUS="426"', "installer")

    dashboard_path = STACK / "grafana/dashboards/finite-production-mvp.json"
    dashboard = json.loads(dashboard_path.read_text(encoding="utf-8"))
    if dashboard.get("uid") != "finite-production-mvp":
        raise SystemExit("dashboard UID changed")
    if len(dashboard.get("panels", [])) != 7:
        raise SystemExit("dashboard must contain exactly seven MVP panels")
    for panel in dashboard["panels"]:
        if panel.get("datasource", {}).get("uid") != "finite-prometheus":
            raise SystemExit(f"panel {panel.get('title')!r} uses the wrong datasource")

    panels_by_title = {panel.get("title"): panel for panel in dashboard["panels"]}
    expected_table_queries = {
        "Running Versions": "1000 * topk by (component, host) (1, timestamp(finite_component_build_info))",
    }
    for title, expected_query in expected_table_queries.items():
        panel = panels_by_title.get(title)
        if panel is None:
            raise SystemExit(f"dashboard is missing {title!r}")
        targets = panel.get("targets", [])
        if not targets or targets[0].get("expr") != expected_query:
            raise SystemExit(f"{title!r} must use the newest-sample table query")
        rendered = json.dumps(panel, sort_keys=True)
        require(rendered, '"Value": "Observed"', title)
        require(rendered, '"dateTimeAsIso"', title)

    runtime_artifacts = panels_by_title.get("Runtime Artifacts")
    if runtime_artifacts is None:
        raise SystemExit("dashboard is missing 'Runtime Artifacts'")
    targets = runtime_artifacts.get("targets", [])
    expected_runtime_query = (
        "sort_desc(sum by (artifact_id, version_label, promoted) "
        "(max by (source_host_id, artifact_id, version_label, promoted) "
        "(finite_runtime_artifact_active_agents)))"
    )
    if not targets or targets[0].get("expr") != expected_runtime_query:
        raise SystemExit("'Runtime Artifacts' must use the active-agent count query")
    rendered = json.dumps(runtime_artifacts, sort_keys=True)
    require(rendered, '"Value": "Active agents"', "Runtime Artifacts")
    require(rendered, "finite_runtime_artifact_active_agents", "Runtime Artifacts")

    version_drift = panels_by_title.get("Version Drift")
    if version_drift is None:
        raise SystemExit("dashboard is missing 'Version Drift'")
    targets = version_drift.get("targets", [])
    expected_drift_query = (
        "sort_desc(max by (component, host) "
        "(finite_component_version_mismatched_active_agents) > 0)"
    )
    if not targets or targets[0].get("expr") != expected_drift_query:
        raise SystemExit("'Version Drift' must use the drifted active-agent count query")
    rendered = json.dumps(version_drift, sort_keys=True)
    require(rendered, '"Value": "Drifted active agents"', "Version Drift")
    require(rendered, "finite_component_version_mismatched_active_agents", "Version Drift")

    print("self-hosted monitoring contract: ok")


if __name__ == "__main__":
    main()

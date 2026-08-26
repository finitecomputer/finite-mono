#!/usr/bin/env python3
"""Values-free contract for the Ubuntu/systemd production monitoring host."""

from __future__ import annotations

import re
from pathlib import Path


UBUNTU = Path(__file__).resolve().parent
ROOT = UBUNTU.parents[2]
README = ROOT / "infra/monitoring/README.md"
DEPLOY = UBUNTU / "deploy"

EXPECTED_FILES = [
    "versions.env",
    "Caddyfile",
    "prometheus.yml",
    "blackbox.yml",
    "loki.yml",
    "grafana/grafana.ini",
    "grafana/provisioning/datasources/finite.yml",
    "grafana/provisioning/dashboards/finite.yml",
    "systemd/finite-monitoring-blackbox-exporter.service",
    "systemd/finite-monitoring-prometheus.service",
    "systemd/finite-monitoring-loki.service",
    "systemd/finite-monitoring-grafana.service",
    "systemd/finite-monitoring-caddy.service",
    "systemd/finite-monitoring-node-exporter.service",
    "systemd/finite-monitoring-tinfoil-collector.service",
    "systemd/finite-monitoring-tinfoil-collector.timer",
]

EXPECTED_VERSION_KEYS = {
    "GRAFANA_VERSION": "13.0.2",
    "GRAFANA_SHA256": r"[0-9a-f]{64}",
    "PROMETHEUS_VERSION": "3.13.2",
    "PROMETHEUS_SHA256": r"[0-9a-f]{64}",
    "BLACKBOX_EXPORTER_VERSION": "0.28.0",
    "BLACKBOX_EXPORTER_SHA256": r"[0-9a-f]{64}",
    "CADDY_VERSION": "2.11.4",
    "CADDY_SHA512": r"[0-9a-f]{128}",
    "LOKI_VERSION": "3.5.8",
    "LOKI_SHA256": r"[0-9a-f]{64}",
    "NODE_EXPORTER_VERSION": "1.9.1",
    "NODE_EXPORTER_SHA256": r"[0-9a-f]{64}",
    "JQ_VERSION": "1.8.1",
    "JQ_SHA256": r"[0-9a-f]{64}",
}


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def require_contains(haystack: str, needle: str, subject: str) -> None:
    require(needle in haystack, f"{subject} missing {needle!r}")


def parse_env(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for raw_line in read(path).splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        require(separator == "=", f"{path} has non-assignment line: {raw_line!r}")
        values[key] = value
    return values


def check_versions() -> None:
    versions = parse_env(UBUNTU / "versions.env")
    for key, expected in EXPECTED_VERSION_KEYS.items():
        require(key in versions, f"versions.env missing {key}")
        value = versions[key]
        if expected.startswith("["):
            require(
                re.fullmatch(expected, value) is not None,
                f"{key} has invalid hash shape",
            )
        else:
            require(value == expected, f"{key} drifted: {value!r}")

    for key in [
        "GRAFANA_URL",
        "PROMETHEUS_URL",
        "BLACKBOX_EXPORTER_URL",
        "CADDY_URL",
        "LOKI_URL",
        "NODE_EXPORTER_URL",
        "JQ_URL",
    ]:
        require(key in versions, f"versions.env missing {key}")
        require(versions[key].startswith("https://"), f"{key} must use https")


def check_caddy() -> None:
    caddy = read(UBUNTU / "Caddyfile")
    require_contains(caddy, "monitoring.finite.computer", "Caddyfile")
    require_contains(caddy, "crm.finite.computer", "Caddyfile")
    require_contains(caddy, "metrics-ingest.finite.computer", "Caddyfile")
    require_contains(
        caddy, "admin unix//run/finite-monitoring-caddy/admin.sock", "Caddyfile"
    )
    require_contains(caddy, "reverse_proxy 127.0.0.1:3000", "Grafana route")
    require_contains(caddy, "reverse_proxy 127.0.0.1:3020", "commercial register route")
    require_contains(caddy, "path /api/v1/write", "Prometheus remote-write route")
    require_contains(caddy, "path /loki/api/v1/push", "Loki push route")
    require_contains(caddy, "{$METRICS_USERNAME}", "Prometheus remote-write auth")
    require_contains(caddy, "{$METRICS_PASSWORD_HASH}", "Prometheus remote-write auth")
    require_contains(caddy, "{$LOGS_USERNAME}", "Loki auth")
    require_contains(caddy, "{$LOGS_PASSWORD_HASH}", "Loki auth")
    require_contains(caddy, "reverse_proxy 127.0.0.1:9090", "Prometheus route")
    require_contains(caddy, "reverse_proxy 127.0.0.1:3100", "Loki route")
    require_contains(caddy, 'respond "Not found" 404', "default ingest response")
    require(
        "reverse_proxy prometheus:" not in caddy,
        "Caddyfile must not use Compose service DNS",
    )
    require(
        "reverse_proxy loki:" not in caddy, "Caddyfile must not use Compose service DNS"
    )


def check_prometheus() -> None:
    prometheus = read(UBUNTU / "prometheus.yml")
    blackbox = read(UBUNTU / "blackbox.yml")
    for job in [
        "finite.computer",
        "chat.finite.computer",
        "brain.finite.computer",
        "crm.finite.computer",
        "finitechat-native-mockup.finite.chat",
        "uptime-probe.docs.finite.chat",
        "finite-tinfoil-collector",
    ]:
        require_contains(prometheus, f"job_name: {job}", "Prometheus public probes")
    require_contains(
        prometheus, "targets: [127.0.0.1:9100]", "Tinfoil textfile scrape target"
    )
    require_contains(prometheus, "regex: finite_tinfoil_.*", "Tinfoil scrape keep-list")
    require_contains(
        prometheus, "replacement: 127.0.0.1:9115", "Prometheus blackbox target"
    )
    require_contains(
        prometheus,
        "targets: [https://chat.finite.computer/readyz]",
        "Chat semantic readiness target",
    )
    require_contains(prometheus, "module: [chat_ready]", "Chat readiness probe module")
    require_contains(prometheus, "scrape_interval: 1m", "Chat readiness cadence")
    require_contains(blackbox, "chat_ready:", "Chat readiness probe module")
    require_contains(blackbox, "timeout: 1500ms", "Chat readiness latency budget")
    require(
        "blackbox-exporter:9115" not in prometheus,
        "Prometheus must not use Compose service DNS",
    )


def check_loki() -> None:
    loki = read(UBUNTU / "loki.yml")
    require_contains(loki, "auth_enabled: false", "Loki config")
    require_contains(loki, "http_listen_address: 127.0.0.1", "Loki config")
    require_contains(loki, "http_listen_port: 3100", "Loki config")
    require_contains(loki, "retention_period: 336h", "Loki config")
    require_contains(loki, "allow_structured_metadata: false", "Loki config")
    require_contains(loki, "delete_request_store: filesystem", "Loki config")


def check_grafana() -> None:
    grafana = read(UBUNTU / "grafana/grafana.ini")
    require_contains(grafana, "http_addr = 127.0.0.1", "Grafana config")
    require_contains(
        grafana,
        "admin_password = $__file{/etc/finite/monitoring/grafana-admin-password}",
        "Grafana config",
    )
    require_contains(
        grafana,
        "secret_key = $__file{/etc/finite/monitoring/grafana-secret-key}",
        "Grafana config",
    )
    require_contains(grafana, "allow_sign_up = false", "Grafana config")

    datasources = read(UBUNTU / "grafana/provisioning/datasources/finite.yml")
    require_contains(datasources, "uid: finite-prometheus", "Grafana datasource")
    require_contains(datasources, "uid: finite-loki", "Grafana datasource")
    require_contains(
        datasources, "url: http://127.0.0.1:9090", "Grafana Prometheus datasource"
    )
    require_contains(
        datasources, "url: http://127.0.0.1:3100", "Grafana Loki datasource"
    )
    require(
        "url: http://prometheus:9090" not in datasources,
        "Grafana must not use Compose service DNS",
    )

    dashboards = read(UBUNTU / "grafana/provisioning/dashboards/finite.yml")
    require_contains(
        dashboards,
        "path: /var/lib/finite-monitoring/grafana/dashboards",
        "Grafana dashboard provider",
    )


def check_systemd() -> None:
    expected_units = {
        "finite-monitoring-blackbox-exporter.service": [
            "User=finite-monitoring",
            "--web.listen-address=127.0.0.1:9115",
        ],
        "finite-monitoring-prometheus.service": [
            "User=finite-monitoring",
            "--web.listen-address=127.0.0.1:9090",
            "--web.enable-remote-write-receiver",
            "--storage.tsdb.retention.time=15d",
            "--storage.tsdb.retention.size=20GB",
        ],
        "finite-monitoring-loki.service": [
            "User=finite-monitoring",
            "/opt/finite-monitoring/bin/loki -config.file=/etc/finite/monitoring/loki.yml",
        ],
        "finite-monitoring-grafana.service": [
            "User=finite-monitoring",
            "--config=/etc/finite/monitoring/grafana/grafana.ini",
            "GF_PATHS_PROVISIONING=/etc/finite/monitoring/grafana/provisioning",
        ],
        "finite-monitoring-caddy.service": [
            "User=finite-monitoring",
            "EnvironmentFile=/etc/finite/monitoring/caddy.env",
            "RuntimeDirectory=finite-monitoring-caddy",
            "ExecStart=/opt/finite-monitoring/bin/caddy run --config /etc/finite/monitoring/Caddyfile --adapter caddyfile",
            "ExecReload=/opt/finite-monitoring/bin/caddy reload --config /etc/finite/monitoring/Caddyfile --adapter caddyfile --address unix//run/finite-monitoring-caddy/admin.sock --force",
            "CAP_NET_BIND_SERVICE",
        ],
        "finite-monitoring-node-exporter.service": [
            "User=finite-monitoring",
            "ExecStart=/opt/finite-monitoring/bin/node_exporter --web.listen-address=127.0.0.1:9100",
            "--collector.disable-defaults",
            "--collector.textfile",
            "--collector.textfile.directory=/var/lib/finite-monitoring/tinfoil/textfile",
        ],
        "finite-monitoring-tinfoil-collector.service": [
            "User=finite-monitoring",
            "Environment=PATH=/opt/finite-monitoring/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin",
            "EnvironmentFile=/etc/finite/monitoring/tinfoil.env",
            "ExecStart=/opt/finite-monitoring/bin/tinfoil-usage-collector",
            "TimeoutStartSec=90",
        ],
        "finite-monitoring-tinfoil-collector.timer": [
            "OnUnitActiveSec=60s",
            "Unit=finite-monitoring-tinfoil-collector.service",
        ],
    }
    oneshot_units = {"finite-monitoring-tinfoil-collector.service"}
    for unit_name, needles in expected_units.items():
        unit = read(UBUNTU / "systemd" / unit_name)
        if unit_name in oneshot_units:
            # Type=oneshot rejects Restart=; the timer owns the cadence.
            require_contains(unit, "Type=oneshot", unit_name)
            require(
                "Restart=" not in unit,
                "oneshot collector must not set Restart=",
            )
            require_contains(unit, "NoNewPrivileges=true", unit_name)
        elif not unit_name.endswith(".timer"):
            require_contains(unit, "Restart=on-failure", unit_name)
            require_contains(unit, "NoNewPrivileges=true", unit_name)
        if unit_name == "finite-monitoring-caddy.service":
            require(
                "--envfile" not in unit,
                "Caddy reload must use systemd EnvironmentFile, not unsupported --envfile",
            )
        for needle in needles:
            require_contains(unit, needle, unit_name)


def check_deploy_script() -> None:
    deploy = read(DEPLOY)
    require_contains(deploy, "--replace-compose", "deploy script")
    require_contains(deploy, "docker compose", "deploy script")
    require_contains(deploy, "finite-monitoring-caddy.service", "deploy script")
    require_contains(deploy, "finite-monitoring-loki.service", "deploy script")
    require_contains(deploy, "grafana-admin-password", "deploy script")
    require_contains(deploy, "grafana-secret-key", "deploy script")
    require_contains(deploy, "METRICS_PASSWORD_HASH", "deploy script")
    require_contains(deploy, "LOGS_PASSWORD_HASH", "deploy script")
    require_contains(deploy, '"${algorithm}sum" -c', "deploy script")
    require_contains(
        deploy, '"${GRAFANA_URL}" sha256 "${GRAFANA_SHA256}"', "deploy script"
    )
    require_contains(deploy, '"${CADDY_URL}" sha512 "${CADDY_SHA512}"', "deploy script")
    require_contains(
        deploy,
        '"${NODE_EXPORTER_URL}" sha256 "${NODE_EXPORTER_SHA256}"',
        "deploy script",
    )
    require_contains(deploy, '"${JQ_URL}" sha256 "${JQ_SHA256}"', "deploy script")
    require_contains(
        deploy, "tinfoil-usage-collector", "deploy script Tinfoil collector"
    )
    require_contains(
        deploy, "/etc/finite/monitoring/tinfoil.env", "deploy script Tinfoil env file"
    )
    require_contains(
        deploy,
        "finite-monitoring-tinfoil-collector.timer",
        "deploy script Tinfoil timer",
    )
    require_contains(
        deploy,
        "tinfoil/textfile/tinfoil.prom",
        "deploy script must require a textfile after the first cycle",
    )


def check_docs() -> None:
    readme = read(README)
    require_contains(readme, "Ubuntu/systemd", "monitoring README")
    require_contains(readme, "No Docker Compose", "monitoring README")
    require_contains(
        readme, "infra/monitoring/ubuntu/deploy --replace-compose", "monitoring README"
    )
    require_contains(readme, "/etc/finite/monitoring/caddy.env", "monitoring README")


def main() -> int:
    for relative in EXPECTED_FILES:
        require((UBUNTU / relative).is_file(), f"missing {relative}")
    require(
        not (UBUNTU / "compose.yaml").exists(),
        "Ubuntu monitoring path must not include Compose",
    )

    check_versions()
    check_caddy()
    check_prometheus()
    check_loki()
    check_grafana()
    check_systemd()
    check_deploy_script()
    check_docs()

    print("monitoring Ubuntu contract OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

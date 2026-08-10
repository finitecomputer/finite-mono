#!/usr/bin/env python3
"""Verify lat1's aggregate healthcheck startup and failure contract."""

from __future__ import annotations

import json
import os
import re
import subprocess
import tempfile
from pathlib import Path

import finite_status

ROOT = Path(__file__).resolve().parents[1]
CONFIG = ".#nixosConfigurations.finite-lat-1.config"
PROBED_SERVICE_UNITS = [
    "finite-saas-core.service",
    "podman-finite-saas-dashboard.service",
    "finitechat-server.service",
    "finitechat-hosted-device.service",
    "finite-brain-app.service",
    "finite-saas-sites.service",
    "podman-searxng.service",
    "podman-firecrawl-api.service",
    "prometheus-node-exporter.service",
]
HEALTH_PROBES = tuple(finite_status.CONTRACT["healthcheck"]["probes"])
METRICS_DIRECTORY = "/run/finite-monitoring"
METRICS_ENVIRONMENT_FILE = "/etc/finite/grafana-cloud-metrics.env"


def nix_eval(attribute: str, *, raw: bool = False) -> str:
    command = ["nix", "eval", "--raw" if raw else "--json", f"{CONFIG}.{attribute}"]
    return subprocess.run(
        command,
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout


def run_synthetic(
    script: str, *, recover_after_first_pass: bool
) -> tuple[subprocess.CompletedProcess[str], str, list[str], int]:
    curl_body = '[ "$curl_calls" -gt 9 ]' if recover_after_first_pass else "return 1"
    harness = f"""
curl_calls=0
curl() {{
  curl_calls=$((curl_calls + 1))
  printf '%s\n' "$curl_calls" > "$CURL_COUNT_FILE"
  {curl_body}
}}
sleep() {{ :; }}
eval "$SCRIPT"
"""
    with tempfile.TemporaryDirectory() as metrics_directory:
        curl_count_path = Path(metrics_directory) / "curl-count"
        result = subprocess.run(
            ["bash", "-c", harness],
            check=False,
            capture_output=True,
            env={
                **os.environ,
                "CURL_COUNT_FILE": str(curl_count_path),
                "FINITE_HEALTHCHECK_METRICS_DIRECTORY": metrics_directory,
                "SCRIPT": script,
            },
            text=True,
        )
        metrics_path = Path(metrics_directory) / "finite-healthcheck.prom"
        metrics = (
            metrics_path.read_text(encoding="utf-8") if metrics_path.exists() else ""
        )
        curl_count = int(curl_count_path.read_text(encoding="utf-8"))
        leftovers = sorted(
            path.name
            for path in Path(metrics_directory).iterdir()
            if path not in {curl_count_path, metrics_path}
        )
    return result, metrics, leftovers, curl_count


def verify_metrics(metrics: str, expected_status: int) -> None:
    for expected in (
        "# HELP finite_healthcheck_success ",
        "# TYPE finite_healthcheck_success gauge",
        "# HELP finite_service_health_status ",
        "# TYPE finite_service_health_status gauge",
    ):
        if expected not in metrics:
            raise SystemExit(f"health metrics are missing {expected!r}:\n{metrics}")

    aggregate = re.search(
        r'^finite_healthcheck_success\{host="finite-lat-1"\} ([01])$',
        metrics,
        re.MULTILINE,
    )
    if aggregate is None or int(aggregate.group(1)) != expected_status:
        raise SystemExit(
            f"aggregate health metric is not {expected_status}:\n{metrics}"
        )

    service_pattern = re.compile(
        r'^finite_service_health_status\{host="finite-lat-1",service="([^"]+)"\} ([01])$',
        re.MULTILINE,
    )
    services = {name: int(status) for name, status in service_pattern.findall(metrics)}
    expected = {name: expected_status for name in HEALTH_PROBES}
    if services != expected:
        raise SystemExit(
            f"per-service health metrics differ from the finite-status contract:\n"
            f"expected={expected}\nactual={services}\n{metrics}"
        )


def main() -> None:
    after = json.loads(nix_eval("systemd.services.finite-healthcheck.after"))
    missing = sorted(set(PROBED_SERVICE_UNITS) - set(after))
    if missing:
        raise SystemExit(f"healthcheck ordering is missing units: {missing}")

    script = nix_eval("systemd.services.finite-healthcheck.script", raw=True)
    timeout = nix_eval(
        "systemd.services.finite-healthcheck.serviceConfig.TimeoutStartSec",
        raw=True,
    )
    if timeout != "2min":
        raise SystemExit(f"healthcheck start timeout is {timeout!r}, expected '2min'")

    collectors = json.loads(
        nix_eval("services.prometheus.exporters.node.enabledCollectors")
    )
    flags = json.loads(nix_eval("services.prometheus.exporters.node.extraFlags"))
    if "textfile" not in collectors:
        raise SystemExit("node-exporter textfile collector is not enabled")
    expected_flag = f"--collector.textfile.directory={METRICS_DIRECTORY}"
    if expected_flag not in flags:
        raise SystemExit(f"node-exporter is missing {expected_flag!r}")

    if json.loads(nix_eval("services.alloy.enable")) is not True:
        raise SystemExit("Grafana Alloy is not enabled")
    environment_file = nix_eval("services.alloy.environmentFile", raw=True)
    if environment_file != METRICS_ENVIRONMENT_FILE:
        raise SystemExit(
            f"Alloy environment file is {environment_file!r}, "
            f"expected {METRICS_ENVIRONMENT_FILE!r}"
        )
    alloy_config = nix_eval('environment.etc."alloy/config.alloy".text', raw=True)
    for expected in (
        '"instance"    = "finite-lat-1"',
        '"job"         = "finite-internal-health"',
        "finite_component_build_info",
        "finite_component_version_mismatch",
        "finite_healthcheck_success",
        "finite_runtime_artifact_info",
        "finite_service_health_status",
        "node_textfile_mtime_seconds",
        "node_textfile_scrape_error",
        'sys.env("GRAFANA_CLOUD_PROMETHEUS_URL")',
        'sys.env("GRAFANA_CLOUD_PROMETHEUS_USERNAME")',
        'sys.env("GRAFANA_CLOUD_PROMETHEUS_PASSWORD")',
    ):
        if expected not in alloy_config:
            raise SystemExit(f"Alloy config is missing {expected!r}")

    recovery, recovery_metrics, recovery_leftovers, recovery_curl_count = run_synthetic(
        script, recover_after_first_pass=True
    )
    recovery_output = recovery.stdout + recovery.stderr
    if recovery.returncode != 0:
        raise SystemExit(f"transient failure did not recover:\n{recovery_output}")
    for expected in (
        "WAIT finite-saas-core",
        "OK   node-exporter",
    ):
        if expected not in recovery_output:
            raise SystemExit(
                f"transient recovery output is missing {expected!r}:\n{recovery_output}"
            )
    if recovery_curl_count != 18:
        raise SystemExit(
            f"transient recovery made {recovery_curl_count} curl calls, expected 18"
        )
    verify_metrics(recovery_metrics, 1)
    if recovery_leftovers:
        raise SystemExit(
            f"healthcheck left temporary metric files: {recovery_leftovers}"
        )

    failure, failure_metrics, failure_leftovers, failure_curl_count = run_synthetic(
        script, recover_after_first_pass=False
    )
    failure_output = failure.stdout + failure.stderr
    if failure.returncode != 1:
        raise SystemExit(
            f"persistent failure exited {failure.returncode}, expected 1:\n"
            f"{failure_output}"
        )
    for expected in (
        "FAIL health endpoints remained unavailable after the bounded startup grace",
    ):
        if expected not in failure_output:
            raise SystemExit(
                f"persistent failure output is missing {expected!r}:\n{failure_output}"
            )
    if failure_curl_count != 117:
        raise SystemExit(
            f"persistent failure made {failure_curl_count} curl calls, expected 117"
        )
    verify_metrics(failure_metrics, 0)
    if failure_leftovers:
        raise SystemExit(
            f"healthcheck left temporary metric files: {failure_leftovers}"
        )

    print("lat1 healthcheck contract: ok")


if __name__ == "__main__":
    main()

"""Read-only Tinfoil monitoring evidence, invoked by scripts/finite-status --tinfoil."""

import datetime
import json
import math
import urllib.parse
import urllib.request

SELECTOR = '{container="finite-private",job="finite-tinfoil-collector"}'
USAGE = (
    "gpu_utilization_percent",
    "gpu_memory_utilization_percent",
    "cpu_utilization_percent",
    "host_memory_utilization_percent",
)


def query(expression):
    url = "http://127.0.0.1:9090/api/v1/query?" + urllib.parse.urlencode(
        {"query": expression}
    )
    with urllib.request.urlopen(url, timeout=5) as response:
        payload = json.load(response)
    if payload.get("status") != "success" or payload["data"]["resultType"] != "vector":
        raise ValueError("invalid Prometheus query response")
    rows = payload["data"]["result"]
    if len(rows) != 1:
        raise ValueError("expected exactly one series")
    value = float(rows[0]["value"][1])
    if not math.isfinite(value):
        raise ValueError("nonfinite sample")
    return value


def collect(fetch=query):
    checks = {}
    expressions = {
        "status_sample_age_seconds": (
            f"time() - finite_tinfoil_status_sample_timestamp_seconds{SELECTOR}",
            lambda v: 0 <= v < 300,
        ),
        "usage_sample_age_seconds": (
            f"time() - finite_tinfoil_source_sample_timestamp_seconds{SELECTOR}",
            lambda v: 0 <= v < 300,
        ),
        "container_ready": (
            f"finite_tinfoil_container_ready{SELECTOR}",
            lambda v: v == 1,
        ),
        "allocated_gpus": (
            f"finite_tinfoil_container_gpus{SELECTOR}",
            lambda v: v > 0 and v.is_integer(),
        ),
    }
    for component in ("upstream", "usage_api"):
        selector = SELECTOR[:-1] + f',component="{component}"' + "}"
        expressions[component + "_ready"] = (
            f"finite_tinfoil_component_ready{selector}",
            lambda v: v == 1,
        )
        expressions[component + "_latency_seconds"] = (
            f"finite_tinfoil_component_probe_duration_seconds{selector}",
            lambda v: v >= 0,
        )
    for key in USAGE:
        selector = SELECTOR[:-1] + ',aggregation="mean"}'
        expressions[key] = (f"finite_tinfoil_{key}{selector}", lambda v: 0 <= v <= 100)
    for name, (expression, valid) in expressions.items():
        try:
            value = fetch(expression)
            checks[name] = {
                "status": "green" if valid(value) else "red",
                "value": value,
            }
        except (OSError, ValueError, KeyError, TypeError):
            checks[name] = {"status": "unknown"}
    states = {entry["status"] for entry in checks.values()}
    status = "red" if "red" in states else "unknown" if "unknown" in states else "green"
    return {
        "schema_version": "finite.status.v1",
        "generated_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "overall_status": status,
        "exit_code": {"green": 0, "red": 1, "unknown": 2}[status],
        "sections": {"tinfoil_monitoring": {"status": status, "checks": checks}},
    }

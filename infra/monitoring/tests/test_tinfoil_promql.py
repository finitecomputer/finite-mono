"""Evaluate actual Tinfoil panel queries, including NaN and stalled collectors."""

import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]


def main():
    panels = {
        p["title"]: p
        for p in json.loads(
            (
                ROOT / "infra/monitoring/grafana/dashboards/finite-tinfoil-gpu.json"
            ).read_text()
        )["panels"]
    }
    tests = []
    for name, timestamp, status, expected, visible in [
        ("fresh", "1140", "1200", 0, True),
        ("aging", "900", "1200", 1, True),
        ("stale usage with healthy status", "600", "1200", 2, False),
        ("future", "1201", "1200", 2, False),
        ("NaN", "NaN", "1200", 2, False),
        ("missing", None, "1200", 2, False),
        ("stopped collector with cached textfile", "1140", "900", 0, False),
    ]:
        series = [
            {
                "series": 'finite_tinfoil_status_sample_timestamp_seconds{container="finite-private"}',
                "values": f"{status}x20",
            },
            {
                "series": 'finite_tinfoil_gpu_utilization_percent{container="finite-private",aggregation="mean"}',
                "values": "0x20",
            },
        ]
        if timestamp is not None:
            series.append(
                {
                    "series": 'finite_tinfoil_source_sample_timestamp_seconds{container="finite-private"}',
                    "values": " ".join([timestamp] * 21),
                }
            )
        labels = (
            '{container="finite-private"}'
            if timestamp not in (None, "NaN", "1201")
            else "{}"
        )
        tests.append(
            {
                "name": name,
                "interval": "1m",
                "input_series": series,
                "promql_expr_test": [
                    {
                        "expr": panels["Data Freshness"]["targets"][0]["expr"],
                        "eval_time": "20m",
                        "exp_samples": [{"labels": labels, "value": expected}],
                    },
                    {
                        "expr": panels["GPU Utilization"]["targets"][0]["expr"],
                        "eval_time": "20m",
                        "exp_samples": [
                            {
                                "labels": 'finite_tinfoil_gpu_utilization_percent{container="finite-private",aggregation="mean"}',
                                "value": 0,
                            }
                        ]
                        if visible
                        else [],
                    },
                ],
            }
        )
    with tempfile.TemporaryDirectory() as directory:
        fixture = Path(directory) / "tinfoil.yml"
        fixture.write_text(json.dumps({"rule_files": [], "tests": tests}))
        subprocess.run(["promtool", "test", "rules", str(fixture)], check=True)


if __name__ == "__main__":
    main()

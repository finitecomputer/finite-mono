"""Evaluate Sites dashboard queries against scrape failures and real zero counts."""

import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]
DASHBOARD = ROOT / "infra/monitoring/grafana/dashboards/finite-production-mvp.json"
SOURCE = 'job="finite-sites-metrics",instance="finite.site"'


def main():
    panels = {
        p["id"]: p
        for p in json.loads(DASHBOARD.read_text())["panels"]
        if p["id"] in (27, 28, 29, 30)
    }
    assert set(panels) == {27, 28, 29, 30}
    daily = panels[30]
    assert daily["type"] == "barchart"
    assert daily["targets"][0]["format"] == "table"
    assert daily["options"]["xField"] == "UTC date"
    assert daily["transformations"][2] == {
        "id": "convertFieldType",
        "options": {
            "conversions": [
                {
                    "targetField": "UTC date",
                    "destinationType": "time",
                    "dateFormat": "YYYY-MM-DD",
                }
            ]
        },
    }
    assert daily["transformations"][0]["options"]["renameByName"]["date"] == "UTC date"
    assert daily["transformations"][1]["options"]["sort"] == [
        {"field": "UTC date", "desc": False}
    ]
    for panel in panels.values():
        assert panel["targets"][0]["instant"] is True
        assert panel["targets"][0]["range"] is False
    tests = []
    for name, up, timestamp, counts, available in [
        ("healthy existing history", 1, 1200, (5, 3, 2), True),
        ("empty is a real zero", 1, 1200, (0, 0, 0), True),
        ("scrape failed after earlier success", 0, 1200, (5, 3, 2), False),
        ("collector stalled at age boundary", 1, 1020, (5, 3, 2), False),
        ("just before age boundary", 1, 1021, (5, 3, 2), True),
        ("future snapshot", 1, 1201, (5, 3, 2), False),
        ("missing snapshot", 1, None, (5, 3, 2), False),
        ("missing scrape job", None, 1200, (5, 3, 2), False),
    ]:
        existing, published, created = counts
        series = [
            {
                "series": f"finite_sites_existing{{{SOURCE}}}",
                "values": f"{existing}x20",
            },
            {
                "series": f"finite_sites_published{{{SOURCE}}}",
                "values": f"{published}x20",
            },
            {
                "series": f'finite_sites_created_by_day{{{SOURCE},date="2026-09-15"}}',
                "values": "0x20",
            },
            {
                "series": f'finite_sites_created_by_day{{{SOURCE},date="2026-09-16"}}',
                "values": f"{created}x20",
            },
            {
                "series": 'finite_sites_existing{job="other",instance="finite.site"}',
                "values": "999x20",
            },
        ]
        if up is not None:
            series.append({"series": f"up{{{SOURCE}}}", "values": f"{up}x20"})
        if timestamp is not None:
            series.append(
                {
                    "series": f"finite_sites_metrics_collected_at_seconds{{{SOURCE}}}",
                    "values": f"{timestamp}x20",
                }
            )
        expected = {
            27: [{"labels": "{}", "value": existing}] if available else [],
            28: [{"labels": "{}", "value": published}] if available else [],
            29: [{"labels": "{}", "value": int(available)}],
            30: [
                {"labels": '{date="2026-09-15"}', "value": 0},
                {"labels": '{date="2026-09-16"}', "value": created},
            ]
            if available
            else [],
        }
        tests.append(
            {
                "name": name,
                "interval": "1m",
                "input_series": series,
                "promql_expr_test": [
                    {
                        "expr": p["targets"][0]["expr"],
                        "eval_time": "20m",
                        "exp_samples": expected[id],
                    }
                    for id, p in panels.items()
                ],
            }
        )
    with tempfile.TemporaryDirectory() as directory:
        # Validate the actual receiver YAML with a disposable credential file;
        # production secrets are never needed by local/CI checks.
        token = Path(directory) / "token"
        token.write_text("a" * 64)
        config = Path(directory) / "prometheus.yml"
        config.write_text(
            (ROOT / "infra/monitoring/ubuntu/prometheus.yml")
            .read_text()
            .replace("/etc/finite/monitoring/sites-metrics-token", str(token))
        )
        subprocess.run(["promtool", "check", "config", str(config)], check=True)
        fixture = Path(directory) / "sites-metrics.yml"
        fixture.write_text(json.dumps({"rule_files": [], "tests": tests}))
        subprocess.run(["promtool", "test", "rules", str(fixture)], check=True)


if __name__ == "__main__":
    main()

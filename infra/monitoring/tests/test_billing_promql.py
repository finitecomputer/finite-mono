"""Evaluate the shipped billing panels, including outages and real zeroes."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT), str(ROOT / "infra/nixos/scripts")]
import finite_billing_metrics as billing  # noqa: E402 — source-tree script import

SOURCE = 'instance="finite-lat-2",job="finite-internal-health"'
DASHBOARD = ROOT / "infra/monitoring/grafana/dashboards/finite-billing-accounts.json"


def main():
    dashboard = json.loads(DASHBOARD.read_text())
    assert dashboard["uid"] == "finite-billing-accounts"
    panels = {p["id"]: p for p in dashboard["panels"] if p["type"] != "text"}
    assert set(panels) == {2, 3, 4, 5, 6, 8}
    for id, panel in panels.items():
        assert panel["datasource"]["uid"] == "finite-prometheus"
        assert len(panel["targets"]) == 1
        assert panel["targets"][0]["instant"] == (id != 8)
        assert panel["targets"][0]["range"] == (id == 8)
    assert panels[8]["fieldConfig"]["defaults"]["custom"]["spanNulls"] is False
    assert (
        panels[5]["fieldConfig"]["defaults"]["mappings"][0]["options"]["0"]["text"]
        == "UNAVAILABLE"
    )
    tests = []
    for name, up, success, stamp, empty, available in [
        ("healthy", 1, 1, 1200, False, True),
        ("empty database", 1, 1, 1200, True, True),
        ("scrape failed", 0, 1, 1200, False, False),
        ("scrape missing", None, 1, 1200, False, False),
        ("query failed", 1, 0, 1200, False, False),
        ("query status missing", 1, None, 1200, False, False),
        ("timestamp missing", 1, 1, None, False, False),
        ("stalled writer at boundary", 1, 1, 600, False, False),
        ("one second before stale boundary", 1, 1, 601, False, True),
        ("future timestamp", 1, 1, 1201, False, False),
    ]:
        counts = {
            status: 0 if empty else i + 1 for i, status in enumerate(billing.STATUSES)
        }
        series = [
            {
                "series": f'finite_billing_accounts{{{SOURCE},status="{status}"}}',
                "values": f"{value}x20",
            }
            for status, value in counts.items()
        ]
        for metric, value in (
            ("up", up),
            (billing.SUCCESS, success),
            ("finite_billing_collected_at_seconds", stamp),
        ):
            if value is not None:
                series.append(
                    {"series": f"{metric}{{{SOURCE}}}", "values": f"{value}x20"}
                )
        # Old app host / unrelated jobs must not mask a failed authoritative source.
        for wrong in (
            SOURCE.replace("lat-2", "lat-1"),
            SOURCE.replace("finite-internal-health", "other"),
        ):
            for metric, value in (
                ("up", 1),
                (billing.SUCCESS, 1),
                ("finite_billing_collected_at_seconds", 1200),
            ):
                series.append(
                    {"series": f"{metric}{{{wrong}}}", "values": f"{value}x20"}
                )
            series.append(
                {
                    "series": f'finite_billing_accounts{{{wrong},status="subscribed"}}',
                    "values": "999x20",
                }
            )
        grouped = [
            {"labels": f'{{status="{status}"}}', "value": count}
            for status, count in counts.items()
        ]
        expected = {
            2: [{"labels": "{}", "value": sum(counts.values())}],
            3: [{"labels": "{}", "value": counts["subscribed"]}],
            4: [
                {
                    "labels": "{}",
                    "value": counts["unknown"] + counts["missing_billing_account"],
                }
            ],
            6: grouped,
            8: grouped,
        }
        tests.append(
            {
                "name": name,
                "interval": "1m",
                "input_series": series,
                "promql_expr_test": [
                    {
                        "expr": panel["targets"][0]["expr"],
                        "eval_time": "20m",
                        "exp_samples": (
                            [{"labels": "{}", "value": int(available)}]
                            if id == 5
                            else expected[id]
                            if available
                            else []
                        ),
                    }
                    for id, panel in panels.items()
                ],
            }
        )
    with tempfile.TemporaryDirectory() as directory:
        fixture = Path(directory) / "billing.yml"
        fixture.write_text(json.dumps({"rule_files": [], "tests": tests}))
        subprocess.run(["promtool", "test", "rules", str(fixture)], check=True)
        subprocess.run(
            ["promtool", "check", "metrics"],
            input=billing.render({"collected_at": 123, "counts": {}}),
            text=True,
            check=True,
        )


if __name__ == "__main__":
    main()

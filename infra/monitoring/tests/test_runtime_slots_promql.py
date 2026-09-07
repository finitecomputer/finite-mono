"""Evaluate the deployed dashboard queries with the flake-pinned promtool."""

import json
import runpy
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
DASHBOARD = ROOT / "infra/monitoring/grafana/dashboards/finite-agent-runtime-slots.json"
SOURCE = 'instance="finite-lat-2",job="finite-internal-health"'
MTIME = f'node_textfile_mtime_seconds{{{SOURCE},file="/run/finite-monitoring/finite-runtime.prom"}}'
CAPACITY = runpy.run_path(str(ROOT / "scripts/check_runner_host_contract.py"))[
    "EXPECTED_MAX_SANDBOXES"
]
# Bind stable panel IDs, not editable titles, to their expected meaning.
PANELS = {
    2: ("finite-lat-3", False),
    3: ("finite-lat-3", True),
    4: ("finite-lat-4", False),
    5: ("finite-lat-4", True),
    6: (None, False),
    7: (None, True),
}


def runtime(host, artifact, count, source=SOURCE):
    return {
        "series": f'finite_runtime_artifact_active_agents{{{source},source_host_id="{host}",artifact_id="{artifact}"}}',
        "values": f"{count}x20",
    }


def samples(panel_id, counts, mtime):
    if panel_id == 8:
        return [] if mtime is None else [{"labels": "{}", "value": 1200 - mtime}]
    host, estimate = PANELS[panel_id]
    return [
        {
            "labels": "{}" if host else f'{{source_host_id="{name}"}}',
            "value": int(CAPACITY[name]) - count if estimate else count,
        }
        for name, count in counts.items()
        if host is None or host == name
    ]


def main():
    panels = [
        p for p in json.loads(DASHBOARD.read_text())["panels"] if p["type"] != "text"
    ]
    assert all(len(p["targets"]) == 1 for p in panels), "unverified query target"
    queries = {p["id"]: p["targets"][0]["expr"] for p in panels}
    assert queries.keys() == PANELS.keys() | {8}, "unverified query panel"
    active = [
        runtime("finite-lat-3", "old", 12),
        runtime("finite-lat-3", "new", 5),
        runtime("finite-lat-4", "new", 42),
    ]
    counts = {"finite-lat-3": 17, "finite-lat-4": 42}
    cases = [
        ("fresh mixed artifacts", 900, active, counts),
        ("just before expiry", 601, active, counts),
        ("collector stalled; scraping continues", 600, active, {}),
        ("missing file age", None, active, {}),
        ("future file age", 1201, active, {}),
        ("empty host", 900, [], {}),
        ("one host missing", 900, active[2:], {"finite-lat-4": 42}),
        (
            "over ceiling",
            900,
            [runtime("finite-lat-3", "new", 43)],
            {"finite-lat-3": 43},
        ),
    ]
    tests = []
    for name, mtime, inputs, expected in cases:
        # Re-scrape a fixed file mtime for 20 minutes: the existing writer
        # retains its file on failure. A fresh retired writer must not mask it.
        retired = 'instance="finite-lat-1",job="finite-internal-health"'
        series = inputs + [
            runtime("finite-lat-3", "retired", 100, retired),
            runtime("finite-lat-1", "other-host", 100),
            runtime(
                "finite-lat-3",
                "other-job",
                100,
                SOURCE.replace("finite-internal-health", "other"),
            ),
            {"series": MTIME.replace(SOURCE, retired), "values": "1200x20"},
            {
                "series": MTIME.replace("finite-runtime.prom", "other.prom"),
                "values": "1200x20",
            },
        ]
        if mtime is not None:
            series.append({"series": MTIME, "values": f"{mtime}x20"})
        tests.append(
            {
                "name": name,
                "interval": "1m",
                "input_series": series,
                "promql_expr_test": [
                    {
                        "expr": expr,
                        "eval_time": "20m",
                        "exp_samples": samples(panel_id, expected, mtime),
                    }
                    for panel_id, expr in queries.items()
                ],
            }
        )
    with tempfile.TemporaryDirectory() as directory:
        fixture = Path(directory) / "runtime-slots.yml"
        fixture.write_text(json.dumps({"rule_files": [], "tests": tests}))
        subprocess.run(["promtool", "test", "rules", str(fixture)], check=True)


if __name__ == "__main__":
    main()

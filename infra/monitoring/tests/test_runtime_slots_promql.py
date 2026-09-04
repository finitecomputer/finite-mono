"""Evaluate the shipped dashboard queries with Prometheus, including old writers.

Run via `just monitoring-nixos-contract` for the flake-pinned promtool.
"""

from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
DASHBOARD = ROOT / "infra/monitoring/grafana/dashboards/finite-agent-runtime-slots.json"
SOURCE = 'instance="finite-lat-2",job="finite-internal-health"'
MTIME = (
    f"node_textfile_mtime_seconds{{{SOURCE},"
    'file="/run/finite-monitoring/finite-runtime.prom"}'
)


def runtime(host: str, artifact: str, count: int, source: str = SOURCE) -> dict:
    return {
        "series": (
            f"finite_runtime_artifact_active_agents{{{source},"
            f'source_host_id="{host}",artifact_id="{artifact}"' + "}"
        ),
        "values": f"{count}x20",
    }


class RuntimeSlotsPromQLTests(unittest.TestCase):
    def test_queries_against_collection_and_scrape_failures(self) -> None:
        panels = json.loads(DASHBOARD.read_text())["panels"]
        active = [
            runtime("finite-lat-3", "old-artifact", 12),
            runtime("finite-lat-3", "new-artifact", 5),
            runtime("finite-lat-4", "new-artifact", 42),
        ]
        expected = {"finite-lat-3": 17, "finite-lat-4": 42}
        # Every series is scraped afresh for 20 minutes. Keeping a fixed
        # mtime models the existing writer retaining its last file on failure.
        scenarios = [
            (
                "fresh source; multiple artifact versions",
                "900x20",
                active,
                expected,
                300,
            ),
            ("just inside age boundary", "601x20", active, expected, 599),
            ("collector stopped, scrape still healthy", "600x20", active, {}, 600),
            ("collector long stopped", "1x20", active, {}, 1199),
            ("older collector with no mtime", None, active, {}, None),
            ("scraping stopped", "900x14 stale _x5", active, {}, None),
            ("future source clock", "1201x20", active, {}, -1),
            ("no counted runtimes", "900x20", [], {}, 300),
            (
                "one host missing is unknown, not zero",
                "900x20",
                active[2:],
                {"finite-lat-4": 42},
                300,
            ),
            (
                "over ceiling remains visible",
                "900x20",
                [runtime("finite-lat-3", "new", 43)],
                {"finite-lat-3": 43},
                300,
            ),
        ]
        tests = []
        for name, mtime, inputs, counts, age in scenarios:
            # A retired app-plane writer must neither add duplicate counts
            # nor make a stale/missing current source look fresh.
            retired = 'instance="finite-lat-1",job="finite-internal-health"'
            series = [
                *inputs,
                runtime("finite-lat-3", "retired-artifact", 100, retired),
                {"series": MTIME.replace(SOURCE, retired), "values": "1200x20"},
            ]
            if mtime is not None:
                series.append({"series": MTIME, "values": mtime})
            assertions = []
            for panel in panels:
                if panel["type"] == "text":
                    continue
                title = panel["title"]
                samples = []
                if title == "Core Sample Age":
                    if age is not None:
                        samples = [{"labels": "{}", "value": age}]
                else:
                    for host, count in counts.items():
                        is_stat = panel["type"] == "stat"
                        if is_stat and not title.startswith(
                            host.replace("finite-lat-", "lat")
                        ):
                            continue
                        samples.append(
                            {
                                "labels": "{}"
                                if is_stat
                                else f'{{source_host_id="{host}"}}',
                                "value": 42 - count if "Unused" in title else count,
                            }
                        )
                assertions.append(
                    {
                        "expr": panel["targets"][0]["expr"],
                        "eval_time": "20m",
                        "exp_samples": samples,
                    }
                )
            tests.append(
                {
                    "name": name,
                    "interval": "1m",
                    "input_series": series,
                    "promql_expr_test": assertions,
                }
            )

        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory) / "runtime-slots.yml"
            # JSON is a YAML subset; queries come directly from the artifact
            # operators deploy, not a duplicate implementation in the test.
            fixture.write_text(json.dumps({"rule_files": [], "tests": tests}))
            result = subprocess.run(
                ["promtool", "test", "rules", str(fixture)],
                capture_output=True,
                text=True,
                check=False,
            )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()

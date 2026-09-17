"""Exercise complete collector cycles with the observed external JSON contracts."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
COLLECTOR = ROOT / "infra/monitoring/tinfoil/tinfoil-usage-collector"


class CollectorTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.env = dict(
            os.environ,
            PATH=f"{self.root}:{os.environ['PATH']}",
            FINITE_TINFOIL_STATE_DIR=str(self.root / "state"),
            FINITE_TINFOIL_CLI=str(self.root / "tinfoil"),
            FIXTURE=str(self.root),
        )
        for name, script in {
            "date": 'echo "${TEST_NOW:-1789668000}"',
            "tinfoil": """if [ "$2" = get ]; then cat "$FIXTURE/status.json"; else
              test "$2 $3 $4 $5 $6 $7" = "metrics test-container --time 1h --output json" || exit 8
              cat "$FIXTURE/usage.json"
            fi""",
            "curl": '''while [ "$#" -gt 0 ]; do
              if [ "$1" = -o ]; then cp "$FIXTURE/health.json" "$2"; fi
              shift
            done
            printf %s "${TEST_HTTP_CODE:-200}"''',
        }.items():
            p = self.root / name
            p.write_text("#!/bin/sh\nset -eu\n" + script + "\n")
            p.chmod(0o755)
        self.status = {
            "id": "test-container",
            "name": "finite-private",
            "status": "ready",
            "gpus": 8,
            "debug": False,
        }
        self.point = {
            "time": "2026-09-17T18:00:00Z",
            "avg_gpu_util": 42,
            "avg_gpu_mem_util": 86,
            "avg_cpu_util": 23,
            "avg_cpu_mem_util": 18,
        }
        self.usage = {"interval": "2m0s", "data_points": [self.point]}
        self.health = {
            "service": "finite-private-limiter",
            "kind": "ready",
            "checkedAtUnixMs": 1789668000000,
            "components": {
                "upstream": {
                    "name": "upstream",
                    "ok": True,
                    "status": 200,
                    "latencyMs": 10,
                },
                "usageApi": {
                    "name": "usage_api",
                    "ok": True,
                    "status": 200,
                    "authenticated": True,
                    "latencyMs": 120,
                },
            },
        }

    def cycle(self):
        for name in ("status", "usage", "health"):
            (self.root / f"{name}.json").write_text(json.dumps(getattr(self, name)))
        result = subprocess.run(
            ["bash", str(COLLECTOR)], env=self.env, capture_output=True, text=True
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return dict(
            line.rsplit(" ", 1)
            for line in (self.root / "state/textfile/tinfoil.prom")
            .read_text()
            .splitlines()
        )

    def value(self, values, key, labels=""):
        return values[f'finite_tinfoil_{key}{{container="finite-private"{labels}}}']

    def test_real_contract_and_zero_usage(self):
        self.point["avg_gpu_util"] = 0
        v = self.cycle()
        self.assertEqual(len(v), 12)
        self.assertEqual(self.value(v, "container_ready"), "1")
        self.assertEqual(self.value(v, "container_gpus"), "8")
        self.assertEqual(
            self.value(v, "gpu_utilization_percent", ',aggregation="mean"'), "0"
        )
        self.assertEqual(
            self.value(v, "component_probe_duration_seconds", ',component="usage_api"'),
            "0.12",
        )

    def test_usage_failure_does_not_refresh_source_timestamp_or_repeat_values(self):
        first = self.cycle()
        self.env["TEST_NOW"] = "1789668060"
        self.usage = {}
        v = self.cycle()
        self.assertEqual(
            self.value(v, "source_sample_timestamp_seconds"),
            self.value(first, "source_sample_timestamp_seconds"),
        )
        self.assertEqual(self.value(v, "status_sample_timestamp_seconds"), "1789668060")
        self.assertEqual(
            self.value(v, "gpu_utilization_percent", ',aggregation="mean"'), "NaN"
        )

    def test_expired_usage_is_gap(self):
        self.env["TEST_NOW"] = "1789668600"
        v = self.cycle()
        self.assertEqual(self.value(v, "source_sample_timestamp_seconds"), "1789668000")
        self.assertEqual(
            self.value(v, "gpu_utilization_percent", ',aggregation="mean"'), "NaN"
        )

    def test_status_failure_is_unknown_and_does_not_read_usage(self):
        self.cycle()
        self.status = {"ready": True, "name": "finite-private", "gpus": 8}
        v = self.cycle()
        self.assertEqual(self.value(v, "container_ready"), "0")
        self.assertEqual(self.value(v, "status_sample_timestamp_seconds"), "NaN")
        self.assertEqual(self.value(v, "source_sample_timestamp_seconds"), "NaN")

    def test_container_replacement_does_not_reuse_old_sample(self):
        self.cycle()
        self.status["id"] = "replacement"
        v = self.cycle()
        self.assertEqual(self.value(v, "source_sample_timestamp_seconds"), "NaN")

    def test_unsorted_buckets_use_latest_timestamp(self):
        self.usage["data_points"].append(
            dict(self.point, time="2026-09-17T17:58:00Z", avg_gpu_util=99)
        )
        self.assertEqual(
            self.value(self.cycle(), "gpu_utilization_percent", ',aggregation="mean"'),
            "42",
        )

    def test_invalid_usage_rejected(self):
        for field, value in [
            ("avg_gpu_util", 101),
            ("avg_cpu_mem_util", None),
            ("time", "2026-09-17T18:02:00Z"),
        ]:
            with self.subTest(field=field):
                old = self.point[field]
                self.point[field] = value
                self.assertEqual(
                    self.value(
                        self.cycle(), "gpu_utilization_percent", ',aggregation="mean"'
                    ),
                    "NaN",
                )
                self.point[field] = old
        self.usage["data_points"].append(dict(self.point))
        self.assertEqual(
            self.value(self.cycle(), "gpu_utilization_percent", ',aggregation="mean"'),
            "NaN",
        )

    def test_regressed_sample_is_rejected(self):
        self.cycle()
        self.point["time"] = "2026-09-17T17:58:00Z"
        v = self.cycle()
        self.assertEqual(self.value(v, "source_sample_timestamp_seconds"), "1789668000")
        self.assertEqual(
            self.value(v, "gpu_utilization_percent", ',aggregation="mean"'), "NaN"
        )

    def test_503_preserves_independent_dependencies(self):
        self.env["TEST_HTTP_CODE"] = "503"
        self.health["components"]["usageApi"].update(ok=False, status=503)
        v = self.cycle()
        self.assertEqual(self.value(v, "component_ready", ',component="upstream"'), "1")
        self.assertEqual(
            self.value(v, "component_ready", ',component="usage_api"'), "0"
        )

    def test_invalid_or_redirected_health_never_reports_ready(self):
        for response in (200, 302):
            self.env["TEST_HTTP_CODE"] = str(response)
            if response == 200:
                self.health = {"ok": True}
            v = self.cycle()
            self.assertEqual(
                self.value(v, "component_ready", ',component="upstream"'), "0"
            )
            self.assertEqual(
                self.value(
                    v, "component_probe_duration_seconds", ',component="upstream"'
                ),
                "NaN",
            )

    def test_accounting_requires_authentication(self):
        self.health["components"]["usageApi"]["authenticated"] = False
        self.assertEqual(
            self.value(self.cycle(), "component_ready", ',component="usage_api"'), "0"
        )

    def test_changed_bucket_interval_fails_closed(self):
        self.usage["interval"] = "48m0s"
        self.assertEqual(
            self.value(self.cycle(), "source_sample_timestamp_seconds"), "NaN"
        )


if __name__ == "__main__":
    unittest.main()

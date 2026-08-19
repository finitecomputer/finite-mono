"""Unit checks for the Hermes adapter regression evidence report."""

from __future__ import annotations

import importlib.util
import subprocess
import types
import unittest
from pathlib import Path
from typing import Any, cast

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPO_ROOT / "scripts" / "hermes-adapter-regression-report.py"

spec = importlib.util.spec_from_file_location("hermes_adapter_regression_report", SCRIPT_PATH)
if spec is None or spec.loader is None:
    raise RuntimeError(f"failed to load {SCRIPT_PATH}")
adapter_report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(adapter_report)


class AdapterRegressionReportTest(unittest.TestCase):
    @staticmethod
    def _passing_unittest_output() -> str:
        return "\n".join(
            f"{test_name.rsplit('.', 1)[-1]} ({test_name}) ... ok"
            for test_name in adapter_report.flattened_tests()
        )

    def test_build_report_records_required_regression_layers(self) -> None:
        original_run = adapter_report.subprocess.run
        captured: dict[str, Any] = {}

        def fake_run(command, **kwargs):
            captured["command"] = command
            captured["kwargs"] = kwargs
            return types.SimpleNamespace(
                returncode=0,
                stdout="",
                stderr=self._passing_unittest_output(),
            )

        try:
            adapter_report.subprocess.run = fake_run
            status, report = adapter_report.build_report(
                types.SimpleNamespace(python="python3", timeout=30)
            )
        finally:
            adapter_report.subprocess.run = original_run

        self.assertEqual(status, 0)
        self.assertEqual(report["status"], "passed")
        self.assertIn("media attachments", report["proof_layers"])
        self.assertIn("receipt/control stream filtering", report["proof_layers"])
        self.assertIn("group sender identity", report["proof_layers"])
        self.assertIn(
            "restart after route learning preserves reply scope",
            report["proof_layers"],
        )
        self.assertIn(
            "in-flight turn retains inbox ownership until completion",
            report["proof_layers"],
        )
        self.assertIn(
            "restart after processing before ack suppresses duplicate turn",
            report["proof_layers"],
        )
        self.assertEqual(report["test_count"], len(adapter_report.flattened_tests()))
        self.assertEqual(report["observed_test_count"], report["test_count"])
        self.assertEqual(report["missing_tests"], [])
        self.assertEqual(report["skipped_tests"], [])
        command = cast(list[str], captured["command"])
        self.assertEqual(command[:3], ["python3", "-m", "unittest"])
        self.assertIn("-v", command)

    def test_durability_scenarios_record_the_asserted_failure_boundaries(self) -> None:
        original_run = adapter_report.subprocess.run

        def fake_run(command, **kwargs):
            del command, kwargs
            return types.SimpleNamespace(
                returncode=0,
                stdout="",
                stderr=self._passing_unittest_output(),
            )

        try:
            adapter_report.subprocess.run = fake_run
            status, report = adapter_report.build_report(
                types.SimpleNamespace(python="python3", timeout=30)
            )
        finally:
            adapter_report.subprocess.run = original_run

        self.assertEqual(status, 0)
        scenarios = {scenario["name"]: scenario for scenario in report["durability_scenarios"]}
        route = scenarios["restart after route learning preserves reply scope"]
        self.assertEqual(route["status"], "passed")
        self.assertEqual(
            route["asserted_observations"]["route_before_restart"],
            {"conversation_id": "topic-build", "segment_id": "chat-build-1"},
        )
        self.assertEqual(
            route["asserted_observations"]["route_after_restart"],
            {"conversation_id": "topic-build", "segment_id": "chat-build-1"},
        )
        self.assertEqual(route["asserted_observations"]["dispatch_count"], 1)
        self.assertEqual(route["asserted_observations"]["ack_attempt_count"], 1)
        self.assertTrue(route["restart_boundary"])

        cancelled = scenarios["cancelled turn leaves event for redelivery"]
        self.assertEqual(cancelled["asserted_observations"]["dispatch_count"], 2)
        self.assertEqual(cancelled["asserted_observations"]["ack_attempt_count"], 0)
        self.assertEqual(cancelled["asserted_observations"]["turn_completion_count"], 1)

    def test_skipped_or_unobserved_required_tests_fail_the_gate(self) -> None:
        tests = adapter_report.flattened_tests()
        skipped_test = tests[0]
        missing_test = tests[1]
        output = []
        for test_name in tests:
            if test_name == missing_test:
                continue
            result = "skipped 'not available'" if test_name == skipped_test else "ok"
            output.append(f"{test_name.rsplit('.', 1)[-1]} ({test_name}) ... {result}")

        original_run = adapter_report.subprocess.run

        def fake_run(command, **kwargs):
            del command, kwargs
            return types.SimpleNamespace(returncode=0, stdout="", stderr="\n".join(output))

        try:
            adapter_report.subprocess.run = fake_run
            status, report = adapter_report.build_report(
                types.SimpleNamespace(python="python3", timeout=30)
            )
        finally:
            adapter_report.subprocess.run = original_run

        self.assertNotEqual(status, 0)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["skipped_tests"], [skipped_test])
        self.assertEqual(report["missing_tests"], [missing_test])
        skipped_layer = next(
            name
            for name, layer_tests in adapter_report.REQUIRED_REGRESSIONS.items()
            if skipped_test in layer_tests
        )
        self.assertIn(skipped_layer, report["required_proof_layers"])
        self.assertNotIn(skipped_layer, report["proof_layers"])

    def test_interleaved_warning_does_not_hide_a_passing_test_result(self) -> None:
        tests = adapter_report.flattened_tests()
        warned_test = tests[0]
        output = []
        for test_name in tests:
            prefix = f"{test_name.rsplit('.', 1)[-1]} ({test_name}) ... "
            if test_name == warned_test:
                output.extend(
                    [
                        prefix + "/tmp/adapter.py:1: ResourceWarning: unclosed database",
                        "ResourceWarning: Enable tracemalloc to get the object allocation traceback",
                        "ok",
                    ]
                )
            else:
                output.append(prefix + "ok")

        original_run = adapter_report.subprocess.run

        def fake_run(command, **kwargs):
            del command, kwargs
            return types.SimpleNamespace(returncode=0, stdout="", stderr="\n".join(output))

        try:
            adapter_report.subprocess.run = fake_run
            status, report = adapter_report.build_report(
                types.SimpleNamespace(python="python3", timeout=30)
            )
        finally:
            adapter_report.subprocess.run = original_run

        self.assertEqual(status, 0)
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["missing_tests"], [])

    def test_timeout_still_produces_failed_report_evidence(self) -> None:
        original_run = adapter_report.subprocess.run

        def fake_run(command, **kwargs):
            del kwargs
            raise subprocess.TimeoutExpired(command, 30, output="partial", stderr="")

        try:
            adapter_report.subprocess.run = fake_run
            status, report = adapter_report.build_report(
                types.SimpleNamespace(python="python3", timeout=30)
            )
        finally:
            adapter_report.subprocess.run = original_run

        self.assertEqual(status, 124)
        self.assertEqual(report["status"], "failed")
        self.assertTrue(report["timed_out"])
        self.assertEqual(report["observed_test_count"], 0)
        self.assertEqual(report["missing_tests"], adapter_report.flattened_tests())


if __name__ == "__main__":
    unittest.main()

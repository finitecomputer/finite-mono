"""Unit checks for the Hermes adapter regression evidence report."""

from __future__ import annotations

import importlib.util
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
    def test_build_report_records_required_regression_layers(self) -> None:
        original_run = adapter_report.subprocess.run
        captured: dict[str, Any] = {}

        def fake_run(command, **kwargs):
            captured["command"] = command
            captured["kwargs"] = kwargs
            return types.SimpleNamespace(returncode=0, stdout="ok\n", stderr="")

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
        self.assertEqual(report["test_count"], len(adapter_report.flattened_tests()))
        command = cast(list[str], captured["command"])
        self.assertEqual(command[:3], ["python3", "-m", "unittest"])
        self.assertIn("-v", command)


if __name__ == "__main__":
    unittest.main()

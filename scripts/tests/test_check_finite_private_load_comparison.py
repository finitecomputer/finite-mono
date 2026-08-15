from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CHECKER = ROOT / "scripts/check_finite_private_load_comparison.py"


def load_log(
    *,
    completion_p95: tuple[float, ...],
    per_request_p50: tuple[float, ...],
    aggregate: tuple[float, ...],
) -> str:
    if not (len(completion_p95) == len(per_request_p50) == len(aggregate)):
        raise ValueError("load fixture metric lengths must match")
    lines: list[str] = []
    for index, (p95, generation, aggregate_rate) in enumerate(
        zip(completion_p95, per_request_p50, aggregate), start=1
    ):
        lines.extend(
            (
                f"=== run={index} ===",
                "requests=32 prompt_tokens=384 completion_tokens=2048 batch_seconds=2.500",
                "time_to_first_byte_seconds p50=0.100 p95=0.200 p99=0.300 max_allowed=90.000",
                f"completion_seconds p50=1.900 p95={p95:.3f} p99=2.700",
                "generation_tokens_per_second "
                f"per_request_p50={generation:.3f} per_request_p95=40.000 "
                f"aggregate={aggregate_rate:.3f}",
            )
        )
    return "\n".join(lines) + "\n"


class FinitePrivateLoadComparisonTests(unittest.TestCase):
    def run_checker(
        self, baseline: str, candidate: str
    ) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            baseline_path = root / "baseline.log"
            candidate_path = root / "candidate.log"
            baseline_path.write_text(baseline, encoding="utf-8")
            candidate_path.write_text(candidate, encoding="utf-8")
            return subprocess.run(
                [str(CHECKER), str(baseline_path), str(candidate_path)],
                cwd=ROOT,
                text=True,
                capture_output=True,
                check=False,
            )

    def test_healthy_service_metrics_pass_even_when_edge_aggregate_is_low(self) -> None:
        baseline = load_log(
            completion_p95=(2.285, 2.147, 2.155),
            per_request_p50=(33.275, 29.501, 33.148),
            aggregate=(885.463, 941.615, 938.546),
        )
        candidate = load_log(
            completion_p95=(2.032, 2.210, 2.213),
            per_request_p50=(36.383, 32.747, 32.186),
            aggregate=(314.521, 371.362, 163.791),
        )

        result = self.run_checker(baseline, candidate)

        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertTrue(report["passed"])
        self.assertEqual(report["aggregate"]["role"], "diagnostic_only")
        self.assertEqual(report["aggregate"]["candidate_median"], 314.521)

    def test_generation_rate_below_ninety_percent_fails(self) -> None:
        baseline = load_log(
            completion_p95=(2.0, 2.1, 2.2),
            per_request_p50=(30.0, 32.0, 34.0),
            aggregate=(850.0, 900.0, 950.0),
        )
        candidate = load_log(
            completion_p95=(2.0, 2.1, 2.2),
            per_request_p50=(27.0, 28.0, 29.0),
            aggregate=(850.0, 900.0, 950.0),
        )

        result = self.run_checker(baseline, candidate)

        self.assertEqual(result.returncode, 1)
        report = json.loads(result.stdout)
        self.assertFalse(report["passed"])
        self.assertIn("per_request_generation_rate", report["violations"])

    def test_completion_p95_above_one_hundred_twenty_five_percent_fails(self) -> None:
        baseline = load_log(
            completion_p95=(2.0, 2.1, 2.2),
            per_request_p50=(30.0, 32.0, 34.0),
            aggregate=(850.0, 900.0, 950.0),
        )
        candidate = load_log(
            completion_p95=(2.7, 2.8, 2.9),
            per_request_p50=(30.0, 32.0, 34.0),
            aggregate=(850.0, 900.0, 950.0),
        )

        result = self.run_checker(baseline, candidate)

        self.assertEqual(result.returncode, 1)
        report = json.loads(result.stdout)
        self.assertFalse(report["passed"])
        self.assertIn("completion_p95", report["violations"])

    def test_missing_run_fails_closed(self) -> None:
        baseline = load_log(
            completion_p95=(2.0, 2.1),
            per_request_p50=(30.0, 32.0),
            aggregate=(850.0, 900.0),
        )
        candidate = load_log(
            completion_p95=(2.0, 2.1, 2.2),
            per_request_p50=(30.0, 32.0, 34.0),
            aggregate=(850.0, 900.0, 950.0),
        )

        result = self.run_checker(baseline, candidate)

        self.assertEqual(result.returncode, 2)
        self.assertIn("expected exactly 3", result.stderr)


if __name__ == "__main__":
    unittest.main()

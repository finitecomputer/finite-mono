from __future__ import annotations

import unittest

from scripts.check_finite_private_glm53_capacity import AcceptanceThresholds, evaluate


class Glm53CapacityAcceptanceTests(unittest.TestCase):
    def test_exact_120_user_floor_passes(self) -> None:
        report = evaluate(
            {
                "concurrency": 120,
                "successes": 120,
                "errors": 0,
                "terminal_streams": 120,
                "aggregate_output_tok_s": 2400.0,
                "per_request_output_tok_s_p10": 10.0,
                "per_request_output_tok_s_p50": 20.0,
                "ttft_p95_s": 10.0,
            },
            AcceptanceThresholds(),
        )
        self.assertTrue(report["passed"])
        self.assertEqual(report["violations"], [])

    def test_slow_tail_fails_even_when_aggregate_is_high(self) -> None:
        report = evaluate(
            {
                "concurrency": 120,
                "successes": 120,
                "errors": 0,
                "terminal_streams": 120,
                "aggregate_output_tok_s": 5000.0,
                "per_request_output_tok_s_p10": 9.99,
                "per_request_output_tok_s_p50": 30.0,
                "ttft_p95_s": 2.0,
            },
            AcceptanceThresholds(),
        )
        self.assertFalse(report["passed"])
        self.assertEqual(report["violations"], ["per_request_output_tok_s_p10"])

    def test_partial_success_never_passes(self) -> None:
        report = evaluate(
            {
                "concurrency": 120,
                "successes": 119,
                "errors": 1,
                "terminal_streams": 119,
                "aggregate_output_tok_s": 5000.0,
                "per_request_output_tok_s_p10": 30.0,
                "per_request_output_tok_s_p50": 40.0,
                "ttft_p95_s": 2.0,
            },
            AcceptanceThresholds(),
        )
        self.assertFalse(report["passed"])
        self.assertEqual(
            report["violations"],
            ["successes", "errors", "terminal_streams"],
        )


if __name__ == "__main__":
    unittest.main()

from __future__ import annotations

import argparse
from datetime import date, timedelta
import io
import json
import unittest
from unittest.mock import patch

from scripts.check_finite_private_glm53_capacity import request_once
from scripts.finite_private_v41_benchmark import (
    check_window, command, measurement_cutoff, parse_tiers, utc, window,
)

DAY = date(2026, 9, 17)
START, END = window(DAY)


class WindowTests(unittest.TestCase):
    def test_central_window_is_eight_utc(self):
        self.assertEqual(utc("2026-09-17T03:00:00-05:00"), START)

    def test_winter_window_uses_central_standard_time(self):
        start, end = window(date(2026, 12, 17))
        self.assertEqual(start, utc("2026-12-17T09:00:00Z"))
        self.assertEqual(end, utc("2026-12-17T12:00:00Z"))

    def test_previous_days_window_cannot_authorize_today(self):
        with self.assertRaises(ValueError):
            check_window(START, END, date(2026, 9, 16))

    def test_no_traffic_before_start_or_after_deadline(self):
        deadline = START + timedelta(minutes=75)
        for now in (START - timedelta(seconds=1), deadline, END):
            with self.subTest(now=now), self.assertRaises(ValueError):
                check_window(now, deadline, DAY)
        check_window(START, deadline, DAY)

    def test_refuse_unbounded_or_naive_deadline(self):
        with self.assertRaises(ValueError):
            check_window(START, END + timedelta(seconds=1), DAY)
        with self.assertRaises(ValueError):
            utc("2026-09-16T04:15:00")

    def test_measurement_cannot_consume_recovery_reserve(self):
        cutoff = measurement_cutoff(DAY)
        check_window(START, cutoff, DAY)
        with self.assertRaises(ValueError):
            check_window(START, cutoff + timedelta(seconds=1), DAY)

    def test_baseline_subset_keeps_bounded_increasing_tiers(self):
        self.assertEqual(parse_tiers("1,8"), (1, 8))
        for value in ("", "0", "256", "8,1", "1,1", "1,no"):
            with self.subTest(value=value), self.assertRaises(argparse.ArgumentTypeError):
                parse_tiers(value)

    def test_identical_workload_for_both_models(self):
        glm = command("glm-5-3-flash", "glm-tag", 32)
        deepseek = command("deepseek-v4-1-flash", "deepseek-tag", 32)
        for flag in ("--output-tokens", "--repetitions", "--warmup", "--thinking",
                     "--reasoning-effort", "--timeout", "--concurrency"):
            self.assertEqual(glm[glm.index(flag) + 1], deepseek[deepseek.index(flag) + 1])


class Response(io.BytesIO):
    status = 200


class ModelIdentityTests(unittest.TestCase):
    def run_stream(self, model):
        event = {"choices": [{"delta": {"content": "test"}}],
                 "usage": {"completion_tokens": 10}}
        if model is not None:
            event["model"] = model
        data = ("data: " + json.dumps(event) + "\n\ndata: [DONE]\n\n").encode()
        args = argparse.Namespace(
            url="https://example.invalid", api_key="synthetic-test-key",
            model="deepseek-v4-1-flash", expected_model="deepseek-v4-1-flash",
            output_tokens=10, thinking="on", reasoning_effort="high", timeout=1,
        )
        with patch("urllib.request.urlopen", return_value=Response(data)):
            return request_once(args, 0, "synthetic")

    def test_stale_glm_route_cannot_pass_deepseek_benchmark(self):
        result = self.run_stream("glm-5-3-flash")
        self.assertEqual(result.error, "stream returned unexpected model")

    def test_missing_identity_fails(self):
        self.assertEqual(self.run_stream(None).error, "stream lacked expected model identity")

    def test_matching_model_and_terminal_usage_pass(self):
        result = self.run_stream("deepseek-v4-1-flash")
        self.assertIsNone(result.error)
        self.assertTrue(result.terminal_stream)
        self.assertEqual(result.completion_tokens, 10)


if __name__ == "__main__":
    unittest.main()

from __future__ import annotations

from datetime import date, timedelta
from contextlib import redirect_stdout
import io
import json
import subprocess
import unittest
from unittest.mock import patch

from scripts.finite_private_v41_start import (
    CONTAINER_ID, HOST, ROLLBACK, check_container, check_start_time, main, window,
)
from scripts.check_finite_private_v41_modelpacks import PACKS


class StartTests(unittest.TestCase):
    def state(self):
        return {"id": CONTAINER_ID, "name": "finite-private", "host_name": HOST,
                "repo": "finitecomputer/confidential-finite-private", "status": "ready",
                "current_tag": ROLLBACK, "gpus": 8, "host_gpu_type": "H200",
                "auto_update": False, "debug": False, "disable_cc_mode": False,
                "update_tag": "", "update_status": "",
                "secrets": ["VLLM_API_KEY", "VLLM_INTERNAL_API_KEY", "FINITE_USAGE_API_SERVICE_KEY"]}

    def test_exact_ready_glm_identity_passes(self):
        check_container(self.state())

    def test_identity_or_pending_update_change_blocks_swap(self):
        for field, value in (("id", "other"), ("host_name", "other"), ("status", "started"),
                             ("current_tag", "other"), ("update_tag", "pending"),
                             ("gpus", 4), ("auto_update", True), ("secrets", []),
                             ("debug", True), ("disable_cc_mode", True)):
            with self.subTest(field=field), self.assertRaises(ValueError):
                check_container({**self.state(), field: value})

    def test_swap_allowed_only_in_first_ten_minutes_of_selected_date(self):
        day = date(2026, 9, 17)
        start, _ = window(day)
        check_start_time(start, day)
        check_start_time(start + timedelta(minutes=9, seconds=59), day)
        for when in (start - timedelta(seconds=1), start + timedelta(minutes=10),
                     start + timedelta(days=1)):
            with self.subTest(when=when), self.assertRaises(ValueError):
                check_start_time(when, day)

    def pack(self, name):
        return {"job_id": "candidate-job" if name == "deepseek" else "zlgwkqrylpqzwjzp",
                "host": HOST, "status": "complete", **PACKS[name]}

    def run_main(self, jobs, execute=False, times=None):
        argv = ["start", "--window-date", "2026-09-17", "--deepseek-job", "candidate-job"]
        if execute:
            argv.append("--execute")
        with patch("sys.argv", argv), redirect_stdout(io.StringIO()), \
                patch("scripts.finite_private_v41_start.api_key", return_value="synthetic"), \
                patch("scripts.finite_private_v41_start.fetch_job", side_effect=jobs), \
                patch("scripts.finite_private_v41_start.datetime") as clock, \
                patch("scripts.finite_private_v41_start.subprocess.run") as run:
            clock.now.side_effect = times or []
            run.return_value = subprocess.CompletedProcess([], 0, stdout=json.dumps(self.state()))
            try:
                result = main()
            except ValueError:
                result = "blocked"
            return result, run.call_args_list

    def test_incomplete_pack_never_reaches_relaunch(self):
        result, calls = self.run_main([
            {**self.pack("deepseek"), "status": "running"}, self.pack("glm")])
        self.assertEqual(result, "blocked")
        self.assertEqual(calls, [])

    def test_successful_dry_run_only_reads_container(self):
        result, calls = self.run_main([self.pack("deepseek"), self.pack("glm")])
        self.assertEqual(result, 0)
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0].args[0][:3], ["tinfoil", "container", "get"])

    def test_slow_remote_checks_cannot_start_late_swap(self):
        start, _ = window(date(2026, 9, 17))
        result, calls = self.run_main([self.pack("deepseek"), self.pack("glm")], execute=True,
                                     times=[start + timedelta(minutes=9),
                                            start + timedelta(minutes=10)])
        self.assertEqual(result, "blocked")
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0].args[0][:3], ["tinfoil", "container", "get"])


if __name__ == "__main__":
    unittest.main()

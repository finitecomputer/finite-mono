from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest


ROOT = Path(__file__).resolve().parents[2]
COMMAND = ROOT / "scripts" / "finite-status"


class SitesBackupStatusTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.state = Path(temporary.name) / "backup-status.json"
        now = int(time.time())
        self.receipt = {
            "status": "ok",
            "started_at": now - 180,
            "snapshot_at": now - 120,
            "uploaded_at": now - 60,
            "archive": "sites-test-archive",
            "archive_id": "0123456789abcdef",
        }

    def write_receipt(self) -> None:
        self.state.write_text(json.dumps(self.receipt), encoding="utf-8")
        self.state.chmod(0o600)

    def run_status(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(COMMAND),
                "--sites-backup-state",
                str(self.state),
                *arguments,
            ],
            capture_output=True,
            text=True,
            timeout=10,
            cwd=self.state.parent,
        )

    def assert_report(self, status: str, exit_code: int, *arguments: str) -> dict:
        result = self.run_status("--json", *arguments)
        self.assertEqual(result.returncode, exit_code, result.stderr)
        self.assertEqual(result.stderr, "")
        report = json.loads(result.stdout)
        self.assertEqual(report["schema_version"], "finite.status.v1")
        self.assertIn("generated_at", report)
        self.assertEqual(report["overall_status"], status)
        self.assertEqual(report["exit_code"], exit_code)
        self.assertEqual(set(report["sections"]), {"sites_backup"})
        section = report["sections"]["sites_backup"]
        self.assertEqual(section["status"], status)
        return section

    def test_healthy_receipt_reports_only_sites_backup_without_mutation(self) -> None:
        self.write_receipt()
        before = self.state.stat()
        contents = self.state.read_bytes()
        section = self.assert_report("green", 0)
        self.assertEqual(section["run_status"], "ok")
        self.assertEqual(section["maximum_age_seconds"], 129600)
        for key in (
            "started_at",
            "snapshot_at",
            "uploaded_at",
            "archive",
            "archive_id",
        ):
            self.assertEqual(section[key], self.receipt[key])
        self.assertEqual(self.state.read_bytes(), contents)
        self.assertEqual(self.state.stat().st_mtime_ns, before.st_mtime_ns)
        self.assertEqual(self.state.stat().st_mode, before.st_mode)

    def test_snapshot_and_upload_freshness_are_independent(self) -> None:
        receipt = self.receipt.copy()
        for field in ("snapshot_at", "uploaded_at"):
            for offset, reason in ((-37 * 3600, "stale"), (3600, "future")):
                with self.subTest(field=field, reason=reason):
                    self.receipt = receipt.copy()
                    self.receipt[field] = int(time.time()) + offset
                    self.write_receipt()
                    section = self.assert_report("red", 1)
                    self.assertIn(field, " ".join(section["errors"]))
                    self.assertIn(reason, " ".join(section["errors"]))
                    if field == "snapshot_at" and reason == "stale":
                        self.assertGreater(section["snapshot_age_seconds"], 129600)
                        self.assertLess(section["upload_age_seconds"], 120)

    def test_failed_recent_run_is_red_despite_previous_success(self) -> None:
        self.receipt.update(
            status="failed",
            started_at=int(time.time()) - 10,
            error="CalledProcessError",
        )
        self.write_receipt()
        section = self.assert_report("red", 1)
        self.assertEqual(section["run_status"], "failed")
        self.assertEqual(section["error"], "CalledProcessError")
        self.assertEqual(section["snapshot_at"], self.receipt["snapshot_at"])
        self.assertEqual(section["uploaded_at"], self.receipt["uploaded_at"])

    def test_running_is_not_healthy_even_with_fresh_previous_success(self) -> None:
        self.receipt.update(status="running", started_at=int(time.time()) - 10)
        self.write_receipt()
        self.assert_report("unknown", 2)

    def test_first_run_can_fail_or_run_before_any_success(self) -> None:
        for status, health, exit_code in (
            ("failed", "red", 1),
            ("running", "unknown", 2),
        ):
            with self.subTest(status=status):
                self.receipt = {"status": status, "started_at": int(time.time()) - 10}
                if status == "failed":
                    self.receipt["error"] = "RuntimeError"
                self.write_receipt()
                self.assert_report(health, exit_code)

    def test_missing_or_unreadable_receipt_is_unknown(self) -> None:
        self.assert_report("unknown", 2)
        self.state.mkdir()
        self.assert_report("unknown", 2)

    def test_malformed_receipts_are_unknown_without_exposing_contents(self) -> None:
        malformed = ["{", "[]", "null", "{}", '"private receipt contents"']
        for field, value in (
            ("status", "unexpected"),
            ("status", []),
            ("started_at", None),
            ("started_at", True),
            ("snapshot_at", "123"),
            ("snapshot_at", -1),
            ("uploaded_at", float("nan")),
            ("uploaded_at", float("inf")),
            ("archive", ""),
            ("archive_id", 123),
        ):
            malformed.append(json.dumps({**self.receipt, field: value}))
        for field in self.receipt:
            malformed.append(
                json.dumps({k: v for k, v in self.receipt.items() if k != field})
            )
        malformed.append(json.dumps({**self.receipt, "status": "failed"}))
        malformed.append(
            json.dumps(
                {
                    **self.receipt,
                    "status": "failed",
                    "error": "private receipt contents",
                }
            )
        )
        for contents in malformed:
            with self.subTest(contents=contents):
                self.state.write_text(contents, encoding="utf-8")
                section = self.assert_report("unknown", 2)
                self.assertTrue(section["errors"])
                self.assertNotIn("private receipt contents", json.dumps(section))
        self.state.write_bytes(b"\xff")
        self.assert_report("unknown", 2)

    def test_max_age_defaults_to_36_hours_and_accepts_seconds(self) -> None:
        self.receipt["snapshot_at"] = int(time.time()) - 35 * 3600
        self.write_receipt()
        self.assert_report("green", 0)
        self.assert_report("red", 1, "--sites-backup-max-age", "3600")
        self.receipt["snapshot_at"] = int(time.time()) - 37 * 3600
        self.write_receipt()
        section = self.assert_report("green", 0, "--sites-backup-max-age", "172800")
        self.assertEqual(section["maximum_age_seconds"], 172800)
        self.assert_report("red", 1, "--sites-backup-max-age", "0")

    def test_invalid_max_age_and_conflicting_modes_are_rejected(self) -> None:
        self.write_receipt()
        for arguments in (
            ("--sites-backup-max-age", "-1"),
            ("--sites-backup-max-age", "nan"),
            ("--sites-backup-max-age", "inf"),
            ("--fixture", "unused.json"),
        ):
            with self.subTest(arguments=arguments):
                result = self.run_status(*arguments)
                self.assertEqual(result.returncode, 2)
                self.assertIn("error:", result.stderr)
                self.assertNotIn("Traceback", result.stderr)
                self.assertEqual(result.stdout, "")

    def test_human_output_labels_snapshot_and_upload_separately(self) -> None:
        self.write_receipt()
        result = self.run_status()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Sites Borg backup [GREEN]", result.stdout)
        self.assertIn("Source snapshot:", result.stdout)
        self.assertIn("Last successful upload:", result.stdout)
        self.assertIn(self.receipt["archive"], result.stdout)
        self.assertIn(self.receipt["archive_id"], result.stdout)
        self.assertNotIn("Fleet convergence", result.stdout)
        self.assertNotIn("rollback window", result.stdout)
        self.receipt.update(status="failed", error="CalledProcessError")
        self.write_receipt()
        result = self.run_status()
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("[RED]", result.stdout)
        self.assertIn("CalledProcessError", result.stdout)
        self.state.unlink()
        result = self.run_status()
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("[UNKNOWN]", result.stdout)
        self.assertEqual(result.stderr, "")

    def test_copied_cli_runs_without_fleet_access_or_backup_imports(self) -> None:
        self.write_receipt()
        installed = self.state.parent / "bin"
        installed.mkdir()
        for name in ("finite-status", "finite_status.py"):
            shutil.copy2(COMMAND.parent / name, installed / name)
        # Block process and network boundaries inside the real CLI interpreter.
        (installed / "sitecustomize.py").write_text(
            "import sys\n"
            "def deny_fleet_access(event, args):\n"
            "    if event.startswith(('subprocess.', 'socket.')):\n"
            "        raise RuntimeError('receipt mode attempted fleet access')\n"
            "sys.addaudithook(deny_fleet_access)\n",
            encoding="utf-8",
        )
        result = subprocess.run(
            [
                sys.executable,
                str(installed / "finite-status"),
                "--sites-backup-state",
                str(self.state),
                "--json",
            ],
            env={
                **os.environ,
                "PATH": "",
                "PYTHONPATH": str(installed),
                "PYTHONDONTWRITEBYTECODE": "1",
            },
            cwd=installed,
            capture_output=True,
            text=True,
            timeout=10,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")
        report = json.loads(result.stdout)
        self.assertEqual(report["overall_status"], "green")
        self.assertEqual(set(report["sections"]), {"sites_backup"})


if __name__ == "__main__":
    unittest.main()

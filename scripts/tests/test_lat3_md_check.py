#!/usr/bin/env python3
"""Execute LAT3's evaluated MD check against synthetic devices and sysfs."""

from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[2]
MODULE = ROOT / "infra/nixos/hosts/finite-lat-3/storage-health.nix"


class Lat3MdCheckTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        # Evaluate the actual service text without building Linux packages.
        expression = (
            f"(import {MODULE} {{ pkgs.writeShellApplication = args: args.text; }})"
            ".systemd.services.finite-md-check.serviceConfig.ExecStart"
        )
        rendered = subprocess.check_output(
            ["nix", "eval", "--impure", "--raw", "--expr", expression],
            cwd=ROOT,
            text=True,
        )
        # ExecStart appends the executable path to the package interpolation.
        # Our stand-in returns its text, so remove only that known suffix.
        suffix = "/bin/finite-md-check"
        if not rendered.endswith(suffix):
            raise AssertionError("unexpected finite-md-check ExecStart")
        cls.source = rendered.removesuffix(suffix)

    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.fixture = Path(self.temp.name)
        (self.fixture / "dev/md").mkdir(parents=True)
        (self.fixture / "bin").mkdir()
        self.log = self.fixture / "actions"
        for name in ("root", "data"):
            (self.fixture / "dev/md" / name).touch()
            md = self.fixture / "sys" / name / "md"
            md.mkdir(parents=True)
            for key, value in {
                "degraded": "0",
                "sync_action": "idle",
                "mismatch_cnt": "0",
                "fixture_result": "0",
            }.items():
                (md / key).write_text(value)

        # Only redirect filesystem boundaries. Block-device readiness is
        # represented by a regular fixture file; production still requires -b.
        self.assertEqual(self.source.count('[[ -b "$device" ]]'), 1)
        source = self.source.replace('[[ -b "$device" ]]', '[[ -f "$device" ]]')
        source = source.replace("/dev/md/", f"{self.fixture}/dev/md/")
        source = source.replace("/sys/block/", f"{self.fixture}/sys/")
        source = source.replace(
            "/run/lock/finite-md-check.lock", str(self.fixture / "check.lock")
        )
        self.script = self.fixture / "check"
        self.script.write_text("set -euo pipefail\n" + source)
        shim = self.fixture / "bin/mock"
        shim.write_text(
            f"#!{sys.executable}\n"
            + textwrap.dedent(
                """\
                import os
                from pathlib import Path
                import sys

                root = Path(os.environ["MD_CHECK_FIXTURE"])
                if Path(sys.argv[0]).name == "flock":
                    assert sys.argv[1:] == ["-n", "9"]
                    raise SystemExit(1 if os.environ.get("FIXTURE_LOCKED") else 0)
                assert sys.argv[1:] == ["30"]
                if os.environ.get("FIXTURE_IO_FAILURE"):
                    raise SystemExit(13)
                for name in ("root", "data"):
                    md = root / "sys" / name / "md"
                    if (md / "sync_action").read_text().strip() == "check":
                        with (root / "actions").open("a") as log:
                            log.write(name + ":check\\n")
                        (md / "mismatch_cnt").write_text(
                            (md / "fixture_result").read_text()
                        )
                        (md / "sync_action").write_text("idle")
                """
            )
        )
        shim.chmod(0o755)
        for name in ("flock", "sleep"):
            (self.fixture / "bin" / name).symlink_to(shim)

    def set_field(self, array: str, field: str, value: str) -> None:
        (self.fixture / "sys" / array / "md" / field).write_text(value)

    def run_check(self, **overrides: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(self.script)],
            env={
                **os.environ,
                "PATH": str(self.fixture / "bin") + os.pathsep + os.environ["PATH"],
                "MD_CHECK_FIXTURE": str(self.fixture),
                **overrides,
            },
            text=True,
            capture_output=True,
            timeout=10,
            check=False,
        )

    def actions(self) -> list[str]:
        return self.log.read_text().splitlines() if self.log.exists() else []

    def test_root_mismatch_does_not_skip_data(self) -> None:
        self.set_field("root", "fixture_result", "82688")
        result = self.run_check()
        self.assertEqual(self.actions(), ["root:check", "data:check"])
        self.assertEqual(result.returncode, 1)
        self.assertIn(
            "mismatch count is 82688; refusing automatic repair", result.stderr
        )
        self.assertIn("data check completed: mismatch count is 0", result.stdout)

    def test_both_clean_succeed_and_report_each_completion(self) -> None:
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.actions(), ["root:check", "data:check"])
        for name in ("root", "data"):
            self.assertIn(f"{name} check completed: mismatch count is 0", result.stdout)

    def test_data_mismatch_still_fails(self) -> None:
        self.set_field("data", "fixture_result", "128")
        result = self.run_check()
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.actions(), ["root:check", "data:check"])
        self.assertIn("mismatch count is 128; refusing automatic repair", result.stderr)

    def test_both_mismatches_are_reported(self) -> None:
        self.set_field("root", "fixture_result", "64")
        self.set_field("data", "fixture_result", "128")
        result = self.run_check()
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.actions(), ["root:check", "data:check"])
        for count in (64, 128):
            self.assertIn(
                f"mismatch count is {count}; refusing automatic repair", result.stderr
            )

    def test_unready_root_still_aborts_before_scan(self) -> None:
        for field, value in (("degraded", "1"), ("sync_action", "resync")):
            with self.subTest(field=field):
                original = (self.fixture / "sys/root/md" / field).read_text()
                self.set_field("root", field, value)
                result = self.run_check()
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(self.actions(), [])
                self.set_field("root", field, original)
        (self.fixture / "dev/md/root").unlink()
        self.assertNotEqual(self.run_check().returncode, 0)
        self.assertEqual(self.actions(), [])

    def test_lock_contention_still_prevents_scan(self) -> None:
        result = self.run_check(FIXTURE_LOCKED="1")
        self.assertEqual(result.returncode, 75)
        self.assertEqual(self.actions(), [])

    def test_root_mismatch_does_not_bypass_busy_data_gate(self) -> None:
        self.set_field("root", "fixture_result", "82688")
        self.set_field("data", "sync_action", "resync")
        result = self.run_check()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.actions(), ["root:check"])
        self.assertEqual(
            (self.fixture / "sys/data/md/sync_action").read_text(), "resync"
        )

    def test_command_failure_is_not_treated_as_completed_check(self) -> None:
        result = self.run_check(FIXTURE_IO_FAILURE="1")
        self.assertEqual(result.returncode, 13)
        self.assertEqual(self.actions(), [])
        self.assertEqual((self.fixture / "sys/data/md/sync_action").read_text(), "idle")


if __name__ == "__main__":
    unittest.main()

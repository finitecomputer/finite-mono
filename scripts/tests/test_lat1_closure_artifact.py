#!/usr/bin/env python3
"""Synthetic checks for lat1 closure artifact deploy helpers."""

from __future__ import annotations

import json
import os
import getpass
import grp
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
DEPLOY = ROOT / "scripts" / "deploy-lat1-closure-cache"
LAT_MONITORING_SECRETS = ROOT / "infra/nixos/scripts/check-lat-monitoring-secrets"
SELECT_HARNESSES = ROOT / "scripts/ci/select-harnesses"


class Lat1ClosureArtifactTests(unittest.TestCase):
    def run_deploy(self, artifact_dir: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(DEPLOY), str(artifact_dir)],
            cwd=ROOT,
            env={**os.environ, "PATH": os.environ["PATH"]},
            text=True,
            capture_output=True,
            check=False,
        )

    def test_missing_manifest_fails_before_git_or_nix(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            result = self.run_deploy(Path(temp))
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact manifest is missing", result.stderr)

    def test_manifest_schema_is_strict(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp)
            (artifact / "manifest.json").write_text(
                json.dumps({"schema": "wrong"}) + "\n",
                encoding="utf-8",
            )
            result = self.run_deploy(artifact)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unexpected manifest schema", result.stderr)

    def test_valid_manifest_requires_file_binary_cache(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp)
            (artifact / "manifest.json").write_text(
                json.dumps(
                    {
                        "schema": "finite.lat1.nixos-closure.v1",
                        "repository": "finite-co/finite-mono",
                        "rev": "a" * 40,
                        "system": "/nix/store/"
                        + "b" * 32
                        + "-nixos-system-finite-lat-1-25.11.test",
                        "disko": "/nix/store/" + "c" * 32 + "-disko",
                        "cache": "nix-cache",
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            result = self.run_deploy(artifact)
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact cache is missing or incomplete", result.stderr)

    def test_legacy_github_artifact_remains_rollback_compatible(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp)
            (artifact / "manifest.json").write_text(
                json.dumps(
                    {
                        "schema": "finite.lat1.nixos-closure.v1",
                        "repository": "finitecomputer/finite-mono",
                        "rev": "a" * 40,
                        "system": "/nix/store/"
                        + "b" * 32
                        + "-nixos-system-finite-lat-1-25.11.test",
                        "disko": "/nix/store/" + "c" * 32 + "-disko",
                        "cache": "nix-cache",
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            result = self.run_deploy(artifact)
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact cache is missing or incomplete", result.stderr)
        self.assertNotIn("unexpected repository", result.stderr)

    def test_validate_only_stops_before_any_remote_operation(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        validation_exit = source.index('if [[ "$mode" == "validate" ]]')

        self.assertLess(validation_exit, source.index("ssh -o BatchMode=yes"))
        self.assertLess(validation_exit, source.index("nix copy --no-check-sigs"))

    def test_deploy_preflights_log_shipping_secrets_before_snapshot(self) -> None:
        text = DEPLOY.read_text(encoding="utf-8")
        self.assertIn("FINITE_LOGS_WRITE_PASSWORD", text)
        self.assertIn("check-lat-monitoring-secrets", text)
        self.assertLess(
            text.index("check-lat-monitoring-secrets"),
            text.index("taking pre-deploy recovery snapshot"),
        )

    def test_ci_selector_keeps_deploy_helper_in_nix_static_lane(self) -> None:
        result = subprocess.run(
            [
                str(SELECT_HARNESSES),
                "--changed-file",
                "scripts/deploy-lat1-closure-cache",
                "--changed-file",
                "scripts/ci/select-harnesses",
            ],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        selected = {
            line.removesuffix("=true")
            for line in result.stdout.splitlines()
            if line.startswith("run_") and line.endswith("=true")
        }
        self.assertEqual(selected, {"run_nix_checks"})


class LatMonitoringSecretsTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name) / "root"
        self.secret_dir = self.root / "etc/finite"
        self.secret_dir.mkdir(parents=True)
        self.metrics_secret = self.secret_dir / "metrics-remote-write.env"
        self.logs_secret = self.secret_dir / "logs-write.env"
        self.metrics_secret.write_text(
            "\n".join(
                [
                    "FINITE_METRICS_REMOTE_WRITE_USERNAME=metrics-writer",
                    "FINITE_METRICS_REMOTE_WRITE_PASSWORD=metrics-secret-value",
                    "",
                ]
            ),
            encoding="utf-8",
        )
        self.logs_secret.write_text(
            "\n".join(
                [
                    "export FINITE_LOGS_WRITE_USERNAME=logs-writer",
                    "FINITE_LOGS_WRITE_PASSWORD=logs-secret-value",
                    "",
                ]
            ),
            encoding="utf-8",
        )
        self.metrics_secret.chmod(0o600)
        self.logs_secret.chmod(0o600)
        self.owner = getpass.getuser()
        self.group = grp.getgrgid(self.logs_secret.stat().st_gid).gr_name

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_checker(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                str(LAT_MONITORING_SECRETS),
                "--root",
                str(self.root),
                "--owner",
                self.owner,
                "--group",
                self.group,
            ],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )

    def combined_output(self, result: subprocess.CompletedProcess[str]) -> str:
        return result.stdout + result.stderr

    def test_valid_files_pass_without_emitting_values(self) -> None:
        result = self.run_checker()
        self.assertEqual(result.returncode, 0, result.stderr)
        output = self.combined_output(result)
        self.assertIn("no values emitted", output)
        self.assertNotIn("metrics-secret-value", output)
        self.assertNotIn("logs-secret-value", output)

    def test_missing_logs_file_fails_closed(self) -> None:
        self.logs_secret.unlink()
        result = self.run_checker()
        self.assertEqual(result.returncode, 1)
        self.assertIn("missing required file: /etc/finite/logs-write.env", result.stderr)

    def test_wrong_mode_and_missing_name_fail_without_emitting_value(self) -> None:
        self.logs_secret.write_text(
            "FINITE_LOGS_WRITE_USERNAME=logs-secret-value\n",
            encoding="utf-8",
        )
        self.logs_secret.chmod(0o644)
        result = self.run_checker()
        self.assertEqual(result.returncode, 1)
        self.assertIn("wrong mode for /etc/finite/logs-write.env", result.stderr)
        self.assertIn(
            "missing required variable FINITE_LOGS_WRITE_PASSWORD",
            result.stderr,
        )
        self.assertNotIn("logs-secret-value", self.combined_output(result))

    def test_malformed_entry_reports_only_file_and_line(self) -> None:
        self.metrics_secret.write_text(
            "FINITE_METRICS_REMOTE_WRITE_USERNAME=metrics-writer\n"
            "this-line-could-contain-a-secret\n",
            encoding="utf-8",
        )
        result = self.run_checker()
        self.assertEqual(result.returncode, 1)
        self.assertIn("malformed environment entry", result.stderr)
        self.assertIn("malformed environment file", result.stderr)
        self.assertNotIn("this-line-could-contain-a-secret", result.stderr)


if __name__ == "__main__":
    unittest.main()

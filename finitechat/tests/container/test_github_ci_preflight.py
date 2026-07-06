"""Unit checks for GitHub Actions setup preflight."""

from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
PREFLIGHT_SCRIPT = REPO_ROOT / "scripts" / "hermes-github-ci-preflight.py"


def write_json(path: Path, value: list[dict[str, str]]) -> None:
    path.write_text(json.dumps(value) + "\n", encoding="utf-8")


def required_variables() -> list[dict[str, str]]:
    return [
        {"name": "FINITE_LATITUDE_STORAGE_BUCKET", "value": "bucket"},
        {
            "name": "FINITE_DOCKER_RESTIC_PREFIX",
            "value": "agent-runtimes/tinfoil-canary-001/restic",
        },
    ]


def run_preflight(
    tmp: Path,
    *,
    secrets: list[dict[str, str]],
    variables: list[dict[str, str]],
) -> tuple[subprocess.CompletedProcess[str], dict]:
    secrets_path = tmp / "secrets.json"
    variables_path = tmp / "variables.json"
    report_path = tmp / "report.json"
    write_json(secrets_path, secrets)
    write_json(variables_path, variables)
    result = subprocess.run(
        [
            str(PREFLIGHT_SCRIPT),
            "--repo",
            "finitecomputer/finitechat",
            "--secrets-json",
            str(secrets_path),
            "--variables-json",
            str(variables_path),
            "--report",
            str(report_path),
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    return result, json.loads(report_path.read_text(encoding="utf-8"))


class GithubCiPreflightTest(unittest.TestCase):
    def test_passes_when_required_names_exist(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            result, report = run_preflight(
                Path(tmp_value),
                secrets=[
                    {"name": "FINITE_DOCKER_RESTIC_PASSWORD"},
                    {"name": "FINITE_DOCKER_RESTIC_AWS_ACCESS_KEY_ID"},
                    {"name": "FINITE_DOCKER_RESTIC_AWS_SECRET_ACCESS_KEY"},
                    {"name": "FINITE_DOCKER_RESTIC_AWS_SESSION_TOKEN"},
                    {"name": "OPENROUTER_API_KEY"},
                ],
                variables=required_variables(),
            )

        self.assertEqual(result.returncode, 0)
        self.assertEqual(report["status"], "ok")
        self.assertEqual(report["missing_required_secrets"], [])
        self.assertEqual(report["missing_required_variables"], [])
        self.assertIn(
            "FINITE_DOCKER_RESTIC_AWS_SESSION_TOKEN",
            report["present_optional_secrets"],
        )

    def test_fails_without_required_secrets(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            result, report = run_preflight(
                Path(tmp_value),
                secrets=[],
                variables=required_variables(),
            )

        self.assertEqual(result.returncode, 2)
        self.assertEqual(report["status"], "failed")
        self.assertIn("FINITE_DOCKER_RESTIC_PASSWORD", report["missing_required_secrets"])
        self.assertIn("OPENROUTER_API_KEY", report["missing_required_secrets"])
        self.assertIn(
            "FINITE_DOCKER_RESTIC_AWS_SECRET_ACCESS_KEY",
            report["missing_required_secrets"],
        )

    def test_fails_without_required_variables(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            result, report = run_preflight(
                Path(tmp_value),
                secrets=[
                    {"name": "FINITE_DOCKER_RESTIC_PASSWORD"},
                    {"name": "FINITE_DOCKER_RESTIC_AWS_ACCESS_KEY_ID"},
                    {"name": "FINITE_DOCKER_RESTIC_AWS_SECRET_ACCESS_KEY"},
                    {"name": "OPENROUTER_API_KEY"},
                ],
                variables=[],
            )

        self.assertEqual(result.returncode, 2)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(
            report["missing_required_variables"],
            ["FINITE_DOCKER_RESTIC_PREFIX", "FINITE_LATITUDE_STORAGE_BUCKET"],
        )

    def test_optional_secrets_do_not_block_preflight(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            result, report = run_preflight(
                Path(tmp_value),
                secrets=[
                    {"name": "FINITE_DOCKER_RESTIC_PASSWORD"},
                    {"name": "FINITE_DOCKER_RESTIC_AWS_ACCESS_KEY_ID"},
                    {"name": "FINITE_DOCKER_RESTIC_AWS_SECRET_ACCESS_KEY"},
                    {"name": "OPENROUTER_API_KEY"},
                ],
                variables=required_variables(),
            )

        self.assertEqual(result.returncode, 0)
        self.assertEqual(report["status"], "ok")
        self.assertIn(
            "FINITE_DOCKER_RESTIC_AWS_SESSION_TOKEN",
            report["optional_secrets"],
        )
        self.assertNotIn(
            "FINITE_DOCKER_RESTIC_AWS_SESSION_TOKEN",
            report["present_optional_secrets"],
        )


if __name__ == "__main__":
    unittest.main()

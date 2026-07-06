"""Unit checks for Docker restic preflight env normalization."""

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
PREFLIGHT_PATH = REPO_ROOT / "scripts" / "hermes-restic-preflight.py"

spec = importlib.util.spec_from_file_location("hermes_restic_preflight", PREFLIGHT_PATH)
assert spec is not None
preflight = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(preflight)


class ResticPreflightTest(unittest.TestCase):
    def test_parse_aws_shared_config_reads_profile(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            credentials_file = tmp / "credentials"
            config_file = tmp / "config"
            credentials_file.write_text(
                "\n".join(
                    [
                        "[finite]",
                        "aws_access_key_id = profile-access",
                        "aws_secret_access_key = profile-secret",
                        "aws_session_token = profile-session",
                    ]
                ),
                encoding="utf-8",
            )
            config_file.write_text(
                "\n".join(
                    [
                        "[profile finite]",
                        "region = us-east-1",
                    ]
                ),
                encoding="utf-8",
            )

            values = preflight.parse_aws_shared_config(
                credentials_file=credentials_file,
                config_file=config_file,
                profile="finite",
            )

        self.assertEqual(values["AWS_ACCESS_KEY_ID"], "profile-access")
        self.assertEqual(values["AWS_SECRET_ACCESS_KEY"], "profile-secret")
        self.assertEqual(values["AWS_SESSION_TOKEN"], "profile-session")
        self.assertEqual(values["AWS_REGION"], "us-east-1")

    def test_aws_shared_config_does_not_override_env(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            credentials_file = tmp / "credentials"
            config_file = tmp / "config"
            credentials_file.write_text(
                "\n".join(
                    [
                        "[default]",
                        "aws_access_key_id = profile-access",
                        "aws_secret_access_key = profile-secret",
                    ]
                ),
                encoding="utf-8",
            )

            values = preflight.merge_aws_shared_config(
                {"AWS_ACCESS_KEY_ID": "env-access"},
                credentials_file=credentials_file,
                config_file=config_file,
                profile="default",
            )

        self.assertEqual(values["AWS_ACCESS_KEY_ID"], "env-access")
        self.assertEqual(values["AWS_SECRET_ACCESS_KEY"], "profile-secret")

    def test_aws_shared_shell_exports_only_missing_values(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            credentials_file = tmp / "credentials"
            config_file = tmp / "config"
            credentials_file.write_text(
                "\n".join(
                    [
                        "[default]",
                        "aws_access_key_id = profile-access",
                        "aws_secret_access_key = profile secret with spaces",
                    ]
                ),
                encoding="utf-8",
            )

            text = preflight.aws_shared_shell_exports(
                {"AWS_ACCESS_KEY_ID": "env-access"},
                credentials_file=credentials_file,
                config_file=config_file,
                profile="default",
            )

        self.assertNotIn("AWS_ACCESS_KEY_ID", text)
        self.assertIn("AWS_SECRET_ACCESS_KEY=", text)
        self.assertIn("'profile secret with spaces'", text)

    def test_accepts_finite_prefixed_aws_credentials(self) -> None:
        status, report = preflight.validate(
            {
                "FINITE_DOCKER_RESTIC_BACKEND": "s3",
                "FINITE_DOCKER_RESTIC_REPOSITORY": (
                    "s3:https://objects.nyc.storage.sh/tinfoil-agent-spike/agent-runtimes/tinfoil-canary-001/restic"
                ),
                "FINITE_DOCKER_RESTIC_PASSWORD": "temporary-canary-backup-secret",
                "FINITE_DOCKER_RESTIC_AWS_ACCESS_KEY_ID": "access",
                "FINITE_DOCKER_RESTIC_AWS_SECRET_ACCESS_KEY": "secret",
            }
        )

        self.assertEqual(status, 0)
        self.assertEqual(report["status"], "ok")
        self.assertEqual(
            report["repository"],
            "s3:https://objects.nyc.storage.sh/tinfoil-agent-spike/agent-runtimes/tinfoil-canary-001/restic",
        )
        env = report["env"]
        self.assertTrue(env["AWS_ACCESS_KEY_ID"])
        self.assertTrue(env["AWS_SECRET_ACCESS_KEY"])
        self.assertTrue(env["FINITE_DOCKER_RESTIC_AWS_ACCESS_KEY_ID"])
        self.assertTrue(env["FINITE_DOCKER_RESTIC_AWS_SECRET_ACCESS_KEY"])

    def test_derives_repository_from_latitude_bucket(self) -> None:
        status, report = preflight.validate(
            {
                "FINITE_DOCKER_RESTIC_BACKEND": "s3",
                "FINITE_DOCKER_RESTIC_PASSWORD": "temporary-canary-backup-secret",
                "FINITE_DOCKER_RESTIC_AWS_ACCESS_KEY_ID": "access",
                "FINITE_DOCKER_RESTIC_AWS_SECRET_ACCESS_KEY": "secret",
                "FINITE_LATITUDE_STORAGE_BUCKET": "tinfoil-agent-spike",
                "FINITE_DOCKER_RESTIC_PREFIX": "agent-runtimes/tinfoil-canary-001/restic",
            }
        )

        self.assertEqual(status, 0)
        self.assertEqual(
            report["repository"],
            "s3:https://objects.nyc.storage.sh/tinfoil-agent-spike/agent-runtimes/tinfoil-canary-001/restic",
        )

    def test_refuses_latitude_bucket_without_explicit_prefix(self) -> None:
        status, report = preflight.validate(
            {
                "FINITE_DOCKER_RESTIC_BACKEND": "s3",
                "FINITE_DOCKER_RESTIC_PASSWORD": "temporary-canary-backup-secret",
                "FINITE_DOCKER_RESTIC_AWS_ACCESS_KEY_ID": "access",
                "FINITE_DOCKER_RESTIC_AWS_SECRET_ACCESS_KEY": "secret",
                "FINITE_LATITUDE_STORAGE_BUCKET": "tinfoil-agent-spike",
            }
        )

        self.assertEqual(status, 2)
        self.assertIn(
            "FINITE_DOCKER_RESTIC_PREFIX is required when deriving "
            "FINITE_DOCKER_RESTIC_REPOSITORY from FINITE_LATITUDE_STORAGE_BUCKET",
            report["errors"],
        )

    def test_s3_backend_requires_explicit_backup_secret(self) -> None:
        status, report = preflight.validate(
            {
                "FINITE_DOCKER_RESTIC_BACKEND": "s3",
                "FINITE_DOCKER_RESTIC_REPOSITORY": (
                    "s3:https://objects.nyc.storage.sh/tinfoil-agent-spike/agent-runtimes/tinfoil-canary-001/restic"
                ),
                "FINITE_DOCKER_RESTIC_AWS_ACCESS_KEY_ID": "access",
                "FINITE_DOCKER_RESTIC_AWS_SECRET_ACCESS_KEY": "secret",
            }
        )

        self.assertEqual(status, 2)
        self.assertEqual(report["status"], "failed")
        self.assertIn("FINITE_DOCKER_RESTIC_PASSWORD is required for backend=s3", report["errors"])


if __name__ == "__main__":
    unittest.main()

from __future__ import annotations

import getpass
import grp
import json
from pathlib import Path
import subprocess
import tempfile
import unittest


REPO_ROOT = Path(__file__).resolve().parents[2]
CHECKER = REPO_ROOT / "scripts/check-lat1-secret-bootstrap"


class SecretBootstrapContractTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name) / "root"
        self.secret = self.root / "etc/finite/test.env"
        self.secret.parent.mkdir(parents=True)
        self.secret.write_text("REQUIRED_KEY=synthetic-value\n", encoding="utf-8")
        self.secret.chmod(0o600)
        self.contract = Path(self.temporary.name) / "contract.json"
        self.contract.write_text(
            json.dumps(
                {
                    "version": 1,
                    "files": [
                        {
                            "path": "/etc/finite/test.env",
                            "mode": "0600",
                            "owner": getpass.getuser(),
                            "group": grp.getgrgid(self.secret.stat().st_gid).gr_name,
                            "kind": "env",
                            "required_names": ["REQUIRED_KEY"],
                            "custody": "synthetic",
                        }
                    ],
                }
            ),
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_checker(self, *extra: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                str(CHECKER),
                "--contract",
                str(self.contract),
                "--root",
                str(self.root),
                *extra,
            ],
            check=False,
            capture_output=True,
            text=True,
        )

    def test_metadata_only_does_not_parse_environment_file(self) -> None:
        self.secret.write_bytes(b"\xff")
        result = self.run_checker()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("metadata only", result.stdout)

    def test_explicit_name_check_passes_without_emitting_value(self) -> None:
        result = self.run_checker("--check-env-names")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("synthetic-value", result.stdout + result.stderr)

    def test_missing_name_and_wrong_mode_fail_closed(self) -> None:
        self.secret.write_text("SOMETHING_ELSE=synthetic-value\n", encoding="utf-8")
        self.secret.chmod(0o644)
        result = self.run_checker("--check-env-names")
        self.assertEqual(result.returncode, 1)
        self.assertIn("wrong mode", result.stderr)
        self.assertIn("missing required variable REQUIRED_KEY", result.stderr)
        self.assertNotIn("synthetic-value", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()

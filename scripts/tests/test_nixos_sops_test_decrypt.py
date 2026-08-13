from __future__ import annotations

import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest


REPO_ROOT = Path(__file__).resolve().parents[2]
HELPER = REPO_ROOT / "scripts/nixos_sops_test_decrypt.py"


class NixosSopsTestDecryptTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.secrets_root = self.root / "secrets"
        self.secrets_root.mkdir()
        (self.secrets_root / ".sops.yaml").write_text(
            "creation_rules:\n  - path_regex: shared/.*\\.env$\n    age: age1synthetic\n",
            encoding="utf-8",
        )
        (self.secrets_root / "README.md").write_text("not a secret\n", encoding="utf-8")
        shared = self.secrets_root / "shared"
        shared.mkdir()
        self.sops_file = shared / "demo.env"
        self.write_sops_file(self.sops_file)
        (shared / "plain.env").write_text(
            "DEMO_TOKEN=synthetic-secret-value\n", encoding="utf-8"
        )
        self.calls = self.root / "calls"
        self.fake_sops = self.root / "fake-sops"
        self.fake_sops.write_text(
            "\n".join(
                [
                    "#!/usr/bin/env python3",
                    "import os",
                    "from pathlib import Path",
                    "import sys",
                    "calls = Path(os.environ['FAKE_SOPS_CALLS'])",
                    "with calls.open('a', encoding='utf-8') as file:",
                    "    file.write(' '.join(sys.argv[1:]) + '\\n')",
                    "fail_on = os.environ.get('FAKE_SOPS_FAIL_ON')",
                    "if fail_on and sys.argv[-1].endswith(fail_on):",
                    "    raise SystemExit(1)",
                    "sys.stdout.write('synthetic-decrypted-value\\n')",
                ]
            ),
            encoding="utf-8",
        )
        self.fake_sops.chmod(
            self.fake_sops.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def write_sops_file(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            json.dumps(
                {
                    "data": "ENC[AES256_GCM,data:fake,type:str]",
                    "sops": {"age": [{"recipient": "age1synthetic"}]},
                }
            ),
            encoding="utf-8",
        )

    def run_helper(self, fail_on: str = "") -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(HELPER),
                "--secrets-root",
                str(self.secrets_root),
                "--sops-bin",
                str(self.fake_sops),
            ],
            check=False,
            capture_output=True,
            text=True,
            env={
                **os.environ,
                "FAKE_SOPS_CALLS": str(self.calls),
                "FAKE_SOPS_FAIL_ON": fail_on,
            },
        )

    def test_reports_true_when_all_existing_files_decrypt(self) -> None:
        result = self.run_helper()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(result.stdout.startswith("true\n"))
        self.assertIn("can decrypt all existing NixOS SOPS secret files", result.stdout)
        combined = result.stdout + result.stderr
        self.assertNotIn("synthetic-secret-value", combined)
        self.assertNotIn("synthetic-decrypted-value", combined)
        calls = self.calls.read_text(encoding="utf-8")
        self.assertIn("decrypt --input-type json --output-type binary", calls)
        self.assertIn(str(self.sops_file), calls)
        self.assertNotIn("plain.env", calls)

    def test_reports_false_when_a_file_cannot_decrypt(self) -> None:
        result = self.run_helper(fail_on="demo.env")
        self.assertEqual(result.returncode, 1)
        self.assertTrue(result.stdout.startswith("false\n"))
        self.assertIn("cannot decrypt existing NixOS SOPS secrets", result.stdout)
        self.assertIn("just nixos-sops-operator-key", result.stdout)
        self.assertIn("just nixos-sops-updatekeys", result.stdout)
        combined = result.stdout + result.stderr
        self.assertNotIn("synthetic-secret-value", combined)
        self.assertNotIn("synthetic-decrypted-value", combined)

    def test_reports_true_when_no_sops_files_exist(self) -> None:
        self.sops_file.unlink()
        result = self.run_helper()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(result.stdout.startswith("true\n"))
        self.assertIn("nothing to decrypt yet", result.stdout)
        self.assertFalse(self.calls.exists())

    def test_reports_false_when_sops_config_is_missing(self) -> None:
        (self.secrets_root / ".sops.yaml").unlink()
        result = self.run_helper()
        self.assertEqual(result.returncode, 2)
        self.assertTrue(result.stdout.startswith("false\n"))
        self.assertIn("SOPS recipients are not configured yet", result.stdout)
        self.assertFalse(self.calls.exists())


if __name__ == "__main__":
    unittest.main()

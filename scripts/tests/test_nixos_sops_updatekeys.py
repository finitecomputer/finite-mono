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
HELPER = REPO_ROOT / "scripts/nixos_sops_updatekeys.py"


class NixosSopsUpdateKeysTest(unittest.TestCase):
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
        self.sops_file.write_text(
            json.dumps(
                {
                    "data": "ENC[AES256_GCM,data:fake,type:str]",
                    "sops": {"age": []},
                }
            ),
            encoding="utf-8",
        )
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
                    "Path(os.environ['FAKE_SOPS_CALLS']).write_text(' '.join(sys.argv[1:]) + '\\n', encoding='utf-8')",
                ]
            ),
            encoding="utf-8",
        )
        self.fake_sops.chmod(
            self.fake_sops.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_helper(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(HELPER),
                "--secrets-root",
                str(self.secrets_root),
                "--sops-bin",
                str(self.fake_sops),
                *args,
            ],
            check=False,
            capture_output=True,
            text=True,
            env={
                **os.environ,
                "FAKE_SOPS_CALLS": str(self.calls),
            },
        )

    def test_updates_only_sops_json_files_without_printing_plaintext(self) -> None:
        result = self.run_helper()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("updated: ", result.stdout)
        self.assertIn("updated 1 SOPS file(s)", result.stdout)
        self.assertNotIn("synthetic-secret-value", result.stdout + result.stderr)
        call = self.calls.read_text(encoding="utf-8")
        self.assertIn("updatekeys --yes --input-type json", call)
        self.assertIn(str(self.sops_file), call)
        self.assertNotIn("plain.env", call)

    def test_dry_run_does_not_call_sops(self) -> None:
        result = self.run_helper("--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("would update 1 SOPS file(s)", result.stdout)
        self.assertFalse(self.calls.exists())

    def test_rejects_explicit_non_sops_file(self) -> None:
        result = self.run_helper("shared/plain.env")
        self.assertEqual(result.returncode, 2)
        self.assertIn("is not a SOPS JSON file", result.stderr)

    def test_requires_sops_config(self) -> None:
        (self.secrets_root / ".sops.yaml").unlink()
        result = self.run_helper()
        self.assertEqual(result.returncode, 2)
        self.assertIn("missing", result.stderr)


if __name__ == "__main__":
    unittest.main()

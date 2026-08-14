from __future__ import annotations

import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest


REPO_ROOT = Path(__file__).resolve().parents[2]
HELPER = REPO_ROOT / "scripts/nixos_sops_operator_key.py"
PUBLIC_RECIPIENT = "age1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqs7q4eu"
PRIVATE_KEY = "AGE-SECRET-KEY-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ"


class NixosSopsOperatorKeyTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.key_file = self.root / "config/sops/age/keys.txt"
        self.fake_age_keygen = self.root / "fake-age-keygen"
        self.fake_age_keygen.write_text(
            "\n".join(
                [
                    "#!/usr/bin/env python3",
                    "from pathlib import Path",
                    "import sys",
                    f"public = {PUBLIC_RECIPIENT!r}",
                    f"private = {PRIVATE_KEY!r}",
                    "if sys.argv[1] == '-o':",
                    "    Path(sys.argv[2]).write_text(private + '\\n', encoding='utf-8')",
                    "elif sys.argv[1] == '-y':",
                    "    print(public)",
                    "else:",
                    "    raise SystemExit(2)",
                ]
            ),
            encoding="utf-8",
        )
        self.fake_age_keygen.chmod(
            self.fake_age_keygen.stat().st_mode
            | stat.S_IXUSR
            | stat.S_IXGRP
            | stat.S_IXOTH
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_helper(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(HELPER),
                "--key-file",
                str(self.key_file),
                "--age-keygen-bin",
                str(self.fake_age_keygen),
            ],
            check=False,
            capture_output=True,
            text=True,
            env=os.environ,
        )

    def test_creates_key_with_private_permissions_and_prints_public_only(self) -> None:
        result = self.run_helper()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.key_file.read_text(encoding="utf-8"), PRIVATE_KEY + "\n")
        self.assertEqual(stat.S_IMODE(self.key_file.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(self.key_file.parent.stat().st_mode), 0o700)
        self.assertIn("operator age key: created", result.stdout)
        self.assertIn(f"public recipient: {PUBLIC_RECIPIENT}", result.stdout)
        self.assertNotIn(PRIVATE_KEY, result.stdout + result.stderr)

    def test_existing_key_is_not_overwritten(self) -> None:
        self.key_file.parent.mkdir(parents=True)
        self.key_file.write_text("AGE-SECRET-KEY-1EXISTING\n", encoding="utf-8")
        result = self.run_helper()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            self.key_file.read_text(encoding="utf-8"), "AGE-SECRET-KEY-1EXISTING\n"
        )
        self.assertIn("operator age key: existing", result.stdout)


if __name__ == "__main__":
    unittest.main()

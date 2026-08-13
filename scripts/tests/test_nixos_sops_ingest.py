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
INGEST = REPO_ROOT / "scripts/nixos_sops_ingest.py"


class NixosSopsIngestTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.secrets_root = self.root / "secrets"
        self.secrets_root.mkdir()
        (self.secrets_root / ".sops.yaml").write_text(
            "creation_rules:\n  - path_regex: shared/.*\\.env$\n    age: age1synthetic\n",
            encoding="utf-8",
        )
        self.capture = self.root / "captured-stdin"
        self.fake_sops = self.root / "fake-sops"
        self.fake_sops.write_text(
            "\n".join(
                [
                    "#!/usr/bin/env python3",
                    "import json",
                    "import os",
                    "from pathlib import Path",
                    "import sys",
                    "if sys.argv[1] == 'decrypt' and os.environ.get('FAKE_SOPS_DECRYPT_FAIL') == '1':",
                    "    raise SystemExit(1)",
                    "if sys.argv[1] == 'decrypt':",
                    "    sys.stdin.buffer.read()",
                    "    sys.stdout.buffer.write(b'synthetic-decrypted-value\\n')",
                    "    raise SystemExit(0)",
                    "payload = sys.stdin.buffer.read()",
                    "Path(os.environ['FAKE_SOPS_CAPTURE']).write_bytes(payload)",
                    "recipients = [r for r in os.environ['FAKE_SOPS_RECIPIENTS'].split(',') if r]",
                    "age = [{'recipient': recipient} for recipient in recipients]",
                    "sys.stdout.write(json.dumps({'data':'ENC[AES256_GCM,data:fake,type:str]','sops':{'age':age}}) + '\\n')",
                ]
            ),
            encoding="utf-8",
        )
        self.fake_sops.chmod(
            self.fake_sops.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_ingest(
        self,
        *args: str,
        secret: bytes = b"synthetic-secret-value\n",
        decrypt_fail: bool = False,
        recipients: str = "age1canonical",
    ) -> subprocess.CompletedProcess[bytes]:
        return subprocess.run(
            [
                sys.executable,
                str(INGEST),
                "--secrets-root",
                str(self.secrets_root),
                "--sops-bin",
                str(self.fake_sops),
                *args,
            ],
            input=secret,
            check=False,
            capture_output=True,
            env={
                **os.environ,
                "FAKE_SOPS_CAPTURE": str(self.capture),
                "FAKE_SOPS_DECRYPT_FAIL": "1" if decrypt_fail else "0",
                "FAKE_SOPS_RECIPIENTS": recipients,
            },
        )

    def write_existing_sops_file(
        self, relative: str, recipients: list[str] | None = None
    ) -> Path:
        path = self.secrets_root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            json.dumps(
                {
                    "data": "ENC[AES256_GCM,data:existing,type:str]",
                    "sops": {
                        "age": [
                            {"recipient": recipient}
                            for recipient in (recipients or ["age1canonical"])
                        ]
                    },
                }
            ),
            encoding="utf-8",
        )
        return path

    def test_ingests_stdin_without_printing_plaintext(self) -> None:
        result = self.run_ingest(
            "shared",
            "metrics-remote-write.env",
            "--logical-name",
            "metrics-remote-write",
            "--required-env-name",
            "FINITE_METRICS_REMOTE_WRITE_USERNAME",
            "--required-env-name",
            "FINITE_METRICS_REMOTE_WRITE_PASSWORD",
            "--consumer",
            "alloy.service",
            "--restart-unit",
            "alloy.service",
        )
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        target = self.secrets_root / "shared/metrics-remote-write.env"
        self.assertTrue(target.exists())
        self.assertNotIn(b"synthetic-secret-value", target.read_bytes())
        self.assertEqual(self.capture.read_bytes(), b"synthetic-secret-value\n")
        combined = result.stdout + result.stderr
        self.assertNotIn(b"synthetic-secret-value", combined)
        self.assertIn(b'finite.secrets.files."metrics-remote-write"', result.stdout)
        self.assertIn(b"restartUnits = [ \"alloy.service\" ];", result.stdout)

    def test_refuses_to_overwrite_without_force(self) -> None:
        target = self.secrets_root / "shared/metrics-remote-write.env"
        target.parent.mkdir(parents=True)
        target.write_bytes(b"existing encrypted data\n")
        result = self.run_ingest("shared", "metrics-remote-write.env")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(target.read_bytes(), b"existing encrypted data\n")
        combined = result.stdout + result.stderr
        self.assertNotIn(b"synthetic-secret-value", combined)

    def test_refuses_to_write_when_operator_cannot_decrypt_result(self) -> None:
        result = self.run_ingest(
            "shared",
            "metrics-remote-write.env",
            decrypt_fail=True,
        )
        self.assertEqual(result.returncode, 1)
        self.assertFalse((self.secrets_root / "shared/metrics-remote-write.env").exists())
        combined = result.stdout + result.stderr
        self.assertIn(b"not decryptable by this operator", combined)
        self.assertNotIn(b"synthetic-secret-value", combined)

    def test_refuses_when_operator_cannot_decrypt_existing_sops_file(self) -> None:
        self.write_existing_sops_file("shared/existing.env")
        result = self.run_ingest(
            "shared",
            "metrics-remote-write.env",
            decrypt_fail=True,
        )
        self.assertEqual(result.returncode, 1)
        combined = result.stdout + result.stderr
        self.assertIn(b"cannot decrypt existing SOPS file", combined)
        self.assertIn(b"just nixos-sops-updatekeys", combined)
        self.assertFalse((self.secrets_root / "shared/metrics-remote-write.env").exists())
        self.assertNotIn(b"synthetic-secret-value", combined)

    def test_refuses_when_same_scope_recipients_are_stale(self) -> None:
        self.write_existing_sops_file("shared/existing.env", ["age1old"])
        result = self.run_ingest(
            "shared",
            "metrics-remote-write.env",
            recipients="age1new",
        )
        self.assertEqual(result.returncode, 1)
        combined = result.stdout + result.stderr
        self.assertIn(b"recipient set differs", combined)
        self.assertIn(b"just nixos-sops-updatekeys", combined)
        self.assertFalse((self.secrets_root / "shared/metrics-remote-write.env").exists())
        self.assertNotIn(b"synthetic-secret-value", combined)

    def test_allows_different_recipients_in_different_scope(self) -> None:
        self.write_existing_sops_file("finite-lat-1/existing.env", ["age1lat1"])
        result = self.run_ingest(
            "shared",
            "metrics-remote-write.env",
            recipients="age1shared",
        )
        self.assertEqual(result.returncode, 0, result.stderr.decode())

    def test_rejects_path_traversal(self) -> None:
        result = self.run_ingest("shared", "../bad.env")
        self.assertEqual(result.returncode, 2)
        self.assertIn(b"relative path without '..'", result.stderr)

    def test_requires_sops_config(self) -> None:
        (self.secrets_root / ".sops.yaml").unlink()
        result = self.run_ingest("shared", "metrics-remote-write.env")
        self.assertEqual(result.returncode, 2)
        self.assertIn(b"missing", result.stderr)


if __name__ == "__main__":
    unittest.main()

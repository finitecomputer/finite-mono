#!/usr/bin/env python3
"""Synthetic tests for the same-run Devfinity Nix closure handoff."""

from __future__ import annotations

import json
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HANDOFF = ROOT / "scripts" / "ci" / "devfinity-nix-handoff"
REVISION = "a" * 40
OTHER_REVISION = "b" * 40
STORE_ROOT = "/nix/store/" + "c" * 32 + "-devfinity"


class DevfinityNixHandoffTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.temp = Path(self.temporary.name)
        self.nix_log = self.temp / "nix.log"
        self.fake_nix = self.temp / "nix"
        self.fake_nix.write_text(
            textwrap.dedent(
                f"""\
                #!/usr/bin/env python3
                import os
                from pathlib import Path
                import sys

                arguments = sys.argv[1:]
                Path({str(self.nix_log)!r}).open("a", encoding="utf-8").write(" ".join(arguments) + "\\n")
                if "--to" in arguments:
                    cache = Path(arguments[arguments.index("--to") + 1].removeprefix("file://"))
                    cache.mkdir(parents=True, exist_ok=True)
                    (cache / "nix-cache-info").write_text("StoreDir: /nix/store\\n", encoding="utf-8")
                    (cache / "fake.narinfo").write_text("StorePath: {STORE_ROOT}\\n", encoding="utf-8")
                sys.exit(int(os.environ.get("FAKE_NIX_EXIT", "0")))
                """
            ),
            encoding="utf-8",
        )
        self.fake_nix.chmod(0o755)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_handoff(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(HANDOFF), *arguments, "--nix-command", str(self.fake_nix)],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )

    def create_artifact(self) -> Path:
        output = self.temp / "output"
        output.write_text(STORE_ROOT + "\n", encoding="utf-8")
        artifact = self.temp / "artifact"
        result = self.run_handoff(
            "create",
            "--artifact-dir",
            str(artifact),
            "--output-path-file",
            str(output),
            "--revision",
            REVISION,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return artifact

    def test_create_writes_strict_manifest_and_file_binary_cache(self) -> None:
        summary = self.temp / "summary"
        output = self.temp / "output"
        output.write_text(STORE_ROOT + "\n", encoding="utf-8")
        artifact = self.temp / "artifact"
        result = self.run_handoff(
            "create",
            "--artifact-dir",
            str(artifact),
            "--output-path-file",
            str(output),
            "--revision",
            REVISION,
            "--summary",
            str(summary),
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            json.loads((artifact / "manifest.json").read_text(encoding="utf-8")),
            {
                "schema": "finite.ci.devfinity-nix-handoff.v1",
                "repository": "finitecomputer/finite-mono",
                "rev": REVISION,
                "root": STORE_ROOT,
                "cache": "nix-cache",
            },
        )
        self.assertTrue((artifact / "nix-cache" / "nix-cache-info").is_file())
        self.assertIn("--to file://", self.nix_log.read_text(encoding="utf-8"))
        self.assertIn(REVISION, summary.read_text(encoding="utf-8"))

    def test_create_rejects_multiple_output_roots_before_nix(self) -> None:
        output = self.temp / "output"
        output.write_text(f"{STORE_ROOT}\n{STORE_ROOT}-other\n", encoding="utf-8")
        result = self.run_handoff(
            "create",
            "--artifact-dir",
            str(self.temp / "artifact"),
            "--output-path-file",
            str(output),
            "--revision",
            REVISION,
        )

        self.assertEqual(result.returncode, 65)
        self.assertIn("exactly one valid Nix store path", result.stderr)
        self.assertFalse(self.nix_log.exists())

    def test_restore_rejects_revision_mismatch_before_nix(self) -> None:
        artifact = self.create_artifact()
        self.nix_log.unlink()
        result = self.run_handoff(
            "restore",
            "--artifact-dir",
            str(artifact),
            "--revision",
            OTHER_REVISION,
        )

        self.assertEqual(result.returncode, 65)
        self.assertIn("handoff revision mismatch", result.stderr)
        self.assertFalse(self.nix_log.exists())

    def test_restore_requires_complete_file_binary_cache(self) -> None:
        artifact = self.create_artifact()
        (artifact / "nix-cache" / "nix-cache-info").unlink()
        self.nix_log.unlink()
        result = self.run_handoff(
            "restore",
            "--artifact-dir",
            str(artifact),
            "--revision",
            REVISION,
        )

        self.assertEqual(result.returncode, 66)
        self.assertIn("missing or incomplete", result.stderr)
        self.assertFalse(self.nix_log.exists())

    def test_restore_copies_exact_manifest_root_and_reports_it(self) -> None:
        artifact = self.create_artifact()
        self.nix_log.unlink()
        summary = self.temp / "summary"
        result = self.run_handoff(
            "restore",
            "--artifact-dir",
            str(artifact),
            "--revision",
            REVISION,
            "--summary",
            str(summary),
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        invocation = self.nix_log.read_text(encoding="utf-8")
        self.assertIn("--from file://", invocation)
        self.assertTrue(invocation.rstrip().endswith(STORE_ROOT), invocation)
        self.assertIn(
            "Restored the file binary cache", summary.read_text(encoding="utf-8")
        )

    def test_missing_artifact_fails_closed_before_nix(self) -> None:
        result = self.run_handoff(
            "restore",
            "--artifact-dir",
            str(self.temp / "missing"),
            "--revision",
            REVISION,
        )

        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact directory is missing", result.stderr)
        self.assertFalse(self.nix_log.exists())


if __name__ == "__main__":
    unittest.main()

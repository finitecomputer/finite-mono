#!/usr/bin/env python3
"""Synthetic checks for lat1 closure artifact deploy helpers."""

from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
DEPLOY = ROOT / "scripts" / "deploy-lat1-closure-cache"


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


if __name__ == "__main__":
    unittest.main()

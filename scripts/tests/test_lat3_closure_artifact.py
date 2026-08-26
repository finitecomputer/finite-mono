#!/usr/bin/env python3
"""Synthetic checks for the finite-lat-3 closure artifact rollout helpers."""

from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
BUILD = ROOT / "scripts/build-lat3-nixos-closure-artifact"
DEPLOY = ROOT / "scripts/deploy-lat3-closure-cache"


class Lat3ClosureArtifactTests(unittest.TestCase):
    def run_deploy(self, artifact_dir: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(DEPLOY), "--validate-only", str(artifact_dir)],
            cwd=ROOT,
            env={**os.environ, "PATH": os.environ["PATH"]},
            text=True,
            capture_output=True,
            check=False,
        )

    def test_missing_manifest_fails_before_remote_access(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            result = self.run_deploy(Path(temp))
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact manifest is missing", result.stderr)

    def test_manifest_schema_and_host_are_strict(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp)
            (artifact / "manifest.json").write_text(
                json.dumps(
                    {
                        "schema": "finite.lat1.nixos-closure.v1",
                        "host": "finite-lat-1",
                    }
                )
                + "\n",
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
                        "schema": "finite.lat3.nixos-closure.v1",
                        "host": "finite-lat-3",
                        "repository": "finite-co/finite-mono",
                        "rev": "a" * 40,
                        "system": "/nix/store/"
                        + "b" * 32
                        + "-nixos-system-finite-lat-3-26.05.test",
                        "cache": "nix-cache",
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            result = self.run_deploy(artifact)
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact cache is missing or incomplete", result.stderr)

    def test_build_is_exact_linux_lat3_system_only(self) -> None:
        source = BUILD.read_text(encoding="utf-8")
        self.assertIn(
            "nixosConfigurations.finite-lat-3.config.system.build.toplevel", source
        )
        self.assertIn('current_system" != "x86_64-linux', source)
        self.assertNotIn("diskoScript", source)

    def test_activation_is_fenced_and_rolls_back_the_profile(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        mutation = source.index('echo "==> mutation boundary:')
        self.assertLess(source.index("dry-activate"), mutation)
        self.assertLess(
            source.index("systemctl stop finite-saas-runner.timer"),
            mutation,
        )
        self.assertIn("previous_system", source)
        self.assertIn('switch-to-configuration" switch', source)
        self.assertIn("rollback", source)
        self.assertIn("systemctl start finite-saas-runner.timer", source)

    def test_activation_preserves_an_intentionally_inactive_timer(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        success = source.index('echo "==> DEPLOYED system=')
        self.assertIn(
            "else\n  systemctl stop finite-saas-runner.timer",
            source[:success],
        )

    def test_extra_units_require_explicit_cli_approval(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        self.assertIn("--allow-unit", source)
        self.assertIn("approved_extra_units", source)
        self.assertIn("invalid explicitly approved unit", source)
        self.assertNotIn(
            "alloy.service|dbus-broker.service|systemd-tmpfiles-resetup.service",
            source,
        )


if __name__ == "__main__":
    unittest.main()

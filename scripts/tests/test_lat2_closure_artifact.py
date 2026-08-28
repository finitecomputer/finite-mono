#!/usr/bin/env python3
"""Synthetic checks for the finite-lat-2 closure artifact rollout helpers."""

from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
BUILD = ROOT / "scripts/build-lat2-nixos-closure-artifact"
DEPLOY = ROOT / "scripts/deploy-lat2-closure-cache"
INSTALL = ROOT / "scripts/install-lat2-from-artifact"

VALID_MANIFEST = {
    "schema": "finite.lat2.nixos-closure.v2",
    "host": "finite-lat-2",
    "repository": "finitecomputer/finite-mono",
    "rev": "a" * 40,
    "system": "/nix/store/" + "b" * 32 + "-nixos-system-finite-lat-2-26.05.test",
    "disko": "/nix/store/" + "c" * 32 + "-disko",
    "kexec": "/nix/store/" + "d" * 32 + "-kexec-tarball",
    "cache": "nix-cache",
}


def write_manifest(artifact: Path, payload: dict) -> None:
    (artifact / "manifest.json").write_text(json.dumps(payload) + "\n", encoding="utf-8")


class Lat2ClosureArtifactTests(unittest.TestCase):
    def run_deploy(self, artifact_dir: Path, *flags: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(DEPLOY), *flags, str(artifact_dir)],
            cwd=ROOT,
            env={**os.environ, "PATH": os.environ["PATH"]},
            text=True,
            capture_output=True,
            check=False,
        )

    def run_install(self, artifact_dir: Path, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(INSTALL), str(artifact_dir), *args],
            cwd=ROOT,
            env={**os.environ, "PATH": os.environ["PATH"]},
            text=True,
            capture_output=True,
            check=False,
        )

    def test_missing_manifest_fails_before_remote_access(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            result = self.run_deploy(Path(temp), "--validate-only")
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact manifest is missing", result.stderr)
        with tempfile.TemporaryDirectory() as temp:
            result = self.run_install(Path(temp), "root@64.34.80.19", "--validate-only")
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact manifest is missing", result.stderr)

    def test_manifest_schema_and_host_are_strict(self) -> None:
        for wrong in (
            {"schema": "finite.lat3.nixos-closure.v1", "host": "finite-lat-3"},
            {"schema": "finite.lat2.nixos-closure.v2", "host": "finite-lat-3"},
        ):
            with tempfile.TemporaryDirectory() as temp:
                artifact = Path(temp)
                write_manifest(artifact, wrong)
                result = self.run_deploy(artifact, "--validate-only")
            self.assertNotEqual(result.returncode, 0)

    def test_uncaptured_storage_ids_block_the_build_script(self) -> None:
        # The build script must fail closed while finite-lat-2's storage
        # identity is still placeholder data (ADR 0007 Gate A/B precondition).
        ids = ROOT / "infra/nixos/hosts/finite-lat-2/storage-ids.nix"
        source = BUILD.read_text(encoding="utf-8")
        self.assertIn("grep -q '^  captured = true;$'", source)
        self.assertIn("captured = false", source)
        self.assertTrue(ids.exists())
        self.assertIn("captured = false", ids.read_text(encoding="utf-8"))

    def test_artifact_includes_the_bare_metal_install_inputs(self) -> None:
        source = BUILD.read_text(encoding="utf-8")
        self.assertIn("config.system.build.diskoScript", source)
        self.assertIn("finite-lat-2-kexec", source)
        self.assertIn('"disko": "$disko_path"', source)
        self.assertIn('"kexec": "$kexec_path"', source)
        self.assertEqual(source.count("nixosConfigurations.finite-lat-2."), 2)

    def test_valid_manifest_requires_file_binary_cache(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp)
            write_manifest(artifact, dict(VALID_MANIFEST))
            result = self.run_deploy(artifact, "--validate-only")
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact cache is missing or incomplete", result.stderr)
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp)
            write_manifest(artifact, dict(VALID_MANIFEST))
            result = self.run_install(artifact, "root@64.34.80.19", "--validate-only")
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact cache is missing or incomplete", result.stderr)

    def test_invalid_system_disko_or_kexec_paths_are_refused(self) -> None:
        cases = [
            ("system", "/nix/store/" + "b" * 32 + "-nixos-system-finite-lat-1-26.05.test"),
            ("disko", "/nix/store/" + "c" * 32 + "-something-else"),
            ("kexec", "/nix/store/" + "d" * 32 + "-disko"),
        ]
        for key, value in cases:
            with tempfile.TemporaryDirectory() as temp:
                artifact = Path(temp)
                payload = {
                    **VALID_MANIFEST,
                    key: value,
                }
                write_manifest(artifact, payload)
                result = self.run_deploy(artifact, "--validate-only")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(f"invalid {key} path", result.stderr)

    def test_deploy_is_an_app_plane_rollout_with_no_runner_machinery(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        # The app-plane host runs no Agent Runner: the rollout script must
        # not carry any runner-only rollout machinery.
        self.assertNotIn("finite-saas-runner.timer", source)
        self.assertNotIn("Runner-only", source)
        self.assertNotIn("ExecStart", source)
        # The absence guard is the only runner reference allowed.
        self.assertIn("finite-saas-runner.service exists on the app-plane host", source)

    def test_dry_activation_fences_the_declared_app_plane_unit_set(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        for unit in (
            "caddy.service",
            "finite-saas-core.service",
            "finitechat-server.service",
            "finitechat-hosted-device.service",
            "finite-saas-sites.service",
            "finite-brain-app.service",
            "finite-identity.service",
            "finite-litestream-finite-chat-server.service",
        ):
            # Declared-set membership: the unit appears as an array entry in
            # the app-plane fence, not merely as a string somewhere.
            self.assertIn(f"\n  {unit}\n", source)
        self.assertIn("--expect-startup", source)
        self.assertIn("unexpected_units", source)
        self.assertIn("refusing app-plane rollout", source)
        self.assertIn("--allow-unit", source)
        self.assertIn("approved_extra_units", source)

    def test_activation_is_fenced_and_rolls_back_the_profile(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        mutation = source.index('echo "==> mutation boundary:')
        self.assertLess(source.index("dry-activate"), mutation)
        self.assertIn("previous_system", source)
        self.assertIn("switch-to-configuration\" switch", source)
        self.assertIn("rollback", source)
        self.assertIn("refusing: finite-saas-runner.service exists on the app-plane host", source)

    def test_go_live_requires_product_health_after_the_switch(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        for unit in (
            "finite-saas-core.service",
            "finitechat-server.service",
            "caddy.service",
            "postgresql.service",
        ):
            self.assertIn(unit, source)
        self.assertIn("--expect-startup", source)
        self.assertIn("is not active after go-live activation", source)
        self.assertIn("is down after a steady-state switch", source)

    def test_extra_units_require_explicit_cli_approval(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        self.assertIn("--allow-unit", source)
        self.assertIn("approved_extra_units", source)
        self.assertIn("invalid explicitly approved unit", source)
        self.assertNotIn(
            "alloy.service|dbus-broker.service|systemd-tmpfiles-resetup.service",
            source,
        )

    def test_install_helper_realizes_from_cache_and_drives_pinned_nixos_anywhere(self) -> None:
        source = INSTALL.read_text(encoding="utf-8")
        self.assertIn('nix copy --option builders \'\'', source)
        self.assertIn('--from "file://$CACHE_DIR"', source)
        self.assertIn('"$SYSTEM" "$DISKO" "$KEXEC"', source)
        self.assertIn("--store-paths \"$SYSTEM\" \"$DISKO\"", source)
        self.assertIn("--kexec \"$KEXEC\"", source)
        self.assertIn("--build-on local", source)
        self.assertIn("packages.x86_64-linux.finite-lat-2-nixos-anywhere", source)
        # Substitution from the artifact cache only: no build invocation.
        self.assertNotIn("nix build", source)

    def test_capture_parser_peels_type_from_the_right(self) -> None:
        # lsblk MODEL fields can contain spaces (SAMSUNG MZQL21T9HCJR-00A07);
        # a fixed left-to-right split silently drops those disks and Gate B
        # would refuse healthy hardware (lat4 hit this for real).
        capture = (ROOT / "infra/nixos/scripts/capture-lat2-host-evidence").read_text(
            encoding="utf-8"
        )
        self.assertIn('rpartition(" ")', capture)
        self.assertNotIn("split(None, 4)", capture)


if __name__ == "__main__":
    unittest.main()

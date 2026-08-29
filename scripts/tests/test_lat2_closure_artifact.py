#!/usr/bin/env python3
"""Synthetic checks for the finite-lat-2 closure artifact rollout helpers."""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
import sys
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

    def test_captured_storage_ids_gate_the_build_script(self) -> None:
        # Gate B contract: the committed lat2 storage identity is captured
        # and placeholder-free, and the build guard that refuses an
        # uncaptured host is still in place (it fires again if the file
        # ever regresses or a future re-capture flips it).
        ids = ROOT / "infra/nixos/hosts/finite-lat-2/storage-ids.nix"
        source = BUILD.read_text(encoding="utf-8")
        self.assertIn("grep -q '^  captured = true;$'", source)
        self.assertTrue(ids.exists())
        ids_text = ids.read_text(encoding="utf-8")
        self.assertIn("captured = true;", ids_text)
        self.assertNotIn("captured = false", ids_text)
        self.assertNotIn("REPLACE-ME", ids_text)
        self.assertIn("/dev/disk/by-id/nvme-eui.", ids_text)
        for path in (ROOT / "infra/nixos/hosts/finite-lat-2").glob("*.nix"):
            self.assertNotIn("REPLACE-ME", path.read_text(encoding="utf-8"))

    def test_artifact_includes_the_bare_metal_install_inputs(self) -> None:
        source = BUILD.read_text(encoding="utf-8")
        # Partitioning uses the plain (unguarded-eval) disko script so the
        # build realizes exactly one disko derivation.
        self.assertIn("packages.x86_64-linux.finite-lat-2-disko", source)
        self.assertIn("finite-lat-2-kexec", source)
        self.assertIn('"disko": "$disko_path"', source)
        self.assertIn('"kexec": "$kexec_path"', source)
        self.assertEqual(source.count("nixosConfigurations.finite-lat-2."), 1)
        # No out-links: the artifact carries store paths, and each build
        # result is filtered to a single expected store path.
        self.assertNotIn("--out-link", source)
        self.assertNotIn("readlink", source)

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
        # The artifact cache is unsigned; the installer must read it with
        # the explicit --no-check-sigs path like every deploy-cache consumer.
        self.assertIn("--no-check-sigs", source)
        self.assertIn('nix copy --no-check-sigs --option builders \'\'', source)
        self.assertIn('--from "file://$CACHE_DIR"', source)
        self.assertIn('"$SYSTEM" "$DISKO" "$KEXEC"', source)
        self.assertIn("--store-paths \"$SYSTEM\" \"$DISKO\"", source)
        self.assertIn("--kexec \"$KEXEC\"", source)
        self.assertIn("--build-on local", source)
        self.assertIn("packages.x86_64-linux.finite-lat-2-nixos-anywhere", source)
        # Substitution from the artifact cache only: no build invocation.
        self.assertNotIn("nix build", source)

    def test_steady_state_requires_core_health_before_mutation(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        mutation = source.index('echo "==> mutation boundary:')
        gate = source.index("refusing steady-state switch")
        # The pre-mutation gate must exist and must run before the boundary.
        self.assertLess(gate, mutation)
        self.assertIn('"$expect_startup" != "1"', source)

    def test_app_plane_host_loads_kvm_for_the_kata_runtime(self) -> None:
        host = (ROOT / "infra/nixos/hosts/finite-lat-2/default.nix").read_text(
            encoding="utf-8"
        )
        self.assertIn('boot.kernelModules = [ "kvm-amd" ]', host)
        self.assertIn("kvm-amd kernel module", host)  # the assertion message

    def test_capture_by_id_filter_keeps_whole_namespaces_only(self) -> None:
        # The by-id listing must keep whole NVMe namespace symlinks (the EUI
        # identities Gate B commits) and drop -partN partitions and md-*
        # entries. Behavior-tested by running the script's own awk program
        # against a synthetic ls -l sample; the escaped-slash predecessor of
        # this filter matched nothing at all.
        capture = (ROOT / "infra/nixos/scripts/capture-lat2-host-evidence").read_text(
            encoding="utf-8"
        )
        match = re.search(r"awk '\$NF ~ /(.+?)/ \{print", capture)
        assert match, "by-id awk program not found in capture script"
        awk_program = match.group(1)

        sample = "\n".join(
            [
                "lrwxrwxrwx 1 root root 33 Aug 28 22:30 "
                "nvme-eui.000000000000000100a075244c213b3a -> ../../nvme0n1",
                "lrwxrwxrwx 1 root root 33 Aug 28 22:30 "
                "nvme-eui.3634473057c127620025385300000001 -> ../../nvme2n1",
                "lrwxrwxrwx 1 root root 14 Aug 28 22:30 "
                "nvme-eui.000000000000000100a075244c213b3a-part1 -> ../../nvme0n1p1",
                "lrwxrwxrwx 1 root root 13 Aug 28 22:30 "
                "md-uuid-3193acbf:4f88dd18:37a637dd:6220fcb6 -> ../../md126",
            ]
        )
        result = subprocess.run(
            ["awk", "$NF ~ /" + awk_program + "/ {print $9, \"->\", $NF}"],
            input=sample,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        kept = result.stdout.splitlines()
        self.assertEqual(len(kept), 2)
        self.assertIn("nvme-eui.000000000000000100a075244c213b3a", result.stdout)
        self.assertIn("nvme-eui.3634473057c127620025385300000001", result.stdout)
        self.assertNotIn("-part1", result.stdout)
        self.assertNotIn("md-uuid", result.stdout)

    def test_capture_parser_handles_spaced_model_names(self) -> None:
        # Behavior contract for the geometry proof: lsblk columns are
        # whitespace-padded and MODEL can contain spaces (SAMSUNG
        # MZQL21T9HCJR-00A07) — the parser must classify every disk by
        # splitting whitespace with TYPE taken from the right (lat4 hit the
        # fixed-split version of this bug: healthy disks silently dropped).
        capture = (ROOT / "infra/nixos/scripts/capture-lat2-host-evidence").read_text(
            encoding="utf-8"
        )
        match = re.search(r"<<'PY'\n(.*?)\nPY\n", capture, re.DOTALL)
        assert match, "geometry-proof python heredoc not found in capture script"
        parser_code = match.group(1)

        root_min = (935331839 + 1) * 512
        data_min = (3747612671 + 1) * 512
        sample = "\n".join(
            [
                "### block devices (bytes)",
                "nvme0n1  480103981056 Micron_7450_MTFDKBA480TFR  24454C213B3A   disk",
                "nvme1n1  480103981056 Micron_7450_MTFDKBA480TFR  24454C213BDD   disk",
                "nvme2n1 1920383410176 SAMSUNG MZQL21T9HCJR-00A07 S64GNS0WC12762 disk",
                "nvme3n1 1920383410176 SAMSUNG MZQL21T9HCJR-00A07 S64GNS0WC12751 disk",
                "### /dev/disk/by-id (whole disks only)",
            ]
        )
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as fh:
            fh.write(sample)
            sample_path = fh.name
        self.addCleanup(os.unlink, sample_path)
        result = subprocess.run(
            [sys.executable, "-", str(sample_path), str(root_min), str(data_min)],
            input=parser_code,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("found 4 disks", result.stdout)
        self.assertIn(f"root-class  (>= {root_min} bytes, < data): 2", result.stdout)
        self.assertIn(f"data-class  (>= {data_min} bytes): 2", result.stdout)
        self.assertIn("too small for the pinned root geometry: 0", result.stdout)
        self.assertIn("SAMSUNG MZQL21T9HCJR-00A07 S64GNS0WC12762", result.stdout)


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3
"""Synthetic checks for the finite-lat-4 closure artifact rollout helpers."""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
BUILD = ROOT / "scripts/build-lat4-nixos-closure-artifact"
DEPLOY = ROOT / "scripts/deploy-lat4-closure-cache"
INSTALL = ROOT / "scripts/install-lat4-from-artifact"


class Lat4ClosureArtifactTests(unittest.TestCase):
    def run_deploy(self, artifact_dir: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(DEPLOY), "--validate-only", str(artifact_dir)],
            cwd=ROOT,
            env={**os.environ, "PATH": os.environ["PATH"]},
            text=True,
            capture_output=True,
            check=False,
        )

    def run_install(
        self, artifact_dir: Path, *args: str
    ) -> subprocess.CompletedProcess[str]:
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
            result = self.run_deploy(Path(temp))
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact manifest is missing", result.stderr)
        with tempfile.TemporaryDirectory() as temp:
            result = self.run_install(
                Path(temp), "root@152.236.34.15", "--validate-only"
            )
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact manifest is missing", result.stderr)

    def test_manifest_schema_and_host_are_strict(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp)
            (artifact / "manifest.json").write_text(
                json.dumps(
                    {
                        "schema": "finite.lat3.nixos-closure.v1",
                        "host": "finite-lat-3",
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            result = self.run_deploy(Path(temp))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unexpected manifest schema", result.stderr)

    def test_captured_storage_ids_satisfy_the_build_guard(self) -> None:
        # finite-lat-4's storage identity was captured from the physical host
        # (docs/runs/lat4-provisioning-prep.md), so the committed file must be
        # captured and placeholder-free, while the build script still fails
        # closed if the captured flag regresses.
        ids = ROOT / "infra/nixos/hosts/finite-lat-4/storage-ids.nix"
        source = BUILD.read_text(encoding="utf-8")
        self.assertIn("grep -q '^  captured = true;$'", source)
        self.assertIn("captured = false", source)
        self.assertTrue(ids.exists())
        ids_text = ids.read_text(encoding="utf-8")
        self.assertIn("captured = true", ids_text)
        self.assertNotIn("captured = false", ids_text)
        self.assertNotIn("REPLACE-ME", ids_text)
        self.assertIn("/dev/disk/by-id/nvme-eui.", ids_text)

    def test_artifact_includes_the_bare_metal_install_inputs(self) -> None:
        source = BUILD.read_text(encoding="utf-8")
        self.assertIn("config.system.build.diskoScript", source)
        self.assertIn("finite-lat-4-kexec", source)
        self.assertIn('"disko": "$disko_path"', source)
        self.assertIn('"kexec": "$kexec_path"', source)
        self.assertEqual(source.count("nixosConfigurations.finite-lat-4."), 2)

    def test_build_script_compares_the_immediate_out_link_target(self) -> None:
        # nix build --out-link points directly at the printed out path, but
        # some outputs (e.g. the disko script) are themselves store symlinks.
        # `readlink -f` resolves through the WHOLE chain and fails the guard
        # on a perfectly valid build (real Gate B failure on 2026-08-29); the
        # guard must compare the out-link's immediate target.
        source = BUILD.read_text(encoding="utf-8")
        for link in ("system", "disko", "kexec"):
            self.assertIn(f'[[ "$(readlink "$out_dir/{link}")"', source)
        self.assertNotIn('readlink -f "$out_dir/', source)

    def test_valid_manifest_requires_file_binary_cache(self) -> None:
        def valid_manifest() -> dict:
            return {
                "schema": "finite.lat4.nixos-closure.v2",
                "host": "finite-lat-4",
                "repository": "finitecomputer/finite-mono",
                "rev": "a" * 40,
                "system": "/nix/store/"
                + "b" * 32
                + "-nixos-system-finite-lat-4-26.05.test",
                "disko": "/nix/store/" + "c" * 32 + "-disko",
                "kexec": "/nix/store/" + "d" * 32 + "-kexec-tarball",
                "cache": "nix-cache",
            }

        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp)
            (artifact / "manifest.json").write_text(
                json.dumps(valid_manifest()) + "\n", encoding="utf-8"
            )
            result = self.run_deploy(Path(temp))
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact cache is missing or incomplete", result.stderr)
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp)
            (artifact / "manifest.json").write_text(
                json.dumps(valid_manifest()) + "\n", encoding="utf-8"
            )
            result = self.run_install(artifact, "root@152.236.34.15", "--validate-only")
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact cache is missing or incomplete", result.stderr)

    def test_invalid_system_disko_or_kexec_paths_are_refused(self) -> None:
        base = {
            "schema": "finite.lat4.nixos-closure.v2",
            "host": "finite-lat-4",
            "repository": "finitecomputer/finite-mono",
            "rev": "a" * 40,
            "cache": "nix-cache",
        }
        cases = [
            (
                "system",
                "/nix/store/" + "b" * 32 + "-nixos-system-finite-lat-1-26.05.test",
            ),
            ("disko", "/nix/store/" + "c" * 32 + "-something-else"),
            ("kexec", "/nix/store/" + "d" * 32 + "-disko"),
        ]
        for key, value in cases:
            with tempfile.TemporaryDirectory() as temp:
                artifact = Path(temp)
                payload = {
                    **base,
                    "system": "/nix/store/"
                    + "b" * 32
                    + "-nixos-system-finite-lat-4-26.05.test",
                    "disko": "/nix/store/" + "c" * 32 + "-disko",
                    "kexec": "/nix/store/" + "d" * 32 + "-kexec-tarball",
                    key: value,
                }
                (artifact / "manifest.json").write_text(
                    json.dumps(payload) + "\n", encoding="utf-8"
                )
                result = self.run_deploy(Path(temp))
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(f"invalid {key} path", result.stderr)

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

    def test_install_helper_realizes_from_cache_and_drives_pinned_nixos_anywhere(
        self,
    ) -> None:
        source = INSTALL.read_text(encoding="utf-8")
        # The artifact cache is unsigned; the installer must read it with
        # the explicit --no-check-sigs path like every deploy-cache consumer.
        self.assertIn("--no-check-sigs", source)
        self.assertIn("nix copy --no-check-sigs --option builders ''", source)
        self.assertIn('--from "file://$CACHE_DIR"', source)
        self.assertIn('"$SYSTEM" "$DISKO" "$KEXEC"', source)
        self.assertIn('--store-paths "$SYSTEM" "$DISKO"', source)
        self.assertIn('--kexec "$KEXEC"', source)
        self.assertIn("--build-on local", source)
        self.assertIn("packages.x86_64-linux.finite-lat-4-nixos-anywhere", source)
        # Substitution from the artifact cache only: no build invocation.
        self.assertNotIn("nix build", source)

    def test_capture_parser_peels_type_from_the_right(self) -> None:
        # lsblk MODEL fields can contain spaces (SAMSUNG MZQL21T9HCJR-00A07);
        # a fixed left-to-right split silently drops those disks and Gate B
        # would refuse healthy hardware (this hit lat4 for real).
        capture = (ROOT / "infra/nixos/scripts/capture-lat4-host-evidence").read_text(
            encoding="utf-8"
        )
        self.assertIn('rpartition(" ")', capture)
        self.assertNotIn("split(None, 4)", capture)

    def test_by_id_filter_selects_whole_nvme_disk_targets(self) -> None:
        # In `ls -l /dev/disk/by-id` output the symlink TARGET ends the line,
        # so a "does not end in a digit" filter drops every whole NVMe disk
        # (../../nvme0n1 ends in a digit) and the capture cannot prove the
        # four committed nvme-eui identities before the destructive Gate A/C
        # wipe. The filter must select whole-disk targets (nvmeNn1) and
        # exclude partitions (nvme0n1p1).
        capture = (ROOT / "infra/nixos/scripts/capture-lat4-host-evidence").read_text(
            encoding="utf-8"
        )
        match = re.search(r"by-id 2>/dev/null \| grep -E '([^']*)'", capture)
        self.assertIsNotNone(match, "by-id grep filter not found in capture script")
        pattern = match.group(1)
        keep = [
            "lrwxrwxrwx 1 root root 10 Aug 28 20:00 nvme-eui.305f88210a0b1c2d -> ../../nvme0n1",
            "lrwxrwxrwx 1 root root 10 Aug 28 20:00 nvme-eui.305f88210a0b1c2e -> ../../nvme1n1",
            "lrwxrwxrwx 1 root root 10 Aug 28 20:00 nvme-eui.305f88210a0b1c2f -> ../../nvme2n1",
            "lrwxrwxrwx 1 root root 10 Aug 28 20:00 nvme-eui.305f88210a0b1c30 -> ../../nvme3n1",
        ]
        drop = [
            "lrwxrwxrwx 1 root root 10 Aug 28 20:00 nvme-eui.305f88210a0b1c2d-part1 -> ../../nvme0n1p1",
            "lrwxrwxrwx 1 root root 10 Aug 28 20:00 nvme-eui.305f88210a0b1c2d-part2 -> ../../nvme0n1p2",
        ]
        kept = [line for line in keep + drop if re.search(pattern, line)]
        self.assertEqual(kept, keep)
        # The filter must actually surface the nvme-eui.* identities that
        # Gate A compares against infra/nixos/hosts/finite-lat-4/storage-ids.nix.
        self.assertIn("nvme-eui.", " ".join(kept))

    def test_runbook_gate_a_cross_checks_committed_storage_ids(self) -> None:
        # The capture only proves the identities if Gate A says to compare
        # the emitted by-id names against the committed storage-ids file.
        # This test is deleted together with the runbook when Gate F closes.
        runbook = (ROOT / "infra/runbooks/lat4-nixos-runner-install.md").read_text(
            encoding="utf-8"
        )
        self.assertIn("storage-ids.nix", runbook)
        self.assertIn("capture-lat4-host-evidence", runbook)

    def test_runbook_gate_f_uses_the_exact_relocation_contract(self) -> None:
        # Gate F's binding move must go through the existing
        # runtime_relocation.v1 transaction (runtime-cold-relocate-exact with
        # the absent-compute variant, per-Runtime), never a bulk Core
        # source_host_id edit: Core replaces a binding only after the target
        # Runner proves the staged state, and a broad update would commit
        # chat routing for unverified Runtimes.
        runbook = (ROOT / "infra/runbooks/lat4-nixos-runner-install.md").read_text(
            encoding="utf-8"
        )
        self.assertIn("runtime-cold-relocate-exact", runbook)
        self.assertIn("--source-compute-absent", runbook)
        self.assertIn("runtime_relocation.v1", runbook)
        self.assertIn("migrated-runtimes.manifest", runbook)
        self.assertNotRegex(runbook, r"UPDATE\s+\w+.*SET.*source_host_id")
        self.assertNotIn(
            "bulk binding change", runbook.replace("no bulk binding change", "")
        )


if __name__ == "__main__":
    unittest.main()

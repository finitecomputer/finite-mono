#!/usr/bin/env python3
"""Synthetic checks for the finite-lat-5 closure artifact rollout helpers."""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
BUILD = ROOT / "scripts/build-lat5-nixos-closure-artifact"
DEPLOY = ROOT / "scripts/deploy-lat5-closure-cache"
INSTALL = ROOT / "scripts/install-lat5-from-artifact"


class Lat5ClosureArtifactTests(unittest.TestCase):
    def run_deploy(self, artifact_dir: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(DEPLOY), "--validate-only", str(artifact_dir)],
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
            result = self.run_deploy(Path(temp))
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact manifest is missing", result.stderr)
        with tempfile.TemporaryDirectory() as temp:
            result = self.run_install(Path(temp), "root@64.34.93.213", "--validate-only")
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
        # finite-lat-5's storage identity was captured from the physical host
        # (docs/runs/lat5-provisioning-prep.md), so the committed file must be
        # captured and placeholder-free, while the build script still fails
        # closed if the captured flag regresses.
        ids = ROOT / "infra/nixos/hosts/finite-lat-5/storage-ids.nix"
        source = BUILD.read_text(encoding="utf-8")
        self.assertIn('grep -q \'^  captured = true;$\'', source)
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
        self.assertIn("finite-lat-5-kexec", source)
        self.assertIn('"disko": "$disko_path"', source)
        self.assertIn('"kexec": "$kexec_path"', source)
        self.assertEqual(source.count("nixosConfigurations.finite-lat-5."), 2)

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
                "schema": "finite.lat5.nixos-closure.v2",
                "host": "finite-lat-5",
                "repository": "finitecomputer/finite-mono",
                "rev": "a" * 40,
                "system": "/nix/store/"
                + "b" * 32
                + "-nixos-system-finite-lat-5-26.05.test",
                "disko": "/nix/store/" + "c" * 32 + "-disko",
                "kexec": "/nix/store/" + "d" * 32 + "-kexec-tarball",
                "cache": "nix-cache",
                "installer": "/nix/store/" + "e" * 32 + "-nixos-anywhere-1.0.0",
                "qualification": "/nix/store/" + "f" * 32 + "-vm-test-run-disko-lat5-storage-boot",
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
            result = self.run_install(artifact, "root@64.34.93.213", "--validate-only")
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact cache is missing or incomplete", result.stderr)

    def test_invalid_system_disko_or_kexec_paths_are_refused(self) -> None:
        base = {
            "schema": "finite.lat5.nixos-closure.v2",
            "host": "finite-lat-5",
            "repository": "finitecomputer/finite-mono",
            "rev": "a" * 40,
            "cache": "nix-cache",
                "installer": "/nix/store/" + "e" * 32 + "-nixos-anywhere-1.0.0",
                "qualification": "/nix/store/" + "f" * 32 + "-vm-test-run-disko-lat5-storage-boot",
        }
        cases = [
            ("system", "/nix/store/" + "b" * 32 + "-nixos-system-finite-lat-1-26.05.test"),
            ("disko", "/nix/store/" + "c" * 32 + "-something-else"),
            ("kexec", "/nix/store/" + "d" * 32 + "-disko"),
        ]
        for key, value in cases:
            with tempfile.TemporaryDirectory() as temp:
                artifact = Path(temp)
                payload = {
                    **base,
                    "system": "/nix/store/" + "b" * 32 + "-nixos-system-finite-lat-5-26.05.test",
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
        self.assertIn("switch-to-configuration\" switch", source)
        self.assertIn("rollback", source)
        self.assertIn("systemctl start finite-saas-runner.timer", source)

    def test_activation_preserves_an_intentionally_inactive_timer(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        success = source.index('echo "==> DEPLOYED system=')
        self.assertIn(
            'else\n  systemctl stop finite-saas-runner.timer',
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

    def test_install_helper_realizes_from_cache_and_drives_pinned_nixos_anywhere(self) -> None:
        source = INSTALL.read_text(encoding="utf-8")
        # The artifact cache is unsigned; the installer must read it with
        # the explicit --no-check-sigs path like every deploy-cache consumer.
        self.assertIn("--no-check-sigs", source)
        self.assertIn('nix copy --no-check-sigs --option builders \'\'', source)
        self.assertIn('--from "file://$CACHE_DIR"', source)
        self.assertIn('"$SYSTEM" "$DISKO" "$KEXEC"', source)
        self.assertIn("--store-paths \"$DISKO\" \"$SYSTEM\"", source)
        self.assertIn('--kexec "${kexec_tarballs[0]}"', source)
        self.assertIn("--build-on local", source)
        self.assertIn('"$INSTALLER/bin/nixos-anywhere"', source)
        self.assertNotIn("nix run", source)
        self.assertNotIn("NIXOS_ANYWHERE_CMD", source)
        # Substitution from the artifact cache only: no build invocation.
        self.assertNotIn("nix build", source)

    def test_capture_parser_peels_type_from_the_right(self) -> None:
        # lsblk MODEL fields can contain spaces (SAMSUNG MZQL21T9HCJR-00A07);
        # a fixed left-to-right split silently drops those disks and Gate B
        # would refuse healthy hardware (this hit lat5 for real).
        capture = (ROOT / "infra/nixos/scripts/capture-lat5-host-evidence").read_text(
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
        capture = (ROOT / "infra/nixos/scripts/capture-lat5-host-evidence").read_text(
            encoding="utf-8"
        )
        match = re.search(r"by-id 2>/dev/null \| grep -E '([^']*)'", capture)
        self.assertIsNotNone(match, "by-id grep filter not found in capture script")
        pattern = match.group(1)
        keep = [
            "lrwxrwxrwx 1 root root 10 Sep 15 20:00 ata-Micron_5400_disk-a -> ../../sda",
            "lrwxrwxrwx 1 root root 10 Sep 15 20:00 ata-Micron_5400_disk-b -> ../../sdb",
            "lrwxrwxrwx 1 root root 10 Aug 28 20:00 nvme-eui.305f88210a0b1c2d -> ../../nvme0n1",
            "lrwxrwxrwx 1 root root 10 Aug 28 20:00 nvme-eui.305f88210a0b1c2e -> ../../nvme1n1",
            "lrwxrwxrwx 1 root root 10 Aug 28 20:00 nvme-eui.305f88210a0b1c2f -> ../../nvme2n1",
            "lrwxrwxrwx 1 root root 10 Aug 28 20:00 nvme-eui.305f88210a0b1c30 -> ../../nvme3n1",
        ]
        drop = [
            "lrwxrwxrwx 1 root root 10 Sep 15 20:00 ata-Micron_5400_disk-a-part1 -> ../../sda1",
            "lrwxrwxrwx 1 root root 10 Aug 28 20:00 nvme-eui.305f88210a0b1c2d-part1 -> ../../nvme0n1p1",
            "lrwxrwxrwx 1 root root 10 Aug 28 20:00 nvme-eui.305f88210a0b1c2d-part2 -> ../../nvme0n1p2",
        ]
        kept = [line for line in keep + drop if re.search(pattern, line)]
        self.assertEqual(kept, keep)
        # The filter must actually surface the nvme-eui.* identities that
        # Gate A compares against infra/nixos/hosts/finite-lat-5/storage-ids.nix.
        self.assertIn("nvme-eui.", " ".join(kept))


    def test_extra_files_option_is_parsed_without_forwarding_arbitrary_flags(self):
        with tempfile.TemporaryDirectory() as temp:
            result = self.run_install(Path(temp), "ubuntu@64.34.93.213", "--extra-files", temp)
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact manifest is missing", result.stderr)
        with tempfile.TemporaryDirectory() as temp:
            result = self.run_install(Path(temp), "ubuntu@64.34.93.213", "--target-host", "root@64.34.82.77")
        self.assertEqual(result.returncode, 64)

    def test_wrong_physical_target_is_rejected_before_artifact_access(self):
        with tempfile.TemporaryDirectory() as temp:
            result = self.run_install(Path(temp), "root@64.34.82.77")
        self.assertEqual(result.returncode, 64)
        self.assertIn("unexpected lat5 install target", result.stderr)

    def test_geometry_fits_captured_disks_and_raid_members(self):
        host = ROOT / "infra/nixos/hosts/finite-lat-5"
        text = (host / "disko.nix").read_text()
        root_end = int(re.search(r'rootMember.*?end = "(\d+)s"', text, re.S)[1])
        data_end = int(re.search(r'dataMember.*?end = "(\d+)s"', text, re.S)[1])
        sizes = [int(n) for n in re.findall(r'--size=(\d+)K', text)]
        for end, start, size, raw in [(root_end, 2099200, sizes[0], 480103981056),
                                      (data_end, 2048, sizes[1], 7681501126656)]:
            self.assertLessEqual(end, raw // 512 - 34)
            self.assertEqual((end + 1) % 2048, 0)
            self.assertLess(size * 1024 + 1024 * 1024, (end - start + 1) * 512)
        ids = (host / "storage-ids.nix").read_text()
        fat_ids = re.findall(r'esp[AB] = "([0-9A-F]{4}-[0-9A-F]{4})"', ids)
        self.assertEqual(len(fat_ids), 2)
        self.assertNotEqual(*fat_ids)
        self.assertIn('/dev/disk/by-id/ata-', ids)



if __name__ == "__main__":
    unittest.main()

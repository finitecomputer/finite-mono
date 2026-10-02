#!/usr/bin/env python3
"""Synthetic checks for pre-pulling the candidate dashboard image (FIN-156).

The dashboard unit runs `podman run --pull missing`, so a new digest is
otherwise downloaded after the old container stops, inside the outage.
"""

from __future__ import annotations

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
PREPULL = ROOT / "infra/nixos/scripts/prepull-lat2-dashboard-image"
DEPLOY = ROOT / "scripts/deploy-lat2-closure-cache"
IMAGE = "ghcr.io/finitecomputer/finite-saas-dashboard@sha256:" + "e" * 64
OTHER_IMAGE = "ghcr.io/finitecomputer/finite-saas-dashboard@sha256:" + "f" * 64


def write_candidate_system(root: Path, start_script_body: str) -> Path:
    """Lay out the two closure files the helper reads: the dashboard unit and
    the start script its ExecStart names."""
    system = root / "system"
    units = system / "etc/systemd/system"
    units.mkdir(parents=True)
    start = root / "podman-finite-saas-dashboard-start"
    start.write_text(start_script_body, encoding="utf-8")
    (units / "podman-finite-saas-dashboard.service").write_text(
        f"[Service]\nExecStart={start} \nType=notify\n", encoding="utf-8"
    )
    return system


def write_podman_shim(bin_dir: Path, log: Path, present: bool) -> None:
    exists_status = 0 if present else 1
    shim = bin_dir / "podman"
    shim.write_text(
        "#!/usr/bin/env bash\n"
        f'printf "%s\\n" "$*" >> {log}\n'
        f'case "$1 $2" in "image exists") exit {exists_status} ;; esac\n'
        "exit 0\n",
        encoding="utf-8",
    )
    shim.chmod(0o755)


class PrepullDashboardImageTests(unittest.TestCase):
    def run_prepull(
        self, start_script_body: str, *, present: bool = False
    ) -> tuple[subprocess.CompletedProcess[str], list[str]]:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            system = write_candidate_system(root, start_script_body)
            shims = root / "bin"
            shims.mkdir()
            log = root / "podman.log"
            write_podman_shim(shims, log, present)
            result = subprocess.run(
                ["bash", str(PREPULL), str(system)],
                env={**os.environ, "PATH": f"{shims}{os.pathsep}{os.environ['PATH']}"},
                text=True,
                capture_output=True,
                check=False,
            )
            calls = log.read_text(encoding="utf-8").splitlines() if log.exists() else []
        return result, calls

    def test_pulls_the_digest_pinned_by_the_candidate_unit(self) -> None:
        result, calls = self.run_prepull(f"exec podman run --pull missing '{IMAGE}'\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [f"image exists {IMAGE}", f"pull {IMAGE}"])

    def test_skips_the_registry_when_the_digest_is_already_local(self) -> None:
        result, calls = self.run_prepull(f"exec podman run {IMAGE}\n", present=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [f"image exists {IMAGE}"])

    def test_refuses_when_the_unit_pins_no_digest(self) -> None:
        result, calls = self.run_prepull(
            "exec podman run ghcr.io/finitecomputer/finite-saas-dashboard:latest\n"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("exactly one pinned dashboard image", result.stderr)
        self.assertEqual(calls, [])

    def test_refuses_an_ambiguous_unit(self) -> None:
        result, calls = self.run_prepull(f"podman run {IMAGE}\npodman run {OTHER_IMAGE}\n")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("exactly one pinned dashboard image", result.stderr)
        self.assertEqual(calls, [])

    def test_deploy_prepulls_after_the_fence_in_both_modes(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        fence = source.index("refusing app-plane rollout")
        prepull = source.index("infra/nixos/scripts/prepull-lat2-dashboard-image")
        prepared_exit = source.index("==> PREPARED rev=")
        activation = source.index("==> activating the fenced app-plane closure")
        # After the fence refuses unexpected units, before --prepare exits,
        # so --prepare and --activate both pull before any switch.
        self.assertLess(fence, prepull)
        self.assertLess(prepull, prepared_exit)
        self.assertLess(prepared_exit, activation)


if __name__ == "__main__":
    unittest.main()

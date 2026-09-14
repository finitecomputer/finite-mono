"""Exercise remote access installation on a synthetic host filesystem."""

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    "install_ci_access", Path(__file__).resolve().parents[1] / "install_ci_access.py"
)
installer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(installer)


class AccessInstallationTest(unittest.TestCase):
    def test_preserves_operator_keys_replaces_only_ci_key_and_backs_up(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            home = root / "home"
            (home / ".ssh").mkdir(parents=True)
            auth = home / ".ssh/authorized_keys"
            operator = "ssh-ed25519 operator-bytes operator"
            auth.write_text(operator + "\nssh-ed25519 old-ci finite-monitoring-ci\n")
            backups = root / "backups"
            # Real remote code and file operations; map privileged host paths
            # into the fixture and emulate ownership on an unprivileged runner.
            setup = f"""
import os, pwd, types
pwd.getpwnam = lambda _: types.SimpleNamespace(pw_dir={str(home)!r}, pw_uid=os.getuid(), pw_gid=os.getgid())
os.geteuid = lambda: 0
os.chown = lambda *args: None
os.fchown = lambda *args: None
"""
            program = setup + installer.INSTALL.replace(
                "/run/lock/finite-monitoring-dashboards.lock", str(root / "lock")
            ).replace("/var/lib/finite-monitoring-ci", str(root / "installed")).replace(
                    "/var/backups", str(backups)
            )
            payload = {
                "user": "synthetic",
                "authorized_key": 'restrict,command="fixed" ssh-ed25519 new-ci finite-monitoring-ci',
                "files": {"ci_dispatch": "first helper\n"},
            }
            for content in ("first helper\n", "updated helper\n"):
                payload["files"]["ci_dispatch"] = content
                subprocess.run(
                    [sys.executable, "-c", program],
                    input=json.dumps(payload),
                    text=True,
                    check=True,
                    stdout=subprocess.DEVNULL,
                )
            self.assertEqual(
                auth.read_text().splitlines(), [operator, payload["authorized_key"]]
            )
            self.assertEqual(auth.stat().st_mode & 0o777, 0o600)
            self.assertEqual(
                (root / "installed/ci_dispatch").read_text(), "updated helper\n"
            )
            self.assertEqual(
                (root / "installed/ci_dispatch").stat().st_mode & 0o777, 0o755
            )
            self.assertEqual(len(list(backups.iterdir())), 2)
            self.assertTrue(
                any(
                    "old-ci" in (p / "authorized_keys").read_text()
                    for p in backups.iterdir()
                )
            )
            self.assertTrue(
                any((p / "ci_dispatch").exists() for p in backups.iterdir())
            )
            # Existing symlinks are rejected before the pointed-to file changes.
            auth.unlink()
            victim = root / "victim"
            victim.write_text("untouched")
            auth.symlink_to(victim)
            result = subprocess.run(
                [sys.executable, "-c", program],
                input=json.dumps(payload),
                text=True,
                capture_output=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(victim.read_text(), "untouched")


if __name__ == "__main__":
    unittest.main()

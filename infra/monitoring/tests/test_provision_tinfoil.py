"""Initial provisioning rejects ambiguous ownership and restores file absence."""

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
import urllib.error

MONITORING = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(MONITORING))
SPEC = importlib.util.spec_from_file_location(
    "provision_tinfoil", MONITORING / "provision_tinfoil.py"
)
provision = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(provision)


def absent(uid):
    raise urllib.error.HTTPError("test", 404, "absent", {}, None)


class ProvisionTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.directory = self.root / "dashboards"
        self.directory.mkdir()
        self.provider = self.root / "provider"
        self.provider.write_text("synthetic provider")
        self.bundle = {
            "schema_version": 1,
            "revision": "a" * 40,
            "helper_sha256": provision.dashboards.digest(
                Path(provision.dashboards.__file__).read_bytes()
            ),
            "provider_sha256": provision.dashboards.digest(self.provider.read_bytes()),
            "files": {
                provision.NAME: {
                    "uid": provision.UID,
                    "content": json.dumps(
                        {
                            "uid": provision.UID,
                            "title": "Tinfoil",
                            "panels": [{"id": 1}],
                        }
                    ),
                }
            },
        }
        self.kwargs = dict(
            directory=self.directory,
            provider=self.provider,
            fetch=absent,
            evidence=lambda: {"overall_status": "green"},
        )

    def test_success_verifies_and_records_receipt(self):
        seen = []
        provision.provision(
            self.bundle,
            **self.kwargs,
            backups=self.root / "backups",
            verify=lambda *args: seen.append(args),
        )
        self.assertTrue((self.directory / provision.NAME).is_file())
        self.assertEqual(len(seen), 1)
        self.assertEqual(len(list((self.root / "backups").glob("*/after.json"))), 1)

    def test_reload_failure_restores_absence(self):
        def fail(*args):
            raise RuntimeError("reload failed")

        with self.assertRaises(RuntimeError):
            provision.provision(
                self.bundle, **self.kwargs, backups=self.root / "backups", verify=fail
            )
        self.assertFalse((self.directory / provision.NAME).exists())

    def test_uid_in_other_file_is_rejected(self):
        (self.directory / "other.json").write_text(json.dumps({"uid": provision.UID}))
        with self.assertRaisesRegex(ValueError, "another file"):
            provision.preflight(self.bundle, **self.kwargs)

    def test_existing_grafana_uid_is_rejected(self):
        self.kwargs["fetch"] = lambda uid: {"dashboard": {"uid": uid}}
        with self.assertRaisesRegex(ValueError, "already exists"):
            provision.preflight(self.bundle, **self.kwargs)

    def test_transport_error_does_not_count_as_absence(self):
        def denied(uid):
            raise urllib.error.HTTPError("test", 401, "denied", {}, None)

        self.kwargs["fetch"] = denied
        with self.assertRaises(urllib.error.HTTPError):
            provision.preflight(self.bundle, **self.kwargs)

    def test_unknown_metrics_block_provisioning(self):
        self.kwargs["evidence"] = lambda: {"overall_status": "unknown"}
        with self.assertRaisesRegex(ValueError, "not ready"):
            provision.preflight(self.bundle, **self.kwargs)
        self.assertFalse((self.directory / provision.NAME).exists())

    def test_read_only_status_unknown_or_stale_fails_closed(self):
        # Exercise the canonical status module used by the provisioning gate.
        from finite_tinfoil_status import collect

        def good(q):
            return 0.0 if q.startswith("time()") else 1.0

        self.assertEqual(collect(good)["overall_status"], "green")
        self.assertEqual(
            collect(lambda q: 600.0 if q.startswith("time()") else good(q))[
                "overall_status"
            ],
            "red",
        )

        def missing(q):
            raise ValueError("missing series")

        self.assertEqual(collect(missing)["overall_status"], "unknown")


class BootstrapRollbackTest(unittest.TestCase):
    def test_exact_existing_files_and_absence_are_restored(self):
        import os
        import subprocess

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            backup = root / "backup"
            backup.mkdir()
            existing = root / "installed"
            existing.write_text("candidate")
            new = root / "new-file"
            new.write_text("candidate")
            saved = Path(str(backup) + str(existing))
            saved.parent.mkdir(parents=True)
            saved.write_text("previous")
            (backup / "files").write_text(f"{existing}\n{new}\n")
            units = (
                "finite-monitoring-tinfoil-collector.timer",
                "finite-monitoring-tinfoil-collector.service",
                "finite-monitoring-node-exporter.service",
            )
            for unit in units:
                (backup / f"{unit}.active").write_text("inactive\n")
                (backup / f"{unit}.enabled").write_text("disabled\n")
            (backup / f"{units[2]}.active").write_text("active\n")
            (backup / f"{units[2]}.enabled").write_text("enabled\n")
            source = (MONITORING / "tinfoil/bootstrap-collector").read_text()
            rollback = source.split("<<'ROLLBACK'\n", 1)[1].split("\nROLLBACK", 1)[0]
            rollback = rollback.replace(
                "/run/lock/finite-monitoring-dashboards.lock", str(root / "lock")
            )
            (backup / "rollback").write_text(rollback)
            for name, body in {
                "flock": "exit 0",
                "systemctl": 'printf "%s\\n" "$*" >> "$CALLS"',
            }.items():
                path = root / name
                path.write_text("#!/bin/sh\n" + body + "\n")
                path.chmod(0o755)
            calls = root / "calls"
            env = dict(
                os.environ, PATH=f"{root}:{os.environ['PATH']}", CALLS=str(calls)
            )
            subprocess.run(["bash", str(backup / "rollback")], env=env, check=True)
            self.assertEqual(existing.read_text(), "previous")
            self.assertFalse(new.exists())
            self.assertIn("start " + units[2], calls.read_text())
            self.assertNotIn("start " + units[0], calls.read_text())
            self.assertIn(
                "--signal=HUP finite-monitoring-prometheus.service", calls.read_text()
            )


if __name__ == "__main__":
    unittest.main()

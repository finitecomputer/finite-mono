"""Exercise the real deployment transaction against synthetic existing state."""

import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace


SPEC = importlib.util.spec_from_file_location(
    "deploy_dashboards", Path(__file__).resolve().parents[1] / "deploy_dashboards.py"
)
deploy = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(deploy)


class DeploymentTest(unittest.TestCase):
    def test_status_artifact_omits_customer_and_diagnostic_details(self):
        report = {
            "schema_version": "finite.status.v1",
            "generated_at": "2026-09-14T00:00:00Z",
            "overall_status": "red",
            "exit_code": 1,
            "sections": {
                name: {
                    "status": "red",
                    "agent_name": "private-agent",
                    "project_id": "private-project",
                    "ip": "private-address",
                    "error": "private-diagnostic",
                }
                for name in (
                    "fleet_convergence",
                    "host_health",
                    "recovery_boundary",
                    "rollout_state",
                    "chat_plane",
                )
            },
        }
        output = Path(self.temporary.name) / "summary.json"
        with patch.object(
            deploy,
            "ssh",
            return_value=SimpleNamespace(returncode=1, stdout=json.dumps(report)),
        ):
            deploy.status(output)
        summary = json.loads(output.read_text())
        self.assertEqual(summary["schema_version"], "finite.status.summary.v1")
        self.assertEqual(summary["overall_status"], "red")
        self.assertNotIn("private-", output.read_text())

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name)
        self.directory = root / "dashboards"
        self.directory.mkdir()
        self.provider = root / "provider.yml"
        self.provider.write_text("synthetic unchanged provider")
        self.backups = root / "backups"
        self.previous = {}
        self.bundle = {
            "schema_version": 1,
            "helper_sha256": deploy.digest(Path(deploy.__file__).read_bytes()),
            "revision": "a" * 40,
            "provider_sha256": deploy.digest(self.provider.read_bytes()),
            "files": {},
        }
        for uid in ("finite-one", "finite-two"):
            name = uid + ".json"
            document = {
                "uid": uid,
                "title": "Previous",
                "panels": [{"id": 1}],
                "id": None,
                "version": 1,
            }
            content = json.dumps(document).encode()
            (self.directory / name).write_bytes(content)
            self.previous[name] = content
            document["title"] = "Candidate"
            self.bundle["files"][name] = {"uid": uid, "content": json.dumps(document)}

    def fetch(self, uid):
        document = json.loads((self.directory / (uid + ".json")).read_bytes())
        document.update(id=123, version=42)
        return {
            "meta": {"provisioned": True, "provisionedExternalId": uid + ".json"},
            "dashboard": document,
        }

    def apply(self, **overrides):
        options = dict(
            directory=self.directory,
            provider=self.provider,
            backups=self.backups,
            fetch=self.fetch,
        )
        options.update(overrides)
        deploy.apply(self.bundle, **options)

    def assert_previous(self):
        for name, content in self.previous.items():
            self.assertEqual((self.directory / name).read_bytes(), content)

    def test_updates_existing_dashboards_and_backs_up_exact_previous_bytes(self):
        self.apply()
        (backup,) = self.backups.iterdir()
        for name, content in self.previous.items():
            self.assertEqual((backup / name).read_bytes(), content)
            self.assertEqual(
                (self.directory / name).read_text(),
                self.bundle["files"][name]["content"],
            )
            self.assertEqual((self.directory / name).stat().st_mode & 0o777, 0o644)
        manifest = json.loads((backup / "manifest.json").read_text())
        self.assertEqual(manifest["revision"], self.bundle["revision"])

    def test_partial_install_failure_restores_both_files(self):
        def fail_second(path, content):
            if path.name == "finite-two.json":
                raise OSError("synthetic disk failure")
            deploy.atomic_write(path, content)

        with self.assertRaisesRegex(OSError, "synthetic disk failure"):
            self.apply(write=fail_second)
        self.assert_previous()

    def test_failed_grafana_reload_restores_and_verifies_previous_files(self):
        calls = []

        def verify(files, fetch):
            calls.append(files)
            if len(calls) == 1:
                raise RuntimeError("synthetic rejected dashboard")
            deploy.wait_for_reload(files, fetch, timeout=0)

        with self.assertRaisesRegex(RuntimeError, "synthetic rejected dashboard"):
            self.apply(verify=verify)
        self.assertEqual(len(calls), 2)
        self.assert_previous()

    def test_preflight_checks_all_ownership_before_mutating_any_file(self):
        def unowned(uid):
            response = self.fetch(uid)
            if uid == "finite-two":
                response["meta"]["provisioned"] = False
            return response

        with self.assertRaisesRegex(ValueError, "ownership differs"):
            self.apply(fetch=unowned)
        self.assert_previous()
        self.assertFalse(self.backups.exists())

    def test_rejects_mixed_version_helper_before_mutation(self):
        self.bundle["helper_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "installed deployment helper differs"):
            self.apply()
        self.assert_previous()
        self.assertFalse(self.backups.exists())

    def test_rejects_provider_drift_missing_file_wrong_uid_and_symlink(self):
        self.provider.write_text("changed")
        with self.assertRaisesRegex(ValueError, "provider differs"):
            self.apply()
        self.assert_previous()
        self.provider.write_text("synthetic unchanged provider")
        path = self.directory / "finite-two.json"
        path.unlink()
        with self.assertRaisesRegex(ValueError, "existing regular file"):
            self.apply()
        path.write_text(json.dumps({"uid": "unrelated"}))
        with self.assertRaisesRegex(ValueError, "existing file UID differs"):
            self.apply()
        path.unlink()
        path.symlink_to(self.directory / "finite-one.json")
        with self.assertRaisesRegex(ValueError, "existing regular file"):
            self.apply()
        self.assertEqual(
            (self.directory / "finite-one.json").read_bytes(),
            self.previous["finite-one.json"],
        )
        self.assertFalse(self.backups.exists())

    def test_idempotent_deploy_does_not_write_or_create_backup(self):
        for name, content in self.previous.items():
            self.bundle["files"][name]["content"] = content.decode()
        self.apply(write=lambda *_: self.fail("unexpected write"))
        self.assertFalse(self.backups.exists())

    def test_manifest_rejects_draft_duplicate_uid_and_path_traversal(self):
        for change in ("draft", "duplicate", "path"):
            with self.subTest(change=change):
                bundle = copy.deepcopy(self.bundle)
                if change == "draft":
                    entry = bundle["files"]["finite-one.json"]
                    document = json.loads(entry["content"])
                    document["tags"] = ["draft"]
                    entry["content"] = json.dumps(document)
                elif change == "duplicate":
                    bundle["files"]["finite-two.json"] = bundle["files"][
                        "finite-one.json"
                    ]
                else:
                    bundle["files"]["../finite-other.json"] = bundle["files"].pop(
                        "finite-one.json"
                    )
                with self.assertRaises(ValueError):
                    deploy.validate(bundle)

    def test_reload_requires_candidate_content_and_file_ownership(self):
        candidate = {
            name: entry["content"].encode()
            for name, entry in self.bundle["files"].items()
        }
        with self.assertRaisesRegex(RuntimeError, "Grafana did not load"):
            deploy.wait_for_reload(candidate, self.fetch, timeout=0)


if __name__ == "__main__":
    unittest.main()

"""Unit checks for the GitHub Actions publish gate wrapper."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPO_ROOT / "scripts" / "hermes-github-publish-gate.py"

spec = importlib.util.spec_from_file_location("hermes_github_publish_gate", SCRIPT_PATH)
assert spec is not None
publish_gate = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(publish_gate)


def args(**overrides: object) -> SimpleNamespace:
    values = {
        "repo": "finitecomputer/finitechat",
        "workflow": "ci.yml",
        "ref": "codex/hermes-sidecar-hardening",
        "branch": None,
        "restic_repository": None,
        "latitude_storage_bucket": None,
        "latitude_object_endpoint": None,
        "restic_prefix": None,
        "tinfoil_config_repo": "finitecomputer/tinfoil-agent-runtime-canary",
        "tinfoil_release_tag": "v0.1.0",
        "preflight_report": "target/hermes-github-ci-preflight.json",
        "artifact_dir": "target/hermes-github-publish-gate/artifacts",
        "report": "target/hermes-github-publish-gate/report.json",
        "dispatch_timeout_seconds": 120,
        "poll_seconds": 30,
        "allow_dirty": False,
        "dry_run": True,
    }
    values.update(overrides)
    return SimpleNamespace(**values)


class GithubPublishGateTest(unittest.TestCase):
    def test_workflow_dispatch_includes_required_s3_publish_inputs(self) -> None:
        command = publish_gate.workflow_run_command(args(), "codex/hermes-sidecar-hardening")
        text = " ".join(command)

        self.assertIn("gh workflow run ci.yml", text)
        self.assertIn("--repo finitecomputer/finitechat", text)
        self.assertIn("--ref codex/hermes-sidecar-hardening", text)
        self.assertIn("-f docker_smoke=true", text)
        self.assertIn("-f publish_runtime_image=true", text)
        self.assertIn("-f restic_backend=s3", text)
        self.assertIn("-f tinfoil_config_repo=finitecomputer/tinfoil-agent-runtime-canary", text)
        self.assertIn("-f tinfoil_release_tag=v0.1.0", text)

    def test_workflow_dispatch_passes_optional_repository_when_set(self) -> None:
        command = publish_gate.workflow_run_command(
            args(restic_repository="s3:https://objects.nyc.storage.sh/bucket/prefix"),
            "codex/hermes-sidecar-hardening",
        )

        self.assertIn(
            "restic_repository=s3:https://objects.nyc.storage.sh/bucket/prefix",
            command,
        )

    def test_dry_run_report_does_not_execute_commands(self) -> None:
        status, report = publish_gate.run_gate(args())

        self.assertEqual(status, 0)
        self.assertEqual(report["status"], "dry_run")
        self.assertIn("preflight", report["commands"])
        self.assertIn("dispatch", report["commands"])
        self.assertEqual(report["ref"], "codex/hermes-sidecar-hardening")
        self.assertEqual(report["branch"], "codex/hermes-sidecar-hardening")
        self.assertIn("local_status", report)

    def test_readiness_errors_require_clean_worktree_and_remote_ref(self) -> None:
        errors = publish_gate.readiness_errors(
            local_status=[" M file"],
            remote_ref={"status": "missing"},
            allow_dirty=False,
        )

        self.assertIn("local worktree has uncommitted changes", errors[0])
        self.assertIn("remote workflow ref is missing", errors[1])

    def test_readiness_allows_dirty_only_when_ref_exists(self) -> None:
        errors = publish_gate.readiness_errors(
            local_status=[" M file"],
            remote_ref={"status": "present", "sha": "abc"},
            allow_dirty=True,
        )

        self.assertEqual(errors, [])

    def test_ingest_artifacts_copies_downloaded_reports_to_canonical_paths(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            artifact_root = tmp / "artifacts" / "hermes-docker-smoke-report"
            repo_root = tmp / "repo"
            for index, relative_path in enumerate(publish_gate.CANONICAL_ARTIFACTS):
                source = artifact_root / relative_path
                source.parent.mkdir(parents=True, exist_ok=True)
                source.write_text(json.dumps({"index": index}) + "\n", encoding="utf-8")

            result = publish_gate.ingest_artifacts(tmp / "artifacts", repo_root=repo_root)

            self.assertEqual(result["status"], "ok")
            self.assertEqual(result["missing"], [])
            self.assertEqual(len(result["copied"]), len(publish_gate.CANONICAL_ARTIFACTS))
            for index, relative_path in enumerate(publish_gate.CANONICAL_ARTIFACTS):
                copied = json.loads((repo_root / relative_path).read_text(encoding="utf-8"))
                self.assertEqual(copied, {"index": index})

    def test_ingest_artifacts_accepts_github_downloads_without_target_prefix(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            artifact_root = tmp / "artifacts" / "hermes-docker-smoke-report"
            repo_root = tmp / "repo"
            for index, relative_path in enumerate(publish_gate.CANONICAL_ARTIFACTS):
                source = artifact_root / Path(*Path(relative_path).parts[1:])
                source.parent.mkdir(parents=True, exist_ok=True)
                source.write_text(json.dumps({"index": index}) + "\n", encoding="utf-8")

            result = publish_gate.ingest_artifacts(tmp / "artifacts", repo_root=repo_root)

            self.assertEqual(result["status"], "ok")
            self.assertEqual(result["missing"], [])
            for index, relative_path in enumerate(publish_gate.CANONICAL_ARTIFACTS):
                copied = json.loads((repo_root / relative_path).read_text(encoding="utf-8"))
                self.assertEqual(copied, {"index": index})

    def test_ingest_artifacts_reports_missing_required_downloads(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            artifact_root = tmp / "artifacts" / "hermes-docker-smoke-report"
            present = publish_gate.CANONICAL_ARTIFACTS[0]
            source = artifact_root / present
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text("{}\n", encoding="utf-8")

            result = publish_gate.ingest_artifacts(tmp / "artifacts", repo_root=tmp / "repo")

            self.assertEqual(result["status"], "missing_artifacts")
            self.assertNotIn(present, result["missing"])
            self.assertIn("target/hermes-docker-smoke/report.json", result["missing"])


if __name__ == "__main__":
    unittest.main()

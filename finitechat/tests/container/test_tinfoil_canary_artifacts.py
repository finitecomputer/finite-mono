"""Unit checks for generated Tinfoil canary artifacts."""

from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
ARTIFACT_SCRIPT = REPO_ROOT / "scripts" / "hermes-tinfoil-canary-artifacts.py"
IMAGE_DIGEST = "ghcr.io/finitecomputer/finite-chat-hermes-runtime:v0.1.0@sha256:published"
RESTIC_REPOSITORY = (
    "s3:https://objects.nyc.storage.sh/tinfoil-agent-spike/agent-runtimes/tinfoil-canary-001/restic"
)
RECOVERY_SCOPE = {
    "snapshot_root": "/data",
    "workspace_path": "/data/workspace",
    "workspace_included": True,
    "application_consistent_snapshot": "unproved",
    "independently_recoverable_key_authority": "unproved",
    "core_owned_empty_target_restore": "unproved",
}


def write_json(path: Path, value: dict) -> None:
    path.write_text(json.dumps(value) + "\n", encoding="utf-8")


def ready_handoff() -> dict:
    return {
        "status": "ready",
        "recovery_scope": dict(RECOVERY_SCOPE),
        "image": {
            "source_image_id": "sha256:local-image",
            "target_ref": "ghcr.io/finitecomputer/finite-chat-hermes-runtime:v0.1.0",
            "digest": IMAGE_DIGEST,
        },
        "runtime": {
            "hermes_agent_version": "0.18.2",
            "restic_version": "restic 0.18.0 compiled with go1.24.4 on linux/arm64",
            "finitechat_hermes_inbound_stream": "1",
            "finite_agent_restore_on_start": "1",
            "finite_agent_restore_latest": "1",
            "finite_agent_backup_on_exit": "1",
            "finite_agent_backup_interval_secs": "30",
            "finite_agent_state_root": "/data",
        },
        "restore": {
            "backend": "s3",
            "repository": {
                "kind": "s3",
                "repository": RESTIC_REPOSITORY,
                "size_bytes": None,
            },
            "seed_snapshot_id": "88929f1f90c5fcadd1d19e33f26609e595af4c2afb1e72b724695435e051900f",
            "seed_snapshot_short_id": "88929f1f",
            "seed_snapshot_time": "2026-06-26T02:26:14Z",
            "restore_selector": "latest",
            "restore_tag": "finite-agent-state",
            "required_secret_env": [
                "FINITE_AGENT_RESTIC_PASSWORD",
                "AWS_ACCESS_KEY_ID",
                "AWS_SECRET_ACCESS_KEY",
                "OPENROUTER_API_KEY",
            ],
            "optional_secret_env": ["AWS_REGION", "AWS_DEFAULT_REGION", "AWS_SESSION_TOKEN"],
            "container_env": {
                "FINITE_AGENT_RESTORE_ON_START": "1",
                "FINITE_AGENT_RESTORE_LATEST": "1",
                "FINITE_AGENT_BACKUP_ON_EXIT": "1",
                "FINITE_AGENT_BACKUP_INTERVAL_SECS": "30",
                "FINITE_AGENT_STATE_ROOT": "/data",
                "FINITE_AGENT_RESTIC_REPOSITORY": RESTIC_REPOSITORY,
                "FINITE_AGENT_RESTIC_BACKUP_TAG": "finite-agent-state",
                "FINITECHAT_HERMES_INBOUND_STREAM": "1",
                "FINITECHAT_HERMES_MODEL": "anthropic/claude-sonnet-4.6",
                "FINITECHAT_HERMES_PROVIDER": "openrouter",
            },
        },
    }


class TinfoilCanaryArtifactsTest(unittest.TestCase):
    def run_script(self, tmp: Path, handoff: dict) -> subprocess.CompletedProcess[str]:
        handoff_path = tmp / "handoff.json"
        write_json(handoff_path, handoff)
        return subprocess.run(
            [
                str(ARTIFACT_SCRIPT),
                "--handoff-report",
                str(handoff_path),
                "--output-dir",
                str(tmp / "out"),
                "--config-repo",
                "finitecomputer/tinfoil-agent-runtime-canary",
                "--tag",
                "v0.1.0",
            ],
            capture_output=True,
            text=True,
            check=False,
        )

    def test_generates_digest_pinned_config_and_runbook(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            result = self.run_script(tmp, ready_handoff())
            config = (tmp / "out" / "tinfoil-config.yml").read_text(encoding="utf-8")
            runbook = (tmp / "out" / "tinfoil-canary-runbook.md").read_text(encoding="utf-8")
            summary = json.loads((tmp / "out" / "tinfoil-canary-summary.json").read_text())

        self.assertEqual(result.returncode, 0)
        self.assertEqual(summary["status"], "ready")
        self.assertEqual(summary["recovery_scope"], RECOVERY_SCOPE)
        self.assertIn(f'image: "{IMAGE_DIGEST}"', config)
        self.assertIn('FINITE_SERVER_URL: "https://chat.finite.computer"', config)
        self.assertIn('FINITECHAT_SERVER_URL: "https://chat.finite.computer"', config)
        self.assertIn('FINITE_AGENT_RESTORE_LATEST: "1"', config)
        self.assertIn('FINITE_AGENT_BACKUP_INTERVAL_SECS: "30"', config)
        self.assertIn('FINITE_AGENT_STATE_ROOT: "/data"', config)
        self.assertNotIn("FINITE_AGENT_RESTIC_SNAPSHOT_ID", config)
        self.assertIn("FINITE_AGENT_RESTIC_PASSWORD", config)
        self.assertIn("OPENROUTER_API_KEY", config)
        self.assertIn('FINITECHAT_HERMES_PROVIDER: "openrouter"', config)
        self.assertIn("upstream-port: 8080", config)
        self.assertIn("/healthz", config)
        self.assertIn("/invite", config)
        self.assertIn("tinfoil container create finite-agent-tinfoil-user-canary", runbook)
        self.assertIn("--debug", runbook)
        self.assertIn("--ssh-key", runbook)
        self.assertIn("curl http://127.0.0.1:3301/invite", runbook)
        self.assertIn("scripts/hermes-tinfoil-canary-evidence.py", runbook)
        self.assertIn("scripts/hermes-tinfoil-canary-result.py", runbook)
        self.assertIn("tinfoil-canary-evidence.json", runbook)
        self.assertIn("--image-digest '<digest-observed-from-tinfoil-container-json>'", runbook)
        self.assertIn("--storage-backend s3", runbook)
        self.assertIn('"expected": {', runbook)
        self.assertIn('"source": "operator_arg"', runbook)
        self.assertIn('"storage_backend": "s3"', runbook)
        self.assertIn("event ID is recorded", runbook)
        self.assertIn("raw source artifact references", runbook)
        self.assertIn("visible to Tinfoil infrastructure", runbook)
        self.assertIn("complete `/data` recovery", runbook)
        self.assertIn("application-consistent snapshot barrier", runbook)
        self.assertIn("independently recoverable key authority", runbook)
        self.assertIn("Core-owned service-consistent empty-target restore", runbook)

    def test_refuses_failed_handoff(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            result = self.run_script(Path(tmp_value), {"status": "failed", "errors": ["nope"]})

        self.assertEqual(result.returncode, 2)
        self.assertIn("handoff status must be ready", result.stderr)

    def test_refuses_handoff_that_overstates_empty_target_restore(self) -> None:
        handoff = ready_handoff()
        handoff["recovery_scope"]["core_owned_empty_target_restore"] = "proven"
        with tempfile.TemporaryDirectory() as tmp_value:
            result = self.run_script(Path(tmp_value), handoff)

        self.assertEqual(result.returncode, 2)
        self.assertIn("core_owned_empty_target_restore must be 'unproved'", result.stderr)


if __name__ == "__main__":
    unittest.main()

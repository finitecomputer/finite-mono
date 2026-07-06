"""Unit checks for the Tinfoil handoff report contract."""

from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
HANDOFF_SCRIPT = REPO_ROOT / "scripts" / "hermes-tinfoil-handoff.py"
RESTIC_REPOSITORY = (
    "s3:https://objects.nyc.storage.sh/tinfoil-agent-spike/agent-runtimes/tinfoil-canary-001/restic"
)
SNAPSHOT_ID = "88929f1f90c5fcadd1d19e33f26609e595af4c2afb1e72b724695435e051900f"
IMAGE_REF = "ghcr.io/finitecomputer/finite-chat-hermes-runtime:canary"
IMAGE_DIGEST = "ghcr.io/finitecomputer/finite-chat-hermes-runtime@sha256:published"


def write_json(path: Path, value: dict[str, object]) -> None:
    path.write_text(json.dumps(value) + "\n", encoding="utf-8")


def proven_smoke_report() -> dict[str, object]:
    return {
        "status": "passed",
        "facts": {
            "image_id": "sha256:local-image",
            "hermes_agent_version_actual": "0.17.0",
            "restic_version": "restic 0.18.0 compiled with go1.24.4 on linux/arm64",
            "restic_backend": "s3",
            "real_gateway_runtime": True,
            "gateway_admission_before_restore": True,
            "gateway_admission_after_restore": True,
            "agent_state_backup": {
                "backend": "restic",
                "repository": {
                    "kind": "s3",
                    "repository": RESTIC_REPOSITORY,
                    "size_bytes": None,
                },
                "snapshot": {
                    "id": SNAPSHOT_ID,
                    "short_id": "88929f1f",
                    "time": "2026-06-26T02:26:14Z",
                    "paths": ["/data/agent"],
                    "tags": ["finite-agent-state"],
                },
                "tag": "finite-agent-state",
                "encrypted": True,
                "source": "entrypoint_backup_on_exit",
            },
        },
    }


class TinfoilHandoffTest(unittest.TestCase):
    def run_handoff(
        self,
        tmp: Path,
        *,
        smoke: dict[str, object] | None = None,
        preflight: dict[str, object] | None = None,
        publish: dict[str, object] | None = None,
    ) -> tuple[subprocess.CompletedProcess[str], dict]:
        smoke_path = tmp / "smoke.json"
        preflight_path = tmp / "preflight.json"
        publish_path = tmp / "publish.json"
        handoff_path = tmp / "handoff.json"
        write_json(smoke_path, smoke or proven_smoke_report())
        write_json(preflight_path, preflight or {"status": "ok", "backend": "s3"})
        write_json(
            publish_path,
            publish
            or {
                "status": "published",
                "source_image_id": "sha256:local-image",
                "target_image_ref": IMAGE_REF,
                "repo_digests": [IMAGE_DIGEST],
            },
        )
        result = subprocess.run(
            [
                str(HANDOFF_SCRIPT),
                "--smoke-report",
                str(smoke_path),
                "--preflight-report",
                str(preflight_path),
                "--publish-report",
                str(publish_path),
                "--handoff-report",
                str(handoff_path),
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        return result, json.loads(handoff_path.read_text(encoding="utf-8"))

    def test_ready_handoff_includes_restore_and_backup_contract(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            result, handoff = self.run_handoff(Path(tmp_value))

        self.assertEqual(result.returncode, 0)
        self.assertEqual(handoff["status"], "ready")
        self.assertEqual(handoff["runtime"]["finite_agent_restore_on_start"], "1")
        self.assertEqual(handoff["runtime"]["finite_agent_restore_latest"], "1")
        self.assertEqual(handoff["runtime"]["finite_agent_backup_on_exit"], "1")
        self.assertEqual(handoff["runtime"]["finite_agent_backup_interval_secs"], "30")
        self.assertEqual(handoff["restore"]["seed_snapshot_id"], SNAPSHOT_ID)
        self.assertEqual(handoff["restore"]["restore_selector"], "latest")
        self.assertEqual(handoff["restore"]["restore_tag"], "finite-agent-state")
        self.assertEqual(
            handoff["restore"]["required_secret_env"],
            [
                "FINITE_AGENT_RESTIC_PASSWORD",
                "AWS_ACCESS_KEY_ID",
                "AWS_SECRET_ACCESS_KEY",
                "OPENROUTER_API_KEY",
            ],
        )
        self.assertEqual(
            handoff["restore"]["container_env"],
            {
                "FINITE_AGENT_RESTORE_ON_START": "1",
                "FINITE_AGENT_RESTORE_LATEST": "1",
                "FINITE_AGENT_BACKUP_ON_EXIT": "1",
                "FINITE_AGENT_BACKUP_INTERVAL_SECS": "30",
                "FINITE_AGENT_RESTIC_REPOSITORY": RESTIC_REPOSITORY,
                "FINITE_AGENT_RESTIC_BACKUP_TAG": "finite-agent-state",
                "FINITECHAT_HERMES_INBOUND_STREAM": "1",
                "FINITECHAT_HERMES_MODEL": "anthropic/claude-sonnet-4.6",
                "FINITECHAT_HERMES_PROVIDER": "openrouter",
            },
        )
        self.assertIn(
            "observe a fresh periodic or exit snapshot of agent state",
            handoff["acceptance"],
        )
        self.assertIn(
            "restore with temporary canary restic password secret",
            handoff["acceptance"],
        )

    def test_handoff_fails_closed_when_publish_does_not_match_proven_image(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            result, handoff = self.run_handoff(
                Path(tmp_value),
                publish={
                    "status": "published",
                    "source_image_id": "sha256:different-image",
                    "target_image_ref": IMAGE_REF,
                    "repo_digests": [IMAGE_DIGEST],
                },
            )

        self.assertEqual(result.returncode, 2)
        self.assertEqual(handoff["status"], "failed")
        self.assertIn(
            "Publish report source image id must match Docker smoke image id",
            handoff["errors"],
        )


if __name__ == "__main__":
    unittest.main()

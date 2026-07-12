"""Unit checks for Tinfoil canary evidence builder."""

from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
EVIDENCE_SCRIPT = REPO_ROOT / "scripts" / "hermes-tinfoil-canary-evidence.py"
RESULT_SCRIPT = REPO_ROOT / "scripts" / "hermes-tinfoil-canary-result.py"
IMAGE_DIGEST = "ghcr.io/finitecomputer/finite-chat-hermes-runtime:v0.1.0@sha256:published"
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


def run_evidence(
    tmp: Path, *, include_container_image_digest: bool = True
) -> tuple[subprocess.CompletedProcess[str], dict]:
    handoff = tmp / "handoff.json"
    summary = tmp / "summary.json"
    container = tmp / "container.json"
    health = tmp / "health.json"
    evidence = tmp / "evidence.json"
    write_json(
        handoff,
        {
            "status": "ready",
            "recovery_scope": dict(RECOVERY_SCOPE),
            "image": {"digest": IMAGE_DIGEST},
            "restore": {"backend": "s3", "restore_tag": "finite-agent-state"},
        },
    )
    write_json(
        summary,
        {
            "container_name": "finite-agent-tinfoil-user-canary",
            "image_digest": IMAGE_DIGEST,
            "config_repo": "finitecomputer/tinfoil-agent-runtime-canary",
            "release_tag": "v0.1.0",
            "recovery_scope": dict(RECOVERY_SCOPE),
        },
    )
    container_payload = {
        "status": "Running",
        "container_url": "https://finite-agent-tinfoil-user-canary.finite.containers.tinfoil.dev",
    }
    if include_container_image_digest:
        container_payload["image_digest"] = IMAGE_DIGEST
    write_json(container, container_payload)
    write_json(health, {"ready": True, "npub": "npub1agent"})
    result = subprocess.run(
        [
            str(EVIDENCE_SCRIPT),
            "--handoff-report",
            str(handoff),
            "--canary-summary",
            str(summary),
            "--container-json",
            str(container),
            "--health-json",
            str(health),
            "--storage-backend",
            "s3",
            "--restore-tag",
            "finite-agent-state",
            "--chat-before-message-id",
            "event-before",
            "--chat-after-message-id",
            "event-after",
            "--backup-observed",
            "--restore-observed",
            "--evidence-json",
            str(evidence),
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    return result, json.loads(evidence.read_text(encoding="utf-8"))


class TinfoilCanaryEvidenceTest(unittest.TestCase):
    def test_builds_evidence_accepted_by_result_validator(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            result, evidence = run_evidence(tmp)
            report_path = tmp / "result.json"
            validation = subprocess.run(
                [
                    str(RESULT_SCRIPT),
                    "--evidence-json",
                    str(tmp / "evidence.json"),
                    "--report",
                    str(report_path),
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            report = json.loads(report_path.read_text(encoding="utf-8"))

        self.assertEqual(result.returncode, 0)
        self.assertEqual(validation.returncode, 0)
        self.assertEqual(report["status"], "passed")
        self.assertEqual(evidence["container"]["name"], "finite-agent-tinfoil-user-canary")
        self.assertTrue(evidence["source_artifacts"]["handoff_report"]["present"])
        self.assertEqual(evidence["expected"]["container_name"], "finite-agent-tinfoil-user-canary")
        self.assertEqual(evidence["image"]["digest"], IMAGE_DIGEST)
        self.assertEqual(evidence["image"]["source"], "container_json")
        self.assertEqual(evidence["storage"]["source"], "operator_arg")
        self.assertTrue(evidence["restart_restore"]["same_npub"])
        self.assertEqual(evidence["recovery_scope"], RECOVERY_SCOPE)

    def test_missing_chat_after_restart_still_emits_invalid_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            result, _ = run_evidence(tmp)
            evidence_path = tmp / "evidence.json"
            evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
            evidence["chat"]["after_restart"] = {"ok": False, "message_id": None}
            evidence_path.write_text(json.dumps(evidence) + "\n", encoding="utf-8")
            report_path = tmp / "result.json"
            validation = subprocess.run(
                [
                    str(RESULT_SCRIPT),
                    "--evidence-json",
                    str(evidence_path),
                    "--report",
                    str(report_path),
                ],
                capture_output=True,
                text=True,
                check=False,
            )

        self.assertEqual(result.returncode, 0)
        self.assertEqual(validation.returncode, 2)

    def test_expected_digest_is_not_used_as_observed_digest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            result, evidence = run_evidence(tmp, include_container_image_digest=False)
            evidence_path = tmp / "evidence.json"
            report_path = tmp / "result.json"
            validation = subprocess.run(
                [
                    str(RESULT_SCRIPT),
                    "--evidence-json",
                    str(evidence_path),
                    "--report",
                    str(report_path),
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            report = json.loads(report_path.read_text(encoding="utf-8"))

        self.assertEqual(result.returncode, 0)
        self.assertIsNone(evidence["image"]["digest"])
        self.assertIsNone(evidence["image"]["source"])
        self.assertEqual(validation.returncode, 2)
        self.assertIn("digest-pinned runtime image", report["missing_proof_layers"])


if __name__ == "__main__":
    unittest.main()

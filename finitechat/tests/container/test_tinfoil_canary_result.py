"""Unit checks for live Tinfoil canary result normalization."""

from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
RESULT_SCRIPT = REPO_ROOT / "scripts" / "hermes-tinfoil-canary-result.py"
IMAGE_DIGEST = "ghcr.io/finitecomputer/finite-chat-hermes-runtime:v0.1.0@sha256:published"


def write_json(path: Path, value: dict) -> None:
    path.write_text(json.dumps(value) + "\n", encoding="utf-8")


def passing_evidence() -> dict:
    return {
        "container": {
            "name": "finite-agent-tinfoil-user-canary",
            "status": "Running",
            "url": "https://finite-agent-tinfoil-user-canary.finite.containers.tinfoil.dev",
        },
        "expected": {
            "container_name": "finite-agent-tinfoil-user-canary",
            "image_digest": IMAGE_DIGEST,
            "storage_backend": "s3",
            "restore_tag": "finite-agent-state",
        },
        "source_artifacts": {
            "handoff_report": {"path": "handoff.json", "present": True},
            "canary_summary": {"path": "summary.json", "present": True},
            "container_json": {"path": "container.json", "present": True},
            "health_json": {"path": "health.json", "present": True},
        },
        "image": {"digest": IMAGE_DIGEST, "source": "container_json"},
        "storage": {
            "backend": "s3",
            "restore_tag": "finite-agent-state",
            "source": "operator_arg",
        },
        "health": {"ready": True, "npub": "npub1agent"},
        "chat": {
            "before_restart": {"ok": True, "message_id": "event-before"},
            "after_restart": {"ok": True, "message_id": "event-after"},
        },
        "restart_restore": {
            "npub_before_restart": "npub1agent",
            "npub_after_restore": "npub1agent",
            "same_npub": True,
            "backup_observed": True,
            "restore_observed": True,
        },
    }


def run_result(tmp: Path, evidence: dict) -> tuple[subprocess.CompletedProcess[str], dict]:
    evidence_path = tmp / "evidence.json"
    report_path = tmp / "result.json"
    write_json(evidence_path, evidence)
    result = subprocess.run(
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
    return result, json.loads(report_path.read_text(encoding="utf-8"))


class TinfoilCanaryResultTest(unittest.TestCase):
    def test_passes_complete_live_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            result, report = run_result(Path(tmp_value), passing_evidence())

        self.assertEqual(result.returncode, 0)
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["missing_proof_layers"], [])
        self.assertEqual(report["facts"]["restic_backend"], "s3")
        self.assertEqual(report["facts"]["image_source"], "container_json")
        self.assertEqual(report["facts"]["storage_source"], "operator_arg")
        self.assertEqual(report["facts"]["agent_npub_before_restart"], "npub1agent")
        self.assertEqual(report["facts"]["agent_npub_after_restore"], "npub1agent")
        self.assertTrue(report["facts"]["container_json_source_present"])
        self.assertTrue(report["facts"]["health_json_source_present"])
        self.assertTrue(report["facts"]["chat_before_restart"])
        self.assertEqual(report["facts"]["chat_before_message_id"], "event-before")
        self.assertTrue(report["facts"]["chat_after_restart"])
        self.assertEqual(report["facts"]["chat_after_message_id"], "event-after")

    def test_fails_when_restore_does_not_keep_same_npub(self) -> None:
        evidence = passing_evidence()
        evidence["restart_restore"]["npub_after_restore"] = "npub1different"
        with tempfile.TemporaryDirectory() as tmp_value:
            result, report = run_result(Path(tmp_value), evidence)

        self.assertEqual(result.returncode, 2)
        self.assertEqual(report["status"], "failed")
        self.assertIn("same agent npub after restore", report["missing_proof_layers"])
        self.assertIn("npub_after_restore", " ".join(report["errors"]))

    def test_fails_when_chat_after_restart_is_missing(self) -> None:
        evidence = passing_evidence()
        evidence["chat"]["after_restart"] = {"ok": False}
        with tempfile.TemporaryDirectory() as tmp_value:
            result, report = run_result(Path(tmp_value), evidence)

        self.assertEqual(result.returncode, 2)
        self.assertEqual(report["status"], "failed")
        self.assertIn("Finite Chat round trip after restore", report["missing_proof_layers"])

    def test_fails_when_chat_event_id_is_missing(self) -> None:
        evidence = passing_evidence()
        evidence["chat"]["before_restart"] = {"ok": True}
        with tempfile.TemporaryDirectory() as tmp_value:
            result, report = run_result(Path(tmp_value), evidence)

        self.assertEqual(result.returncode, 2)
        self.assertEqual(report["status"], "failed")
        self.assertIn("Finite Chat round trip before restart", report["missing_proof_layers"])
        self.assertIn("message_id", " ".join(report["errors"]))

    def test_fails_when_runtime_image_does_not_match_expected_handoff(self) -> None:
        evidence = passing_evidence()
        evidence["image"]["digest"] = (
            "ghcr.io/finitecomputer/finite-chat-hermes-runtime:v0.1.0@sha256:different"
        )
        with tempfile.TemporaryDirectory() as tmp_value:
            result, report = run_result(Path(tmp_value), evidence)

        self.assertEqual(result.returncode, 2)
        self.assertEqual(report["status"], "failed")
        self.assertIn("digest-pinned runtime image", report["missing_proof_layers"])
        self.assertIn("expected.image_digest", " ".join(report["errors"]))

    def test_fails_when_runtime_image_has_no_observation_source(self) -> None:
        evidence = passing_evidence()
        evidence["image"]["source"] = None
        with tempfile.TemporaryDirectory() as tmp_value:
            result, report = run_result(Path(tmp_value), evidence)

        self.assertEqual(result.returncode, 2)
        self.assertEqual(report["status"], "failed")
        self.assertIn("digest-pinned runtime image", report["missing_proof_layers"])
        self.assertIn("container_json or operator_arg", " ".join(report["errors"]))

    def test_fails_when_storage_has_no_observation_source(self) -> None:
        evidence = passing_evidence()
        evidence["storage"]["source"] = None
        with tempfile.TemporaryDirectory() as tmp_value:
            result, report = run_result(Path(tmp_value), evidence)

        self.assertEqual(result.returncode, 2)
        self.assertEqual(report["status"], "failed")
        self.assertIn("S3 restic repository", report["missing_proof_layers"])
        self.assertIn("health_or_container_json or operator_arg", " ".join(report["errors"]))

    def test_fails_when_raw_health_artifact_is_missing(self) -> None:
        evidence = passing_evidence()
        evidence["source_artifacts"]["health_json"] = {"path": "health.json", "present": False}
        with tempfile.TemporaryDirectory() as tmp_value:
            result, report = run_result(Path(tmp_value), evidence)

        self.assertEqual(result.returncode, 2)
        self.assertEqual(report["status"], "failed")
        self.assertIn("attested health proxy ready", report["missing_proof_layers"])
        self.assertIn("source_artifacts.health_json", " ".join(report["errors"]))


if __name__ == "__main__":
    unittest.main()

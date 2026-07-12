#!/usr/bin/env python3
"""Validate live Tinfoil canary evidence into a hardening audit report."""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path
from typing import Any

REQUIRED_PROOF_LAYERS = {
    "Tinfoil container running",
    "digest-pinned runtime image",
    "S3 restic repository",
    "attested health proxy ready",
    "agent npub observed before restart",
    "Finite Chat round trip before restart",
    "fresh restic backup observed before restart",
    "latest-by-tag restore observed after restart",
    "same agent npub after restore",
    "Finite Chat round trip after restore",
}
REQUIRED_RECOVERY_SCOPE = {
    "snapshot_root": "/data",
    "workspace_path": "/data/workspace",
    "workspace_included": True,
    "application_consistent_snapshot": "unproved",
    "independently_recoverable_key_authority": "unproved",
    "core_owned_empty_target_restore": "unproved",
}


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def nested_dict(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def artifact_present(source_artifacts: dict[str, Any], name: str) -> bool:
    artifact = nested_dict(source_artifacts.get(name))
    return artifact.get("present") is True and bool(non_empty_string(artifact.get("path")))


def chat_round_trip(value: Any) -> tuple[bool, str | None]:
    if not isinstance(value, dict):
        return False, None
    message_id = non_empty_string(value.get("message_id"))
    ok = value.get("ok") is True or value.get("passed") is True
    return bool(ok and message_id), message_id


def non_empty_string(value: Any) -> str | None:
    return value if isinstance(value, str) and value.strip() else None


def normalize_status(value: Any) -> str:
    if isinstance(value, str):
        return value.strip().lower()
    return ""


def recovery_scope_errors(value: Any, *, label: str) -> list[str]:
    scope = nested_dict(value)
    return [
        f"{label}.{key}={scope.get(key)!r}; expected {expected!r}"
        for key, expected in REQUIRED_RECOVERY_SCOPE.items()
        if scope.get(key) != expected
    ]


def add_layer(
    proof_layers: set[str],
    errors: list[str],
    *,
    condition: bool,
    layer: str,
    error: str,
) -> None:
    if condition:
        proof_layers.add(layer)
    else:
        errors.append(error)


def validate(evidence: dict[str, Any]) -> tuple[int, dict[str, Any]]:
    errors: list[str] = []
    proof_layers: set[str] = set()
    container = nested_dict(evidence.get("container"))
    image = nested_dict(evidence.get("image"))
    storage = nested_dict(evidence.get("storage"))
    expected = nested_dict(evidence.get("expected"))
    recovery_scope = nested_dict(evidence.get("recovery_scope"))
    expected_recovery_scope = nested_dict(expected.get("recovery_scope"))
    source_artifacts = nested_dict(evidence.get("source_artifacts"))
    health = nested_dict(evidence.get("health"))
    chat = nested_dict(evidence.get("chat"))
    restart_restore = nested_dict(evidence.get("restart_restore"))

    container_name = non_empty_string(container.get("name"))
    container_url = non_empty_string(container.get("url"))
    container_status = normalize_status(container.get("status"))
    image_digest = non_empty_string(image.get("digest"))
    image_source = non_empty_string(image.get("source"))
    restic_backend = non_empty_string(storage.get("backend"))
    restore_tag = non_empty_string(storage.get("restore_tag"))
    storage_source = non_empty_string(storage.get("source"))
    expected_container_name = non_empty_string(expected.get("container_name"))
    expected_image_digest = non_empty_string(expected.get("image_digest"))
    expected_storage_backend = non_empty_string(expected.get("storage_backend"))
    expected_restore_tag = non_empty_string(expected.get("restore_tag"))
    health_ready = health.get("ready") is True
    health_npub = non_empty_string(health.get("npub"))
    before_npub = non_empty_string(restart_restore.get("npub_before_restart"))
    after_npub = non_empty_string(restart_restore.get("npub_after_restore"))
    chat_before, chat_before_message_id = chat_round_trip(chat.get("before_restart"))
    chat_after, chat_after_message_id = chat_round_trip(chat.get("after_restart"))
    backup_observed = restart_restore.get("backup_observed") is True
    restore_observed = restart_restore.get("restore_observed") is True
    same_npub_declared = restart_restore.get("same_npub") is True
    same_npub_observed = bool(before_npub and after_npub and before_npub == after_npub)
    expected_container_matches = bool(
        expected_container_name and container_name and expected_container_name == container_name
    )
    expected_image_matches = bool(
        expected_image_digest and image_digest and expected_image_digest == image_digest
    )
    expected_storage_matches = bool(
        expected_storage_backend
        and restic_backend
        and expected_storage_backend == restic_backend
        and expected_restore_tag
        and restore_tag
        and expected_restore_tag == restore_tag
    )
    handoff_source_present = artifact_present(source_artifacts, "handoff_report")
    summary_source_present = artifact_present(source_artifacts, "canary_summary")
    container_source_present = artifact_present(source_artifacts, "container_json")
    health_source_present = artifact_present(source_artifacts, "health_json")

    errors.extend(recovery_scope_errors(recovery_scope, label="recovery_scope"))
    errors.extend(recovery_scope_errors(expected_recovery_scope, label="expected.recovery_scope"))

    add_layer(
        proof_layers,
        errors,
        condition=bool(
            container_name
            and container_url
            and container_status == "running"
            and expected_container_matches
            and container_source_present
        ),
        layer="Tinfoil container running",
        error=(
            "container.name, container.url, container.status='running', and "
            "expected.container_name matching container.name are required from "
            "source_artifacts.container_json"
        ),
    )
    add_layer(
        proof_layers,
        errors,
        condition=bool(
            image_digest
            and "@sha256:" in image_digest
            and image_source in {"container_json", "operator_arg"}
            and expected_image_matches
            and handoff_source_present
            and summary_source_present
        ),
        layer="digest-pinned runtime image",
        error=(
            "image.digest must be observed from container_json or operator_arg, "
            "be digest-pinned with @sha256:, and match expected.image_digest "
            "from source handoff/summary artifacts"
        ),
    )
    add_layer(
        proof_layers,
        errors,
        condition=(
            restic_backend == "s3"
            and restore_tag == "finite-agent-state"
            and storage_source in {"health_or_container_json", "operator_arg"}
            and expected_storage_matches
            and handoff_source_present
        ),
        layer="S3 restic repository",
        error=(
            "storage.backend/restic tag must be observed from health_or_container_json "
            "or operator_arg, storage.backend must be 's3', storage.restore_tag "
            "must be 'finite-agent-state', and both must match expected storage "
            "values from source_artifacts.handoff_report"
        ),
    )
    add_layer(
        proof_layers,
        errors,
        condition=bool(health_ready and health_npub and health_source_present),
        layer="attested health proxy ready",
        error="health.ready=true and health.npub are required from source_artifacts.health_json",
    )
    add_layer(
        proof_layers,
        errors,
        condition=bool(before_npub),
        layer="agent npub observed before restart",
        error="restart_restore.npub_before_restart is required",
    )
    add_layer(
        proof_layers,
        errors,
        condition=chat_before,
        layer="Finite Chat round trip before restart",
        error="chat.before_restart must be {ok: true, message_id: '<event-id>'}",
    )
    add_layer(
        proof_layers,
        errors,
        condition=backup_observed,
        layer="fresh restic backup observed before restart",
        error="restart_restore.backup_observed=true is required",
    )
    add_layer(
        proof_layers,
        errors,
        condition=restore_observed,
        layer="latest-by-tag restore observed after restart",
        error="restart_restore.restore_observed=true is required",
    )
    add_layer(
        proof_layers,
        errors,
        condition=same_npub_declared and same_npub_observed and health_npub == after_npub,
        layer="same agent npub after restore",
        error=(
            "restart_restore.same_npub=true, matching npub_before_restart/"
            "npub_after_restore, and health.npub matching npub_after_restore are required"
        ),
    )
    add_layer(
        proof_layers,
        errors,
        condition=chat_after,
        layer="Finite Chat round trip after restore",
        error="chat.after_restart must be {ok: true, message_id: '<event-id>'}",
    )

    status = "passed" if not errors else "failed"
    report = {
        "status": status,
        "schema_version": 1,
        "generated_at_unix": int(time.time()),
        "proof_layers": sorted(proof_layers),
        "missing_proof_layers": sorted(REQUIRED_PROOF_LAYERS - proof_layers),
        "errors": errors,
        "recovery_scope": recovery_scope,
        "facts": {
            "container_name": container_name,
            "container_url": container_url,
            "container_status": container_status or None,
            "image_digest": image_digest,
            "image_source": image_source,
            "restic_backend": restic_backend,
            "restore_tag": restore_tag,
            "storage_source": storage_source,
            "expected_container_name": expected_container_name,
            "expected_image_digest": expected_image_digest,
            "expected_storage_backend": expected_storage_backend,
            "expected_restore_tag": expected_restore_tag,
            "handoff_source_present": handoff_source_present,
            "canary_summary_source_present": summary_source_present,
            "container_json_source_present": container_source_present,
            "health_json_source_present": health_source_present,
            "health_ready": health_ready,
            "health_npub": health_npub,
            "agent_npub_before_restart": before_npub,
            "agent_npub_after_restore": after_npub,
            "chat_before_restart": chat_before,
            "chat_before_message_id": chat_before_message_id,
            "chat_after_restart": chat_after,
            "chat_after_message_id": chat_after_message_id,
            "backup_observed": backup_observed,
            "restore_observed": restore_observed,
            "same_npub": same_npub_declared and same_npub_observed,
        },
    }
    return (0 if status == "passed" else 2), report


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--evidence-json",
        required=True,
        help="Operator-collected live Tinfoil canary evidence JSON.",
    )
    parser.add_argument(
        "--report",
        default="target/hermes-docker-smoke/tinfoil-canary-result.json",
    )
    args = parser.parse_args()

    evidence_path = Path(args.evidence_json)
    report_path = Path(args.report)
    status, report = validate(load_json(evidence_path))
    report["evidence_json"] = str(evidence_path)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(report, indent=2) + "\n"
    report_path.write_text(text, encoding="utf-8")
    print(text, end="")
    return status


if __name__ == "__main__":
    sys.exit(main())

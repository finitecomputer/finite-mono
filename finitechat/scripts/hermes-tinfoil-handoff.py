#!/usr/bin/env python3
"""Build a redacted Tinfoil handoff report from proven Docker/S3 artifacts."""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path
from typing import Any

DEFAULT_HERMES_MODEL = "anthropic/claude-sonnet-4.6"
DEFAULT_HERMES_PROVIDER = "openrouter"
DEFAULT_AGENT_BACKUP_INTERVAL_SECS = "30"


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def require(condition: bool, message: str, errors: list[str]) -> None:
    if not condition:
        errors.append(message)


def first_digest(publish: dict[str, Any]) -> str | None:
    digests = publish.get("repo_digests")
    if isinstance(digests, list) and digests:
        return str(digests[0])
    return None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--smoke-report", required=True)
    parser.add_argument("--preflight-report", required=True)
    parser.add_argument("--publish-report", required=True)
    parser.add_argument("--handoff-report", required=True)
    args = parser.parse_args()

    smoke_path = Path(args.smoke_report)
    preflight_path = Path(args.preflight_report)
    publish_path = Path(args.publish_report)
    handoff_path = Path(args.handoff_report)
    smoke = load_json(smoke_path)
    preflight = load_json(preflight_path)
    publish = load_json(publish_path)
    facts = smoke.get("facts") if isinstance(smoke.get("facts"), dict) else {}
    backup = (
        facts.get("agent_state_backup") if isinstance(facts.get("agent_state_backup"), dict) else {}
    )
    snapshot = backup.get("snapshot") if isinstance(backup.get("snapshot"), dict) else {}
    repository = backup.get("repository") if isinstance(backup.get("repository"), dict) else {}
    errors: list[str] = []

    require(smoke.get("status") == "passed", "Docker smoke report must be passed", errors)
    require(preflight.get("status") == "ok", "Restic preflight must be ok", errors)
    require(publish.get("status") == "published", "Image publish report must be published", errors)
    require(facts.get("restic_backend") == "s3", "Docker smoke must use restic_backend=s3", errors)
    require(
        facts.get("real_gateway_runtime") is True,
        "Docker smoke must prove real_gateway_runtime=true",
        errors,
    )
    require(
        facts.get("gateway_admission_before_restore") is True,
        "Docker smoke must prove gateway admission before restore",
        errors,
    )
    require(
        facts.get("gateway_admission_after_restore") is True,
        "Docker smoke must prove gateway admission after restore",
        errors,
    )
    require(preflight.get("backend") == "s3", "Restic preflight must use backend=s3", errors)
    require(repository.get("kind") == "s3", "Restic repository proof must be kind=s3", errors)
    require(bool(snapshot.get("id")), "Restic snapshot id is required", errors)
    require(bool(facts.get("image_id")), "Docker smoke image id is required", errors)
    require(
        publish.get("source_image_id") == facts.get("image_id"),
        "Publish report source image id must match Docker smoke image id",
        errors,
    )
    digest = first_digest(publish)
    require(bool(digest), "Published image repo digest is required", errors)

    status = "ready" if not errors else "failed"
    handoff: dict[str, Any] = {
        "status": status,
        "generated_at_unix": int(time.time()),
        "errors": errors,
        "source_reports": {
            "smoke": str(smoke_path),
            "preflight": str(preflight_path),
            "publish": str(publish_path),
        },
        "image": {
            "source_image_id": facts.get("image_id"),
            "target_ref": publish.get("target_image_ref"),
            "digest": digest,
        },
        "runtime": {
            "hermes_agent_version": facts.get("hermes_agent_version_actual"),
            "restic_version": facts.get("restic_version"),
            "finitechat_hermes_inbound_stream": "1",
            "finitechat_hermes_model": DEFAULT_HERMES_MODEL,
            "finitechat_hermes_provider": DEFAULT_HERMES_PROVIDER,
            "finite_agent_restore_on_start": "1",
            "finite_agent_restore_latest": "1",
            "finite_agent_backup_on_exit": "1",
            "finite_agent_backup_interval_secs": DEFAULT_AGENT_BACKUP_INTERVAL_SECS,
        },
        "restore": {
            "backend": facts.get("restic_backend"),
            "repository": repository,
            "seed_snapshot_id": snapshot.get("id"),
            "seed_snapshot_short_id": snapshot.get("short_id"),
            "seed_snapshot_time": snapshot.get("time"),
            "restore_selector": "latest",
            "restore_tag": backup.get("tag"),
            "required_secret_env": [
                "FINITE_AGENT_RESTIC_PASSWORD",
                "AWS_ACCESS_KEY_ID",
                "AWS_SECRET_ACCESS_KEY",
                "OPENROUTER_API_KEY",
            ],
            "optional_secret_env": [
                "AWS_REGION",
                "AWS_DEFAULT_REGION",
                "AWS_SESSION_TOKEN",
                "ANTHROPIC_API_KEY",
                "OPENAI_API_KEY",
            ],
            "container_env": {
                "FINITE_AGENT_RESTORE_ON_START": "1",
                "FINITE_AGENT_RESTORE_LATEST": "1",
                "FINITE_AGENT_BACKUP_ON_EXIT": "1",
                "FINITE_AGENT_BACKUP_INTERVAL_SECS": DEFAULT_AGENT_BACKUP_INTERVAL_SECS,
                "FINITE_AGENT_RESTIC_REPOSITORY": repository.get("repository"),
                "FINITE_AGENT_RESTIC_BACKUP_TAG": backup.get("tag"),
                "FINITECHAT_HERMES_INBOUND_STREAM": "1",
                "FINITECHAT_HERMES_MODEL": DEFAULT_HERMES_MODEL,
                "FINITECHAT_HERMES_PROVIDER": DEFAULT_HERMES_PROVIDER,
            },
        },
        "acceptance": [
            "create Tinfoil container from image.digest",
            "start with empty local disk",
            "entrypoint restores latest restore.restore_tag snapshot from restore.repository",
            "restore with temporary canary restic password secret",
            "print invite URL",
            "chat once from Finite Chat",
            "observe a fresh periodic or exit snapshot of agent state",
            "restart container from empty local disk",
            "restore again",
            "chat again",
        ],
    }

    handoff_path.parent.mkdir(parents=True, exist_ok=True)
    handoff_path.write_text(json.dumps(handoff, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(handoff, indent=2))
    return 0 if not errors else 2


if __name__ == "__main__":
    sys.exit(main())

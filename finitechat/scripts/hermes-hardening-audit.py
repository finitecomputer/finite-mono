#!/usr/bin/env python3
"""Audit Hermes hardening evidence reports without overstating completion."""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path
from typing import Any

REQUIRED_DOCKER_PROOF_LAYERS = {
    "docker image build",
    "hermes-agent 0.17 runtime",
    "finitechat binary in image",
    "finitechat plugin in image",
    "real Hermes gateway process",
    "gateway invite admission before restore",
    "entrypoint restic encrypted agent state snapshot on shutdown",
    "restic repository check",
    "agent state volume wipe",
    "fresh agent container with empty local state",
    "entrypoint restic latest-by-tag restore into fresh volume",
    "same agent npub after restore",
    "runtime HTTP health endpoint after restore",
    "gateway invite admission after restore",
}
REQUIRED_SIDECAR_PROOF_LAYERS = {
    "finitechat-server",
    "finitechat hermes CLI",
    "encrypted client stores",
    "finitechat hermes serve",
    "sidecar /v1/hermes/inbound NDJSON",
    "ack/drain",
    "agent reply",
    "user decrypt",
}
REQUIRED_ADAPTER_REGRESSION_LAYERS = {
    "plain message mapping",
    "redelivery dedupe",
    "ack retry without duplicate dispatch",
    "transient poll recovery",
    "sidecar startup",
    "service fallback",
    "service serialization",
    "media attachments",
    "outbound edit route",
    "typing activity",
    "room filtering",
    "group sender identity",
    "receipt/control stream filtering",
    "inbound stream fallback",
}
REQUIRED_MEDIA_E2E_STEPS = {
    "server_ready",
    "agent_init",
    "adapter_connect",
    "user_join",
    "user_send_media",
    "agent_receive_media",
    "user_receive_agent_replies",
}
REQUIRED_IOS_MEDIA_E2E_STEPS = {
    "server_ready",
    "agent_init",
    "adapter_connect",
    "ios_app_launch",
    "agent_receive_ios_media",
    "ios_receive_agent_replies",
}
REQUIRED_TINFOIL_PROOF_LAYERS = {
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
REQUIRED_CANARY_CONFIG_SNIPPETS = {
    "FINITE_AGENT_RESTORE_ON_START",
    "FINITE_SERVER_URL",
    "FINITECHAT_SERVER_URL",
    "FINITE_AGENT_RESTORE_LATEST",
    "FINITE_AGENT_BACKUP_ON_EXIT",
    "FINITE_AGENT_BACKUP_INTERVAL_SECS",
    "FINITE_AGENT_RESTIC_REPOSITORY",
    "FINITE_AGENT_RESTIC_BACKUP_TAG",
    "FINITECHAT_HERMES_INBOUND_STREAM",
    "OPENROUTER_API_KEY",
    "/healthz",
    "/invite",
    "upstream-port: 8080",
}
REQUIRED_CANARY_RUNBOOK_SNIPPETS = {
    "scripts/hermes-tinfoil-canary-evidence.py",
    "scripts/hermes-tinfoil-canary-result.py",
    "--image-digest",
    "--storage-backend s3",
    "--restore-tag finite-agent-state",
    "--chat-before-message-id",
    "--chat-after-message-id",
    "--debug",
    "--ssh-key",
}
REQUIRED_CANARY_SECRET_ENV = {
    "FINITE_AGENT_RESTIC_PASSWORD",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "OPENROUTER_API_KEY",
}
REQUIRED_GITHUB_PUBLISH_ARTIFACTS = {
    "target/hermes-hardening-audit.json",
    "target/hermes-docker-smoke/report.json",
    "target/hermes-docker-smoke/restic-preflight.json",
    "target/hermes-docker-smoke/image-publish.json",
    "target/hermes-docker-smoke/tinfoil-handoff.json",
    "target/hermes-docker-smoke/tinfoil-canary/tinfoil-canary-summary.json",
}
REQUIRED_HANDOFF_RUNTIME = {
    "hermes_agent_version": "0.18.2",
    "finitechat_hermes_inbound_stream": "1",
    "finite_agent_restore_on_start": "1",
    "finite_agent_restore_latest": "1",
    "finite_agent_backup_on_exit": "1",
    "finite_agent_backup_interval_secs": "30",
}
REQUIRED_HANDOFF_CONTAINER_ENV = {
    "FINITE_AGENT_RESTORE_ON_START": "1",
    "FINITE_AGENT_RESTORE_LATEST": "1",
    "FINITE_AGENT_BACKUP_ON_EXIT": "1",
    "FINITE_AGENT_BACKUP_INTERVAL_SECS": "30",
    "FINITE_AGENT_RESTIC_BACKUP_TAG": "finite-agent-state",
    "FINITECHAT_HERMES_INBOUND_STREAM": "1",
    "FINITECHAT_HERMES_MODEL": "anthropic/claude-sonnet-4.6",
    "FINITECHAT_HERMES_PROVIDER": "openrouter",
}
REQUIRED_PUBLISH_PROOF = {
    "smoke_status": "passed",
    "hermes_agent_version_actual": "0.18.2",
    "restic_backend": "s3",
    "real_gateway_runtime": True,
    "gateway_admission_before_restore": True,
    "gateway_admission_after_restore": True,
}
RESTIC_SNAPSHOT_TAG = "finite-agent-state"
RESTIC_AGENT_STATE_PATH = "/data/agent"


def load_optional_json(path: Path) -> dict[str, Any] | None:
    if not path.exists():
        return None
    return json.loads(path.read_text(encoding="utf-8"))


def add_check(
    checks: list[dict[str, Any]],
    *,
    name: str,
    status: str,
    evidence: str | None = None,
    detail: str | None = None,
) -> None:
    checks.append(
        {
            "name": name,
            "status": status,
            "evidence": evidence,
            "detail": detail,
        }
    )


def missing_layers(report: dict[str, Any], required: set[str]) -> list[str]:
    present = set(report.get("proof_layers") or [])
    return sorted(required - present)


def list_detail(value: Any) -> str:
    if isinstance(value, list):
        return ", ".join(str(item) for item in value)
    return str(value) if value else ""


def non_empty_str(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def resolve_summary_artifact_path(summary_path: Path, value: Any) -> Path | None:
    if not non_empty_str(value):
        return None
    path = Path(value)
    if path.is_absolute():
        return path
    if path.exists():
        return path
    sibling = summary_path.parent / path.name
    if sibling.exists():
        return sibling
    return path


def read_existing_text(path: Path | None) -> str | None:
    if path is None or not path.exists() or not path.is_file():
        return None
    return path.read_text(encoding="utf-8")


def validate_canary_summary(
    canary_summary: dict[str, Any] | None,
    canary_summary_path: Path,
    handoff: dict[str, Any] | None,
) -> list[str]:
    if not canary_summary:
        return ["requires generated digest-pinned Tinfoil config/runbook"]

    errors: list[str] = []
    if canary_summary.get("status") != "ready":
        errors.append(f"summary status={canary_summary.get('status')!r}")

    image_digest = canary_summary.get("image_digest")
    if not non_empty_str(image_digest) or "@sha256:" not in str(image_digest):
        errors.append("summary image_digest must pin sha256")

    handoff_image = handoff.get("image") if isinstance(handoff, dict) else None
    handoff_digest = handoff_image.get("digest") if isinstance(handoff_image, dict) else None
    if (
        isinstance(handoff, dict)
        and handoff.get("status") == "ready"
        and non_empty_str(handoff_digest)
        and image_digest != handoff_digest
    ):
        errors.append("summary image_digest does not match handoff image.digest")

    for key in ("config_repo", "release_tag", "container_name", "finite_server_url"):
        if not non_empty_str(canary_summary.get(key)):
            errors.append(f"summary {key} is required")
    config_repo = canary_summary.get("config_repo")
    if non_empty_str(config_repo) and "/" not in str(config_repo):
        errors.append("summary config_repo must be owner/repo")

    secret_env = canary_summary.get("secret_env")
    if not isinstance(secret_env, list):
        errors.append("summary secret_env must be a list")
    else:
        missing_secret_env = sorted(REQUIRED_CANARY_SECRET_ENV - {str(item) for item in secret_env})
        if missing_secret_env:
            errors.append(f"summary secret_env missing: {', '.join(missing_secret_env)}")

    config_path = resolve_summary_artifact_path(canary_summary_path, canary_summary.get("config"))
    runbook_path = resolve_summary_artifact_path(canary_summary_path, canary_summary.get("runbook"))
    config_text = read_existing_text(config_path)
    runbook_text = read_existing_text(runbook_path)
    if config_text is None:
        errors.append("summary config path is missing or unreadable")
    else:
        if non_empty_str(image_digest) and str(image_digest) not in config_text:
            errors.append("config does not contain summary image_digest")
        missing_config = sorted(
            snippet for snippet in REQUIRED_CANARY_CONFIG_SNIPPETS if snippet not in config_text
        )
        if missing_config:
            errors.append(f"config missing snippets: {', '.join(missing_config)}")
    if runbook_text is None:
        errors.append("summary runbook path is missing or unreadable")
    else:
        if non_empty_str(image_digest) and str(image_digest) not in runbook_text:
            errors.append("runbook does not contain summary image_digest")
        missing_runbook = sorted(
            snippet for snippet in REQUIRED_CANARY_RUNBOOK_SNIPPETS if snippet not in runbook_text
        )
        if missing_runbook:
            errors.append(f"runbook missing snippets: {', '.join(missing_runbook)}")
    return errors


def path_matches_artifact(value: str, artifact: str) -> bool:
    normalized = value.replace("\\", "/")
    return normalized == artifact or normalized.endswith(f"/{artifact}")


def validate_github_publish_gate(github_publish_gate: dict[str, Any] | None) -> list[str]:
    if not github_publish_gate:
        return ["requires pushed branch and successful GitHub publish-gate run"]

    errors: list[str] = []
    if github_publish_gate.get("status") != "passed":
        report_errors = list_detail(github_publish_gate.get("errors"))
        errors.append(report_errors or f"publish-gate status={github_publish_gate.get('status')!r}")

    for key in ("run_id", "run_url"):
        if not non_empty_str(github_publish_gate.get(key)):
            errors.append(f"publish-gate {key} is required")

    for key in ("watch_exit_code", "download_exit_code", "local_audit_exit_code"):
        if github_publish_gate.get(key) != 0:
            errors.append(f"publish-gate {key}={github_publish_gate.get(key)!r}; expected 0")

    downloaded_files = github_publish_gate.get("downloaded_files")
    if not isinstance(downloaded_files, list) or not downloaded_files:
        errors.append("publish-gate downloaded_files must be non-empty")

    artifact_ingest = github_publish_gate.get("artifact_ingest")
    if not isinstance(artifact_ingest, dict):
        errors.append("publish-gate artifact_ingest is required")
        return errors

    if artifact_ingest.get("status") != "ok":
        errors.append(f"publish-gate artifact_ingest status={artifact_ingest.get('status')!r}")
    missing = artifact_ingest.get("missing")
    if isinstance(missing, list) and missing:
        errors.append(
            f"publish-gate artifact_ingest missing: {', '.join(str(item) for item in missing)}"
        )
    elif not isinstance(missing, list):
        errors.append("publish-gate artifact_ingest missing must be a list")

    copied = artifact_ingest.get("copied")
    if not isinstance(copied, list):
        errors.append("publish-gate artifact_ingest copied must be a list")
        return errors
    destinations = {
        str(item.get("destination"))
        for item in copied
        if isinstance(item, dict) and non_empty_str(item.get("destination"))
    }
    missing_copied = sorted(
        artifact
        for artifact in REQUIRED_GITHUB_PUBLISH_ARTIFACTS
        if not any(path_matches_artifact(destination, artifact) for destination in destinations)
    )
    if missing_copied:
        errors.append(f"publish-gate artifact_ingest copied missing: {', '.join(missing_copied)}")
    return errors


def validate_restic_repository(
    value: Any,
    *,
    expected_kind: str | None = None,
    label: str,
    require_local_size: bool = True,
) -> list[str]:
    if not isinstance(value, dict):
        return [f"{label} is required"]

    errors: list[str] = []
    kind = value.get("kind")
    if expected_kind and kind != expected_kind:
        errors.append(f"{label}.kind={kind!r}; expected {expected_kind!r}")
    elif not expected_kind and kind not in {"local", "s3"}:
        errors.append(f"{label}.kind={kind!r}; expected 'local' or 's3'")

    if kind == "s3":
        repository = value.get("repository")
        if not non_empty_str(repository) or not str(repository).startswith("s3:"):
            errors.append(f"{label}.repository must be an s3: URL")
    elif kind == "local":
        if not non_empty_str(value.get("path")):
            errors.append(f"{label}.path is required for local restic repositories")
        size_bytes = value.get("size_bytes")
        if require_local_size and (not isinstance(size_bytes, int) or size_bytes <= 0):
            errors.append(f"{label}.size_bytes must be a positive integer")
        elif not require_local_size and size_bytes is not None and not isinstance(size_bytes, int):
            errors.append(f"{label}.size_bytes must be an integer when present")

    return errors


def restic_repository_identity(value: Any) -> tuple[str, str] | None:
    if not isinstance(value, dict):
        return None
    kind = value.get("kind")
    if kind == "s3" and non_empty_str(value.get("repository")):
        return ("s3", str(value["repository"]))
    if kind == "local" and non_empty_str(value.get("path")):
        return ("local", str(value["path"]))
    return None


def validate_restic_snapshot(value: Any, *, label: str) -> list[str]:
    if not isinstance(value, dict):
        return [f"{label} is required"]

    errors: list[str] = []
    for key in ("id", "short_id", "time"):
        if not non_empty_str(value.get(key)):
            errors.append(f"{label}.{key} is required")

    paths = value.get("paths")
    if not isinstance(paths, list) or RESTIC_AGENT_STATE_PATH not in {str(item) for item in paths}:
        errors.append(f"{label}.paths must include {RESTIC_AGENT_STATE_PATH}")

    tags = value.get("tags")
    if not isinstance(tags, list) or RESTIC_SNAPSHOT_TAG not in {str(item) for item in tags}:
        errors.append(f"{label}.tags must include {RESTIC_SNAPSHOT_TAG}")

    return errors


def validate_agent_state_backup(
    facts: dict[str, Any],
    *,
    expected_repository_kind: str | None = None,
) -> list[str]:
    errors: list[str] = []
    restic_backend = facts.get("restic_backend")
    if restic_backend not in {"local", "s3"}:
        errors.append(f"restic_backend={restic_backend!r}; expected 'local' or 's3'")

    backup = facts.get("agent_state_backup")
    if not isinstance(backup, dict):
        return [*errors, "agent_state_backup is required"]

    if backup.get("backend") != "restic":
        errors.append(f"agent_state_backup.backend={backup.get('backend')!r}; expected 'restic'")
    if backup.get("source") != "entrypoint_backup_on_exit":
        errors.append("agent_state_backup.source must be 'entrypoint_backup_on_exit'")
    if backup.get("encrypted") is not True:
        errors.append("agent_state_backup.encrypted must be true")
    if backup.get("tag") != RESTIC_SNAPSHOT_TAG:
        errors.append(
            f"agent_state_backup.tag={backup.get('tag')!r}; expected {RESTIC_SNAPSHOT_TAG!r}"
        )
    if not non_empty_str(facts.get("restic_version")):
        errors.append("restic_version is required")

    repository_kind = expected_repository_kind or (
        str(restic_backend) if restic_backend in {"local", "s3"} else None
    )
    errors.extend(
        validate_restic_repository(
            backup.get("repository"),
            expected_kind=repository_kind,
            label="agent_state_backup.repository",
        )
    )
    errors.extend(
        validate_restic_snapshot(backup.get("snapshot"), label="agent_state_backup.snapshot")
    )

    top_level_repository = facts.get("restic_repository")
    errors.extend(
        validate_restic_repository(
            top_level_repository,
            expected_kind=repository_kind,
            label="restic_repository",
            require_local_size=False,
        )
    )
    backup_identity = restic_repository_identity(backup.get("repository"))
    top_level_identity = restic_repository_identity(top_level_repository)
    if backup_identity and top_level_identity and backup_identity != top_level_identity:
        errors.append("agent_state_backup.repository does not match restic_repository")

    return errors


def validate_real_s3_smoke_facts(facts: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if facts.get("restic_backend") != "s3":
        errors.append(f"restic_backend={facts.get('restic_backend')!r}; expected 's3'")
    if facts.get("s3_endpoint_kind") == "local_emulator":
        errors.append("s3_endpoint_kind='local_emulator'; expected real S3")
    errors.extend(validate_agent_state_backup(facts, expected_repository_kind="s3"))
    return errors


def validate_s3_preflight(preflight: dict[str, Any] | None) -> list[str]:
    if not preflight:
        return ["requires S3 restic preflight report"]

    errors: list[str] = []
    if preflight.get("status") != "ok":
        errors.append(f"preflight status={preflight.get('status')!r}; expected 'ok'")
    if preflight.get("backend") != "s3":
        errors.append(f"preflight backend={preflight.get('backend')!r}; expected 's3'")
    repository = preflight.get("repository")
    if not non_empty_str(repository) or not str(repository).startswith("s3:"):
        errors.append("preflight repository must be an s3: URL")

    report_errors = preflight.get("errors")
    if isinstance(report_errors, list) and report_errors:
        errors.append(f"preflight errors: {', '.join(str(item) for item in report_errors)}")

    env = preflight.get("env")
    if not isinstance(env, dict):
        errors.append("preflight env is required")
    else:
        for key in (
            "FINITE_DOCKER_RESTIC_REPOSITORY",
            "FINITE_DOCKER_RESTIC_PASSWORD",
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
        ):
            if env.get(key) is not True:
                errors.append(f"preflight env.{key} must be true")

    return errors


def validate_tinfoil_handoff(
    handoff: dict[str, Any] | None,
    *,
    docker_facts: dict[str, Any],
    publish: dict[str, Any] | None,
) -> list[str]:
    if not handoff:
        return ["requires ready handoff from S3 smoke and published digest"]

    errors: list[str] = []
    if handoff.get("status") != "ready":
        report_errors = list_detail(handoff.get("errors"))
        errors.append(report_errors or f"handoff status={handoff.get('status')!r}")
    handoff_errors = handoff.get("errors")
    if isinstance(handoff_errors, list) and handoff_errors:
        errors.append(f"handoff errors: {', '.join(str(item) for item in handoff_errors)}")

    source_reports = handoff.get("source_reports")
    if not isinstance(source_reports, dict):
        errors.append("handoff source_reports is required")
    else:
        for key in ("smoke", "preflight", "publish"):
            if not non_empty_str(source_reports.get(key)):
                errors.append(f"handoff source_reports.{key} is required")

    image = handoff.get("image")
    if not isinstance(image, dict):
        errors.append("handoff image is required")
    else:
        image_digest = image.get("digest")
        if not non_empty_str(image_digest) or "@sha256:" not in str(image_digest):
            errors.append("handoff image.digest must pin sha256")
        if not non_empty_str(image.get("target_ref")):
            errors.append("handoff image.target_ref is required")
        if not non_empty_str(image.get("source_image_id")):
            errors.append("handoff image.source_image_id is required")
        docker_image_id = docker_facts.get("image_id")
        if non_empty_str(docker_image_id) and image.get("source_image_id") != docker_image_id:
            errors.append("handoff image.source_image_id does not match Docker smoke image_id")
        repo_digests = publish.get("repo_digests") if isinstance(publish, dict) else None
        if isinstance(repo_digests, list) and repo_digests and image_digest not in repo_digests:
            errors.append("handoff image.digest is not in publish repo_digests")

    runtime = handoff.get("runtime")
    if not isinstance(runtime, dict):
        errors.append("handoff runtime is required")
    else:
        for key, expected in REQUIRED_HANDOFF_RUNTIME.items():
            if runtime.get(key) != expected:
                errors.append(f"handoff runtime.{key}={runtime.get(key)!r}; expected {expected!r}")

    restore = handoff.get("restore")
    if not isinstance(restore, dict):
        errors.append("handoff restore is required")
        return errors
    if restore.get("backend") != "s3":
        errors.append(f"handoff restore.backend={restore.get('backend')!r}; expected 's3'")
    if restore.get("restore_selector") != "latest":
        errors.append(
            f"handoff restore.restore_selector={restore.get('restore_selector')!r}; expected 'latest'"
        )
    if restore.get("restore_tag") != "finite-agent-state":
        errors.append(
            f"handoff restore.restore_tag={restore.get('restore_tag')!r}; expected 'finite-agent-state'"
        )
    if not non_empty_str(restore.get("seed_snapshot_id")):
        errors.append("handoff restore.seed_snapshot_id is required")

    repository = restore.get("repository")
    if not isinstance(repository, dict):
        errors.append("handoff restore.repository is required")
    else:
        if repository.get("kind") != "s3":
            errors.append(
                f"handoff restore.repository.kind={repository.get('kind')!r}; expected 's3'"
            )
        repository_url = repository.get("repository")
        if not non_empty_str(repository_url) or not str(repository_url).startswith("s3:"):
            errors.append("handoff restore.repository.repository must be an s3: URL")

    secret_env = restore.get("required_secret_env")
    if not isinstance(secret_env, list):
        errors.append("handoff restore.required_secret_env must be a list")
    else:
        missing_secret_env = sorted(REQUIRED_CANARY_SECRET_ENV - {str(item) for item in secret_env})
        if missing_secret_env:
            errors.append(
                f"handoff restore.required_secret_env missing: {', '.join(missing_secret_env)}"
            )

    container_env = restore.get("container_env")
    if not isinstance(container_env, dict):
        errors.append("handoff restore.container_env is required")
    else:
        for key, expected in REQUIRED_HANDOFF_CONTAINER_ENV.items():
            if container_env.get(key) != expected:
                errors.append(
                    f"handoff restore.container_env.{key}={container_env.get(key)!r}; "
                    f"expected {expected!r}"
                )
        repository_url = container_env.get("FINITE_AGENT_RESTIC_REPOSITORY")
        if not non_empty_str(repository_url) or not str(repository_url).startswith("s3:"):
            errors.append(
                "handoff restore.container_env.FINITE_AGENT_RESTIC_REPOSITORY must be an s3: URL"
            )

    return errors


def validate_publish_report(
    publish: dict[str, Any] | None,
    *,
    docker_facts: dict[str, Any],
) -> list[str]:
    if not publish:
        return ["requires published image report with repo digest matching Docker smoke image id"]

    errors: list[str] = []
    if publish.get("status") != "published":
        errors.append(f"publish status={publish.get('status')!r}; expected 'published'")
    if publish.get("pushed") is not True:
        errors.append(f"publish pushed={publish.get('pushed')!r}; expected True")

    for key in ("source_report", "source_image", "source_image_id", "target_image_ref"):
        if not non_empty_str(publish.get(key)):
            errors.append(f"publish {key} is required")

    docker_image = docker_facts.get("image")
    if non_empty_str(docker_image) and publish.get("source_image") != docker_image:
        errors.append("publish source_image does not match Docker smoke image")
    docker_image_id = docker_facts.get("image_id")
    if non_empty_str(docker_image_id) and publish.get("source_image_id") != docker_image_id:
        errors.append("publish source_image_id does not match Docker smoke image_id")

    repo_digests = publish.get("repo_digests")
    if not isinstance(repo_digests, list) or not repo_digests:
        errors.append("publish repo_digests must be a non-empty list")
    else:
        digest_values = [str(item) for item in repo_digests]
        if not any("@sha256:" in value for value in digest_values):
            errors.append("publish repo_digests must include a sha256 digest")

    proof = publish.get("proof")
    if not isinstance(proof, dict):
        errors.append("publish proof is required")
        return errors
    for key, expected in REQUIRED_PUBLISH_PROOF.items():
        if proof.get(key) != expected:
            errors.append(f"publish proof.{key}={proof.get(key)!r}; expected {expected!r}")
    if not non_empty_str(proof.get("restic_version")):
        errors.append("publish proof.restic_version is required")
    if not non_empty_str(proof.get("agent_npub_after_restore")):
        errors.append("publish proof.agent_npub_after_restore is required")
    docker_npub = docker_facts.get("agent_npub_after_restore")
    if non_empty_str(docker_npub) and proof.get("agent_npub_after_restore") != docker_npub:
        errors.append("publish proof.agent_npub_after_restore does not match Docker smoke")
    return errors


def step_names(report: dict[str, Any]) -> set[str]:
    steps = report.get("steps")
    if not isinstance(steps, list):
        return set()
    return {
        str(step.get("name"))
        for step in steps
        if isinstance(step, dict) and isinstance(step.get("name"), str)
    }


def audit(args: argparse.Namespace) -> dict[str, Any]:
    adapter_regression_path = Path(args.adapter_regression_report)
    sidecar_path = Path(args.sidecar_report)
    media_e2e_path = Path(args.media_e2e_report)
    ios_media_e2e_path = Path(args.ios_media_e2e_report)
    docker_path = Path(args.docker_report)
    s3_emulator_path = Path(args.s3_emulator_report)
    github_setup_path = Path(args.github_setup_report)
    github_publish_gate_path = Path(args.github_publish_gate_report)
    preflight_path = Path(args.preflight_report)
    publish_path = Path(args.publish_report)
    handoff_path = Path(args.handoff_report)
    canary_summary_path = Path(args.canary_summary)
    tinfoil_result_path = Path(args.tinfoil_result)

    adapter_regression = load_optional_json(adapter_regression_path)
    sidecar = load_optional_json(sidecar_path)
    media_e2e = load_optional_json(media_e2e_path)
    ios_media_e2e = load_optional_json(ios_media_e2e_path)
    docker = load_optional_json(docker_path)
    s3_emulator = load_optional_json(s3_emulator_path)
    github_setup = load_optional_json(github_setup_path)
    github_publish_gate = load_optional_json(github_publish_gate_path)
    preflight = load_optional_json(preflight_path)
    publish = load_optional_json(publish_path)
    handoff = load_optional_json(handoff_path)
    canary_summary = load_optional_json(canary_summary_path)
    tinfoil_result = load_optional_json(tinfoil_result_path)

    checks: list[dict[str, Any]] = []
    adapter_missing = missing_layers(adapter_regression or {}, REQUIRED_ADAPTER_REGRESSION_LAYERS)
    adapter_passed = (
        bool(adapter_regression)
        and adapter_regression.get("status") == "passed"
        and not adapter_missing
        and int(adapter_regression.get("test_count") or 0)
        >= len(REQUIRED_ADAPTER_REGRESSION_LAYERS)
    )
    add_check(
        checks,
        name="adapter_focused_regressions",
        status="passed" if adapter_passed else "missing",
        evidence=str(adapter_regression_path) if adapter_regression else None,
        detail=None
        if adapter_passed
        else f"missing regression layers: {', '.join(adapter_missing)}",
    )

    add_check(
        checks,
        name="local_sidecar_smoke",
        status="passed"
        if sidecar
        and sidecar.get("status") == "passed"
        and not missing_layers(sidecar, REQUIRED_SIDECAR_PROOF_LAYERS)
        else "missing",
        evidence=str(sidecar_path) if sidecar else None,
        detail=None
        if sidecar and not missing_layers(sidecar, REQUIRED_SIDECAR_PROOF_LAYERS)
        else f"missing layers: {', '.join(missing_layers(sidecar or {}, REQUIRED_SIDECAR_PROOF_LAYERS))}",
    )

    media_facts = media_e2e.get("facts", {}) if isinstance(media_e2e, dict) else {}
    media_steps_missing = sorted(REQUIRED_MEDIA_E2E_STEPS - step_names(media_e2e or {}))
    media_texts = media_facts.get("user_received_text")
    media_types = media_facts.get("agent_received_media_types")
    media_passed = (
        bool(media_e2e)
        and media_e2e.get("status") == "passed"
        and not media_steps_missing
        and media_facts.get("adapter_inbound_stream") is True
        and media_facts.get("adapter_service_url_present") is True
        and isinstance(media_types, list)
        and "image/png" in media_types
        and isinstance(media_texts, list)
        and "agent text echo: user media hello" in media_texts
        and "agent media echo" in media_texts
        and int(media_facts.get("user_received_media_count") or 0) >= 1
    )
    add_check(
        checks,
        name="local_hermes_agent_media_e2e",
        status="passed" if media_passed else "missing",
        evidence=str(media_e2e_path) if media_e2e else None,
        detail=None
        if media_passed
        else (
            "requires live hermes-agent media e2e report with sidecar stream, "
            f"text and image replies, and user decrypt; missing steps: {', '.join(media_steps_missing)}"
        ),
    )

    ios_facts = ios_media_e2e.get("facts", {}) if isinstance(ios_media_e2e, dict) else {}
    ios_steps_missing = sorted(REQUIRED_IOS_MEDIA_E2E_STEPS - step_names(ios_media_e2e or {}))
    ios_texts = ios_facts.get("ios_received_text")
    ios_media_types = ios_facts.get("agent_received_media_types")
    ios_passed = (
        bool(ios_media_e2e)
        and ios_media_e2e.get("status") == "passed"
        and ios_media_e2e.get("name") == "ios_simulator_hermes_agent_media_e2e"
        and not ios_steps_missing
        and ios_facts.get("platform") == "ios_simulator"
        and ios_facts.get("adapter_inbound_stream") is True
        and ios_facts.get("adapter_service_url_present") is True
        and bool(ios_facts.get("simulator_udid"))
        and isinstance(ios_media_types, list)
        and "image/png" in ios_media_types
        and isinstance(ios_texts, list)
        and "agent text echo: ios media hello" in ios_texts
        and "agent media echo" in ios_texts
        and int(ios_facts.get("ios_received_media_count") or 0) >= 1
    )
    add_check(
        checks,
        name="ios_simulator_media_e2e",
        status="passed" if ios_passed else "missing",
        evidence=str(ios_media_e2e_path) if ios_media_e2e else None,
        detail=None
        if ios_passed
        else (
            "requires live iOS Simulator media e2e report with sidecar stream, "
            f"native store decrypt, and text/image replies; missing steps: {', '.join(ios_steps_missing)}"
        ),
    )

    docker_missing = missing_layers(docker or {}, REQUIRED_DOCKER_PROOF_LAYERS)
    docker_facts = docker.get("facts", {}) if isinstance(docker, dict) else {}
    docker_backup_errors = validate_agent_state_backup(docker_facts)
    docker_passed = (
        bool(docker)
        and docker.get("status") == "passed"
        and not docker_missing
        and not docker_backup_errors
        and docker_facts.get("hermes_agent_version_actual") == "0.18.2"
        and docker_facts.get("agent_npub") == docker_facts.get("agent_npub_after_restore")
        and docker_facts.get("real_gateway_runtime") is True
        and docker_facts.get("gateway_admission_before_restore") is True
        and docker_facts.get("gateway_admission_after_restore") is True
    )
    add_check(
        checks,
        name="docker_runtime_local_or_s3_smoke",
        status="passed" if docker_passed else "missing",
        evidence=str(docker_path) if docker else None,
        detail=None
        if docker_passed
        else (
            f"missing layers: {', '.join(docker_missing)}; "
            f"backup proof errors: {'; '.join(docker_backup_errors)}"
        ),
    )
    s3_endpoint_kind = docker_facts.get("s3_endpoint_kind")
    s3_smoke_errors = validate_real_s3_smoke_facts(docker_facts)
    s3_smoke = docker_passed and not s3_smoke_errors

    s3_emulator_missing = missing_layers(s3_emulator or {}, REQUIRED_DOCKER_PROOF_LAYERS)
    s3_emulator_facts = s3_emulator.get("facts", {}) if isinstance(s3_emulator, dict) else {}
    s3_emulator_backup_errors = validate_agent_state_backup(
        s3_emulator_facts,
        expected_repository_kind="s3",
    )
    s3_emulator_passed = (
        bool(s3_emulator)
        and s3_emulator.get("status") == "passed"
        and not s3_emulator_missing
        and not s3_emulator_backup_errors
        and s3_emulator_facts.get("restic_backend") == "s3"
        and s3_emulator_facts.get("s3_endpoint_kind") == "local_emulator"
        and s3_emulator_facts.get("hermes_agent_version_actual") == "0.18.2"
        and s3_emulator_facts.get("agent_npub") == s3_emulator_facts.get("agent_npub_after_restore")
        and s3_emulator_facts.get("real_gateway_runtime") is True
        and s3_emulator_facts.get("gateway_admission_before_restore") is True
        and s3_emulator_facts.get("gateway_admission_after_restore") is True
    )
    add_check(
        checks,
        name="docker_runtime_s3_emulator_smoke",
        status="passed" if s3_emulator_passed or s3_smoke else "missing",
        evidence=str(s3_emulator_path) if s3_emulator else None,
        detail=None
        if s3_emulator_passed or s3_smoke
        else (
            "requires local S3-compatible Docker smoke for the restic S3 code path; "
            f"missing layers: {', '.join(s3_emulator_missing)}; "
            f"backup proof errors: {'; '.join(s3_emulator_backup_errors)}"
        ),
    )

    github_setup_ready = (
        bool(github_setup)
        and github_setup.get("status") in {"ready", "applied"}
        and not github_setup.get("missing_required_secrets")
        and not github_setup.get("missing_required_variables")
    )
    github_setup_detail = (
        "requires GitHub secret/variable setup for S3 CI gate"
        if not github_setup
        else list_detail(github_setup.get("errors"))
        or "requires GitHub secret/variable setup for S3 CI gate"
    )
    add_check(
        checks,
        name="github_actions_s3_setup_ready",
        status="passed" if github_setup_ready or s3_smoke else "missing",
        evidence=str(github_setup_path) if github_setup else None,
        detail=None if github_setup_ready or s3_smoke else github_setup_detail,
    )

    github_publish_gate_errors = validate_github_publish_gate(github_publish_gate)
    github_publish_gate_ready = not github_publish_gate_errors
    add_check(
        checks,
        name="github_publish_gate_ready",
        status="passed" if github_publish_gate_ready else "missing",
        evidence=str(github_publish_gate_path) if github_publish_gate else None,
        detail=None if github_publish_gate_ready else "; ".join(github_publish_gate_errors),
    )

    add_check(
        checks,
        name="docker_runtime_s3_smoke",
        status="passed" if s3_smoke else "missing",
        evidence=str(docker_path) if docker else None,
        detail=None
        if s3_smoke
        else (
            f"restic_backend={docker_facts.get('restic_backend')!r}, "
            f"s3_endpoint_kind={s3_endpoint_kind!r}; expected real S3; "
            f"proof errors: {'; '.join(s3_smoke_errors)}"
        ),
    )

    preflight_s3_errors = validate_s3_preflight(preflight)
    preflight_s3 = not preflight_s3_errors
    add_check(
        checks,
        name="s3_restic_preflight",
        status="passed" if preflight_s3 else "missing",
        evidence=str(preflight_path) if preflight else None,
        detail=None if preflight_s3 else "; ".join(preflight_s3_errors),
    )

    publish_errors = validate_publish_report(publish, docker_facts=docker_facts)
    publish_passed = not publish_errors
    add_check(
        checks,
        name="proven_image_published",
        status="passed" if publish_passed else "missing",
        evidence=str(publish_path) if publish else None,
        detail=None if publish_passed else "; ".join(publish_errors),
    )

    handoff_errors = validate_tinfoil_handoff(handoff, docker_facts=docker_facts, publish=publish)
    handoff_ready = not handoff_errors
    add_check(
        checks,
        name="tinfoil_handoff_ready",
        status="passed" if handoff_ready else "missing",
        evidence=str(handoff_path) if handoff else None,
        detail=None if handoff_ready else "; ".join(handoff_errors),
    )

    canary_errors = validate_canary_summary(canary_summary, canary_summary_path, handoff)
    canary_ready = not canary_errors
    add_check(
        checks,
        name="tinfoil_canary_artifacts_ready",
        status="passed" if canary_ready else "missing",
        evidence=str(canary_summary_path) if canary_summary else None,
        detail=None if canary_ready else "; ".join(canary_errors),
    )

    tinfoil_missing = missing_layers(tinfoil_result or {}, REQUIRED_TINFOIL_PROOF_LAYERS)
    tinfoil_facts = tinfoil_result.get("facts", {}) if isinstance(tinfoil_result, dict) else {}
    tinfoil_passed = (
        bool(tinfoil_result)
        and tinfoil_result.get("status") == "passed"
        and not tinfoil_missing
        and tinfoil_facts.get("restic_backend") == "s3"
        and tinfoil_facts.get("restore_tag") == "finite-agent-state"
        and tinfoil_facts.get("agent_npub_before_restart")
        == tinfoil_facts.get("agent_npub_after_restore")
        and tinfoil_facts.get("health_npub") == tinfoil_facts.get("agent_npub_after_restore")
        and tinfoil_facts.get("chat_before_restart") is True
        and tinfoil_facts.get("chat_after_restart") is True
    )
    add_check(
        checks,
        name="tinfoil_canary_runtime",
        status="passed" if tinfoil_passed else "missing",
        evidence=str(tinfoil_result_path) if tinfoil_result else None,
        detail=None
        if tinfoil_passed
        else (
            "requires validated live Tinfoil start/health/chat/backup/restart/restore/chat "
            f"evidence; missing layers: {', '.join(tinfoil_missing)}"
        ),
    )

    missing = [check for check in checks if check["status"] != "passed"]
    return {
        "status": "complete" if not missing else "incomplete",
        "generated_at_unix": int(time.time()),
        "checks": checks,
        "missing": [check["name"] for check in missing],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--adapter-regression-report",
        default="target/hermes-adapter-regressions/report.json",
    )
    parser.add_argument(
        "--sidecar-report",
        default="target/hermes-sidecar-smoke/report.json",
    )
    parser.add_argument(
        "--media-e2e-report",
        default="target/hermes-agent-media-e2e/report.json",
    )
    parser.add_argument(
        "--ios-media-e2e-report",
        default="target/ios-hermes-agent-media-e2e/report.json",
    )
    parser.add_argument(
        "--docker-report",
        default="target/hermes-docker-smoke/report.json",
    )
    parser.add_argument(
        "--s3-emulator-report",
        default="target/hermes-docker-s3-emulator-smoke/report.json",
    )
    parser.add_argument(
        "--github-setup-report",
        default="target/hermes-github-secrets-setup.json",
    )
    parser.add_argument(
        "--github-publish-gate-report",
        default="target/hermes-github-publish-gate/report.json",
    )
    parser.add_argument(
        "--preflight-report",
        default="target/hermes-docker-smoke/restic-preflight.json",
    )
    parser.add_argument(
        "--publish-report",
        default="target/hermes-docker-smoke/image-publish.json",
    )
    parser.add_argument(
        "--handoff-report",
        default="target/hermes-docker-smoke/tinfoil-handoff.json",
    )
    parser.add_argument(
        "--canary-summary",
        default="target/hermes-docker-smoke/tinfoil-canary/tinfoil-canary-summary.json",
    )
    parser.add_argument(
        "--tinfoil-result",
        default="target/hermes-docker-smoke/tinfoil-canary-result.json",
    )
    parser.add_argument("--report", default="target/hermes-hardening-audit.json")
    parser.add_argument("--require-complete", action="store_true")
    args = parser.parse_args()

    report = audit(args)
    report_path = Path(args.report)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(report, indent=2) + "\n"
    report_path.write_text(text, encoding="utf-8")
    print(text, end="")
    if args.require_complete and report["status"] != "complete":
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Generate Tinfoil canary config and runbook from a ready handoff report."""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path
from typing import Any

DEFAULT_FINITE_SERVER_URL = "https://chat.finite.computer"
DEFAULT_CONTAINER_NAME = "finite-agent-tinfoil-user-canary"
DEFAULT_RELEASE_TAG = "v0.1.0"
DEFAULT_CVM_VERSION = "0.7.5"
DEFAULT_CPUS = 4
DEFAULT_MEMORY = 16384
DEFAULT_HTTP_PORT = "8080"
RECOVERY_SCOPE = {
    "snapshot_root": "/data",
    "workspace_path": "/data/workspace",
    "workspace_included": True,
    "application_consistent_snapshot": "unproved",
    "independently_recoverable_key_authority": "unproved",
    "core_owned_empty_target_restore": "unproved",
}
REQUIRED_RUNTIME_ENV = {
    "FINITE_AGENT_RESTORE_ON_START",
    "FINITE_AGENT_RESTORE_LATEST",
    "FINITE_AGENT_BACKUP_ON_EXIT",
    "FINITE_AGENT_BACKUP_INTERVAL_SECS",
    "FINITE_AGENT_STATE_ROOT",
    "FINITE_AGENT_RESTIC_REPOSITORY",
    "FINITE_AGENT_RESTIC_BACKUP_TAG",
    "FINITECHAT_HERMES_INBOUND_STREAM",
    "FINITECHAT_HERMES_MODEL",
    "FINITECHAT_HERMES_PROVIDER",
}


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def object_dict(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def yaml_scalar(value: object) -> str:
    return json.dumps(str(value))


def require(condition: bool, message: str, errors: list[str]) -> None:
    if not condition:
        errors.append(message)


def require_product_server_url(finite_server_url: str) -> None:
    if finite_server_url.rstrip("/") != DEFAULT_FINITE_SERVER_URL:
        raise ValueError(
            "Tinfoil product canary artifacts must use "
            f"{DEFAULT_FINITE_SERVER_URL}; got {finite_server_url!r}"
        )


def validate_handoff(handoff: dict[str, Any]) -> tuple[dict[str, Any], list[str]]:
    errors: list[str] = []
    image = object_dict(handoff.get("image"))
    restore = object_dict(handoff.get("restore"))
    recovery_scope = object_dict(handoff.get("recovery_scope"))
    container_env = object_dict(restore.get("container_env"))
    require(handoff.get("status") == "ready", "handoff status must be ready", errors)
    digest = image.get("digest")
    require(
        isinstance(digest, str) and "@sha256:" in digest, "image.digest must pin sha256", errors
    )
    require(restore.get("backend") == "s3", "restore backend must be s3", errors)
    require(restore.get("restore_selector") == "latest", "restore selector must be latest", errors)
    for key, expected in RECOVERY_SCOPE.items():
        require(
            recovery_scope.get(key) == expected,
            f"recovery_scope.{key} must be {expected!r}",
            errors,
        )
    missing_env = sorted(REQUIRED_RUNTIME_ENV - set(container_env))
    require(not missing_env, f"handoff container_env missing: {', '.join(missing_env)}", errors)
    return {
        "image": image,
        "restore": restore,
        "container_env": container_env,
        "recovery_scope": recovery_scope,
    }, errors


def tinfoil_config(
    handoff: dict[str, Any],
    *,
    container_name: str,
    finite_server_url: str,
    cvm_version: str,
    cpus: int,
    memory: int,
    http_port: str,
) -> str:
    context, errors = validate_handoff(handoff)
    if errors:
        raise ValueError("; ".join(errors))
    image_digest = context["image"]["digest"]
    restore = context["restore"]
    env = {
        "FINITE_SERVER_URL": finite_server_url,
        "FINITECHAT_SERVER_URL": finite_server_url,
        "FINITECHAT_HOME": "/data/agent",
        "FINITE_AGENT_HTTP_PORT": http_port,
        **context["container_env"],
    }
    secret_names = restore.get("required_secret_env") or []
    healthcheck_cmd = [
        "CMD",
        "python",
        "-c",
        (
            "import urllib.request; "
            f"urllib.request.urlopen('http://127.0.0.1:{http_port}/healthz', timeout=5).read()"
        ),
    ]
    lines = [
        f"cvm-version: {yaml_scalar(cvm_version)}",
        f"cpus: {cpus}",
        f"memory: {memory}",
        "",
        "containers:",
        f"  - name: {yaml_scalar(container_name)}",
        f"    image: {yaml_scalar(image_digest)}",
        "    env:",
    ]
    for key, value in env.items():
        lines.append(f"      - {key}: {yaml_scalar(value)}")
    lines.extend(
        [
            "    secrets:",
            *[f"      - {name}" for name in secret_names],
            "    healthcheck:",
            f"      test: {json.dumps(healthcheck_cmd)}",
            "      interval: 30s",
            "      timeout: 5s",
            "      start_period: 5m",
            "      retries: 10",
            "    restart: always",
            "",
            "shim:",
            f"  upstream-port: {http_port}",
            "  paths:",
            "    - /healthz",
            "    - /invite",
            "",
        ]
    )
    return "\n".join(lines)


def runbook(
    handoff: dict[str, Any],
    *,
    config_repo: str,
    release_tag: str,
    container_name: str,
    config_path: Path,
    finite_server_url: str,
) -> str:
    context, errors = validate_handoff(handoff)
    if errors:
        raise ValueError("; ".join(errors))
    image_digest = context["image"]["digest"]
    restore = context["restore"]
    secret_names = restore.get("required_secret_env") or []
    secret_flags = " ".join(f"--secret {name}" for name in secret_names)
    lines = [
        "# Tinfoil Hermes Canary Runbook",
        "",
        "## Inputs",
        "",
        f"- Config repo: `{config_repo}`",
        f"- Release tag: `{release_tag}`",
        f"- Container name: `{container_name}`",
        f"- Image digest: `{image_digest}`",
        f"- Finite Chat server: `{finite_server_url}`",
        f"- Restore repository: `{restore.get('repository', {}).get('repository')}`",
        f"- Restore selector: latest snapshot tagged `{restore.get('restore_tag')}`",
        f"- Seed snapshot proof: `{restore.get('seed_snapshot_short_id')}`",
        f"- Snapshot root: `{RECOVERY_SCOPE['snapshot_root']}` (includes `/data/workspace`)",
        "",
        "## Security Note",
        "",
        "This canary uses Tinfoil container secrets for the restic password and",
        "object-storage credentials because the current runtime must decrypt state",
        "during entrypoint restore. Tinfoil documents that container secrets are not",
        "public in the repo or dashboard, but are visible to Tinfoil infrastructure.",
        "That is acceptable for this plumbing canary only. The production privacy",
        "target still needs user-mediated or attestation-gated key release before",
        "this path can satisfy an operator-can't-peek threat model.",
        "",
        "This canary must be created with Tinfoil debug mode so the runtime can be",
        "inspected while we are still validating the deployment shape. Debug mode is",
        "not attested and is not a production privacy posture.",
        "",
        "## Recovery Scope",
        "",
        "This canary requires Restic evidence rooted at the complete `/data` recovery",
        "root, including `/data/workspace`. Its passing result remains narrower than",
        "Agent Runtime Recovery Readiness: it does not prove an application-consistent snapshot barrier,",
        "independently recoverable key authority, or the Core-owned service-consistent empty-target restore.",
        "Those three properties remain explicitly unproved.",
        "",
        "## Files",
        "",
        f"- Copy `{config_path}` to the root of the public config repo as",
        "  `tinfoil-config.yml`.",
        "",
        "## Commands",
        "",
        "```bash",
        "tinfoil whoami",
        "tinfoil ssh-key list",
        'debug_ssh_key="${TINFOIL_DEBUG_SSH_KEY:?set TINFOIL_DEBUG_SSH_KEY to a Tinfoil SSH key name}"',
        *[
            f"tinfoil secret create {name} --value-file -  # paste value, then Ctrl-D"
            for name in secret_names
        ],
        f"gh workflow run tinfoil-release.yml -R {config_repo} -f version={release_tag}",
        f"tinfoil container create {container_name} \\",
        f"  --repo {config_repo} \\",
        f"  --tag {release_tag} \\",
        "  --debug \\",
        '  --ssh-key "$debug_ssh_key" \\',
        f"  {secret_flags}",
        f"tinfoil container get {container_name} -o json",
        f"tinfoil container connect {container_name} -p 3301",
        "curl http://127.0.0.1:3301/healthz",
        "curl http://127.0.0.1:3301/invite",
        "scripts/hermes-tinfoil-canary-evidence.py \\",
        "  --handoff-report target/hermes-docker-smoke/tinfoil-handoff.json \\",
        "  --canary-summary target/hermes-docker-smoke/tinfoil-canary/tinfoil-canary-summary.json \\",
        "  --container-json target/hermes-docker-smoke/tinfoil-canary/container.json \\",
        "  --health-json target/hermes-docker-smoke/tinfoil-canary/health.json \\",
        "  --image-digest '<digest-observed-from-tinfoil-container-json>' \\",
        "  --storage-backend s3 \\",
        "  --restore-tag finite-agent-state \\",
        "  --chat-before-message-id '<finite-chat-event-id-before-restart>' \\",
        "  --chat-after-message-id '<finite-chat-event-id-after-restart>' \\",
        "  --backup-observed \\",
        "  --restore-observed \\",
        "  --evidence-json target/hermes-docker-smoke/tinfoil-canary-evidence.json",
        "scripts/hermes-tinfoil-canary-result.py \\",
        "  --evidence-json target/hermes-docker-smoke/tinfoil-canary-evidence.json \\",
        "  --report target/hermes-docker-smoke/tinfoil-canary-result.json",
        "```",
        "",
        "## Evidence File",
        "",
        "After the manual canary, write the observed facts to",
        "`target/hermes-docker-smoke/tinfoil-canary/container.json`,",
        "`target/hermes-docker-smoke/tinfoil-canary/health.json`, and explicit",
        "observed image/storage fields, chat event IDs, and backup/restore flags. Build",
        "`target/hermes-docker-smoke/tinfoil-canary-evidence.json` with",
        "`scripts/hermes-tinfoil-canary-evidence.py`, then validate it with",
        "`scripts/hermes-tinfoil-canary-result.py`. The validator writes",
        "`target/hermes-docker-smoke/tinfoil-canary-result.json`, which is the",
        "only live Tinfoil result consumed by the hardening audit. A passing",
        "result must preserve raw source artifact references and match the",
        "generated handoff/config expectations for container name, image digest,",
        "storage backend, and restore tag.",
        "",
        "```json",
        "{",
        '  "expected": {',
        f'    "container_name": "{container_name}",',
        f'    "image_digest": "{image_digest}",',
        '    "storage_backend": "s3",',
        '    "restore_tag": "finite-agent-state"',
        "  },",
        '  "container": {',
        f'    "name": "{container_name}",',
        '    "status": "running",',
        '    "url": "https://..."',
        "  },",
        '  "image": {"digest": "' + image_digest + '", "source": "operator_arg"},',
        '  "storage": {"backend": "s3", "restore_tag": "finite-agent-state", "source": "operator_arg"},',
        '  "health": {"ready": true, "npub": "npub1..."},',
        '  "chat": {',
        '    "before_restart": {"ok": true, "message_id": "..."},',
        '    "after_restart": {"ok": true, "message_id": "..."}',
        "  },",
        '  "restart_restore": {',
        '    "npub_before_restart": "npub1...",',
        '    "npub_after_restore": "npub1...",',
        '    "same_npub": true,',
        '    "backup_observed": true,',
        '    "restore_observed": true',
        "  }",
        "}",
        "```",
        "",
        "## Acceptance",
        "",
        "- Container reaches Running.",
        "- `curl http://127.0.0.1:3301/healthz` returns `ready: true` through",
        "  the attested local proxy.",
        "- `curl http://127.0.0.1:3301/invite` returns the same runtime invite",
        "  shape used by the Docker canary.",
        "- User chats once from Finite Chat and the resulting event ID is recorded.",
        "- A fresh periodic or exit restic backup is observed before restart.",
        "- Restart restores the latest tagged restic snapshot and the same npub.",
        "- User chats again after restore and the resulting event ID is recorded.",
        "- `scripts/hermes-tinfoil-canary-result.py` writes a passed",
        "  `target/hermes-docker-smoke/tinfoil-canary-result.json` report.",
        "- Passing this canary does not promote any of the three unproved Recovery Set",
        "  properties listed above.",
        "",
        "## References",
        "",
        "- https://docs.tinfoil.sh/containers/overview",
        "- https://docs.tinfoil.sh/containers/configuration.md",
        "- https://docs.tinfoil.sh/containers/cli.md",
        "- https://docs.tinfoil.sh/containers/secrets-and-env-vars.md",
        "",
    ]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--handoff-report", required=True)
    parser.add_argument("--output-dir", required=True)
    parser.add_argument("--config-repo", required=True, help="public GitHub owner/repo")
    parser.add_argument("--tag", default=DEFAULT_RELEASE_TAG)
    parser.add_argument("--container-name", default=DEFAULT_CONTAINER_NAME)
    parser.add_argument("--finite-server-url", default=DEFAULT_FINITE_SERVER_URL)
    parser.add_argument("--cvm-version", default=DEFAULT_CVM_VERSION)
    parser.add_argument("--cpus", type=int, default=DEFAULT_CPUS)
    parser.add_argument("--memory", type=int, default=DEFAULT_MEMORY)
    parser.add_argument("--http-port", default=DEFAULT_HTTP_PORT)
    args = parser.parse_args()

    handoff_path = Path(args.handoff_report)
    output_dir = Path(args.output_dir)
    try:
        require_product_server_url(args.finite_server_url)
    except ValueError as exc:
        print(str(exc), file=sys.stderr)
        return 2
    handoff = load_json(handoff_path)
    _, errors = validate_handoff(handoff)
    if errors:
        print("Cannot generate Tinfoil artifacts:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 2

    output_dir.mkdir(parents=True, exist_ok=True)
    config_path = output_dir / "tinfoil-config.yml"
    runbook_path = output_dir / "tinfoil-canary-runbook.md"
    summary_path = output_dir / "tinfoil-canary-summary.json"
    config_text = tinfoil_config(
        handoff,
        container_name=args.container_name,
        finite_server_url=args.finite_server_url,
        cvm_version=args.cvm_version,
        cpus=args.cpus,
        memory=args.memory,
        http_port=args.http_port,
    )
    runbook_text = runbook(
        handoff,
        config_repo=args.config_repo,
        release_tag=args.tag,
        container_name=args.container_name,
        config_path=config_path,
        finite_server_url=args.finite_server_url,
    )
    config_path.write_text(config_text, encoding="utf-8")
    runbook_path.write_text(runbook_text, encoding="utf-8")
    summary = {
        "status": "ready",
        "generated_at_unix": int(time.time()),
        "handoff_report": str(handoff_path),
        "config": str(config_path),
        "runbook": str(runbook_path),
        "config_repo": args.config_repo,
        "release_tag": args.tag,
        "container_name": args.container_name,
        "image_digest": handoff["image"]["digest"],
        "finite_server_url": args.finite_server_url,
        "secret_env": handoff["restore"]["required_secret_env"],
        "recovery_scope": dict(RECOVERY_SCOPE),
        "tinfoil_debug": True,
        "tinfoil_debug_ssh_key_env": "TINFOIL_DEBUG_SSH_KEY",
    }
    summary_path.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())

"""Unit checks for the Hermes hardening evidence audit."""

from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
AUDIT_SCRIPT = REPO_ROOT / "scripts" / "hermes-hardening-audit.py"
IMAGE_ID = "sha256:local-image"
IMAGE_REF = "ghcr.io/finitecomputer/finite-chat-hermes-runtime:v0.1.0"
IMAGE_DIGEST = "ghcr.io/finitecomputer/finite-chat-hermes-runtime:v0.1.0@sha256:published"
RESTIC_REPOSITORY = (
    "s3:https://objects.nyc.storage.sh/tinfoil-agent-spike/agent-runtimes/tinfoil-canary-001/restic"
)
EMULATOR_RESTIC_REPOSITORY = (
    "s3:http://127.0.0.1:39000/finite-hermes-runtime-smoke/agent-runtimes/tinfoil-canary-001/restic"
)
SNAPSHOT_ID = "88929f1f90c5fcadd1d19e33f26609e595af4c2afb1e72b724695435e051900f"
CONFIG_REPO = "finitecomputer/tinfoil-agent-runtime-canary"
RELEASE_TAG = "v0.1.0"
CONTAINER_NAME = "finite-agent-tinfoil-user-canary"
RECOVERY_SCOPE = {
    "snapshot_root": "/data",
    "workspace_path": "/data/workspace",
    "workspace_included": True,
    "application_consistent_snapshot": "unproved",
    "independently_recoverable_key_authority": "unproved",
    "core_owned_empty_target_restore": "unproved",
}
GITHUB_PUBLISH_ARTIFACTS = [
    "target/hermes-hardening-audit.json",
    "target/hermes-docker-smoke/report.json",
    "target/hermes-docker-smoke/restic-preflight.json",
    "target/hermes-docker-smoke/image-publish.json",
    "target/hermes-docker-smoke/tinfoil-handoff.json",
    "target/hermes-docker-smoke/tinfoil-canary/tinfoil-canary-summary.json",
]

SIDECAR_LAYERS = [
    "finitechat-server",
    "finitechat hermes CLI",
    "encrypted client stores",
    "finitechat hermes serve",
    "sidecar /v1/hermes/inbound NDJSON",
    "ack/drain",
    "agent reply",
    "user decrypt",
]
DOCKER_LAYERS = [
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
    "workspace file restored from full recovery root",
    "same agent npub after restore",
    "runtime HTTP health endpoint after restore",
    "gateway invite admission after restore",
]
TINFOIL_LAYERS = [
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
]
MEDIA_STEPS = [
    "server_ready",
    "agent_init",
    "adapter_connect",
    "user_join",
    "user_send_media",
    "agent_receive_media",
    "user_receive_agent_replies",
]
IOS_MEDIA_STEPS = [
    "server_ready",
    "agent_init",
    "adapter_connect",
    "ios_app_launch",
    "agent_receive_ios_media",
    "ios_receive_agent_replies",
]
ADAPTER_REGRESSION_LAYERS = [
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
]


def write_json(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value) + "\n", encoding="utf-8")


def sidecar_report() -> dict:
    return {"status": "passed", "proof_layers": SIDECAR_LAYERS}


def adapter_regression_report() -> dict:
    return {
        "status": "passed",
        "proof_layers": ADAPTER_REGRESSION_LAYERS,
        "test_count": len(ADAPTER_REGRESSION_LAYERS),
    }


def media_e2e_report() -> dict:
    return {
        "status": "passed",
        "facts": {
            "adapter_inbound_stream": True,
            "adapter_service_url_present": True,
            "agent_received_media_types": ["image/png"],
            "user_received_text": ["agent text echo: user media hello", "agent media echo"],
            "user_received_media_count": 1,
        },
        "steps": [{"name": name, "elapsed_ms": 1} for name in MEDIA_STEPS],
    }


def ios_media_e2e_report() -> dict:
    return {
        "status": "passed",
        "name": "ios_simulator_hermes_agent_media_e2e",
        "facts": {
            "platform": "ios_simulator",
            "simulator_udid": "booted-simulator",
            "adapter_inbound_stream": True,
            "adapter_service_url_present": True,
            "agent_received_media_types": ["image/png"],
            "ios_received_text": ["agent text echo: ios media hello", "agent media echo"],
            "ios_received_media_count": 1,
        },
        "steps": [{"name": name, "elapsed_ms": 1} for name in IOS_MEDIA_STEPS],
    }


def restic_repository_report(
    restic_backend: str = "s3",
    *,
    repository: str = RESTIC_REPOSITORY,
) -> dict:
    if restic_backend == "local":
        return {
            "kind": "local",
            "path": "/tmp/finite-hermes-restic-repo",
            "size_bytes": 55149,
        }
    return {
        "kind": "s3",
        "repository": repository,
        "size_bytes": None,
    }


def agent_state_backup_report(
    restic_backend: str = "s3",
    *,
    repository: str = RESTIC_REPOSITORY,
) -> dict:
    return {
        "backend": "restic",
        "repository": restic_repository_report(restic_backend, repository=repository),
        "snapshot": {
            "id": SNAPSHOT_ID,
            "short_id": "88929f1f",
            "time": "2026-06-26T02:26:14Z",
            "paths": ["/data"],
            "tags": ["finite-agent-state"],
        },
        "tag": "finite-agent-state",
        "encrypted": True,
        "source": "entrypoint_backup_on_exit",
    }


def docker_report(
    restic_backend: str = "s3",
    *,
    repository: str = RESTIC_REPOSITORY,
) -> dict:
    return {
        "status": "passed",
        "recovery_scope": dict(RECOVERY_SCOPE),
        "proof_layers": DOCKER_LAYERS,
        "facts": {
            "image": "finite-chat-hermes-runtime:smoke",
            "image_id": IMAGE_ID,
            "restic_version": "restic 0.18.0 compiled with go1.24.4 on linux/arm64",
            "restic_backend": restic_backend,
            "restic_repository": restic_repository_report(
                restic_backend,
                repository=repository,
            ),
            "hermes_agent_version_actual": "0.18.2",
            "agent_npub": "npub1agent",
            "agent_npub_after_restore": "npub1agent",
            "real_gateway_runtime": True,
            "gateway_admission_before_restore": True,
            "gateway_admission_after_restore": True,
            "workspace_restored_after_restore": True,
            "agent_state_backup": agent_state_backup_report(
                restic_backend,
                repository=repository,
            ),
        },
    }


def s3_emulator_report() -> dict:
    report = docker_report(restic_backend="s3", repository=EMULATOR_RESTIC_REPOSITORY)
    report["facts"]["s3_endpoint_kind"] = "local_emulator"
    report["facts"]["s3_emulator"] = {
        "endpoint": "http://127.0.0.1:39000",
        "bucket": "finite-hermes-runtime-smoke",
        "prefix": "agent-runtimes/tinfoil-canary-001/restic",
    }
    return report


def tinfoil_result() -> dict:
    return {
        "status": "passed",
        "recovery_scope": dict(RECOVERY_SCOPE),
        "proof_layers": TINFOIL_LAYERS,
        "facts": {
            "restic_backend": "s3",
            "restore_tag": "finite-agent-state",
            "health_npub": "npub1agent",
            "agent_npub_before_restart": "npub1agent",
            "agent_npub_after_restore": "npub1agent",
            "chat_before_restart": True,
            "chat_after_restart": True,
        },
    }


def handoff_report(image_digest: str = IMAGE_DIGEST) -> dict:
    return {
        "status": "ready",
        "errors": [],
        "recovery_scope": dict(RECOVERY_SCOPE),
        "source_reports": {
            "smoke": "target/hermes-docker-smoke/report.json",
            "preflight": "target/hermes-docker-smoke/restic-preflight.json",
            "publish": "target/hermes-docker-smoke/image-publish.json",
        },
        "image": {
            "source_image_id": IMAGE_ID,
            "target_ref": IMAGE_REF,
            "digest": image_digest,
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
            "seed_snapshot_id": SNAPSHOT_ID,
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


def publish_report(restic_backend: str = "s3") -> dict:
    return {
        "status": "published",
        "generated_at_unix": 1,
        "source_report": "target/hermes-docker-smoke/report.json",
        "source_image": "finite-chat-hermes-runtime:smoke",
        "source_image_id": IMAGE_ID,
        "target_image_ref": IMAGE_REF,
        "pushed": True,
        "repo_digests": [IMAGE_DIGEST],
        "recovery_scope": dict(RECOVERY_SCOPE),
        "proof": {
            "smoke_status": "passed",
            "hermes_agent_version_actual": "0.18.2",
            "restic_version": "restic 0.18.0 compiled with go1.24.4 on linux/arm64",
            "agent_npub_after_restore": "npub1agent",
            "restic_backend": restic_backend,
            "real_gateway_runtime": True,
            "gateway_admission_before_restore": True,
            "gateway_admission_after_restore": True,
        },
    }


def s3_preflight_report() -> dict:
    return {
        "status": "ok",
        "backend": "s3",
        "repository": RESTIC_REPOSITORY,
        "env": {
            "FINITE_DOCKER_RESTIC_BACKEND": True,
            "FINITE_DOCKER_RESTIC_REPOSITORY": True,
            "FINITE_DOCKER_RESTIC_PASSWORD": True,
            "AWS_ACCESS_KEY_ID": True,
            "AWS_SECRET_ACCESS_KEY": True,
            "AWS_SESSION_TOKEN": False,
            "AWS_REGION": True,
            "AWS_DEFAULT_REGION": False,
            "FINITE_DOCKER_RESTIC_AWS_ACCESS_KEY_ID": True,
            "FINITE_DOCKER_RESTIC_AWS_SECRET_ACCESS_KEY": True,
            "FINITE_LATITUDE_STORAGE_BUCKET": False,
        },
        "warnings": [],
        "errors": [],
    }


def write_canary_artifacts(tmp: Path, *, image_digest: str = IMAGE_DIGEST) -> None:
    output_dir = tmp / "tinfoil-canary"
    config_path = output_dir / "tinfoil-config.yml"
    runbook_path = output_dir / "tinfoil-canary-runbook.md"
    output_dir.mkdir(parents=True, exist_ok=True)
    config_path.write_text(
        "\n".join(
            [
                'cvm-version: "0.7.5"',
                "cpus: 4",
                "memory: 16384",
                "containers:",
                f'  - name: "{CONTAINER_NAME}"',
                f'    image: "{image_digest}"',
                "    env:",
                '      - FINITE_SERVER_URL: "https://chat.finite.computer"',
                '      - FINITECHAT_SERVER_URL: "https://chat.finite.computer"',
                '      - FINITE_AGENT_RESTORE_ON_START: "1"',
                '      - FINITE_AGENT_RESTORE_LATEST: "1"',
                '      - FINITE_AGENT_BACKUP_ON_EXIT: "1"',
                '      - FINITE_AGENT_BACKUP_INTERVAL_SECS: "30"',
                '      - FINITE_AGENT_STATE_ROOT: "/data"',
                (
                    "      - FINITE_AGENT_RESTIC_REPOSITORY: "
                    '"s3:https://objects.nyc.storage.sh/tinfoil-agent-spike/agent-runtimes/tinfoil-canary-001/restic"'
                ),
                '      - FINITE_AGENT_RESTIC_BACKUP_TAG: "finite-agent-state"',
                '      - FINITECHAT_HERMES_INBOUND_STREAM: "1"',
                '      - FINITECHAT_HERMES_MODEL: "anthropic/claude-sonnet-4.6"',
                '      - FINITECHAT_HERMES_PROVIDER: "openrouter"',
                "    secrets:",
                "      - FINITE_AGENT_RESTIC_PASSWORD",
                "      - AWS_ACCESS_KEY_ID",
                "      - AWS_SECRET_ACCESS_KEY",
                "      - OPENROUTER_API_KEY",
                "    healthcheck:",
                '      test: ["CMD", "python", "-c", "curl /healthz"]',
                "shim:",
                "  upstream-port: 8080",
                "  paths:",
                "    - /healthz",
                "    - /invite",
                "",
            ]
        ),
        encoding="utf-8",
    )
    runbook_path.write_text(
        "\n".join(
            [
                "# Tinfoil Hermes Canary Runbook",
                f"- Config repo: `{CONFIG_REPO}`",
                f"- Release tag: `{RELEASE_TAG}`",
                f"- Container name: `{CONTAINER_NAME}`",
                f"- Image digest: `{image_digest}`",
                "tinfoil container create finite-agent-tinfoil-user-canary \\",
                "  --debug \\",
                '  --ssh-key "$debug_ssh_key"',
                "scripts/hermes-tinfoil-canary-evidence.py \\",
                "  --image-digest '<digest-observed-from-tinfoil-container-json>' \\",
                "  --storage-backend s3 \\",
                "  --restore-tag finite-agent-state \\",
                "  --chat-before-message-id '<finite-chat-event-id-before-restart>' \\",
                "  --chat-after-message-id '<finite-chat-event-id-after-restart>'",
                "scripts/hermes-tinfoil-canary-result.py \\",
                "  --evidence-json target/hermes-docker-smoke/tinfoil-canary-evidence.json",
                "application-consistent snapshot barrier",
                "independently recoverable key authority",
                "Core-owned service-consistent empty-target restore",
                "",
            ]
        ),
        encoding="utf-8",
    )
    write_json(
        tmp / "canary-summary.json",
        {
            "status": "ready",
            "generated_at_unix": 1,
            "config": str(config_path),
            "runbook": str(runbook_path),
            "config_repo": CONFIG_REPO,
            "release_tag": RELEASE_TAG,
            "container_name": CONTAINER_NAME,
            "image_digest": image_digest,
            "finite_server_url": "https://chat.finite.computer",
            "secret_env": [
                "FINITE_AGENT_RESTIC_PASSWORD",
                "AWS_ACCESS_KEY_ID",
                "AWS_SECRET_ACCESS_KEY",
                "OPENROUTER_API_KEY",
            ],
            "recovery_scope": dict(RECOVERY_SCOPE),
            "tinfoil_debug": True,
            "tinfoil_debug_ssh_key_env": "TINFOIL_DEBUG_SSH_KEY",
        },
    )


def github_publish_gate_report() -> dict:
    return {
        "status": "passed",
        "run_id": "28222040402",
        "run_url": "https://github.com/finitecomputer/finitechat/actions/runs/28222040402",
        "watch_exit_code": 0,
        "download_exit_code": 0,
        "local_audit_exit_code": 0,
        "downloaded_files": [
            f"hermes-docker-smoke-report/{artifact}" for artifact in GITHUB_PUBLISH_ARTIFACTS
        ],
        "artifact_ingest": {
            "status": "ok",
            "missing": [],
            "copied": [
                {
                    "source": f"/tmp/artifacts/hermes-docker-smoke-report/{artifact}",
                    "destination": f"/repo/{artifact}",
                }
                for artifact in GITHUB_PUBLISH_ARTIFACTS
            ],
        },
    }


def run_audit(tmp: Path, *, require_complete: bool = False) -> tuple[int, dict]:
    args = [
        str(AUDIT_SCRIPT),
        "--adapter-regression-report",
        str(tmp / "adapter-regressions.json"),
        "--sidecar-report",
        str(tmp / "sidecar.json"),
        "--media-e2e-report",
        str(tmp / "media-e2e.json"),
        "--ios-media-e2e-report",
        str(tmp / "ios-media-e2e.json"),
        "--docker-report",
        str(tmp / "docker.json"),
        "--s3-emulator-report",
        str(tmp / "s3-emulator.json"),
        "--github-setup-report",
        str(tmp / "github-setup.json"),
        "--github-publish-gate-report",
        str(tmp / "github-publish-gate.json"),
        "--preflight-report",
        str(tmp / "preflight.json"),
        "--publish-report",
        str(tmp / "publish.json"),
        "--handoff-report",
        str(tmp / "handoff.json"),
        "--canary-summary",
        str(tmp / "canary-summary.json"),
        "--tinfoil-result",
        str(tmp / "tinfoil-result.json"),
        "--report",
        str(tmp / "audit.json"),
    ]
    if require_complete:
        args.append("--require-complete")
    result = subprocess.run(args, capture_output=True, text=True, check=False)
    return result.returncode, json.loads((tmp / "audit.json").read_text(encoding="utf-8"))


def write_complete_audit_inputs(
    tmp: Path,
    *,
    docker: dict | None = None,
    preflight: dict | None = None,
) -> None:
    write_json(tmp / "adapter-regressions.json", adapter_regression_report())
    write_json(tmp / "sidecar.json", sidecar_report())
    write_json(tmp / "media-e2e.json", media_e2e_report())
    write_json(tmp / "ios-media-e2e.json", ios_media_e2e_report())
    write_json(tmp / "docker.json", docker if docker is not None else docker_report())
    write_json(tmp / "s3-emulator.json", s3_emulator_report())
    write_json(tmp / "github-setup.json", {"status": "ready"})
    write_json(tmp / "github-publish-gate.json", github_publish_gate_report())
    write_json(
        tmp / "preflight.json",
        preflight if preflight is not None else s3_preflight_report(),
    )
    write_json(tmp / "publish.json", publish_report())
    write_json(tmp / "handoff.json", handoff_report())
    write_canary_artifacts(tmp)
    write_json(tmp / "tinfoil-result.json", tinfoil_result())


class HardeningAuditTest(unittest.TestCase):
    def test_audit_marks_local_only_smoke_incomplete(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            write_json(tmp / "sidecar.json", sidecar_report())
            write_json(tmp / "docker.json", docker_report(restic_backend="local"))
            write_json(tmp / "preflight.json", {"status": "ok", "backend": "local"})
            status, audit = run_audit(tmp, require_complete=True)

        self.assertEqual(status, 2)
        self.assertEqual(audit["status"], "incomplete")
        self.assertIn("adapter_focused_regressions", audit["missing"])
        self.assertIn("local_hermes_agent_media_e2e", audit["missing"])
        self.assertIn("ios_simulator_media_e2e", audit["missing"])
        self.assertIn("docker_runtime_s3_emulator_smoke", audit["missing"])
        self.assertIn("github_actions_s3_setup_ready", audit["missing"])
        self.assertIn("github_publish_gate_ready", audit["missing"])
        self.assertIn("docker_runtime_s3_smoke", audit["missing"])
        self.assertIn("proven_image_published", audit["missing"])
        self.assertIn("tinfoil_canary_runtime", audit["missing"])

    def test_audit_marks_complete_evidence_complete(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            write_complete_audit_inputs(tmp)
            status, audit = run_audit(tmp, require_complete=True)

        self.assertEqual(status, 0)
        self.assertEqual(audit["status"], "complete")
        self.assertEqual(audit["missing"], [])
        self.assertEqual(audit["recovery_scope"], RECOVERY_SCOPE)
        self.assertIn("not Agent Runtime Recovery Readiness", audit["completion_scope"])

    def test_audit_rejects_legacy_agent_only_snapshot_root(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            docker = docker_report()
            docker["facts"]["agent_state_backup"]["snapshot"]["paths"] = ["/data/agent"]
            write_complete_audit_inputs(tmp, docker=docker)
            status, audit = run_audit(tmp, require_complete=True)

        self.assertEqual(status, 2)
        self.assertIn("docker_runtime_local_or_s3_smoke", audit["missing"])
        details = {check["name"]: check["detail"] for check in audit["checks"]}
        self.assertIn("paths must include /data", details["docker_runtime_local_or_s3_smoke"])

    def test_audit_rejects_recovery_scope_overclaim(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            docker = docker_report()
            docker["recovery_scope"]["application_consistent_snapshot"] = "proven"
            write_complete_audit_inputs(tmp, docker=docker)
            status, audit = run_audit(tmp, require_complete=True)

        self.assertEqual(status, 2)
        self.assertIn("docker_runtime_local_or_s3_smoke", audit["missing"])
        details = {check["name"]: check["detail"] for check in audit["checks"]}
        self.assertIn(
            "application_consistent_snapshot='proven'; expected 'unproved'",
            details["docker_runtime_local_or_s3_smoke"],
        )

    def test_audit_rejects_s3_preflight_status_without_repository_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            write_complete_audit_inputs(tmp, preflight={"status": "ok", "backend": "s3"})
            status, audit = run_audit(tmp, require_complete=True)

        self.assertEqual(status, 2)
        self.assertIn("s3_restic_preflight", audit["missing"])
        details = {check["name"]: check["detail"] for check in audit["checks"]}
        self.assertIn("repository", details["s3_restic_preflight"])

    def test_audit_rejects_s3_smoke_without_snapshot_proof(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            docker = docker_report()
            del docker["facts"]["agent_state_backup"]["snapshot"]
            write_complete_audit_inputs(tmp, docker=docker)
            status, audit = run_audit(tmp, require_complete=True)

        self.assertEqual(status, 2)
        self.assertIn("docker_runtime_local_or_s3_smoke", audit["missing"])
        self.assertIn("docker_runtime_s3_smoke", audit["missing"])
        details = {check["name"]: check["detail"] for check in audit["checks"]}
        self.assertIn("agent_state_backup.snapshot", details["docker_runtime_s3_smoke"])

    def test_audit_rejects_s3_smoke_without_repository_proof(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            docker = docker_report()
            del docker["facts"]["restic_repository"]
            write_complete_audit_inputs(tmp, docker=docker)
            status, audit = run_audit(tmp, require_complete=True)

        self.assertEqual(status, 2)
        self.assertIn("docker_runtime_s3_smoke", audit["missing"])
        details = {check["name"]: check["detail"] for check in audit["checks"]}
        self.assertIn("restic_repository", details["docker_runtime_s3_smoke"])

    def test_audit_rejects_unencrypted_s3_backup(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            docker = docker_report()
            docker["facts"]["agent_state_backup"]["encrypted"] = False
            write_complete_audit_inputs(tmp, docker=docker)
            status, audit = run_audit(tmp, require_complete=True)

        self.assertEqual(status, 2)
        self.assertIn("docker_runtime_s3_smoke", audit["missing"])
        details = {check["name"]: check["detail"] for check in audit["checks"]}
        self.assertIn("encrypted", details["docker_runtime_s3_smoke"])

    def test_audit_rejects_unvalidated_tinfoil_success_flag(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            write_json(tmp / "adapter-regressions.json", adapter_regression_report())
            write_json(tmp / "sidecar.json", sidecar_report())
            write_json(tmp / "media-e2e.json", media_e2e_report())
            write_json(tmp / "ios-media-e2e.json", ios_media_e2e_report())
            write_json(tmp / "docker.json", docker_report())
            write_json(tmp / "s3-emulator.json", s3_emulator_report())
            write_json(tmp / "github-setup.json", {"status": "ready"})
            write_json(tmp / "github-publish-gate.json", github_publish_gate_report())
            write_json(tmp / "preflight.json", s3_preflight_report())
            write_json(tmp / "publish.json", publish_report())
            write_json(tmp / "handoff.json", handoff_report())
            write_canary_artifacts(tmp)
            write_json(tmp / "tinfoil-result.json", {"status": "passed"})
            status, audit = run_audit(tmp, require_complete=True)

        self.assertEqual(status, 2)
        self.assertEqual(audit["status"], "incomplete")
        self.assertIn("tinfoil_canary_runtime", audit["missing"])

    def test_audit_rejects_placeholder_github_publish_gate_success(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            write_json(tmp / "adapter-regressions.json", adapter_regression_report())
            write_json(tmp / "sidecar.json", sidecar_report())
            write_json(tmp / "media-e2e.json", media_e2e_report())
            write_json(tmp / "ios-media-e2e.json", ios_media_e2e_report())
            write_json(tmp / "docker.json", docker_report())
            write_json(tmp / "s3-emulator.json", s3_emulator_report())
            write_json(tmp / "github-setup.json", {"status": "ready"})
            write_json(tmp / "github-publish-gate.json", {"status": "passed"})
            write_json(tmp / "preflight.json", s3_preflight_report())
            write_json(tmp / "publish.json", publish_report())
            write_json(tmp / "handoff.json", handoff_report())
            write_canary_artifacts(tmp)
            write_json(tmp / "tinfoil-result.json", tinfoil_result())
            status, audit = run_audit(tmp, require_complete=True)

        self.assertEqual(status, 2)
        self.assertEqual(audit["status"], "incomplete")
        self.assertIn("github_publish_gate_ready", audit["missing"])
        details = {check["name"]: check["detail"] for check in audit["checks"]}
        self.assertIn("run_id", details["github_publish_gate_ready"])

    def test_audit_rejects_placeholder_image_publish_report(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            write_json(tmp / "adapter-regressions.json", adapter_regression_report())
            write_json(tmp / "sidecar.json", sidecar_report())
            write_json(tmp / "media-e2e.json", media_e2e_report())
            write_json(tmp / "ios-media-e2e.json", ios_media_e2e_report())
            write_json(tmp / "docker.json", docker_report())
            write_json(tmp / "s3-emulator.json", s3_emulator_report())
            write_json(tmp / "github-setup.json", {"status": "ready"})
            write_json(tmp / "github-publish-gate.json", github_publish_gate_report())
            write_json(tmp / "preflight.json", s3_preflight_report())
            write_json(
                tmp / "publish.json",
                {
                    "status": "published",
                    "source_image_id": IMAGE_ID,
                    "repo_digests": [IMAGE_DIGEST],
                },
            )
            write_json(tmp / "handoff.json", handoff_report())
            write_canary_artifacts(tmp)
            write_json(tmp / "tinfoil-result.json", tinfoil_result())
            status, audit = run_audit(tmp, require_complete=True)

        self.assertEqual(status, 2)
        self.assertEqual(audit["status"], "incomplete")
        self.assertIn("proven_image_published", audit["missing"])
        details = {check["name"]: check["detail"] for check in audit["checks"]}
        self.assertIn("source_report", details["proven_image_published"])
        self.assertIn("pushed", details["proven_image_published"])

    def test_audit_rejects_placeholder_tinfoil_handoff(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            write_json(tmp / "adapter-regressions.json", adapter_regression_report())
            write_json(tmp / "sidecar.json", sidecar_report())
            write_json(tmp / "media-e2e.json", media_e2e_report())
            write_json(tmp / "ios-media-e2e.json", ios_media_e2e_report())
            write_json(tmp / "docker.json", docker_report())
            write_json(tmp / "s3-emulator.json", s3_emulator_report())
            write_json(tmp / "github-setup.json", {"status": "ready"})
            write_json(tmp / "github-publish-gate.json", github_publish_gate_report())
            write_json(tmp / "preflight.json", s3_preflight_report())
            write_json(tmp / "publish.json", publish_report())
            write_json(tmp / "handoff.json", {"status": "ready"})
            write_canary_artifacts(tmp)
            write_json(tmp / "tinfoil-result.json", tinfoil_result())
            status, audit = run_audit(tmp, require_complete=True)

        self.assertEqual(status, 2)
        self.assertEqual(audit["status"], "incomplete")
        self.assertIn("tinfoil_handoff_ready", audit["missing"])
        details = {check["name"]: check["detail"] for check in audit["checks"]}
        self.assertIn("source_reports", details["tinfoil_handoff_ready"])

    def test_audit_rejects_placeholder_canary_summary(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            write_json(tmp / "adapter-regressions.json", adapter_regression_report())
            write_json(tmp / "sidecar.json", sidecar_report())
            write_json(tmp / "media-e2e.json", media_e2e_report())
            write_json(tmp / "ios-media-e2e.json", ios_media_e2e_report())
            write_json(tmp / "docker.json", docker_report())
            write_json(tmp / "s3-emulator.json", s3_emulator_report())
            write_json(tmp / "github-setup.json", {"status": "ready"})
            write_json(tmp / "github-publish-gate.json", github_publish_gate_report())
            write_json(tmp / "preflight.json", s3_preflight_report())
            write_json(tmp / "publish.json", publish_report())
            write_json(tmp / "handoff.json", handoff_report())
            write_json(tmp / "canary-summary.json", {"status": "ready"})
            write_json(tmp / "tinfoil-result.json", tinfoil_result())
            status, audit = run_audit(tmp, require_complete=True)

        self.assertEqual(status, 2)
        self.assertEqual(audit["status"], "incomplete")
        self.assertIn("tinfoil_canary_artifacts_ready", audit["missing"])
        details = {check["name"]: check["detail"] for check in audit["checks"]}
        self.assertIn("image_digest", details["tinfoil_canary_artifacts_ready"])

    def test_audit_reports_github_setup_errors_before_s3_smoke(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            write_json(tmp / "adapter-regressions.json", adapter_regression_report())
            write_json(tmp / "sidecar.json", sidecar_report())
            write_json(tmp / "media-e2e.json", media_e2e_report())
            write_json(tmp / "ios-media-e2e.json", ios_media_e2e_report())
            write_json(tmp / "docker.json", docker_report(restic_backend="local"))
            write_json(tmp / "s3-emulator.json", s3_emulator_report())
            write_json(
                tmp / "github-setup.json",
                {
                    "status": "failed",
                    "errors": ["missing required secret values: FINITE_DOCKER_RESTIC_PASSWORD"],
                },
            )
            write_json(
                tmp / "github-publish-gate.json",
                {
                    "status": "not_ready",
                    "errors": ["remote workflow ref is missing; push the branch before dispatch"],
                },
            )
            status, audit = run_audit(tmp, require_complete=True)

        self.assertEqual(status, 2)
        details = {check["name"]: check["detail"] for check in audit["checks"]}
        self.assertIn("FINITE_DOCKER_RESTIC_PASSWORD", details["github_actions_s3_setup_ready"])
        self.assertIn("remote workflow ref is missing", details["github_publish_gate_ready"])

    def test_audit_rejects_ios_success_flag_without_native_store_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            write_json(tmp / "adapter-regressions.json", adapter_regression_report())
            write_json(tmp / "sidecar.json", sidecar_report())
            write_json(tmp / "media-e2e.json", media_e2e_report())
            write_json(
                tmp / "ios-media-e2e.json",
                {
                    "status": "passed",
                    "name": "ios_simulator_hermes_agent_media_e2e",
                },
            )
            write_json(tmp / "docker.json", docker_report())
            write_json(tmp / "s3-emulator.json", s3_emulator_report())
            write_json(tmp / "github-setup.json", {"status": "ready"})
            write_json(tmp / "github-publish-gate.json", github_publish_gate_report())
            write_json(tmp / "preflight.json", s3_preflight_report())
            write_json(tmp / "publish.json", publish_report())
            write_json(tmp / "handoff.json", handoff_report())
            write_canary_artifacts(tmp)
            write_json(tmp / "tinfoil-result.json", tinfoil_result())
            status, audit = run_audit(tmp, require_complete=True)

        self.assertEqual(status, 2)
        self.assertEqual(audit["status"], "incomplete")
        self.assertIn("ios_simulator_media_e2e", audit["missing"])

    def test_audit_rejects_adapter_regression_report_missing_layers(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            report = adapter_regression_report()
            report["proof_layers"] = [
                layer for layer in ADAPTER_REGRESSION_LAYERS if layer != "typing activity"
            ]
            write_json(tmp / "adapter-regressions.json", report)
            write_json(tmp / "sidecar.json", sidecar_report())
            write_json(tmp / "media-e2e.json", media_e2e_report())
            write_json(tmp / "ios-media-e2e.json", ios_media_e2e_report())
            write_json(tmp / "docker.json", docker_report())
            write_json(tmp / "s3-emulator.json", s3_emulator_report())
            write_json(tmp / "github-setup.json", {"status": "ready"})
            write_json(tmp / "github-publish-gate.json", github_publish_gate_report())
            write_json(tmp / "preflight.json", s3_preflight_report())
            write_json(tmp / "publish.json", publish_report())
            write_json(tmp / "handoff.json", handoff_report())
            write_canary_artifacts(tmp)
            write_json(tmp / "tinfoil-result.json", tinfoil_result())
            status, audit = run_audit(tmp, require_complete=True)

        self.assertEqual(status, 2)
        self.assertEqual(audit["status"], "incomplete")
        self.assertIn("adapter_focused_regressions", audit["missing"])

    def test_audit_rejects_s3_emulator_as_real_s3_smoke(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            write_json(tmp / "adapter-regressions.json", adapter_regression_report())
            write_json(tmp / "sidecar.json", sidecar_report())
            write_json(tmp / "media-e2e.json", media_e2e_report())
            write_json(tmp / "ios-media-e2e.json", ios_media_e2e_report())
            write_json(tmp / "docker.json", s3_emulator_report())
            write_json(tmp / "s3-emulator.json", s3_emulator_report())
            write_json(tmp / "github-setup.json", {"status": "ready"})
            write_json(tmp / "github-publish-gate.json", github_publish_gate_report())
            write_json(tmp / "preflight.json", s3_preflight_report())
            write_json(tmp / "publish.json", publish_report())
            write_json(tmp / "handoff.json", handoff_report())
            write_canary_artifacts(tmp)
            write_json(tmp / "tinfoil-result.json", tinfoil_result())
            status, audit = run_audit(tmp, require_complete=True)

        self.assertEqual(status, 2)
        self.assertIn("docker_runtime_s3_smoke", audit["missing"])
        self.assertNotIn("docker_runtime_s3_emulator_smoke", audit["missing"])


if __name__ == "__main__":
    unittest.main()

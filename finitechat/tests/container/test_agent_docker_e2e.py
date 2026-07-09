"""Docker runtime smoke for the Finite Chat Hermes sidecar.

Gated behind FINITE_DOCKER_E2E=1 (run scripts/hermes-sidecar-docker-smoke.sh).
Builds the real container image from containers/agent/Dockerfile, starts the
real Hermes gateway, drives finitechat CLI users through invite/PIN admission,
and writes a JSON evidence report for Docker/Tinfoil baseline comparisons.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import time
import unittest
from collections.abc import Mapping
from pathlib import Path
from typing import Any
from unittest import mock
from urllib.parse import urlsplit, urlunsplit

from tests.container.test_agent_container_e2e import run, stage_build_context

REPO_ROOT = Path(__file__).resolve().parents[2]
IMAGE = os.environ.get("FINITE_DOCKER_IMAGE", "finite-agent-docker-e2e")
SKIP_IMAGE_BUILD = os.environ.get("FINITE_DOCKER_SKIP_IMAGE_BUILD", "").lower() in {
    "1",
    "true",
    "yes",
    "on",
}
CONTAINER = os.environ.get("FINITE_DOCKER_CONTAINER", "finite-agent-docker-e2e-run")
SERVER_PORT = int(os.environ.get("FINITE_DOCKER_SERVER_PORT", "18789"))
DOCKER_HOST = os.environ.get("FINITE_DOCKER_HOST", "host.docker.internal")
HERMES_AGENT_VERSION = os.environ.get("FINITE_HERMES_AGENT_VERSION", "0.17.0")
RESTIC_BACKEND = os.environ.get("FINITE_DOCKER_RESTIC_BACKEND", "local").strip().lower()
RESTIC_REPOSITORY = os.environ.get("FINITE_DOCKER_RESTIC_REPOSITORY", "").strip()
RESTIC_SNAPSHOT_TAG = os.environ.get("FINITE_DOCKER_RESTIC_SNAPSHOT_TAG", "finite-agent-state")
DEFAULT_LOCAL_RESTIC_PASSWORD = "finite-docker-smoke-restic-key"
AWS_RESTIC_ENV_NAMES = (
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_DEFAULT_REGION",
    "AWS_REGION",
)
AWS_RESTIC_ENV_ALIASES = {
    "AWS_ACCESS_KEY_ID": "FINITE_DOCKER_RESTIC_AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY": "FINITE_DOCKER_RESTIC_AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN": "FINITE_DOCKER_RESTIC_AWS_SESSION_TOKEN",
    "AWS_DEFAULT_REGION": "FINITE_DOCKER_RESTIC_AWS_DEFAULT_REGION",
    "AWS_REGION": "FINITE_DOCKER_RESTIC_AWS_REGION",
}
MODEL_ENV_NAMES = (
    "OPENROUTER_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "FINITE_DEFAULT_INFERENCE_PROFILE",
    "FINITE_PRIVATE_MODEL",
    "FINITE_PRIVATE_BASE_URL",
    "FINITE_PRIVATE_API_KEY",
    "FINITECHAT_HERMES_MODEL",
    "FINITECHAT_HERMES_PROVIDER",
    "FINITECHAT_HERMES_BASE_URL",
    "FINITECHAT_HERMES_API_MODE",
    "FINITECHAT_HERMES_API_KEY",
)


def restic_password_from_env(env: Mapping[str, str]) -> str:
    return env.get("FINITE_DOCKER_RESTIC_PASSWORD") or DEFAULT_LOCAL_RESTIC_PASSWORD


RESTIC_PASSWORD = restic_password_from_env(os.environ)


def restic_env_value(name: str) -> str | None:
    return os.environ.get(name) or os.environ.get(AWS_RESTIC_ENV_ALIASES[name])


def directory_size(path: Path) -> int:
    return sum(entry.stat().st_size for entry in path.rglob("*") if entry.is_file())


def redact_restic_repository(repository: str) -> str:
    if not repository.startswith("s3:"):
        return repository
    parsed = urlsplit(repository[3:])
    if not parsed.username and not parsed.password:
        return repository
    host = parsed.hostname or ""
    if parsed.port is not None:
        host = f"{host}:{parsed.port}"
    return "s3:" + urlunsplit((parsed.scheme, host, parsed.path, parsed.query, parsed.fragment))


class SmokeReport:
    def __init__(self, name: str):
        self.name = name
        self.path = os.environ.get("FINITE_HERMES_DOCKER_SMOKE_REPORT")
        self.started = time.monotonic()
        self.facts: dict[str, Any] = {}
        self.steps: list[dict[str, Any]] = []

    def fact(self, key: str, value: Any) -> None:
        if self.path:
            self.facts[key] = value

    def time(self, name: str, fn):
        started = time.monotonic()
        value = fn()
        self.step(name, time.monotonic() - started)
        return value

    def step(self, name: str, elapsed: float) -> None:
        if self.path:
            self.steps.append({"name": name, "elapsed_ms": int(elapsed * 1000)})

    def finish(self) -> None:
        if not self.path:
            return
        path = Path(self.path)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            json.dumps(
                {
                    "status": "passed",
                    "name": self.name,
                    "elapsed_ms": int((time.monotonic() - self.started) * 1000),
                    "facts": self.facts,
                    "steps": self.steps,
                    "proof_layers": [
                        "docker image build",
                        "hermes-agent 0.17 runtime",
                        "finitechat binary in image",
                        "finitechat plugin in image",
                        "finitechat-server on host",
                        "agent container with persistent state volume",
                        "user finitechat CLI in Docker",
                        "real Hermes gateway process",
                        "gateway invite admission before restore",
                        "restic encrypted repository init",
                        "entrypoint restic encrypted agent state snapshot on shutdown",
                        "restic repository check",
                        "agent state volume wipe",
                        "fresh agent container with empty local state",
                        "entrypoint restic latest-by-tag restore into fresh volume",
                        "same agent npub after restore",
                        "runtime HTTP health endpoint after restore",
                        "gateway invite admission after restore",
                    ],
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        print(f"Hermes Docker smoke report: {path}")


class ResticRepositoryHelperTest(unittest.TestCase):
    def test_empty_restic_password_uses_local_default(self) -> None:
        self.assertEqual(
            restic_password_from_env({"FINITE_DOCKER_RESTIC_PASSWORD": ""}),
            DEFAULT_LOCAL_RESTIC_PASSWORD,
        )

    def test_redact_restic_repository_removes_s3_userinfo(self) -> None:
        self.assertEqual(
            redact_restic_repository(
                "s3:https://access:secret@objects.nyc.storage.sh/bucket/prefix"
            ),
            "s3:https://objects.nyc.storage.sh/bucket/prefix",
        )

    def test_redact_restic_repository_leaves_plain_s3_url(self) -> None:
        repository = "s3:https://objects.nyc.storage.sh/bucket/prefix"
        self.assertEqual(redact_restic_repository(repository), repository)

    def test_restic_env_value_accepts_finite_prefixed_alias(self) -> None:
        with mock.patch.dict(
            os.environ,
            {"FINITE_DOCKER_RESTIC_AWS_ACCESS_KEY_ID": "finite-access"},
            clear=True,
        ):
            self.assertEqual(restic_env_value("AWS_ACCESS_KEY_ID"), "finite-access")


class AgentRuntimeLauncherConfigTest(unittest.TestCase):
    def test_gateway_launcher_does_not_persist_raw_finite_private_key(self) -> None:
        script = (REPO_ROOT / "containers/agent/run_hermes_gateway.sh").read_text(encoding="utf-8")

        self.assertIn("api_key: ${FINITE_PRIVATE_API_KEY}", script)
        self.assertIn("api_key: ${FINITECHAT_HERMES_API_KEY}", script)
        self.assertNotIn("api_key: ${api_key}", script)

    def test_gateway_launcher_defaults_startup_invite_room_to_home_channel(self) -> None:
        script = (REPO_ROOT / "containers/agent/run_hermes_gateway.sh").read_text(encoding="utf-8")

        self.assertIn("home-channel show", script)
        self.assertIn("home-channel set", script)
        self.assertIn("invite_room_id", script)
        self.assertIn("export FINITECHAT_HOME_CHANNEL", script)
        self.assertIn("gateway_home_channel_yaml", script)
        self.assertIn("home_channel:", script)
        self.assertIn("chat_id: ${FINITECHAT_HOME_CHANNEL}", script)
        self.assertIn("FINITE_AGENT_HOME_CHANNEL_WARN", script)


@unittest.skipUnless(
    os.environ.get("FINITE_DOCKER_E2E") == "1",
    "set FINITE_DOCKER_E2E=1 (scripts/hermes-sidecar-docker-smoke.sh) to run",
)
class AgentDockerE2ETest(unittest.TestCase):
    def setUp(self):
        if shutil.which("docker") is None:
            self.fail("Docker is not installed")
        status = run(["docker", "info"], check=False, timeout=60)
        if status.returncode != 0:
            self.fail("Docker daemon is not running")
        self.tmp = tempfile.TemporaryDirectory(dir=REPO_ROOT / "target")
        self.addCleanup(self.tmp.cleanup)
        self.server_proc = None
        self.agent_volume = f"{CONTAINER}-agent-{int(time.time() * 1000)}"
        self.user_volume = f"{CONTAINER}-user-{int(time.time() * 1000)}"
        self.restored_user_volume = f"{CONTAINER}-restored-user-{int(time.time() * 1000)}"
        self.addCleanup(self._teardown)

    def _teardown(self):
        run(["docker", "rm", "-f", CONTAINER], check=False, timeout=120)
        run(["docker", "volume", "rm", "-f", self.agent_volume], check=False, timeout=120)
        run(["docker", "volume", "rm", "-f", self.user_volume], check=False, timeout=120)
        run(
            ["docker", "volume", "rm", "-f", self.restored_user_volume],
            check=False,
            timeout=120,
        )
        if self.server_proc is not None:
            self.server_proc.terminate()
            try:
                self.server_proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.server_proc.kill()

    def docker_args(self) -> list[str]:
        return ["docker", "run", "--rm", "--add-host", f"{DOCKER_HOST}:host-gateway"]

    def docker_user_hermes(
        self,
        *args: str,
        timeout: int = 180,
        volume: str | None = None,
    ) -> dict[str, Any]:
        result = run(
            [
                *self.docker_args(),
                "-e",
                "FINITE_HOME=/data/user",
                "--mount",
                f"type=volume,src={volume or self.user_volume},dst=/data/user",
                IMAGE,
                "finitechat",
                "hermes",
                "--home",
                "/data/user",
                *args,
            ],
            timeout=timeout,
        )
        return json.loads(result.stdout)

    def runtime_invite(self) -> dict[str, Any]:
        result = run(
            [
                "docker",
                "exec",
                CONTAINER,
                "cat",
                "/tmp/finitechat-invite.json",
            ],
            timeout=60,
        )
        return json.loads(result.stdout)

    def create_runtime_invite(self, room_id: str) -> dict[str, Any]:
        result = run(
            [
                "docker",
                "exec",
                CONTAINER,
                "finitechat",
                "hermes",
                "--home",
                "/data/agent",
                "invite",
                "--room-id",
                room_id,
                "--max-joins",
                "1",
                "--json",
            ],
            timeout=60,
        )
        return json.loads(result.stdout)

    def start_agent_container(
        self,
        server_url: str,
        restore_repository: dict[str, Any] | None = None,
        restore_snapshot_id: str | None = None,
        restore_latest: bool = False,
        backup_repository: dict[str, Any] | None = None,
    ) -> None:
        run(["docker", "rm", "-f", CONTAINER], check=False, timeout=120)
        command = [
            "docker",
            "run",
            "--name",
            CONTAINER,
            "--detach",
            "--add-host",
            f"{DOCKER_HOST}:host-gateway",
            "--mount",
            f"type=volume,src={self.agent_volume},dst=/data/agent",
            "--env",
            f"FINITE_SERVER_URL={server_url}",
            "--env",
            "FINITECHAT_HERMES_INBOUND_STREAM=1",
        ]
        env = os.environ.copy()
        for name in MODEL_ENV_NAMES:
            if os.environ.get(name):
                command.extend(["--env", name])
        runtime_repository = restore_repository or backup_repository
        if restore_repository is not None and backup_repository is not None:
            self.assertEqual(restore_repository["kind"], backup_repository["kind"])
            self.assertEqual(
                restore_repository["repository"],
                backup_repository["repository"],
            )
        if runtime_repository is not None:
            command.extend(["--env", "FINITE_AGENT_RESTIC_PASSWORD"])
            env["FINITE_AGENT_RESTIC_PASSWORD"] = RESTIC_PASSWORD
            if runtime_repository["kind"] == "local":
                command.extend(
                    [
                        "--mount",
                        f"type=bind,src={runtime_repository['host_path']},dst=/backup-repo",
                        "--env",
                        "FINITE_AGENT_RESTIC_REPOSITORY=/backup-repo",
                    ]
                )
            else:
                command.extend(
                    [
                        "--env",
                        f"FINITE_AGENT_RESTIC_REPOSITORY={runtime_repository['repository']}",
                    ]
                )
                for name in AWS_RESTIC_ENV_NAMES:
                    value = restic_env_value(name)
                    if value:
                        env[name] = value
                        command.extend(["--env", name])
        if backup_repository is not None:
            command.extend(
                [
                    "--env",
                    "FINITE_AGENT_BACKUP_ON_EXIT=1",
                    "--env",
                    "FINITE_AGENT_BACKUP_ACTIVITY_STALE_SECS=0",
                    "--env",
                    f"FINITE_AGENT_RESTIC_BACKUP_TAG={RESTIC_SNAPSHOT_TAG}",
                ]
            )
        if restore_repository is not None:
            if restore_snapshot_id is None and not restore_latest:
                self.fail(
                    "restore_snapshot_id or restore_latest=True is required with restore_repository"
                )
            command.extend(["--env", "FINITE_AGENT_RESTORE_ON_START=1"])
            if restore_latest:
                command.extend(["--env", "FINITE_AGENT_RESTORE_LATEST=1"])
            if restore_snapshot_id is not None:
                command.extend(["--env", f"FINITE_AGENT_RESTIC_SNAPSHOT_ID={restore_snapshot_id}"])
        command.append(IMAGE)
        run(command, timeout=300, env=env)

    def agent_identity(self) -> dict[str, Any]:
        return json.loads(
            run(
                [
                    "docker",
                    "exec",
                    CONTAINER,
                    "finitechat",
                    "auth",
                    "status",
                ],
                timeout=60,
            ).stdout
        )

    def agent_http_health(self) -> dict[str, Any]:
        return json.loads(
            run(
                [
                    "docker",
                    "exec",
                    CONTAINER,
                    "python",
                    "-c",
                    (
                        "import urllib.request; "
                        "print(urllib.request.urlopen("
                        "'http://127.0.0.1:8080/healthz', timeout=5"
                        ").read().decode())"
                    ),
                ],
                timeout=60,
            ).stdout
        )

    def restic_repository(self, tmp: Path) -> dict[str, Any]:
        if RESTIC_BACKEND == "local":
            return {
                "kind": "local",
                "host_path": self.restic_repository_path(tmp),
                "repository": "/backup-repo",
            }
        if RESTIC_BACKEND == "s3":
            if not RESTIC_REPOSITORY.startswith("s3:"):
                self.fail(
                    "FINITE_DOCKER_RESTIC_BACKEND=s3 requires "
                    "FINITE_DOCKER_RESTIC_REPOSITORY=s3:https://endpoint/bucket/prefix"
                )
            missing = [
                name
                for name in ("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY")
                if not restic_env_value(name)
            ]
            if missing:
                self.fail(
                    "FINITE_DOCKER_RESTIC_BACKEND=s3 requires "
                    f"{', '.join(missing)} in the environment"
                )
            if not os.environ.get("FINITE_DOCKER_RESTIC_PASSWORD"):
                self.fail(
                    "FINITE_DOCKER_RESTIC_BACKEND=s3 requires "
                    "FINITE_DOCKER_RESTIC_PASSWORD in the environment"
                )
            if RESTIC_PASSWORD == DEFAULT_LOCAL_RESTIC_PASSWORD:
                self.fail(
                    "FINITE_DOCKER_RESTIC_BACKEND=s3 requires an explicit "
                    "FINITE_DOCKER_RESTIC_PASSWORD, not the local smoke default"
                )
            return {
                "kind": "s3",
                "host_path": None,
                "repository": RESTIC_REPOSITORY,
            }
        self.fail("FINITE_DOCKER_RESTIC_BACKEND must be 'local' or 's3'")

    def reset_restic_repository(self, repository: dict[str, Any]) -> None:
        if repository["kind"] != "local":
            return
        path = repository["host_path"]
        if path.exists():
            shutil.rmtree(path)

    def init_restic_repository(self, repository: dict[str, Any]) -> None:
        if repository["kind"] == "local":
            repository["host_path"].mkdir(parents=True, exist_ok=True)
        status = self.restic(repository, ["snapshots", "--json"], check=False, timeout=120)
        if status.returncode == 0:
            return
        self.restic(repository, ["init"], timeout=120)

    def backup_agent_state(self, repository: dict[str, Any]) -> dict[str, Any]:
        result = self.restic(
            repository,
            [
                "backup",
                "/data/agent",
                "--tag",
                RESTIC_SNAPSHOT_TAG,
                "--json",
            ],
            agent_mount="ro",
            timeout=600,
        )
        summary = self.restic_backup_summary(result.stdout)
        snapshot = self.restic_snapshot(repository, summary["snapshot_id"])
        return {
            "backend": "restic",
            "repository": self.restic_repository_report(repository),
            "snapshot": snapshot,
            "summary": summary,
            "tag": RESTIC_SNAPSHOT_TAG,
            "encrypted": True,
        }

    def entrypoint_backup_metadata(self, repository: dict[str, Any]) -> dict[str, Any]:
        snapshot = self.latest_restic_snapshot(repository)
        return {
            "backend": "restic",
            "repository": self.restic_repository_report(repository),
            "snapshot": snapshot,
            "tag": RESTIC_SNAPSHOT_TAG,
            "encrypted": True,
            "source": "entrypoint_backup_on_exit",
        }

    def wipe_agent_volume(self) -> None:
        run(["docker", "rm", "-f", CONTAINER], check=False, timeout=120)
        run(["docker", "volume", "rm", "-f", self.agent_volume], timeout=120)
        run(["docker", "volume", "create", self.agent_volume], timeout=120)

    def check_restic_repository(self, repository: dict[str, Any]) -> None:
        self.restic(repository, ["check"], timeout=600)

    def test_docker_real_gateway_admission_and_restore_with_json_evidence(self):
        report = SmokeReport("docker_real_gateway_admission_and_restore")
        tmp = Path(self.tmp.name)

        def build_host_binaries() -> None:
            run(
                ["cargo", "build", "--release", "-p", "finitechat-server"],
                cwd=REPO_ROOT,
                timeout=1800,
            )

        report.time("host_server_binary_build", build_host_binaries)
        server_bin = REPO_ROOT / "target/release/finitechat-server"

        if SKIP_IMAGE_BUILD:
            report.step("docker_image_prebuilt", 0)
            run(["docker", "image", "inspect", IMAGE], timeout=60)
        else:
            ctx = tmp / "ctx"
            ctx.mkdir()
            report.time("stage_docker_context", lambda: stage_build_context(ctx))
            report.time(
                "docker_image_build",
                lambda: run(
                    [
                        "docker",
                        "build",
                        "--build-arg",
                        f"HERMES_AGENT_VERSION={HERMES_AGENT_VERSION}",
                        "--tag",
                        IMAGE,
                        "--file",
                        str(ctx / "finitechat/containers/agent/Dockerfile"),
                        str(ctx),
                    ],
                    timeout=3600,
                ),
            )
        report.fact("image", IMAGE)
        report.fact("image_build", "prebuilt" if SKIP_IMAGE_BUILD else "smoke")
        image_metadata = self.docker_image_metadata()
        report.fact("image_id", image_metadata["id"])
        report.fact("image_metadata", image_metadata)
        report.fact("agent_state_volume", self.agent_volume)
        report.fact("user_state_volume", self.user_volume)
        report.fact("hermes_agent_version_expected", HERMES_AGENT_VERSION)

        self.server_proc = subprocess.Popen(
            [
                str(server_bin),
                "serve",
                f"0.0.0.0:{SERVER_PORT}",
                "--sqlite",
                str(tmp / "server.sqlite3"),
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.STDOUT,
        )
        report.time(
            "host_server_ready",
            lambda: self._wait_for_health(f"http://127.0.0.1:{SERVER_PORT}/health"),
        )
        server_url = f"http://{DOCKER_HOST}:{SERVER_PORT}"
        report.fact("server_url_from_docker", server_url)

        repository = self.restic_repository(tmp)
        self.reset_restic_repository(repository)
        report.fact("restic_backend", repository["kind"])
        report.fact("restic_repository", self.restic_repository_report(repository))
        report.time(
            "agent_state_restic_repo_init",
            lambda: self.init_restic_repository(repository),
        )

        report.time(
            "agent_container_start",
            lambda: self.start_agent_container(
                server_url,
                backup_repository=repository,
            ),
        )
        report.time(
            "real_gateway_runtime_log",
            lambda: self._wait_for_log("FINITE_AGENT_RUNTIME real_hermes_gateway=true", 180),
        )
        hermes_version = run(
            [
                "docker",
                "exec",
                CONTAINER,
                "python",
                "-c",
                "import importlib.metadata; print(importlib.metadata.version('hermes-agent'))",
            ],
            timeout=60,
        ).stdout.strip()
        self.assertEqual(hermes_version, HERMES_AGENT_VERSION)
        report.fact("hermes_agent_version_actual", hermes_version)
        restic_version = run(
            ["docker", "exec", CONTAINER, "restic", "version"],
            timeout=60,
        ).stdout.strip()
        self.assertTrue(restic_version.startswith("restic "))
        report.fact("restic_version", restic_version)

        invite_info = self.runtime_invite()
        agent_identity = self.agent_identity()
        health = report.time("agent_http_health", self.agent_http_health)
        self.assertTrue(health["ready"])
        self.assertEqual(health["npub"], agent_identity["npub"])
        report.fact("agent_npub", agent_identity["npub"])
        report.fact("real_gateway_runtime", True)
        invite_url = invite_info["url"]

        report.time(
            "user_init_in_docker",
            lambda: self.docker_user_hermes("init", "--server", server_url),
        )
        joined = report.time(
            "user_join_in_docker",
            lambda: self.docker_user_hermes(
                "join",
                "--url",
                invite_url,
                "--timeout-ms",
                "120000",
                timeout=180,
            ),
        )
        self.assertEqual(joined["state"], "joined")
        room_id = joined["room_id"]
        report.fact("room_id", room_id)
        report.fact("gateway_admission_before_restore", True)

        agent_config = json.loads(
            run(
                ["docker", "exec", CONTAINER, "cat", "/data/agent/config.json"],
                timeout=60,
            ).stdout
        )
        report.fact("agent_account_id", agent_config["account_id"])

        report.time(
            "agent_container_stop_with_entrypoint_backup",
            lambda: run(["docker", "stop", "--time", "60", CONTAINER], timeout=120),
        )
        report.time(
            "agent_entrypoint_restic_backup",
            lambda: self._wait_for_log("FINITE_AGENT_BACKUP_COMPLETE", 60),
        )
        backup_meta = self.entrypoint_backup_metadata(repository)
        report.fact("agent_state_backup", backup_meta)
        report.time(
            "agent_state_restic_check",
            lambda: self.check_restic_repository(repository),
        )
        report.time("agent_state_volume_wipe", self.wipe_agent_volume)
        report.time(
            "agent_container_restore_start",
            lambda: self.start_agent_container(
                server_url,
                restore_repository=repository,
                restore_latest=True,
            ),
        )
        report.time(
            "agent_entrypoint_restic_restore",
            lambda: self._wait_for_log("FINITE_AGENT_RESTORE_COMPLETE", 180),
        )
        report.time(
            "agent_ready_log_after_restore",
            lambda: self._wait_for_log("FINITE_AGENT_RUNTIME real_hermes_gateway=true", 180),
        )
        restored_identity = self.agent_identity()
        self.assertEqual(restored_identity["npub"], agent_identity["npub"])
        self.assertEqual(restored_identity["account_id"], agent_config["account_id"])
        restored_health = report.time("agent_http_health_after_restore", self.agent_http_health)
        self.assertTrue(restored_health["ready"])
        self.assertEqual(restored_health["npub"], restored_identity["npub"])
        report.fact("agent_npub_after_restore", restored_identity["npub"])

        cached_restored_invite_info = self.runtime_invite()
        self.assertEqual(cached_restored_invite_info["room_id"], room_id)
        self.assertEqual(cached_restored_invite_info["invite_id"], invite_info["invite_id"])
        restored_invite_info = report.time(
            "restored_agent_fresh_invite",
            lambda: self.create_runtime_invite(room_id),
        )
        self.assertEqual(restored_invite_info["room_id"], room_id)
        self.assertNotEqual(restored_invite_info["invite_id"], invite_info["invite_id"])
        report.time(
            "restored_user_init_in_docker",
            lambda: self.docker_user_hermes(
                "init",
                "--server",
                server_url,
                volume=self.restored_user_volume,
            ),
        )
        restored_join = report.time(
            "restored_user_join_in_docker",
            lambda: self.docker_user_hermes(
                "join",
                "--url",
                restored_invite_info["url"],
                "--timeout-ms",
                "120000",
                timeout=180,
                volume=self.restored_user_volume,
            ),
        )
        self.assertEqual(restored_join["state"], "joined")
        self.assertEqual(restored_join["room_id"], room_id)
        report.fact("gateway_admission_after_restore", True)
        report.finish()

    def restic_repository_path(self, tmp: Path) -> Path:
        report_path = os.environ.get("FINITE_HERMES_DOCKER_SMOKE_REPORT")
        if not report_path:
            return tmp / "restic-repo"
        path = Path(report_path)
        if not path.is_absolute():
            path = REPO_ROOT / path
        path = path.parent / "restic-repo"
        path.parent.mkdir(parents=True, exist_ok=True)
        return path

    def restic(
        self,
        repository: dict[str, Any],
        args: list[str],
        *,
        agent_mount: str | None = None,
        timeout: int = 300,
        check: bool = True,
    ) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["RESTIC_PASSWORD"] = RESTIC_PASSWORD
        command = [
            *self.docker_args(),
            "--env",
            "RESTIC_PASSWORD",
            "--env",
            "RESTIC_CACHE_DIR=/tmp/restic-cache",
        ]
        if repository["kind"] == "local":
            command.extend(
                [
                    "--mount",
                    f"type=bind,src={repository['host_path']},dst=/backup-repo",
                ]
            )
        elif repository["kind"] == "s3":
            for name in AWS_RESTIC_ENV_NAMES:
                value = restic_env_value(name)
                if value:
                    env[name] = value
                    command.extend(["--env", name])
        if agent_mount is not None:
            suffix = ",ro" if agent_mount == "ro" else ""
            command.extend(
                [
                    "--mount",
                    f"type=volume,src={self.agent_volume},dst=/data/agent{suffix}",
                ]
            )
        command.extend([IMAGE, "restic", "-r", repository["repository"], *args])
        return run(command, timeout=timeout, check=check, env=env)

    def restic_snapshot(self, repository: dict[str, Any], snapshot_id: str) -> dict[str, Any]:
        result = self.restic(
            repository,
            ["snapshots", snapshot_id, "--json"],
            timeout=120,
        )
        snapshots = json.loads(result.stdout)
        self.assertEqual(len(snapshots), 1)
        snapshot = snapshots[0]
        return {
            "id": snapshot["id"],
            "short_id": snapshot["short_id"],
            "time": snapshot["time"],
            "paths": snapshot["paths"],
            "tags": snapshot.get("tags") or [],
        }

    def latest_restic_snapshot(self, repository: dict[str, Any]) -> dict[str, Any]:
        result = self.restic(
            repository,
            ["snapshots", "latest", "--tag", RESTIC_SNAPSHOT_TAG, "--json"],
            timeout=120,
        )
        snapshots = json.loads(result.stdout)
        self.assertEqual(len(snapshots), 1)
        snapshot = snapshots[0]
        return {
            "id": snapshot["id"],
            "short_id": snapshot["short_id"],
            "time": snapshot["time"],
            "paths": snapshot["paths"],
            "tags": snapshot.get("tags") or [],
        }

    def restic_repository_report(self, repository: dict[str, Any]) -> dict[str, Any]:
        if repository["kind"] == "local":
            path = repository["host_path"]
            return {
                "kind": "local",
                "path": str(path),
                "size_bytes": directory_size(path) if path.exists() else 0,
            }
        return {
            "kind": "s3",
            "repository": redact_restic_repository(repository["repository"]),
            "size_bytes": None,
        }

    def restic_backup_summary(self, stdout: str) -> dict[str, Any]:
        summary: dict[str, Any] = {}
        for line in stdout.splitlines():
            if not line.strip():
                continue
            item = json.loads(line)
            if item.get("message_type") == "summary":
                summary = item
        self.assertTrue(summary, "restic backup did not print a JSON summary")
        return {
            "snapshot_id": summary["snapshot_id"],
            "total_files_processed": summary["total_files_processed"],
            "total_bytes_processed": summary["total_bytes_processed"],
            "files_new": summary["files_new"],
            "dirs_new": summary["dirs_new"],
            "data_blobs": summary["data_blobs"],
            "tree_blobs": summary["tree_blobs"],
        }

    def docker_image_metadata(self) -> dict[str, Any]:
        image = json.loads(run(["docker", "image", "inspect", IMAGE], timeout=60).stdout)[0]
        return {
            "id": image["Id"],
            "repo_tags": image.get("RepoTags") or [],
            "repo_digests": image.get("RepoDigests") or [],
            "created": image.get("Created"),
            "size_bytes": image.get("Size"),
        }

    def _wait_for_health(self, url: str, timeout: float = 30) -> None:
        import urllib.request

        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                with urllib.request.urlopen(url, timeout=2) as response:
                    if response.status == 200:
                        return
            except Exception:
                time.sleep(0.2)
        self.fail(f"server at {url} never became healthy")

    def _wait_for_log(self, marker: str, timeout: float = 60) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            logs = run(["docker", "logs", CONTAINER], check=False, timeout=60)
            if marker in (logs.stdout or ""):
                return
            time.sleep(2)
        logs = run(["docker", "logs", CONTAINER], check=False, timeout=60)
        self.fail(f"container never printed {marker!r}; logs:\n{(logs.stdout or '')[-4000:]}")

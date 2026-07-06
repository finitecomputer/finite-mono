#!/usr/bin/env python3
"""Run the remote Docker layer of the Hermes phone canary loop.

This is the Layer 2 gate from docs/hermes-phone-canary-loop.md. It builds the
real runtime image on a remote Docker daemon, starts it against the hosted
Finite Chat server, proves invite admission plus real Hermes model replies,
then proves entrypoint backup/restore before handing the invite to a human.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import secrets
import shlex
import subprocess
import sys
import tempfile
import time
import urllib.parse
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SERVER_URL = "https://chat.finite.computer"
DEFAULT_DOCKER_HOST = "ssh://finite-lat-2"
DEFAULT_HERMES_VERSION = "0.17.0"
MODEL_ENV_NAMES = (
    "OPENROUTER_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "FINITECHAT_HERMES_MODEL",
    "FINITECHAT_HERMES_PROVIDER",
    "FINITECHAT_HERMES_BASE_URL",
    "FINITECHAT_HERMES_API_MODE",
)
BUILD_EXCLUDES = (
    ".git",
    "target",
    "__pycache__",
    ".DS_Store",
    ".env",
    ".env.*",
    ".finitechat",
    ".state",
    "secrets",
)


class CanaryFailure(RuntimeError):
    pass


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--docker-host",
        default=os.environ.get("FINITECHAT_REMOTE_DOCKER_HOST", DEFAULT_DOCKER_HOST),
        help="Docker host URI. Defaults to ssh://finite-lat-2.",
    )
    parser.add_argument(
        "--server-url",
        default=os.environ.get("FINITECHAT_REMOTE_DOCKER_SERVER_URL", DEFAULT_SERVER_URL),
        help="Finite Chat server URL. Product remote Docker canaries must use https://chat.finite.computer.",
    )
    parser.add_argument(
        "--state-root",
        default=os.environ.get("FINITECHAT_REMOTE_DOCKER_STATE_ROOT", ""),
        help="Local evidence root. Defaults to target/hermes-phone-canary/remote-docker/<run-id>.",
    )
    parser.add_argument(
        "--report",
        default=os.environ.get("FINITECHAT_REMOTE_DOCKER_REPORT", ""),
        help="JSON report path. Defaults to <state-root>/report.json.",
    )
    parser.add_argument(
        "--image",
        default=os.environ.get("FINITECHAT_REMOTE_DOCKER_IMAGE", ""),
        help="Remote image tag. Defaults to finite-agent-remote-canary:<run-id>.",
    )
    parser.add_argument(
        "--container",
        default=os.environ.get("FINITECHAT_REMOTE_DOCKER_CONTAINER", ""),
        help="Remote container name. Defaults to finite-agent-remote-canary-<run-id>.",
    )
    parser.add_argument(
        "--hermes-agent-version",
        default=os.environ.get("FINITE_HERMES_AGENT_VERSION", DEFAULT_HERMES_VERSION),
    )
    parser.add_argument(
        "--room-name",
        default=os.environ.get("FINITECHAT_REMOTE_DOCKER_ROOM_NAME", "Remote Docker Hermes Canary"),
    )
    parser.add_argument(
        "--agent-device-id",
        default=os.environ.get("FINITECHAT_REMOTE_DOCKER_AGENT_DEVICE_ID", "remote-docker"),
    )
    parser.add_argument(
        "--local-phone-report",
        default=os.environ.get("FINITECHAT_LOCAL_PHONE_REPORT", ""),
        help="Passed local phone report to promote from. Defaults to the latest local canary report.",
    )
    parser.add_argument(
        "--skip-local-phone-report",
        action="store_true",
        help="Do not require a passed local phone report before remote Docker.",
    )
    parser.add_argument(
        "--skip-build", action="store_true", help="Use an existing remote image tag."
    )
    parser.add_argument(
        "--keep-running",
        action="store_true",
        help="Leave the restored remote container running after the canary passes.",
    )
    parser.add_argument(
        "--keep-failed",
        action="store_true",
        help="Leave failed remote containers/volumes behind for debugging.",
    )
    parser.add_argument(
        "--env-file",
        action="append",
        default=[],
        help="Optional KEY=VALUE env file for model provider settings. May be repeated.",
    )
    parser.add_argument(
        "--no-default-env-file",
        action="store_true",
        help="Do not auto-load local repo/finitecomputer env files.",
    )
    return parser.parse_args()


def run(
    args: list[str],
    *,
    env: dict[str, str] | None = None,
    timeout: float = 60,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    proc = subprocess.run(
        args,
        cwd=REPO_ROOT,
        env=env,
        text=True,
        capture_output=True,
        timeout=timeout,
    )
    if check and proc.returncode != 0:
        raise CanaryFailure(
            "command failed: "
            + repr(args)
            + f"\nexit={proc.returncode}\nstdout={proc.stdout[-3000:]}\nstderr={proc.stderr[-3000:]}"
        )
    return proc


def run_json(
    args: list[str],
    *,
    env: dict[str, str] | None = None,
    timeout: float = 60,
) -> dict[str, Any]:
    proc = run(args, env=env, timeout=timeout)
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise CanaryFailure(
            f"command did not emit JSON: {args!r}\nstdout={proc.stdout[-3000:]}"
        ) from exc


def docker(
    opts: argparse.Namespace,
    args: list[str],
    *,
    env: dict[str, str] | None = None,
    timeout: float = 60,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    return run(["docker", "--host", opts.docker_host, *args], env=env, timeout=timeout, check=check)


def docker_json(
    opts: argparse.Namespace,
    args: list[str],
    *,
    env: dict[str, str] | None = None,
    timeout: float = 60,
) -> dict[str, Any]:
    proc = docker(opts, args, env=env, timeout=timeout)
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise CanaryFailure(
            f"docker command did not emit JSON: {args!r}\nstdout={proc.stdout[-3000:]}"
        ) from exc


def enforce_product_server_url(server_url: str) -> None:
    parsed = urllib.parse.urlparse(server_url)
    host = (parsed.hostname or "").lower()
    if parsed.scheme not in {"http", "https"} or not host:
        raise CanaryFailure(f"server URL must be an http(s) origin, got {server_url!r}")
    if parsed.path not in {"", "/"} or parsed.query or parsed.fragment:
        raise CanaryFailure(f"server URL must be an origin, got {server_url!r}")
    normalized = f"{parsed.scheme}://{host}"
    if parsed.port is not None:
        normalized = f"{normalized}:{parsed.port}"
    if normalized.rstrip("/") != DEFAULT_SERVER_URL:
        raise CanaryFailure(
            "product remote Docker canary must use "
            f"{DEFAULT_SERVER_URL}; got {server_url!r}. Use lower-level Docker smoke "
            "tests for local delivery-server experiments."
        )
    if host in {"localhost", "127.0.0.1", "::1"} or host.startswith("127."):
        raise CanaryFailure(
            f"remote Docker canary must not use loopback server URL: {server_url!r}"
        )


def timestamp_id() -> str:
    return time.strftime("run-%Y%m%d-%H%M%S")


def slug_from_run_id(run_id: str) -> str:
    return re.sub(r"[^a-z0-9_.-]+", "-", run_id.lower())


def parse_env_file(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    if not path.is_file():
        return values
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key):
            continue
        value = value.strip()
        if (value.startswith('"') and value.endswith('"')) or (
            value.startswith("'") and value.endswith("'")
        ):
            value = value[1:-1]
        values[key] = value
    return values


def load_canary_env(opts: argparse.Namespace) -> tuple[dict[str, str], list[str]]:
    env = os.environ.copy()
    loaded: list[str] = []
    env_files = [Path(p) for p in opts.env_file]
    explicit = os.environ.get("FINITECHAT_REMOTE_DOCKER_ENV_FILE")
    if explicit:
        env_files.append(Path(explicit))
    if not opts.no_default_env_file:
        finite_root = REPO_ROOT.parent.parent
        env_files.extend(
            [
                REPO_ROOT / ".env",
                finite_root / "finitecomputer/secrets/shared-provider-keys.env",
                finite_root / "finitecomputer/.state/chat-local/.env",
            ]
        )
    for env_file in env_files:
        values = parse_env_file(env_file)
        if values:
            env.update(values)
            loaded.append(str(env_file))
    return env, loaded


def git_source() -> dict[str, Any]:
    def maybe(args: list[str]) -> str:
        proc = run(["git", *args], check=False)
        return proc.stdout.strip() if proc.returncode == 0 else ""

    porcelain = maybe(["status", "--porcelain"])
    return {
        "repo": "finitecomputer/finitechat",
        "branch": maybe(["rev-parse", "--abbrev-ref", "HEAD"]),
        "commit": maybe(["rev-parse", "HEAD"]),
        "dirty": bool(porcelain),
        "status_short": maybe(["status", "--short", "--branch"]),
    }


def sha256_tree(path: Path) -> str:
    digest = hashlib.sha256()
    for file_path in sorted(p for p in path.rglob("*") if p.is_file()):
        rel = file_path.relative_to(path).as_posix().encode("utf-8")
        digest.update(rel)
        digest.update(b"\0")
        digest.update(file_path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def latest_local_phone_report() -> Path | None:
    root = REPO_ROOT / "target/hermes-phone-canary/local"
    reports = sorted(root.glob("run-*/report.json"), key=lambda p: p.stat().st_mtime, reverse=True)
    return reports[0] if reports else None


def load_required_local_report(opts: argparse.Namespace) -> dict[str, Any] | None:
    if opts.skip_local_phone_report:
        return None
    path = Path(opts.local_phone_report) if opts.local_phone_report else latest_local_phone_report()
    if path is None:
        raise CanaryFailure(
            "no local phone canary report found; pass --skip-local-phone-report to override"
        )
    report = json.loads(path.read_text(encoding="utf-8"))
    if report.get("status") != "passed":
        raise CanaryFailure(f"local phone canary report is not passed: {path}")
    return {
        "path": str(path),
        "status": report.get("status"),
        "run_id": report.get("run_id"),
        "source_commit": (report.get("source") or {}).get("commit"),
        "agent_npub": (report.get("agent") or {}).get("npub"),
        "model_smoke": report.get("model_smoke"),
    }


def stage_build_context(ctx: Path) -> Path:
    dest = ctx / "finitechat"
    dest.mkdir(parents=True, exist_ok=True)
    command = ["rsync", "-a", "--delete"]
    for item in BUILD_EXCLUDES:
        command.extend(["--exclude", item])
    command.extend([f"{REPO_ROOT}/", f"{dest}/"])
    run(command, timeout=600)
    return ctx


def wait_container_log(
    opts: argparse.Namespace, container: str, marker: str, *, timeout: float
) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        logs = docker(opts, ["logs", container], check=False, timeout=60)
        if marker in (logs.stdout or "") or marker in (logs.stderr or ""):
            return
        ensure_container_running(opts, container)
        time.sleep(2)
    logs = docker(opts, ["logs", container], check=False, timeout=60)
    raise CanaryFailure(
        f"container never printed {marker!r}; logs:\n{(logs.stdout or logs.stderr)[-5000:]}"
    )


def ensure_container_running(opts: argparse.Namespace, container: str) -> None:
    inspect = docker(opts, ["inspect", container], check=False, timeout=30)
    if inspect.returncode != 0:
        raise CanaryFailure(f"container {container} is not inspectable: {inspect.stderr[-1000:]}")
    data = json.loads(inspect.stdout)[0]
    state = data.get("State") or {}
    if state.get("Running") is True:
        return
    logs = docker(opts, ["logs", container], check=False, timeout=60)
    raise CanaryFailure(
        f"container {container} is not running; state={state}; logs:\n{(logs.stdout or logs.stderr)[-5000:]}"
    )


def container_http_json(
    opts: argparse.Namespace, container: str, path: str, *, timeout: float = 30
) -> dict[str, Any]:
    code = (
        "import json,sys,urllib.request;"
        f"print(urllib.request.urlopen('http://127.0.0.1:8080{path}', timeout=5).read().decode())"
    )
    return docker_json(opts, ["exec", container, "python", "-c", code], timeout=timeout)


def wait_container_http_json(
    opts: argparse.Namespace, container: str, path: str, *, timeout: float, name: str
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    last_error = ""
    while time.monotonic() < deadline:
        try:
            payload = container_http_json(opts, container, path, timeout=15)
            if payload.get("ready") is not False:
                return payload
            last_error = json.dumps(payload)
        except Exception as exc:
            last_error = str(exc)
        ensure_container_running(opts, container)
        time.sleep(1)
    raise CanaryFailure(f"{name} did not become ready at {path}: {last_error}")


def wait_fresh_invite(
    opts: argparse.Namespace, container: str, *, timeout: float = 45
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    last: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        invite = wait_container_http_json(
            opts, container, "/invite", timeout=15, name="container invite"
        )
        last = invite
        if invite.get("url") and invite.get("room_id") and invite.get("invite_id"):
            return invite
        time.sleep(1)
    if last:
        return last
    raise CanaryFailure("container invite endpoint did not return an invite URL")


def docker_image_metadata(opts: argparse.Namespace, image: str) -> dict[str, Any]:
    data = docker_json(opts, ["image", "inspect", image], timeout=60)
    item = data[0]
    return {
        "id": item.get("Id"),
        "repo_tags": item.get("RepoTags") or [],
        "repo_digests": item.get("RepoDigests") or [],
        "created": item.get("Created"),
        "size_bytes": item.get("Size"),
    }


def docker_info(opts: argparse.Namespace) -> dict[str, Any]:
    data = docker_json(opts, ["info", "--format", "{{json .}}"], timeout=60)
    return {
        "name": data.get("Name"),
        "server_version": data.get("ServerVersion"),
        "operating_system": data.get("OperatingSystem"),
        "architecture": data.get("Architecture"),
        "ncpu": data.get("NCPU"),
        "memory_bytes": data.get("MemTotal"),
    }


def docker_volume_create(opts: argparse.Namespace, name: str) -> None:
    docker(opts, ["volume", "create", name], timeout=60)


def docker_volume_rm(opts: argparse.Namespace, name: str) -> None:
    docker(opts, ["volume", "rm", "-f", name], check=False, timeout=120)


def docker_container_rm(opts: argparse.Namespace, container: str) -> None:
    docker(opts, ["rm", "-f", container], check=False, timeout=120)


def init_restic_repository(
    opts: argparse.Namespace,
    *,
    image: str,
    restic_volume: str,
    restic_password: str,
    env: dict[str, str],
) -> None:
    restic_env = env.copy()
    restic_env["RESTIC_PASSWORD"] = restic_password
    base = [
        "run",
        "--rm",
        "--mount",
        f"type=volume,src={restic_volume},dst=/backup-repo",
        "--env",
        "RESTIC_PASSWORD",
        "--env",
        "RESTIC_CACHE_DIR=/tmp/restic-cache",
        image,
        "restic",
        "-r",
        "/backup-repo",
    ]
    status = docker(opts, [*base, "snapshots", "--json"], env=restic_env, check=False, timeout=180)
    if status.returncode == 0:
        return
    docker(opts, [*base, "init"], env=restic_env, timeout=180)


def container_env_args(env: dict[str, str], names: tuple[str, ...]) -> list[str]:
    args: list[str] = []
    for name in names:
        if env.get(name):
            args.extend(["--env", name])
    return args


def start_agent_container(
    opts: argparse.Namespace,
    *,
    image: str,
    container: str,
    agent_volume: str,
    restic_volume: str,
    restic_password: str,
    server_url: str,
    restore_latest: bool,
    env: dict[str, str],
) -> str:
    docker_container_rm(opts, container)
    runtime_env = env.copy()
    runtime_env["FINITE_AGENT_RESTIC_PASSWORD"] = restic_password
    command = [
        "run",
        "--name",
        container,
        "--detach",
        "--mount",
        f"type=volume,src={agent_volume},dst=/data/agent",
        "--mount",
        f"type=volume,src={restic_volume},dst=/backup-repo",
        "--env",
        f"FINITE_SERVER_URL={server_url}",
        "--env",
        "FINITECHAT_HERMES_INBOUND_STREAM=1",
        "--env",
        "FINITECHAT_HERMES_PLUGIN_NAME=finitechat",
        "--env",
        f"FINITECHAT_HERMES_ROOM_NAME={opts.room_name}",
        "--env",
        f"FINITECHAT_HERMES_AGENT_DEVICE_ID={opts.agent_device_id}",
        "--env",
        "FINITE_AGENT_RESTIC_REPOSITORY=/backup-repo",
        "--env",
        "FINITE_AGENT_RESTIC_PASSWORD",
        "--env",
        "FINITE_AGENT_BACKUP_ON_EXIT=1",
        "--env",
        "FINITE_AGENT_RESTIC_BACKUP_TAG=remote-docker-canary",
        *container_env_args(env, MODEL_ENV_NAMES),
    ]
    if restore_latest:
        command.extend(
            [
                "--env",
                "FINITE_AGENT_RESTORE_ON_START=1",
                "--env",
                "FINITE_AGENT_RESTORE_LATEST=1",
            ]
        )
    command.append(image)
    return docker(opts, command, env=runtime_env, timeout=300).stdout.strip()


def docker_user_hermes(
    opts: argparse.Namespace,
    *,
    image: str,
    volume: str,
    args: list[str],
    env: dict[str, str],
    timeout: float = 180,
) -> dict[str, Any]:
    return docker_json(
        opts,
        [
            "run",
            "--rm",
            "--mount",
            f"type=volume,src={volume},dst=/data/user",
            image,
            "finitechat",
            "hermes",
            "--home",
            "/data/user",
            *args,
        ],
        env=env,
        timeout=timeout,
    )


def docker_user_app(
    opts: argparse.Namespace,
    *,
    image: str,
    volume: str,
    server_url: str,
    args: list[str],
    env: dict[str, str],
    timeout: float = 180,
) -> dict[str, Any]:
    return docker_json(
        opts,
        [
            "run",
            "--rm",
            "--mount",
            f"type=volume,src={volume},dst=/data/user",
            image,
            "finitechat",
            "app",
            "--data-dir",
            "/data/user",
            "--server",
            server_url,
            "--device-id",
            "probe",
            *args,
        ],
        env=env,
        timeout=timeout,
    )


def admit_user(
    opts: argparse.Namespace,
    *,
    image: str,
    user_volume: str,
    server_url: str,
    invite: dict[str, Any],
    display_name: str,
    env: dict[str, str],
) -> dict[str, Any]:
    docker_user_hermes(
        opts,
        image=image,
        volume=user_volume,
        args=["init", "--server", server_url, "--device-id", "probe", "--skip-agent-profile"],
        env=env,
        timeout=120,
    )
    started = time.monotonic()
    joined = docker_user_hermes(
        opts,
        image=image,
        volume=user_volume,
        args=[
            "join",
            "--url",
            str(invite["url"]),
            "--timeout-ms",
            "120000",
        ],
        env=env,
        timeout=180,
    )
    if joined.get("state") != "joined":
        raise CanaryFailure(f"admission did not join: {joined!r}")
    joined["elapsed_ms"] = int((time.monotonic() - started) * 1000)
    return joined


def message_matches(text: str, expected: str) -> bool:
    return expected.lower() in text.lower()


def run_model_smoke(
    opts: argparse.Namespace,
    *,
    image: str,
    user_volume: str,
    server_url: str,
    room_id: str,
    expected: str,
    env: dict[str, str],
) -> dict[str, Any]:
    prompt = f"Reply with exactly: {expected}"
    started = time.monotonic()
    sent = docker_user_app(
        opts,
        image=image,
        volume=user_volume,
        server_url=server_url,
        args=["send", "--room-id", room_id, "--text", prompt],
        env=env,
        timeout=120,
    )
    deadline = time.monotonic() + 180
    last_state: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        state = docker_user_app(
            opts,
            image=image,
            volume=user_volume,
            server_url=server_url,
            args=[
                "state",
                "--start-runtime",
                "--wait-update-ms",
                "4000",
                "--room-id",
                room_id,
            ],
            env=env,
            timeout=60,
        )
        last_state = state
        for message in state.get("messages") or []:
            if not message.get("is_mine") and message_matches(
                str(message.get("text") or ""), expected
            ):
                return {
                    "status": "passed",
                    "elapsed_ms": int((time.monotonic() - started) * 1000),
                    "prompt_message_id": first_matching_mine_message_id(sent, prompt),
                    "reply_message_id": message.get("message_id"),
                    "reply_text": message.get("text"),
                }
        time.sleep(2)
    sample = [
        {
            "is_mine": message.get("is_mine"),
            "text": message.get("text"),
            "message_id": message.get("message_id"),
        }
        for message in ((last_state or {}).get("messages") or [])[-8:]
    ]
    raise CanaryFailure(
        f"Hermes model smoke did not receive expected reply {expected!r}; recent messages={sample!r}"
    )


def first_matching_mine_message_id(state: dict[str, Any], prompt: str) -> str | None:
    for message in state.get("messages") or []:
        if message.get("is_mine") and str(message.get("text") or "") == prompt:
            value = message.get("message_id")
            return str(value) if value else None
    return None


def write_stop_script(
    *,
    path: Path,
    docker_host: str,
    container: str,
    volumes: list[str],
) -> None:
    lines = [
        "#!/usr/bin/env bash",
        "set -euo pipefail",
        f"docker --host {shlex.quote(docker_host)} rm -f {shlex.quote(container)} >/dev/null 2>&1 || true",
    ]
    if volumes:
        quoted = " ".join(shlex.quote(volume) for volume in volumes)
        lines.append(
            f"docker --host {shlex.quote(docker_host)} volume rm -f {quoted} >/dev/null 2>&1 || true"
        )
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    path.chmod(0o755)


def stop_for_backup(opts: argparse.Namespace, container: str) -> None:
    docker(opts, ["stop", "--time", "60", container], timeout=120)
    wait_container_log(opts, container, "FINITE_AGENT_BACKUP_COMPLETE", timeout=90)


def main() -> int:
    opts = parse_args()
    enforce_product_server_url(opts.server_url)
    run_id = timestamp_id()
    slug = slug_from_run_id(run_id)
    state_root = (
        Path(opts.state_root)
        if opts.state_root
        else REPO_ROOT / "target/hermes-phone-canary/remote-docker" / run_id
    )
    report_path = Path(opts.report) if opts.report else state_root / "report.json"
    state_root.mkdir(parents=True, exist_ok=True)
    logs_dir = state_root / "logs"
    logs_dir.mkdir(parents=True, exist_ok=True)
    image = opts.image or f"finite-agent-remote-canary:{slug}"
    container = opts.container or f"finite-agent-remote-canary-{slug}"
    agent_volume = f"{container}-agent"
    restic_volume = f"{container}-restic"
    user_volume = f"{container}-user"
    restored_user_volume = f"{container}-restored-user"
    restic_password = os.environ.get(
        "FINITECHAT_REMOTE_DOCKER_RESTIC_PASSWORD"
    ) or secrets.token_urlsafe(32)
    stop_script = state_root / "stop.sh"

    env, env_files_loaded = load_canary_env(opts)
    local_report = load_required_local_report(opts)
    plugin_dir = REPO_ROOT / "integrations/hermes/finitechat"
    started = time.monotonic()
    report: dict[str, Any] = {
        "status": "running",
        "layer": "remote-docker",
        "run_id": run_id,
        "source": git_source(),
        "local_phone_report": local_report,
        "server": {
            "url": opts.server_url,
            "phone_reachable": True,
        },
        "docker": {
            "host": opts.docker_host,
            "container": container,
            "agent_volume": agent_volume,
            "restic_volume": restic_volume,
            "user_volume": user_volume,
            "restored_user_volume": restored_user_volume,
            "info": None,
        },
        "runtime": {
            "hermes_agent_version_expected": opts.hermes_agent_version,
            "plugin_name": "finitechat",
            "plugin_hash": sha256_tree(plugin_dir),
            "image_ref": image,
            "image_id": None,
            "image_digest": None,
            "model_env_present": {name: bool(env.get(name)) for name in MODEL_ENV_NAMES},
            "env_files_loaded": env_files_loaded,
        },
        "restic": {
            "backend": "docker-volume",
            "repository": "/backup-repo",
            "tag": "remote-docker-canary",
            "encrypted": True,
            "password_source": "env:FINITECHAT_REMOTE_DOCKER_RESTIC_PASSWORD"
            if os.environ.get("FINITECHAT_REMOTE_DOCKER_RESTIC_PASSWORD")
            else "generated_for_run",
        },
        "agent": {
            "restored": False,
            "npub": None,
        },
        "invite": None,
        "admission_probe": None,
        "model_smoke": None,
        "restore": None,
        "phone": {
            "installed": None,
            "human_chat_event_ids": [],
        },
        "steps": [],
    }

    def write_report() -> None:
        report_path.write_text(
            json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )

    def step(name: str, **facts: Any) -> None:
        report["steps"].append(
            {"name": name, "elapsed_ms": int((time.monotonic() - started) * 1000), **facts}
        )
        write_report()

    write_report()
    cleanup_volumes = [agent_volume, restic_volume, user_volume, restored_user_volume]
    keep_remote = False
    try:
        report["docker"]["info"] = docker_info(opts)
        step("docker.remote_info")

        if not opts.skip_build:
            with tempfile.TemporaryDirectory(dir=state_root) as tmpdir:
                ctx = stage_build_context(Path(tmpdir) / "ctx")
                step("docker.context_staged")
                docker(
                    opts,
                    [
                        "build",
                        "--build-arg",
                        f"HERMES_AGENT_VERSION={opts.hermes_agent_version}",
                        "--tag",
                        image,
                        "--file",
                        str(ctx / "finitechat/containers/agent/Dockerfile"),
                        str(ctx),
                    ],
                    timeout=3600,
                )
                step("docker.image_built", image=image)
        image_meta = docker_image_metadata(opts, image)
        report["runtime"]["image_id"] = image_meta.get("id")
        report["runtime"]["image_digest"] = (image_meta.get("repo_digests") or [None])[0]
        report["runtime"]["image_metadata"] = image_meta
        step("docker.image_metadata", image_id=image_meta.get("id"))

        docker_container_rm(opts, container)
        for volume in cleanup_volumes:
            docker_volume_rm(opts, volume)
            docker_volume_create(opts, volume)
        step("docker.volumes_created")

        init_restic_repository(
            opts,
            image=image,
            restic_volume=restic_volume,
            restic_password=restic_password,
            env=env,
        )
        step("restic.repository_initialized")

        container_id = start_agent_container(
            opts,
            image=image,
            container=container,
            agent_volume=agent_volume,
            restic_volume=restic_volume,
            restic_password=restic_password,
            server_url=opts.server_url,
            restore_latest=False,
            env=env,
        )
        report["docker"]["container_id_initial"] = container_id
        step("agent.container_started")
        wait_container_log(
            opts, container, "FINITE_AGENT_RUNTIME real_hermes_gateway=true", timeout=240
        )
        step("agent.runtime_log_ready")
        health = wait_container_http_json(
            opts, container, "/healthz", timeout=120, name="container health"
        )
        report["agent"].update(
            {"npub": health.get("npub"), "account_id": health.get("account_id"), "healthz": health}
        )
        step("agent.healthz", npub=health.get("npub"))

        invite = wait_fresh_invite(opts, container)
        report["invite"] = invite
        step("invite.ready", room_id=invite.get("room_id"), invite_id=invite.get("invite_id"))

        joined = admit_user(
            opts,
            image=image,
            user_volume=user_volume,
            server_url=opts.server_url,
            invite=invite,
            display_name="Remote Docker Canary User",
            env=env,
        )
        report["admission_probe"] = joined
        step("admission_probe.joined", room_id=joined.get("room_id"))

        model = run_model_smoke(
            opts,
            image=image,
            user_volume=user_volume,
            server_url=opts.server_url,
            room_id=str(joined["room_id"]),
            expected="remote docker canary ok",
            env=env,
        )
        report["model_smoke"] = model
        step("model_smoke.before_restore", reply_message_id=model.get("reply_message_id"))

        stop_for_backup(opts, container)
        step("agent.container_stopped_with_backup")

        docker_container_rm(opts, container)
        docker_volume_rm(opts, agent_volume)
        docker_volume_create(opts, agent_volume)
        step("agent.volume_wiped")

        restored_container_id = start_agent_container(
            opts,
            image=image,
            container=container,
            agent_volume=agent_volume,
            restic_volume=restic_volume,
            restic_password=restic_password,
            server_url=opts.server_url,
            restore_latest=True,
            env=env,
        )
        report["docker"]["container_id_restored"] = restored_container_id
        wait_container_log(opts, container, "FINITE_AGENT_RESTORE_COMPLETE", timeout=240)
        wait_container_log(
            opts, container, "FINITE_AGENT_RUNTIME real_hermes_gateway=true", timeout=240
        )
        restored_health = wait_container_http_json(
            opts, container, "/healthz", timeout=120, name="restored container health"
        )
        if restored_health.get("npub") != report["agent"].get("npub"):
            raise CanaryFailure(
                f"restored npub mismatch: expected {report['agent'].get('npub')}, got {restored_health.get('npub')}"
            )
        restored_invite = wait_fresh_invite(opts, container)
        if restored_invite.get("room_id") != invite.get("room_id"):
            raise CanaryFailure(
                f"restored invite room mismatch: expected {invite.get('room_id')}, got {restored_invite.get('room_id')}"
            )
        report["restore"] = {
            "status": "passed",
            "same_agent_npub": True,
            "same_room_id": True,
            "healthz": restored_health,
            "invite": restored_invite,
        }
        report["agent"]["restored"] = True
        report["invite"] = restored_invite
        step("agent.restored", npub=restored_health.get("npub"))

        restored_model = run_model_smoke(
            opts,
            image=image,
            user_volume=user_volume,
            server_url=opts.server_url,
            room_id=str(joined["room_id"]),
            expected="remote docker restore ok",
            env=env,
        )
        report["restore"]["existing_user_model_smoke"] = restored_model
        step(
            "model_smoke.after_restore_existing_user",
            reply_message_id=restored_model.get("reply_message_id"),
        )

        restored_join = admit_user(
            opts,
            image=image,
            user_volume=restored_user_volume,
            server_url=opts.server_url,
            invite=restored_invite,
            display_name="Remote Docker Restored Canary User",
            env=env,
        )
        if restored_join.get("room_id") != joined.get("room_id"):
            raise CanaryFailure(
                f"restored admission room mismatch: expected {joined.get('room_id')}, got {restored_join.get('room_id')}"
            )
        report["restore"]["admission_probe_after_restore"] = restored_join
        step("admission_probe.after_restore_joined", room_id=restored_join.get("room_id"))

        final_invite = wait_fresh_invite(opts, container)
        report["invite"] = final_invite
        report["status"] = "passed"
        report["stop_script"] = str(stop_script)
        write_stop_script(
            path=stop_script,
            docker_host=opts.docker_host,
            container=container,
            volumes=cleanup_volumes,
        )
        keep_remote = opts.keep_running
        step("canary.passed", kept_running=opts.keep_running)

        if not opts.keep_running:
            stop_for_backup(opts, container)
            docker_container_rm(opts, container)
            for volume in cleanup_volumes:
                docker_volume_rm(opts, volume)
            step("canary.cleaned_up")
        else:
            report["kept_running"] = True
            write_report()

        print(
            json.dumps(
                {
                    "status": "passed",
                    "layer": "remote-docker",
                    "docker_host": opts.docker_host,
                    "container": container,
                    "image": image,
                    "image_id": report["runtime"]["image_id"],
                    "agent_npub": report["agent"]["npub"],
                    "room_id": final_invite.get("room_id"),
                    "invite_id": final_invite.get("invite_id"),
                    "invite_url": final_invite.get("url"),
                    "report": str(report_path),
                    "stop_script": str(stop_script),
                    "kept_running": opts.keep_running,
                },
                indent=2,
            )
        )
        return 0
    except Exception as exc:
        report["status"] = "failed"
        report["failure"] = str(exc)
        try:
            logs = docker(opts, ["logs", container], check=False, timeout=60)
            log_path = logs_dir / "container.log"
            log_path.write_text((logs.stdout or "") + (logs.stderr or ""), encoding="utf-8")
            report["logs"] = {"container": str(log_path)}
        except Exception:
            pass
        write_report()
        if not opts.keep_failed and not keep_remote:
            docker_container_rm(opts, container)
        print(
            json.dumps(
                {"status": "failed", "report": str(report_path), "failure": str(exc)}, indent=2
            ),
            file=sys.stderr,
        )
        return 1


if __name__ == "__main__":
    sys.exit(main())

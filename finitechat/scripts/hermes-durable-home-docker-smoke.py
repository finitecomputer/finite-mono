#!/usr/bin/env python3
"""Docker smoke for the canonical Agent Runtime's durable /home/node contract."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import time
import urllib.parse
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_IMAGE = "finite-agent-durable-home-smoke"
DEFAULT_SERVER_URL = "https://chat.finite.computer"
MODEL_ENV_NAMES = (
    "OPENROUTER_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "FINITECHAT_HERMES_API_KEY",
    "FINITECHAT_HERMES_MODEL",
    "FINITECHAT_HERMES_PROVIDER",
    "FINITECHAT_HERMES_BASE_URL",
    "FINITECHAT_HERMES_API_MODE",
)
INFERENCE_CREDENTIAL_ENV_NAMES = (
    "OPENROUTER_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "FINITECHAT_HERMES_API_KEY",
)


class SmokeFailure(RuntimeError):
    pass


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", default=os.environ.get("FINITE_DOCKER_IMAGE", DEFAULT_IMAGE))
    parser.add_argument(
        "--server-url",
        default=os.environ.get("FINITECHAT_DURABLE_DOCKER_SERVER_URL", DEFAULT_SERVER_URL),
    )
    parser.add_argument(
        "--report",
        default=os.environ.get(
            "FINITECHAT_DURABLE_DOCKER_REPORT",
            "target/hermes-durable-home-docker-smoke/report.json",
        ),
    )
    parser.add_argument("--container", default="")
    parser.add_argument("--keep-running", action="store_true")
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
        raise SmokeFailure(
            f"command failed: {args!r}\nexit={proc.returncode}\n"
            f"stdout={proc.stdout[-3000:]}\nstderr={proc.stderr[-3000:]}"
        )
    return proc


def run_json(
    args: list[str], *, env: dict[str, str] | None = None, timeout: float = 60
) -> dict[str, Any]:
    proc = run(args, env=env, timeout=timeout)
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise SmokeFailure(
            f"command did not emit JSON: {args!r}\nstdout={proc.stdout[-3000:]}"
        ) from exc


def reject_loopback(server_url: str) -> None:
    parsed = urllib.parse.urlparse(server_url)
    host = (parsed.hostname or "").lower()
    if parsed.scheme not in {"http", "https"} or not host:
        raise SmokeFailure(f"server URL must be an http(s) origin, got {server_url!r}")
    if host in {"localhost", "127.0.0.1", "::1"} or host.startswith("127."):
        raise SmokeFailure(f"durable-home smoke must not use loopback server URL: {server_url!r}")


def timestamp_id() -> str:
    return time.strftime("run-%Y%m%d-%H%M%S")


def slug(value: str) -> str:
    return re.sub(r"[^a-z0-9_.-]+", "-", value.lower())


def docker_image_metadata(image: str) -> dict[str, Any]:
    inspected = run_json(["docker", "image", "inspect", image], timeout=60)
    image_meta = inspected[0]
    return {
        "id": image_meta["Id"],
        "repo_tags": image_meta.get("RepoTags") or [],
        "repo_digests": image_meta.get("RepoDigests") or [],
        "created": image_meta.get("Created"),
        "size_bytes": image_meta.get("Size"),
    }


def container_env_args(env: dict[str, str], names: tuple[str, ...]) -> list[str]:
    args: list[str] = []
    for name in names:
        if env.get(name):
            args.extend(["--env", name])
    return args


def docker_volume_rm(name: str) -> None:
    run(["docker", "volume", "rm", "-f", name], check=False, timeout=120)


def docker_container_rm(name: str) -> None:
    run(["docker", "rm", "-f", name], check=False, timeout=120)


def wait_container_log(container: str, marker: str, *, timeout: float) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        logs = run(["docker", "logs", container], check=False, timeout=30)
        if marker in (logs.stdout or ""):
            return
        ensure_container_running(container)
        time.sleep(1)
    logs = run(["docker", "logs", container], check=False, timeout=30)
    raise SmokeFailure(f"container never printed {marker!r}; logs:\n{(logs.stdout or '')[-4000:]}")


def ensure_container_running(container: str) -> None:
    inspected = run(["docker", "inspect", container], check=False, timeout=30)
    if inspected.returncode != 0:
        raise SmokeFailure(f"container {container} is not inspectable: {inspected.stderr[-1000:]}")
    state = json.loads(inspected.stdout)[0].get("State") or {}
    if not state.get("Running"):
        logs = run(["docker", "logs", container], check=False, timeout=30)
        raise SmokeFailure(
            f"container {container} is not running; state={state}; "
            f"logs:\n{(logs.stdout or '')[-4000:]}"
        )


def container_http_json(container: str, path: str) -> dict[str, Any]:
    code = (
        "import json, urllib.request; "
        f"u='http://127.0.0.1:8080{path}'; "
        "r=urllib.request.urlopen(u, timeout=5); "
        "print(json.dumps(json.load(r), sort_keys=True))"
    )
    return run_json(["docker", "exec", container, "python", "-c", code], timeout=20)


def wait_container_http_json(
    container: str, path: str, *, timeout: float, name: str
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    last_error = ""
    while time.monotonic() < deadline:
        try:
            payload = container_http_json(container, path)
            if payload.get("ready"):
                return payload
            last_error = json.dumps(payload, sort_keys=True)
        except Exception as exc:
            last_error = str(exc)
        ensure_container_running(container)
        time.sleep(1)
    raise SmokeFailure(f"{name} did not become ready: {last_error}")


def start_agent_container(
    *,
    image: str,
    container: str,
    home_volume: str,
    server_url: str,
    env: dict[str, str],
) -> str:
    docker_container_rm(container)
    command = [
        "docker",
        "run",
        "--name",
        container,
        "--detach",
        "--mount",
        f"type=volume,src={home_volume},dst=/home/node",
        "--env",
        f"FINITE_SERVER_URL={server_url}",
        "--env",
        "FINITECHAT_HOME=/home/node/.finitechat/agent",
        "--env",
        # Shared Finite identity on the same durable volume as the agent home
        # (overrides the image default of /data/agent).
        "FINITE_HOME=/home/node/.finitechat/agent",
        "--env",
        "HERMES_HOME=/home/node/.hermes",
        "--env",
        "FINITECHAT_WORKSPACE=/home/node/workspace",
        "--env",
        "FINITECHAT_HERMES_INBOUND_STREAM=1",
        "--env",
        "FINITECHAT_HERMES_PLUGIN_NAME=finitechat",
        "--env",
        "FINITECHAT_HERMES_ROOM_NAME=Finite Durable Docker Smoke",
        "--env",
        "FINITECHAT_HERMES_AGENT_DEVICE_ID=durable-docker",
        "--env",
        # The canonical image defaults to Finite Private inference. This
        # dispatch rung deliberately exercises the operator-supplied
        # OpenRouter credential instead, so select that profile explicitly.
        "FINITE_DEFAULT_INFERENCE_PROFILE=openrouter",
        *container_env_args(env, MODEL_ENV_NAMES),
        image,
    ]
    return run(command, env=env, timeout=300).stdout.strip()


def docker_agent_hermes(
    *,
    container: str,
    args: list[str],
    timeout: float = 180,
) -> dict[str, Any]:
    return run_json(
        [
            "docker",
            "exec",
            container,
            "finitechat",
            "hermes",
            "--home",
            "/home/node/.finitechat/agent",
            *args,
        ],
        timeout=timeout,
    )


def docker_agent_app_state(*, container: str, server_url: str) -> dict[str, Any]:
    # This is deliberately `app state` without `--start-runtime`: waiting for
    # the resident sidecar to persist the room must not itself claim the
    # Welcome and accidentally turn the canary into a polling loop.
    return run_json(
        [
            "docker",
            "exec",
            container,
            "finitechat",
            "app",
            "--data-dir",
            "/home/node/.finitechat/agent",
            "--server",
            server_url,
            "--device-id",
            "durable-docker",
            "state",
        ],
        timeout=30,
    )


def docker_user_app(
    *,
    image: str,
    user_volume: str,
    server_url: str,
    args: list[str],
    env: dict[str, str],
    timeout: float = 180,
) -> dict[str, Any]:
    return run_json(
        [
            "docker",
            "run",
            "--rm",
            "--mount",
            f"type=volume,src={user_volume},dst=/data/user",
            "--env",
            # App account keys come from the shared Finite Identity root, not
            # the app data-dir. Keep both on the durable probe volume so every
            # one-shot CLI invocation is the same user Device.
            "FINITE_HOME=/data/user",
            "--env",
            "FINITE_AGENT_SUPERVISE=0",
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


def create_welcome_room(
    *,
    image: str,
    user_volume: str,
    server_url: str,
    agent_account_id: str,
    env: dict[str, str],
) -> dict[str, Any]:
    # StartRuntime publishes the user's KeyPackages and performs an initial
    # sync. The user then creates the room and commits an MLS Add for the
    # already-running Agent Principal. The agent receives a Welcome; there is
    # no invite session, join URL, PIN, or second admission protocol.
    docker_user_app(
        image=image,
        user_volume=user_volume,
        server_url=server_url,
        args=["state", "--start-runtime"],
        env=env,
        timeout=120,
    )
    created = docker_user_app(
        image=image,
        user_volume=user_volume,
        server_url=server_url,
        args=["create-room", "--display-name", "Finite Durable Docker Smoke"],
        env=env,
        timeout=120,
    )
    room_id = created.get("selected_room_id")
    if not isinstance(room_id, str) or not room_id:
        raise SmokeFailure(f"room creation did not select a room: {created!r}")
    added = docker_user_app(
        image=image,
        user_volume=user_volume,
        server_url=server_url,
        args=[
            "add-member",
            "--room-id",
            room_id,
            "--account-id",
            agent_account_id,
            "--display-name",
            "Finite Agent",
        ],
        env=env,
        timeout=120,
    )
    if added.get("status") != "people added":
        raise SmokeFailure(f"MLS Add did not complete: {added!r}")
    return {"room_id": room_id, "add_status": added.get("status")}


def wait_agent_room_connected(
    container: str, room_id: str, server_url: str, *, timeout: float = 120
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    last: dict[str, Any] | None = None
    last_error = ""
    while time.monotonic() < deadline:
        try:
            state = docker_agent_app_state(container=container, server_url=server_url)
            room = next(
                (
                    candidate
                    for candidate in state.get("rooms") or []
                    if candidate.get("room_id") == room_id
                ),
                None,
            )
            last = room
            if room and room.get("state") == "Connected":
                # The non-mutating state read above is the actual wait gate.
                # Once connected, collect the richer member/paired evidence.
                status = docker_agent_hermes(
                    container=container,
                    args=["room-status", "--room-id", room_id, "--json"],
                    timeout=30,
                )
                last = status
                if (
                    status.get("room_id") == room_id
                    and status.get("connected") is True
                    and status.get("paired") is True
                ):
                    return status
                last_error = json.dumps(status, sort_keys=True)
                time.sleep(1)
                continue
            last_error = json.dumps(room or state, sort_keys=True)
        except Exception as exc:
            last_error = str(exc)
        ensure_container_running(container)
        time.sleep(1)
    raise SmokeFailure(
        f"agent did not claim the MLS Welcome for room {room_id!r}: {last_error or repr(last)}"
    )


def first_matching_mine_message_id(state: dict[str, Any], prompt: str) -> str | None:
    for message in state.get("messages") or []:
        if message.get("is_mine") and str(message.get("text") or "") == prompt:
            value = message.get("message_id")
            return str(value) if value else None
    return None


def run_model_smoke(
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
        image=image,
        user_volume=user_volume,
        server_url=server_url,
        args=["send", "--room-id", room_id, "--text", prompt],
        env=env,
        timeout=120,
    )
    deadline = time.monotonic() + 180
    last_state: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        state = docker_user_app(
            image=image,
            user_volume=user_volume,
            server_url=server_url,
            args=["state", "--start-runtime", "--wait-update-ms", "4000", "--room-id", room_id],
            env=env,
            timeout=60,
        )
        last_state = state
        for message in state.get("messages") or []:
            text = str(message.get("text") or "")
            if not message.get("is_mine") and expected.lower() in text.lower():
                return {
                    "status": "passed",
                    "elapsed_ms": int((time.monotonic() - started) * 1000),
                    "prompt_message_id": first_matching_mine_message_id(sent, prompt),
                    "reply_message_id": message.get("message_id"),
                    "reply_text": text,
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
    raise SmokeFailure(f"expected Hermes reply {expected!r} not found; recent messages={sample!r}")


def write_stop_script(path: Path, *, container: str, volumes: list[str]) -> None:
    volume_args = " ".join(volumes)
    path.write_text(
        "#!/usr/bin/env bash\n"
        "set -euo pipefail\n"
        f"docker rm -f {container!r} >/dev/null 2>&1 || true\n"
        f"docker volume rm -f {volume_args} >/dev/null 2>&1 || true\n",
        encoding="utf-8",
    )
    path.chmod(0o755)


def main() -> int:
    args = parse_args()
    reject_loopback(args.server_url)
    run_id = timestamp_id()
    name = args.container or f"finite-agent-durable-home-smoke-{slug(run_id)}"
    home_volume = f"{name}-home"
    user_volume = f"{name}-user"
    report_path = REPO_ROOT / args.report
    report_path.parent.mkdir(parents=True, exist_ok=True)
    stop_script = report_path.parent / "stop.sh"
    env = os.environ.copy()
    started = time.monotonic()
    report: dict[str, Any] = {
        "status": "running",
        "name": "docker_durable_home_welcome_restart",
        "layer": "durable-home-docker",
        "run_id": run_id,
        "server": {"url": args.server_url, "phone_reachable": True},
        "facts": {
            "image": args.image,
            "image_id": None,
            "image_metadata": None,
            "state_volume": home_volume,
            "state_mount": "/home/node",
            "finitechat_home": "/home/node/.finitechat/agent",
            "hermes_home": "/home/node/.hermes",
            "workspace": "/home/node/workspace",
            "restic_backend": None,
            "real_gateway_runtime": False,
            "welcome_admission_before_restart": False,
            "welcome_admission_after_restart": False,
            "same_agent_npub_after_restart": False,
            "same_room_id_after_restart": False,
            "model_env_present": {name: bool(env.get(name)) for name in MODEL_ENV_NAMES},
            "inference_credential_present": any(
                env.get(name) for name in INFERENCE_CREDENTIAL_ENV_NAMES
            ),
        },
        "steps": [],
    }

    def write_report() -> None:
        report_path.write_text(
            json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )

    def step(step_name: str, **facts: Any) -> None:
        report["steps"].append(
            {"name": step_name, "elapsed_ms": int((time.monotonic() - started) * 1000), **facts}
        )
        write_report()

    write_report()
    cleanup_volumes = [home_volume, user_volume]
    keep_running = False
    try:
        if not report["facts"]["inference_credential_present"]:
            raise SmokeFailure(
                "durable-home chat smoke requires a real inference credential; "
                f"set one of {INFERENCE_CREDENTIAL_ENV_NAMES!r}"
            )
        image_meta = docker_image_metadata(args.image)
        report["facts"]["image_id"] = image_meta["id"]
        report["facts"]["image_metadata"] = image_meta
        step("docker.image_metadata", image_id=image_meta["id"])

        docker_container_rm(name)
        for volume in cleanup_volumes:
            docker_volume_rm(volume)
            run(["docker", "volume", "create", volume], timeout=60)
        step("docker.volumes_created")

        container_id = start_agent_container(
            image=args.image,
            container=name,
            home_volume=home_volume,
            server_url=args.server_url,
            env=env,
        )
        report["facts"]["container_id_initial"] = container_id
        step("agent.container_started")
        wait_container_log(name, "FINITE_AGENT_RUNTIME real_hermes_gateway=true", timeout=180)
        report["facts"]["real_gateway_runtime"] = True
        health = wait_container_http_json(name, "/healthz", timeout=120, name="container health")
        agent_account_id = health.get("account_id")
        if not isinstance(agent_account_id, str) or not agent_account_id:
            raise SmokeFailure(f"runtime health omitted the Agent Principal: {health!r}")
        report["facts"]["agent_npub"] = health.get("npub")
        report["facts"]["agent_account_id"] = agent_account_id
        step("agent.ready", npub=health.get("npub"))

        welcome = create_welcome_room(
            image=args.image,
            user_volume=user_volume,
            server_url=args.server_url,
            agent_account_id=agent_account_id,
            env=env,
        )
        room_id = str(welcome["room_id"])
        report["facts"]["room_id"] = room_id
        room_status = wait_agent_room_connected(name, room_id, args.server_url)
        report["facts"]["welcome_admission_before_restart"] = True
        report["facts"]["room_status_before_restart"] = room_status
        step("welcome.before_restart", room_id=room_id)
        before_model = run_model_smoke(
            image=args.image,
            user_volume=user_volume,
            server_url=args.server_url,
            room_id=room_id,
            expected="durable docker before restart ok",
            env=env,
        )
        report["facts"]["model_smoke_before_restart"] = before_model
        step("model.before_restart", reply_message_id=before_model.get("reply_message_id"))

        run(["docker", "restart", "--time", "60", name], timeout=120)
        step("agent.container_restarted")
        restarted_health = wait_container_http_json(
            name, "/healthz", timeout=120, name="restarted health"
        )
        restarted_room_status = wait_agent_room_connected(name, room_id, args.server_url)
        report["facts"]["agent_npub_after_restart"] = restarted_health.get("npub")
        report["facts"]["same_agent_npub_after_restart"] = restarted_health.get("npub") == report[
            "facts"
        ].get("agent_npub")
        report["facts"]["same_room_id_after_restart"] = (
            restarted_room_status.get("room_id") == room_id
        )
        report["facts"]["room_status_after_restart"] = restarted_room_status
        if not report["facts"]["same_agent_npub_after_restart"]:
            raise SmokeFailure("agent npub changed after Docker restart")
        if not report["facts"]["same_room_id_after_restart"]:
            raise SmokeFailure("MLS room changed after Docker restart")
        step("agent.ready_after_restart", npub=restarted_health.get("npub"))

        after_model = run_model_smoke(
            image=args.image,
            user_volume=user_volume,
            server_url=args.server_url,
            room_id=room_id,
            expected="durable docker after restart ok",
            env=env,
        )
        report["facts"]["model_smoke_after_restart"] = after_model
        report["facts"]["welcome_admission_after_restart"] = True
        step("model.after_restart", reply_message_id=after_model.get("reply_message_id"))

        report["status"] = "passed"
        write_stop_script(stop_script, container=name, volumes=cleanup_volumes)
        report["stop_script"] = str(stop_script)
        keep_running = args.keep_running
        if keep_running:
            report["kept_running"] = True
        write_report()
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0
    except Exception as exc:
        report["status"] = "failed"
        report["failure"] = str(exc)
        write_report()
        raise
    finally:
        if not keep_running:
            docker_container_rm(name)
            for volume in cleanup_volumes:
                docker_volume_rm(volume)


if __name__ == "__main__":
    raise SystemExit(main())

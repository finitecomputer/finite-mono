#!/usr/bin/env python3
"""Prove fresh chats survive real-Hermes interruption and empty-target restore."""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import re
import shutil
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
DURABLE_SMOKE_PATH = REPO_ROOT / "scripts" / "hermes-durable-home-docker-smoke.py"
DEFAULT_IMAGE = "finite-agent-chat-interruption-smoke"
EXPECTED_HERMES_VERSION = "0.18.2"
DOCKER_HOST_ARGS = ["--add-host", "host.docker.internal:host-gateway"]

spec = importlib.util.spec_from_file_location("hermes_durable_smoke", DURABLE_SMOKE_PATH)
assert spec is not None and spec.loader is not None
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


class SmokeFailure(RuntimeError):
    pass


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", default=os.environ.get("FINITE_DOCKER_IMAGE", DEFAULT_IMAGE))
    parser.add_argument(
        "--server-bin",
        default=os.environ.get(
            "FINITECHAT_SERVER_BIN", str(REPO_ROOT.parent / "target/debug/finitechat-server")
        ),
    )
    parser.add_argument(
        "--report",
        default=os.environ.get(
            "FINITECHAT_INTERRUPTION_REPORT",
            "target/hermes-chat-interruption-docker-smoke/report.json",
        ),
    )
    parser.add_argument("--container", default="")
    parser.add_argument("--keep-state", action="store_true")
    return parser.parse_args()


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def wait_http(url: str, *, timeout: float, name: str) -> None:
    deadline = time.monotonic() + timeout
    last_error = ""
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=1) as response:
                if 200 <= response.status < 300:
                    return
        except Exception as exc:
            last_error = str(exc)
        time.sleep(0.1)
    raise SmokeFailure(f"{name} did not become ready: {last_error}")


def all_text(value: Any) -> str:
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return "\n".join(all_text(item) for item in value)
    if isinstance(value, dict):
        return "\n".join(all_text(item) for item in value.values())
    return ""


class FakeModelState:
    def __init__(self) -> None:
        self.condition = threading.Condition()
        self.seen: set[str] = set()
        self.released: set[str] = set()
        self.requests: list[dict[str, Any]] = []

    def observe(self, payload: dict[str, Any]) -> str | None:
        text = all_text(payload.get("messages") or payload.get("input") or payload)
        matches = re.findall(r"FINITE_INTERRUPT_STALL:([a-z0-9-]+)", text)
        stall = matches[-1] if matches else None
        with self.condition:
            self.requests.append(
                {
                    "stream": payload.get("stream"),
                    "stall": stall,
                }
            )
            return stall

    def mark_seen(self, name: str) -> None:
        with self.condition:
            self.seen.add(name)
            self.condition.notify_all()

    def wait_seen(self, name: str, *, timeout: float = 60) -> None:
        deadline = time.monotonic() + timeout
        with self.condition:
            while name not in self.seen:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise SmokeFailure(f"fake model never observed stalled turn {name!r}")
                self.condition.wait(remaining)

    def wait_released(self, name: str, *, timeout: float = 120) -> None:
        deadline = time.monotonic() + timeout
        with self.condition:
            while name not in self.released:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    return
                self.condition.wait(remaining)

    def release(self, name: str) -> None:
        with self.condition:
            self.released.add(name)
            self.condition.notify_all()


def expected_reply(payload: dict[str, Any]) -> str:
    text = all_text(payload.get("messages") or payload.get("input") or payload)
    matches = re.findall(r"Reply with exactly:\s*([^\n]+)", text)
    if not matches:
        return "finite deterministic reply"
    return matches[-1].strip().strip('"')


def parse_hermes_version(output: str) -> str:
    match = re.search(r"Hermes Agent v([^\s]+)", output)
    if match is None:
        raise SmokeFailure(f"could not parse Hermes version from {output!r}")
    return match.group(1)


def start_fake_model(state: FakeModelState, port: int) -> ThreadingHTTPServer:
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, _format: str, *_args: object) -> None:
            return

        def do_GET(self) -> None:
            body = json.dumps({"object": "list", "data": []}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_POST(self) -> None:
            length = int(self.headers.get("Content-Length") or "0")
            payload = json.loads(self.rfile.read(length) or b"{}")
            stall = state.observe(payload)
            reply = expected_reply(payload)
            if payload.get("stream") is True:
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Cache-Control", "no-cache")
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.flush()
                if stall:
                    state.mark_seen(stall)
                    state.wait_released(stall)
                chunk = {
                    "id": "chatcmpl-finite-interruption-smoke",
                    "object": "chat.completion.chunk",
                    "created": 0,
                    "model": "finite-deterministic",
                    "choices": [
                        {
                            "index": 0,
                            "delta": {"role": "assistant", "content": reply},
                            "finish_reason": "stop",
                        }
                    ],
                }
                try:
                    self.wfile.write(f"data: {json.dumps(chunk)}\n\ndata: [DONE]\n\n".encode())
                    self.wfile.flush()
                except (BrokenPipeError, ConnectionResetError):
                    pass
                self.close_connection = True
                return

            body = json.dumps(
                {
                    "id": "chatcmpl-finite-interruption-smoke",
                    "object": "chat.completion",
                    "created": 0,
                    "model": "finite-deterministic",
                    "choices": [
                        {
                            "index": 0,
                            "message": {"role": "assistant", "content": reply},
                            "finish_reason": "stop",
                        }
                    ],
                }
            ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = ThreadingHTTPServer(("0.0.0.0", port), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def terminate(proc: subprocess.Popen[str] | None) -> None:
    if proc is None or proc.poll() is not None:
        return
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)


def user_app(
    *, image: str, volume: str, server_url: str, args: list[str], env: dict[str, str]
) -> dict[str, Any]:
    return smoke.docker_user_app(
        image=image,
        user_volume=volume,
        server_url=server_url,
        args=args,
        env=env,
        timeout=60,
        docker_extra_args=DOCKER_HOST_ARGS,
    )


def wait_reply(
    *,
    image: str,
    volume: str,
    server_url: str,
    room_id: str,
    expected: str,
    env: dict[str, str],
) -> dict[str, Any]:
    prompt = f"Reply with exactly: {expected}"
    sent = user_app(
        image=image,
        volume=volume,
        server_url=server_url,
        args=["send", "--room-id", room_id, "--text", prompt],
        env=env,
    )
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        state = user_app(
            image=image,
            volume=volume,
            server_url=server_url,
            args=["state", "--start-runtime", "--wait-update-ms", "2000", "--room-id", room_id],
            env=env,
        )
        for message in state.get("messages") or []:
            if not message.get("is_mine") and expected in str(message.get("text") or ""):
                return {
                    "prompt_message_id": smoke.first_matching_mine_message_id(sent, prompt),
                    "reply_message_id": message.get("message_id"),
                    "reply_text": message.get("text"),
                }
    raise SmokeFailure(f"fresh reply {expected!r} did not arrive")


def volume_archive(*, image: str, source_volume: str, snapshot_volume: str) -> str:
    smoke.run(["docker", "volume", "create", snapshot_volume])
    smoke.run(
        [
            "docker",
            "run",
            "--rm",
            "--entrypoint",
            "/bin/sh",
            "--mount",
            f"type=volume,src={source_volume},dst=/source,readonly",
            "--mount",
            f"type=volume,src={snapshot_volume},dst=/snapshot",
            image,
            "-c",
            "cd /source && tar -cpf /snapshot/data.tar . && sha256sum /snapshot/data.tar",
        ],
        timeout=300,
    )
    result = smoke.run(
        [
            "docker",
            "run",
            "--rm",
            "--entrypoint",
            "/bin/sh",
            "--mount",
            f"type=volume,src={snapshot_volume},dst=/snapshot,readonly",
            image,
            "-c",
            "sha256sum /snapshot/data.tar",
        ],
        timeout=120,
    )
    return result.stdout.split()[0]


def restore_volume(*, image: str, target_volume: str, snapshot_volume: str) -> None:
    smoke.docker_volume_rm(target_volume)
    smoke.run(["docker", "volume", "create", target_volume])
    smoke.run(
        [
            "docker",
            "run",
            "--rm",
            "--entrypoint",
            "/bin/sh",
            "--mount",
            f"type=volume,src={snapshot_volume},dst=/snapshot,readonly",
            "--mount",
            f"type=volume,src={target_volume},dst=/target",
            image,
            "-c",
            'test -z "$(find /target -mindepth 1 -print -quit)" && cd /target && tar -xpf /snapshot/data.tar',
        ],
        timeout=300,
    )


def main() -> int:
    args = parse_args()
    image = args.image
    server_bin = Path(args.server_bin).resolve()
    if not server_bin.is_file():
        raise SmokeFailure(f"finitechat-server binary not found: {server_bin}")

    run_id = time.strftime("run-%Y%m%d-%H%M%S")
    name = args.container or f"finite-chat-interruption-{run_id.lower()}"
    home_volume = f"{name}-home"
    user_volume = f"{name}-user"
    snapshot_volume = f"{name}-snapshot"
    cleanup_volumes = [home_volume, user_volume, snapshot_volume]
    report_path = REPO_ROOT / args.report
    report_path.parent.mkdir(parents=True, exist_ok=True)
    state_dir = Path(tempfile.mkdtemp(prefix="finite-chat-interruption-"))
    server_port = free_port()
    model_port = free_port()
    server_url = f"http://host.docker.internal:{server_port}"
    model_url = f"http://host.docker.internal:{model_port}/v1"
    model_state = FakeModelState()
    model_server = start_fake_model(model_state, model_port)
    server_log = state_dir / "finitechat-server.log"
    server: subprocess.Popen[str] | None = None
    report: dict[str, Any] = {
        "status": "running",
        "name": "hermes-chat-interruption-docker-smoke",
        "image": image,
        "cases": [],
        "coverage": {
            "real_hermes": None,
            "real_finitechat_bridge_and_crypto": True,
            "provider": "deterministic local OpenAI-compatible SSE",
            "interruption_boundary": "SSE headers flushed before first data frame",
            "local_snapshot_fence": "Docker container removed and volume unmounted before tar",
            "production_kata_task_and_stable_manifest_gate": False,
        },
    }

    report["image_id"] = smoke.run(
        ["docker", "image", "inspect", "--format", "{{.Id}}", image], timeout=30
    ).stdout.strip()

    def write_report() -> None:
        report_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")

    env = os.environ.copy()
    env.update(
        {
            "FINITECHAT_HERMES_API_KEY": "local-smoke-key-not-a-secret",
            "OPENROUTER_API_KEY": "local-smoke-key-not-a-secret",
            "OPENAI_API_KEY": "local-smoke-key-not-a-secret",
            "FINITECHAT_HERMES_MODEL": "finite-deterministic",
            "FINITECHAT_HERMES_PROVIDER": "custom",
            "FINITECHAT_HERMES_BASE_URL": model_url,
            "FINITECHAT_HERMES_API_MODE": "chat_completions",
        }
    )

    def start_agent() -> dict[str, Any]:
        smoke.start_agent_container(
            image=image,
            container=name,
            home_volume=home_volume,
            server_url=server_url,
            env=env,
            docker_extra_args=DOCKER_HOST_ARGS,
        )
        smoke.wait_container_log(name, "FINITE_AGENT_RUNTIME real_hermes_gateway=true", timeout=180)
        health = smoke.wait_container_http_json(name, "/healthz", timeout=120, name="agent")
        version_result = smoke.run(
            ["docker", "exec", name, "/runtime/hermes-venv/bin/hermes", "--version"],
            timeout=30,
        )
        version = parse_hermes_version(version_result.stdout)
        if version != EXPECTED_HERMES_VERSION:
            raise SmokeFailure(
                f"expected Hermes {EXPECTED_HERMES_VERSION}, canonical image has {version}"
            )
        report["coverage"]["real_hermes"] = version
        return health

    def interrupt(case_name: str, *, kill: bool, restore: bool) -> None:
        prompt = f"FINITE_INTERRUPT_STALL:{case_name} keep this turn open"
        user_app(
            image=image,
            volume=user_volume,
            server_url=server_url,
            args=["send", "--room-id", room_id, "--text", prompt],
            env=env,
        )
        model_state.wait_seen(case_name)
        case: dict[str, Any] = {
            "name": case_name,
            "signal": "SIGKILL" if kill else "SIGTERM",
            "empty_target_restore": restore,
            "provider_stream_in_flight": True,
        }
        if kill:
            smoke.run(["docker", "kill", "--signal", "KILL", name], timeout=30)
        else:
            smoke.run(["docker", "stop", "--time", "15", name], timeout=30)
        exit_code = int(
            smoke.run(
                ["docker", "inspect", "--format", "{{.State.ExitCode}}", name], timeout=30
            ).stdout.strip()
        )
        if kill and exit_code != 137:
            raise SmokeFailure(f"{case_name} exited {exit_code}, expected SIGKILL exit 137")
        if not kill and exit_code == 137:
            raise SmokeFailure(f"{case_name} escalated to SIGKILL instead of stopping gracefully")
        case["container_exit_code"] = exit_code
        model_state.release(case_name)
        smoke.docker_container_rm(name)
        if restore:
            case["archive_sha256"] = volume_archive(
                image=image,
                source_volume=home_volume,
                snapshot_volume=snapshot_volume,
            )
            restore_volume(
                image=image,
                target_volume=home_volume,
                snapshot_volume=snapshot_volume,
            )
        health = start_agent()
        status = smoke.wait_agent_room_connected(name, room_id, server_url)
        if health.get("npub") != agent_npub or status.get("room_id") != room_id:
            raise SmokeFailure(f"{case_name} changed the Agent identity or room")
        case["fresh_turns"] = [
            wait_reply(
                image=image,
                volume=user_volume,
                server_url=server_url,
                room_id=room_id,
                expected=f"{case_name} fresh chat one ok",
                env=env,
            ),
            wait_reply(
                image=image,
                volume=user_volume,
                server_url=server_url,
                room_id=room_id,
                expected=f"{case_name} fresh chat two ok",
                env=env,
            ),
        ]
        case["status"] = "passed"
        report["cases"].append(case)
        write_report()

    try:
        server = subprocess.Popen(
            [
                str(server_bin),
                "serve",
                f"0.0.0.0:{server_port}",
                "--sqlite",
                str(state_dir / "server.sqlite3"),
            ],
            cwd=REPO_ROOT,
            stdout=server_log.open("w", encoding="utf-8"),
            stderr=subprocess.STDOUT,
            text=True,
        )
        wait_http(f"http://127.0.0.1:{server_port}/health", timeout=10, name="finitechat-server")
        smoke.docker_container_rm(name)
        for volume in cleanup_volumes:
            smoke.docker_volume_rm(volume)
        smoke.run(["docker", "volume", "create", home_volume])
        smoke.run(["docker", "volume", "create", user_volume])

        health = start_agent()
        agent_npub = str(health.get("npub") or "")
        account_id = health.get("account_id")
        if not agent_npub or not isinstance(account_id, str):
            raise SmokeFailure(f"agent health omitted identity: {health}")
        welcome = smoke.create_welcome_room(
            image=image,
            user_volume=user_volume,
            server_url=server_url,
            agent_account_id=account_id,
            env=env,
            docker_extra_args=DOCKER_HOST_ARGS,
        )
        room_id = str(welcome["room_id"])
        smoke.wait_agent_room_connected(name, room_id, server_url)
        report["agent_npub"] = agent_npub
        report["room_id"] = room_id
        write_report()

        interrupt("graceful-stop", kill=False, restore=False)
        interrupt("sigkill", kill=True, restore=False)
        interrupt("empty-target-restore", kill=False, restore=True)
        stalled_requests = [request for request in model_state.requests if request.get("stall")]
        if not stalled_requests or not all(
            request.get("stream") is True for request in stalled_requests
        ):
            raise SmokeFailure("an interrupted Hermes turn did not use the streaming provider path")
        report["provider_request_count"] = len(model_state.requests)
        report["status"] = "passed"
        write_report()
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0
    except Exception as exc:
        report["status"] = "failed"
        report["failure"] = str(exc)
        report["finitechat_server_log"] = (
            server_log.read_text(errors="replace")[-4000:] if server_log.exists() else ""
        )
        write_report()
        raise
    finally:
        model_server.shutdown()
        model_server.server_close()
        terminate(server)
        if not args.keep_state:
            smoke.docker_container_rm(name)
            for volume in cleanup_volumes:
                smoke.docker_volume_rm(volume)
            shutil.rmtree(state_dir, ignore_errors=True)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (SmokeFailure, smoke.SmokeFailure) as error:
        print(f"error: {error}")
        raise SystemExit(1) from error

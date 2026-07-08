#!/usr/bin/env python3
"""Smoke the real Hermes gateway admission path.

This is intentionally stronger than the adapter echo tests:

  finitechat-server
  -> finitechat hermes serve
  -> hermes-agent `gateway run --replace`
  -> throwaway finitechat client joins via invite URL

The pass condition is that Hermes admits the pending join through the
finitechat plugin. No direct adapter import or test echo handler is used.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_REPORT = REPO_ROOT / "target/hermes-real-gateway-admission-smoke/report.json"


class SmokeFailure(RuntimeError):
    pass


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--report",
        default=os.environ.get("FINITECHAT_HERMES_REAL_GATEWAY_REPORT", str(DEFAULT_REPORT)),
    )
    parser.add_argument(
        "--keep-state",
        action="store_true",
        help="Keep the temporary state directory after the run.",
    )
    parser.add_argument(
        "--timeout-ms",
        type=int,
        default=int(os.environ.get("FINITECHAT_HERMES_REAL_GATEWAY_TIMEOUT_MS", "30000")),
    )
    parser.add_argument(
        "--hermes-package",
        default=os.environ.get("FINITECHAT_HERMES_PACKAGE", "hermes-agent==0.18.0"),
    )
    parser.add_argument(
        "--skip-build", action="store_true", help="Use existing target/debug binaries."
    )
    parser.add_argument(
        "--hold-seconds",
        type=int,
        default=0,
        help="Keep the local server, sidecar, and gateway alive after the proof join.",
    )
    return parser.parse_args()


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def tail(path: Path, limit: int = 8000) -> str:
    try:
        data = path.read_bytes()
    except FileNotFoundError:
        return ""
    return data[-limit:].decode("utf-8", errors="replace")


def run(
    args: list[str],
    *,
    env: dict[str, str] | None = None,
    timeout: float = 60,
    cwd: Path = REPO_ROOT,
) -> subprocess.CompletedProcess[str]:
    proc = subprocess.run(
        args,
        cwd=cwd,
        env=env,
        text=True,
        capture_output=True,
        timeout=timeout,
    )
    if proc.returncode != 0:
        raise SmokeFailure(
            "command failed: "
            + repr(args)
            + f"\nexit={proc.returncode}\nstdout={proc.stdout[-2000:]}\nstderr={proc.stderr[-2000:]}"
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
        raise SmokeFailure(
            f"command did not emit JSON: {args!r}\nstdout={proc.stdout[-2000:]}"
        ) from exc


def wait_http_ok(url: str, *, timeout: float, name: str) -> None:
    deadline = time.monotonic() + timeout
    last_error = ""
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=1.0) as response:
                if 200 <= response.status < 300:
                    return
                last_error = f"HTTP {response.status}"
        except Exception as exc:
            last_error = str(exc)
        time.sleep(0.1)
    raise SmokeFailure(f"{name} did not become ready at {url}: {last_error}")


def terminate(child: subprocess.Popen[str] | None) -> None:
    if child is None or child.poll() is not None:
        return
    child.terminate()
    try:
        child.wait(timeout=5)
    except subprocess.TimeoutExpired:
        child.kill()
        child.wait(timeout=5)


def write_hermes_config(
    path: Path, *, agent_home: Path, service_url: str, service_addr: str
) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        f"""model:
  default: anthropic/claude-sonnet-4.6
  provider: openrouter
  base_url: https://openrouter.ai/api/v1
  api_mode: chat_completions
plugins:
  enabled:
    - finitechat
gateway:
  platforms:
    finitechat:
      enabled: true
      extra:
        home: "{agent_home}"
        inbound_stream: true
        service_url: "{service_url}"
        service_addr: "{service_addr}"
        poll_timeout_secs: 1
        poll_limit: 10
terminal:
  backend: local
  cwd: "{REPO_ROOT}"
  persistent_shell: true
approvals:
  mode: off
display:
  streaming: false
security:
  redact_secrets: true
_config_version: 10
""",
        encoding="utf-8",
    )


def main() -> int:
    args = parse_args()
    report_path = Path(args.report)
    report_path.parent.mkdir(parents=True, exist_ok=True)

    finitechat_bin = Path(os.environ.get("FINITECHAT_BIN", REPO_ROOT / "target/debug/finitechat"))
    server_bin = Path(
        os.environ.get("FINITECHAT_SERVER_BIN", REPO_ROOT / "target/debug/finitechat-server")
    )
    uvx_bin = shutil.which("uvx")
    if uvx_bin is None:
        raise SmokeFailure("uvx is required to run hermes-agent 0.17")

    if not args.skip_build:
        run(
            ["cargo", "build", "-q", "-p", "finitechat-cli", "-p", "finitechat-server"], timeout=240
        )
    if not finitechat_bin.exists():
        raise SmokeFailure(f"finitechat binary not found: {finitechat_bin}")
    if not server_bin.exists():
        raise SmokeFailure(f"finitechat-server binary not found: {server_bin}")

    started = time.monotonic()
    state_ctx = tempfile.TemporaryDirectory(prefix="finitechat-hermes-real-gateway-")
    state_root = Path(state_ctx.name)
    agent_home = state_root / "agent-home"
    user_home = state_root / "user-home"
    user_device_id = "electron-Pauls-MacBook-Pro-2.local"
    hermes_home = state_root / "hermes-home"
    server_port = free_port()
    service_port = free_port()
    server_addr = f"127.0.0.1:{server_port}"
    server_url = f"http://{server_addr}"
    service_addr = f"127.0.0.1:{service_port}"
    service_url = f"http://{service_addr}"
    logs_dir = state_root / "logs"
    logs_dir.mkdir(parents=True, exist_ok=True)

    server: subprocess.Popen[str] | None = None
    sidecar: subprocess.Popen[str] | None = None
    gateway: subprocess.Popen[str] | None = None
    report: dict[str, Any] = {
        "status": "running",
        "name": "hermes-real-gateway-admission-smoke",
        "state_root": str(state_root),
        "server_url": server_url,
        "sidecar_url": service_url,
        "finitechat_bin": str(finitechat_bin),
        "server_bin": str(server_bin),
        "hermes_package": args.hermes_package,
        "timeout_ms": args.timeout_ms,
        "steps": [],
    }
    base_env = os.environ.copy()
    agent_env = {**base_env, "FINITE_HOME": str(agent_home), "FINITECHAT_HOME": str(agent_home)}
    user_env = {**base_env, "FINITE_HOME": str(user_home), "FINITECHAT_HOME": str(user_home)}

    def step(name: str, **facts: Any) -> None:
        report["steps"].append(
            {"name": name, "elapsed_ms": int((time.monotonic() - started) * 1000), **facts}
        )

    try:
        server_log = logs_dir / "finitechat-server.log"
        server = subprocess.Popen(
            [str(server_bin), "serve", server_addr, "--sqlite", str(state_root / "server.sqlite3")],
            cwd=REPO_ROOT,
            stdout=server_log.open("w", encoding="utf-8"),
            stderr=subprocess.STDOUT,
            text=True,
        )
        wait_http_ok(f"{server_url}/health", timeout=10, name="finitechat-server")
        step("server.ready")

        agent_init = run_json(
            [
                str(finitechat_bin),
                "hermes",
                "--agent-home",
                str(agent_home),
                "init",
                "--server",
                server_url,
            ],
            env=agent_env,
            timeout=30,
        )
        user_init = run_json(
            [
                str(finitechat_bin),
                "hermes",
                "--agent-home",
                str(user_home),
                "init",
                "--server",
                server_url,
                "--device-id",
                user_device_id,
            ],
            env=user_env,
            timeout=30,
        )
        report["agent"] = {
            "account_id": agent_init.get("account_id"),
            "npub": agent_init.get("npub"),
        }
        report["user"] = {
            "account_id": user_init.get("account_id"),
            "npub": user_init.get("npub"),
            "device_id": user_device_id,
        }
        step("homes.initialized")

        install = run_json(
            [
                str(finitechat_bin),
                "hermes",
                "--agent-home",
                str(agent_home),
                "install",
                "--plugins-dir",
                str(hermes_home / "plugins"),
                "--plugin-name",
                "finitechat",
                "--finitechat-bin",
                str(finitechat_bin),
                "--service-url",
                service_url,
                "--force",
                "--json",
            ],
            env=agent_env,
            timeout=30,
        )
        report["plugin_install"] = install
        write_hermes_config(
            hermes_home / "config.yaml",
            agent_home=agent_home,
            service_url=service_url,
            service_addr=service_addr,
        )
        step("plugin.installed", plugin_dir=install.get("plugin_dir"))

        invite = run_json(
            [
                str(finitechat_bin),
                "hermes",
                "--agent-home",
                str(agent_home),
                "invite",
                "--room-name",
                "Hermes Gateway Admission Smoke",
                "--json",
            ],
            env=agent_env,
            timeout=30,
        )
        report["invite"] = {
            "room_id": invite.get("room_id"),
            "invite_id": invite.get("invite_id"),
            "npub": invite.get("npub"),
        }
        step("invite.created", room_id=invite.get("room_id"))

        env = agent_env.copy()
        env.update(
            {
                "HERMES_HOME": str(hermes_home),
                "FINITECHAT_HOME": str(agent_home),
                "FINITE_HOME": str(agent_home),
                "FINITE_AGENT_HOME": str(agent_home),
                "FINITECHAT_BIN": str(finitechat_bin),
                "FINITECHAT_HERMES_INBOUND_STREAM": "1",
                "FINITECHAT_HERMES_SERVICE_ADDR": service_addr,
                "FINITECHAT_HERMES_SERVICE_URL": service_url,
                "FINITECHAT_ALLOW_ALL_USERS": "true",
                "FINITE_ALLOW_ALL_USERS": "true",
                "GATEWAY_ALLOW_ALL_USERS": "true",
                "FINITE_AGENT_ID": "agent_hermes_real_gateway_smoke",
                "FINITE_AGENT_NAME": "Hermes Real Gateway Smoke",
            }
        )

        sidecar_log = logs_dir / "finitechat-hermes-serve.log"
        ready_file = state_root / "finitechat-hermes-ready.json"
        sidecar = subprocess.Popen(
            [
                str(finitechat_bin),
                "hermes",
                "--agent-home",
                str(agent_home),
                "serve",
                "--addr",
                service_addr,
                "--ready-file",
                str(ready_file),
                "--json",
            ],
            cwd=REPO_ROOT,
            env=env,
            stdout=sidecar_log.open("w", encoding="utf-8"),
            stderr=subprocess.STDOUT,
            text=True,
        )
        wait_http_ok(f"{service_url}/readyz", timeout=10, name="finitechat hermes serve")
        step("sidecar.ready")

        gateway_log = logs_dir / "hermes-gateway.log"
        gateway = subprocess.Popen(
            [
                uvx_bin,
                "--no-config",
                "--from",
                args.hermes_package,
                "hermes",
                "gateway",
                "run",
                "--replace",
            ],
            cwd=REPO_ROOT,
            env=env,
            stdout=gateway_log.open("w", encoding="utf-8"),
            stderr=subprocess.STDOUT,
            text=True,
            start_new_session=True,
        )
        time.sleep(2.0)
        if gateway.poll() is not None:
            raise SmokeFailure(f"hermes gateway exited early with {gateway.returncode}")
        step("gateway.started")

        join = run_json(
            [
                str(finitechat_bin),
                "hermes",
                "--agent-home",
                str(user_home),
                "join",
                "--url",
                str(invite["url"]),
                "--timeout-ms",
                str(args.timeout_ms),
            ],
            env=user_env,
            timeout=max(30, args.timeout_ms / 1000 + 20),
        )
        report["join"] = join
        step("join.completed", state=join.get("state"), room_id=join.get("room_id"))
        if join.get("state") != "joined":
            raise SmokeFailure(f"join did not complete: {join}")

        report["status"] = "passed"
        report["elapsed_ms"] = int((time.monotonic() - started) * 1000)
        if args.hold_seconds > 0:
            print(
                "HERMES_REAL_GATEWAY_HOLD "
                + json.dumps(
                    {
                        "state_root": str(state_root),
                        "agent_home": str(agent_home),
                        "user_home": str(user_home),
                        "server_url": server_url,
                        "sidecar_url": service_url,
                        "invite_url": invite.get("url"),
                        "invite_id": invite.get("invite_id"),
                        "room_id": invite.get("room_id"),
                        "hold_seconds": args.hold_seconds,
                    },
                    sort_keys=True,
                ),
                flush=True,
            )
            time.sleep(args.hold_seconds)
        return 0
    except Exception as exc:
        report["status"] = "failed"
        report["failure"] = str(exc)
        report["elapsed_ms"] = int((time.monotonic() - started) * 1000)
        return 1
    finally:
        if gateway is not None and gateway.poll() is None:
            try:
                os.killpg(gateway.pid, signal.SIGTERM)
                gateway.wait(timeout=5)
            except Exception:
                terminate(gateway)
        terminate(sidecar)
        terminate(server)
        report["logs"] = {
            "finitechat_server": tail(logs_dir / "finitechat-server.log"),
            "finitechat_hermes_serve": tail(logs_dir / "finitechat-hermes-serve.log"),
            "hermes_gateway": tail(logs_dir / "hermes-gateway.log"),
        }
        report_path.write_text(
            json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        print(json.dumps(report, indent=2, sort_keys=True))
        if report["status"] == "passed" and not args.keep_state:
            state_ctx.cleanup()
        elif args.keep_state:
            print(f"kept state at {state_root}", file=sys.stderr)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except SmokeFailure as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1) from error

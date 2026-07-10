#!/usr/bin/env python3
"""Start a fresh real-Hermes agent for the physical-phone canary.

This is the local layer of docs/hermes-phone-canary-loop.md. It uses the
hosted Finite Chat server by default, starts finitechat hermes serve before the
real Hermes gateway, proves owner-side invite admission with a throwaway CLI
client, then prints a human invite URL only after the preflight passes.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import time
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SERVER_URL = "https://chat.finite.computer"
DEFAULT_HERMES_PACKAGE = "hermes-agent==0.18.2"
DEFAULT_BUNDLE_ID = "computer.finite.finitechat"
DEFAULT_TEAM = ""
MODEL_ENV_NAMES = (
    "OPENROUTER_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "FINITECHAT_HERMES_MODEL",
    "FINITECHAT_HERMES_PROVIDER",
    "FINITECHAT_HERMES_BASE_URL",
    "FINITECHAT_HERMES_API_MODE",
)


class CanaryFailure(RuntimeError):
    pass


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--server-url",
        default=os.environ.get("FINITECHAT_PHONE_CANARY_SERVER_URL", DEFAULT_SERVER_URL),
        help="Finite Chat server URL. Product phone canaries must use https://chat.finite.computer.",
    )
    parser.add_argument(
        "--state-root",
        default=os.environ.get("FINITECHAT_PHONE_CANARY_STATE_ROOT", ""),
        help="Run state root. Defaults to target/hermes-phone-canary/local/<timestamp>.",
    )
    parser.add_argument(
        "--report",
        default=os.environ.get("FINITECHAT_PHONE_CANARY_REPORT", ""),
        help="JSON report path. Defaults to <state-root>/report.json.",
    )
    parser.add_argument(
        "--hermes-package",
        default=os.environ.get("FINITECHAT_HERMES_PACKAGE", DEFAULT_HERMES_PACKAGE),
    )
    parser.add_argument(
        "--agent-device-id",
        default=os.environ.get("FINITECHAT_PHONE_CANARY_AGENT_DEVICE_ID", "hpc"),
        help="Short Finite Chat device id for the agent.",
    )
    parser.add_argument(
        "--room-name",
        default=os.environ.get("FINITECHAT_PHONE_CANARY_ROOM_NAME", "Local Hermes Canary"),
    )
    parser.add_argument("--timeout-ms", type=int, default=60000)
    parser.add_argument(
        "--skip-model-smoke",
        action="store_true",
        help="Only prove admission. By default the canary also requires one real Hermes model reply.",
    )
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument(
        "--keep-running",
        action="store_true",
        help="Leave sidecar and Hermes gateway running after the admission preflight passes.",
    )
    parser.add_argument(
        "--install-phone-app",
        action="store_true",
        help="Build and install the current iOS app on a paired physical phone.",
    )
    parser.add_argument(
        "--ios-device",
        default=os.environ.get("FINITECHAT_IOS_DEVICE_ID") or os.environ.get("IOS_DEVICE_ID") or "",
        help="CoreDevice identifier or hardware UDID. Required only with --install-phone-app.",
    )
    parser.add_argument(
        "--ios-development-team",
        default=os.environ.get("RMP_IOS_DEVELOPMENT_TEAM") or DEFAULT_TEAM,
        help="Apple development team for physical-device signing. If omitted, xcodebuild uses local signing defaults.",
    )
    parser.add_argument(
        "--bundle-id", default=os.environ.get("FINITECHAT_IOS_BUNDLE_ID", DEFAULT_BUNDLE_ID)
    )
    parser.add_argument(
        "--env-file",
        action="append",
        default=[],
        help="Optional KEY=VALUE env file to pass provider keys to Hermes. May be repeated.",
    )
    parser.add_argument(
        "--no-default-env-file",
        action="store_true",
        help="Do not auto-load .env when present.",
    )
    return parser.parse_args()


def run(
    args: list[str],
    *,
    cwd: Path = REPO_ROOT,
    env: dict[str, str] | None = None,
    timeout: float = 60,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    proc = subprocess.run(
        args,
        cwd=cwd,
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


def run_inherit(
    args: list[str],
    *,
    cwd: Path = REPO_ROOT,
    env: dict[str, str] | None = None,
    timeout: float = 1800,
) -> None:
    proc = subprocess.run(args, cwd=cwd, env=env, timeout=timeout)
    if proc.returncode != 0:
        raise CanaryFailure(f"command failed: {args!r} (exit {proc.returncode})")


def run_json(
    args: list[str], *, env: dict[str, str] | None = None, timeout: float = 60
) -> dict[str, Any]:
    proc = run(args, env=env, timeout=timeout)
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise CanaryFailure(
            f"command did not emit JSON: {args!r}\nstdout={proc.stdout[-3000:]}"
        ) from exc


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def wait_json_url(url: str, *, timeout: float, name: str) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    last_error = ""
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=2.0) as response:
                body = response.read().decode("utf-8", errors="replace")
                if 200 <= response.status < 300:
                    return json.loads(body)
                last_error = f"HTTP {response.status}: {body[:300]}"
        except Exception as exc:
            last_error = str(exc)
        time.sleep(0.2)
    raise CanaryFailure(f"{name} did not become ready at {url}: {last_error}")


def wait_http_ok(url: str, *, timeout: float, name: str) -> None:
    deadline = time.monotonic() + timeout
    last_error = ""
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=2.0) as response:
                if 200 <= response.status < 300:
                    return
                last_error = f"HTTP {response.status}"
        except Exception as exc:
            last_error = str(exc)
        time.sleep(0.2)
    raise CanaryFailure(f"{name} did not become reachable at {url}: {last_error}")


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
            "product phone canary must use "
            f"{DEFAULT_SERVER_URL}; got {server_url!r}. Use lower-level simulator or "
            "gateway diagnostics for local delivery-server experiments."
        )
    if host in {"localhost", "127.0.0.1", "::1"} or host.startswith("127."):
        raise CanaryFailure(
            f"server URL {server_url!r} is loopback; a physical phone cannot reach the Mac through loopback"
        )


def timestamp_id() -> str:
    return time.strftime("run-%Y%m%d-%H%M%S")


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


def load_canary_env(args: argparse.Namespace) -> tuple[dict[str, str], list[str]]:
    env = os.environ.copy()
    loaded: list[str] = []
    env_files = [Path(p) for p in args.env_file]
    explicit = os.environ.get("FINITECHAT_PHONE_CANARY_ENV_FILE")
    if explicit:
        env_files.append(Path(explicit))
    if not args.no_default_env_file:
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


def write_hermes_config(
    path: Path,
    *,
    agent_home: Path,
    service_url: str,
    service_addr: str,
    finitechat_bin: Path,
    env: dict[str, str],
) -> None:
    provider = env.get("FINITECHAT_HERMES_PROVIDER", "openrouter")
    model = env.get("FINITECHAT_HERMES_MODEL", "anthropic/claude-sonnet-4.6")
    base_url = env.get("FINITECHAT_HERMES_BASE_URL", "https://openrouter.ai/api/v1")
    api_mode = env.get("FINITECHAT_HERMES_API_MODE", "chat_completions")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        f"""model:
  default: {model}
  provider: {provider}
  base_url: {base_url}
  api_mode: {api_mode}
plugins:
  enabled:
    - finitechat
gateway:
  platforms:
    finitechat:
      enabled: true
      extra:
        home: "{agent_home}"
        finitechat_bin: "{finitechat_bin}"
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


def tail(path: Path, limit: int = 12000) -> str:
    try:
        data = path.read_bytes()
    except FileNotFoundError:
        return ""
    return data[-limit:].decode("utf-8", errors="replace")


def terminate(child: subprocess.Popen[str] | None) -> None:
    if child is None or child.poll() is not None:
        return
    try:
        if child.pid:
            os.killpg(child.pid, signal.SIGTERM)
    except Exception:
        child.terminate()
    try:
        child.wait(timeout=5)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except Exception:
            child.kill()
        child.wait(timeout=5)


def write_stop_script(state_root: Path, children: dict[str, subprocess.Popen[str]]) -> None:
    lines = [
        "#!/usr/bin/env bash",
        "set -euo pipefail",
    ]
    for name, child in children.items():
        if child.poll() is None:
            pid_file = state_root / f"{name}.pid"
            pid_file.write_text(f"{child.pid}\n", encoding="utf-8")
            quoted = shlex.quote(str(pid_file))
            lines.append(f"if [[ -f {quoted} ]]; then")
            lines.append(f"  pid=$(cat {quoted})")
            lines.append('  if kill -0 "$pid" 2>/dev/null; then')
            lines.append(
                '    kill -TERM "-$pid" 2>/dev/null || kill -TERM "$pid" 2>/dev/null || true'
            )
            lines.append("  fi")
            lines.append("fi")
    stop_script = state_root / "stop.sh"
    stop_script.write_text("\n".join(lines) + "\n", encoding="utf-8")
    stop_script.chmod(0o755)


def discover_device(requested: str) -> dict[str, str]:
    with subprocess.Popen(
        ["xcrun", "devicectl", "list", "devices", "--json-output", "-"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    ) as proc:
        stdout, stderr = proc.communicate(timeout=60)
    if proc.returncode != 0:
        raise CanaryFailure(f"devicectl list devices failed: {stderr[-1000:]}")
    data = json.loads(stdout)
    candidates = data.get("result", {}).get("devices", [])
    for device in candidates:
        identifier = str(device.get("identifier") or "")
        hardware_udid = str(device.get("hardwareProperties", {}).get("udid") or "")
        name = str(device.get("name") or "")
        if requested and requested not in {identifier, hardware_udid, name}:
            continue
        if not identifier:
            continue
        state = str(device.get("connectionProperties", {}).get("tunnelState") or "").lower()
        if "unavailable" in state:
            continue
        return {
            "identifier": identifier,
            "hardware_udid": hardware_udid or identifier,
            "name": name or identifier,
        }
    if requested:
        raise CanaryFailure(f"no available paired iPhone matched {requested!r}")
    raise CanaryFailure("no available paired iPhone found; pass --ios-device")


def install_phone_app(args: argparse.Namespace, state_root: Path) -> dict[str, Any]:
    device = discover_device(args.ios_device)
    derived_data = state_root / "xcode-device-derived-data"
    team = args.ios_development_team.strip()
    run_inherit(
        ["cargo", "run", "-q", "-p", "finitechat-rmp", "--", "bindings", "swift"], timeout=2400
    )
    run_inherit(["xcodegen", "generate"], cwd=REPO_ROOT / "ios", timeout=300)
    xcode_cmd = [
        "xcrun",
        "xcodebuild",
        "-project",
        str(REPO_ROOT / "ios/FiniteChat.xcodeproj"),
        "-scheme",
        "FiniteChat",
        "-destination",
        f"id={device['hardware_udid']}",
        "-configuration",
        "Debug",
        "-sdk",
        "iphoneos",
        "-derivedDataPath",
        str(derived_data),
    ]
    if team:
        xcode_cmd.extend(["-allowProvisioningUpdates", "-allowProvisioningDeviceRegistration"])
    xcode_cmd.extend(
        [
            "build",
            "ARCHS=arm64",
            "ONLY_ACTIVE_ARCH=YES",
            f"PRODUCT_BUNDLE_IDENTIFIER={args.bundle_id}",
        ]
    )
    if team:
        xcode_cmd.append(f"DEVELOPMENT_TEAM={team}")
    run_inherit(xcode_cmd, timeout=2400)
    app_path = derived_data / "Build/Products/Debug-iphoneos/FiniteChat.app"
    if not app_path.is_dir():
        raise CanaryFailure(f"built app not found at {app_path}")
    run(
        [
            "xcrun",
            "devicectl",
            "device",
            "uninstall",
            "app",
            "--device",
            device["identifier"],
            args.bundle_id,
        ],
        check=False,
        timeout=120,
    )
    run(
        [
            "xcrun",
            "devicectl",
            "device",
            "install",
            "app",
            "--device",
            device["identifier"],
            str(app_path),
        ],
        timeout=180,
    )
    return {
        "device_name": device["name"],
        "device_identifier": device["identifier"],
        "hardware_udid": device["hardware_udid"],
        "installed_bundle_id": args.bundle_id,
        "installed": True,
        "app_path": str(app_path),
    }


def message_matches_phone_canary(text: str) -> bool:
    return "phone canary cli ok" in text.lower()


def run_model_smoke(
    *,
    finitechat_bin: Path,
    probe_home: Path,
    server_url: str,
    room_id: str,
    env: dict[str, str],
) -> dict[str, Any]:
    prompt = "Reply with exactly: phone canary cli ok"
    started = time.monotonic()
    sent = run_json(
        [
            str(finitechat_bin),
            "app",
            "--data-dir",
            str(probe_home),
            "--server",
            server_url,
            "--device-id",
            "probe",
            "send",
            "--room-id",
            room_id,
            "--text",
            prompt,
        ],
        env=env,
        timeout=60,
    )
    deadline = time.monotonic() + 120
    last_state: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        state = run_json(
            [
                str(finitechat_bin),
                "app",
                "--data-dir",
                str(probe_home),
                "--server",
                server_url,
                "--device-id",
                "probe",
                "state",
                "--start-runtime",
                "--wait-update-ms",
                "3000",
                "--room-id",
                room_id,
            ],
            env=env,
            timeout=30,
        )
        last_state = state
        for message in state.get("messages") or []:
            if not message.get("is_mine") and message_matches_phone_canary(
                str(message.get("text") or "")
            ):
                return {
                    "status": "passed",
                    "elapsed_ms": int((time.monotonic() - started) * 1000),
                    "prompt_message_id": first_matching_mine_message_id(sent),
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
        f"Hermes model smoke did not receive expected reply; recent messages={sample!r}"
    )


def first_matching_mine_message_id(state: dict[str, Any]) -> str | None:
    for message in state.get("messages") or []:
        if (
            message.get("is_mine")
            and str(message.get("text") or "") == "Reply with exactly: phone canary cli ok"
        ):
            value = message.get("message_id")
            return str(value) if value else None
    return None


def finite_identity_env(base: dict[str, str], finite_home: Path) -> dict[str, str]:
    scoped = base.copy()
    scoped["FINITE_HOME"] = str(finite_home)
    return scoped


def main() -> int:
    args = parse_args()
    enforce_product_server_url(args.server_url)
    run_id = timestamp_id()
    state_root = (
        Path(args.state_root)
        if args.state_root
        else REPO_ROOT / "target/hermes-phone-canary/local" / run_id
    )
    report_path = Path(args.report) if args.report else state_root / "report.json"
    state_root.mkdir(parents=True, exist_ok=True)
    logs_dir = state_root / "logs"
    logs_dir.mkdir(parents=True, exist_ok=True)

    finitechat_bin = REPO_ROOT / "target/debug/finitechat"
    uvx_bin = shutil.which("uvx")
    if uvx_bin is None:
        raise CanaryFailure("uvx is required to run hermes-agent")

    env, env_files_loaded = load_canary_env(args)
    env.update(
        {
            "FINITECHAT_HERMES_INBOUND_STREAM": "1",
            "FINITECHAT_ALLOW_ALL_USERS": "true",
            "FINITE_ALLOW_ALL_USERS": "true",
            "GATEWAY_ALLOW_ALL_USERS": "true",
            "FINITE_AGENT_ID": f"agent_{args.agent_device_id}",
            "FINITE_AGENT_NAME": args.agent_device_id,
        }
    )

    source = git_source()
    plugin_dir = REPO_ROOT / "integrations/hermes/finitechat"
    report: dict[str, Any] = {
        "status": "running",
        "layer": "local-phone",
        "run_id": run_id,
        "source": source,
        "runtime": {
            "finitechat_bin": str(finitechat_bin),
            "hermes_agent_version_expected": args.hermes_package,
            "plugin_name": "finitechat",
            "plugin_hash": sha256_tree(plugin_dir),
            "image_ref": None,
            "image_digest": None,
            "model_env_present": {name: bool(env.get(name)) for name in MODEL_ENV_NAMES},
            "env_files_loaded": env_files_loaded,
        },
        "server": {
            "url": args.server_url,
            "phone_reachable": True,
        },
        "agent": {
            "state_root": str(state_root),
            "restored": False,
            "device_id": args.agent_device_id,
        },
        "phone": {
            "installed": False,
        },
        "steps": [],
    }
    started = time.monotonic()

    def step(name: str, **facts: Any) -> None:
        report["steps"].append(
            {"name": name, "elapsed_ms": int((time.monotonic() - started) * 1000), **facts}
        )
        report_path.write_text(
            json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )

    sidecar: subprocess.Popen[str] | None = None
    gateway: subprocess.Popen[str] | None = None
    keep_children = False
    try:
        if not args.skip_build:
            run_inherit(["cargo", "build", "-q", "-p", "finitechat-cli"], timeout=600)
            step("finitechat.build")
        if not finitechat_bin.exists():
            raise CanaryFailure(f"finitechat binary not found: {finitechat_bin}")

        wait_http_ok(f"{args.server_url.rstrip('/')}/health", timeout=15, name="finitechat server")
        step("server.health")

        if args.install_phone_app:
            report["phone"] = install_phone_app(args, state_root)
            step("phone.app_installed", device=report["phone"].get("device_name"))

        agent_home = state_root / "agent-home"
        probe_home = state_root / "probe-home"
        hermes_home = state_root / "hermes-home"
        agent_env = finite_identity_env(env, agent_home)
        probe_env = finite_identity_env(env, probe_home)
        service_port = free_port()
        service_addr = f"127.0.0.1:{service_port}"
        service_url = f"http://{service_addr}"
        report["agent"].update(
            {
                "agent_home": str(agent_home),
                "probe_home": str(probe_home),
                "hermes_home": str(hermes_home),
                "sidecar_url": service_url,
            }
        )

        agent_init = run_json(
            [
                str(finitechat_bin),
                "hermes",
                "--agent-home",
                str(agent_home),
                "init",
                "--server",
                args.server_url,
                "--device-id",
                args.agent_device_id,
                "--agent-name",
                args.room_name,
            ],
            env=agent_env,
            timeout=60,
        )
        report["agent"].update(
            {
                "account_id": agent_init.get("account_id"),
                "npub": agent_init.get("npub"),
                "profile": agent_init.get("profile"),
            }
        )
        step("agent.init")

        run_json(
            [
                str(finitechat_bin),
                "hermes",
                "--agent-home",
                str(probe_home),
                "init",
                "--server",
                args.server_url,
                "--device-id",
                "probe",
                "--skip-agent-profile",
            ],
            env=probe_env,
            timeout=60,
        )
        step("probe.init")

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
            timeout=60,
        )
        report["runtime"]["plugin_install"] = install
        step("plugin.install", plugin_dir=install.get("plugin_dir"))

        invite = run_json(
            [
                str(finitechat_bin),
                "hermes",
                "--agent-home",
                str(agent_home),
                "invite",
                "--room-name",
                args.room_name,
                "--max-joins",
                "8",
                "--json",
            ],
            env=agent_env,
            timeout=60,
        )
        report["invite"] = {
            "room_id": invite.get("room_id"),
            "invite_id": invite.get("invite_id"),
            "url": invite.get("url"),
        }
        step("invite.created", room_id=invite.get("room_id"))

        write_hermes_config(
            hermes_home / "config.yaml",
            agent_home=agent_home,
            service_url=service_url,
            service_addr=service_addr,
            finitechat_bin=finitechat_bin,
            env=agent_env,
        )
        step("hermes.config_written")

        runtime_env = agent_env.copy()
        runtime_env.update(
            {
                "HERMES_HOME": str(hermes_home),
                "FINITECHAT_HOME": str(agent_home),
                "FINITE_AGENT_HOME": str(agent_home),
                "FINITECHAT_BIN": str(finitechat_bin),
                "FINITECHAT_HERMES_SERVICE_ADDR": service_addr,
                "FINITECHAT_HERMES_SERVICE_URL": service_url,
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
            env=runtime_env,
            stdout=sidecar_log.open("w", encoding="utf-8"),
            stderr=subprocess.STDOUT,
            text=True,
            start_new_session=True,
        )
        healthz = wait_json_url(
            f"{service_url}/healthz", timeout=15, name="finitechat hermes serve healthz"
        )
        readyz = wait_json_url(
            f"{service_url}/readyz", timeout=15, name="finitechat hermes serve readyz"
        )
        report["agent"]["healthz"] = healthz
        report["agent"]["readyz"] = readyz
        if not readyz.get("ready", True):
            raise CanaryFailure(f"sidecar readyz is not ready: {readyz}")
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
            env=runtime_env,
            stdout=gateway_log.open("w", encoding="utf-8"),
            stderr=subprocess.STDOUT,
            text=True,
            start_new_session=True,
        )
        time.sleep(3.0)
        if gateway.poll() is not None:
            raise CanaryFailure(
                f"hermes gateway exited early with {gateway.returncode}: {tail(gateway_log)}"
            )
        step("gateway.started")

        join_started = time.monotonic()
        joined = run_json(
            [
                str(finitechat_bin),
                "hermes",
                "--agent-home",
                str(probe_home),
                "join",
                "--url",
                str(invite["url"]),
                "--timeout-ms",
                str(args.timeout_ms),
            ],
            env=probe_env,
            timeout=max(60, args.timeout_ms / 1000 + 30),
        )
        report["admission_probe"] = {
            "state": joined.get("state"),
            "room_id": joined.get("room_id"),
            "elapsed_ms": int((time.monotonic() - join_started) * 1000),
        }
        if joined.get("state") != "joined":
            raise CanaryFailure(f"admission probe did not join: {joined}")
        step("admission_probe.joined")

        if not args.skip_model_smoke:
            if not any(
                env.get(name)
                for name in ("OPENROUTER_API_KEY", "ANTHROPIC_API_KEY", "OPENAI_API_KEY")
            ):
                raise CanaryFailure(
                    "model smoke requires OPENROUTER_API_KEY, ANTHROPIC_API_KEY, or OPENAI_API_KEY"
                )
            report["model_smoke"] = run_model_smoke(
                finitechat_bin=finitechat_bin,
                probe_home=probe_home,
                server_url=args.server_url,
                room_id=str(invite["room_id"]),
                env=probe_env,
            )
            step(
                "model_smoke.reply", reply_message_id=report["model_smoke"].get("reply_message_id")
            )

        report["status"] = "passed"
        report["elapsed_ms"] = int((time.monotonic() - started) * 1000)
        report["logs"] = {
            "finitechat_hermes_serve": str(sidecar_log),
            "hermes_gateway": str(gateway_log),
        }
        if args.keep_running:
            keep_children = True
            children = {"sidecar": sidecar, "gateway": gateway}
            write_stop_script(state_root, children)
            report["agent"]["stop_script"] = str(state_root / "stop.sh")
            report["agent"]["sidecar_pid"] = sidecar.pid
            report["agent"]["gateway_pid"] = gateway.pid
        report_path.write_text(
            json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        print(
            json.dumps(
                {
                    "status": "passed",
                    "report": str(report_path),
                    "state_root": str(state_root),
                    "stop_script": report["agent"].get("stop_script"),
                    "agent_npub": report["agent"].get("npub"),
                    "room_id": report["invite"].get("room_id"),
                    "invite_id": report["invite"].get("invite_id"),
                    "invite_url": report["invite"].get("url"),
                    "kept_running": bool(args.keep_running),
                },
                indent=2,
                sort_keys=True,
            )
        )
        return 0
    except Exception as exc:
        report["status"] = "failed"
        report["failure"] = str(exc)
        report["elapsed_ms"] = int((time.monotonic() - started) * 1000)
        if logs_dir.exists():
            report["log_tails"] = {
                "finitechat_hermes_serve": tail(logs_dir / "finitechat-hermes-serve.log"),
                "hermes_gateway": tail(logs_dir / "hermes-gateway.log"),
            }
        report_path.write_text(
            json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        print(
            json.dumps(
                {"status": "failed", "report": str(report_path), "failure": str(exc)}, indent=2
            ),
            file=sys.stderr,
        )
        return 1
    finally:
        if not keep_children:
            terminate(gateway)
            terminate(sidecar)


if __name__ == "__main__":
    raise SystemExit(main())

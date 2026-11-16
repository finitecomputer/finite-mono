#!/usr/bin/env python3
"""Qualify a target Hermes Telegram adapter, and check for other pollers.

Full-fidelity cutovers rely on the target gateway keeping the Telegram update
queue (`extra.preserve_pending_updates`). A string check of the adapter is not
proof, so `qualify_target_telegram` drives the adapter the target gateway
would load (`get_bundled_plugins_dir()/platforms/telegram`) through its real
cold-start and 409-conflict code against a loopback fake Bot API, with
preservation on and, as a control, off. It runs in a child process with a
scratch HOME and HERMES_HOME, needs no network beyond loopback and never sees
a real token. The result carries a fingerprint of the qualified adapter
(capability marker, Hermes version, adapter path and hash), which the
operator compares with the same qualification run inside the Runtime image
Core will boot.

`check_no_other_poller` is the pre-restart check: one `getWebhookInfo` and one
`getUpdates` without an offset (it acknowledges nothing). It reports only
status codes and counts, never a token or update content.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

REQUIRED_CAPABILITY = "finite.telegram.preserve-pending/2"
REQUIRED_SITES = ("cold-start", "conflict-retry")
PROBE_TOKEN = "1000000001:AAprobeTokenForFiniteQualification_000"
PROBE_TIMEOUT_SECONDS = 180


class GateError(Exception):
    pass


# --------------------------------------------------------------------------
# Parent side


def _child_env(scratch: Path) -> dict[str, str]:
    env = {
        "HOME": str(scratch / "home"),
        "HERMES_HOME": str(scratch / "hermes"),
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "TZ": "UTC",
        "LANG": "C.UTF-8",
        "HERMES_TELEGRAM_DISABLE_FALLBACK_IPS": "1",
        "PYTHONDONTWRITEBYTECODE": "1",
    }
    for name, value in os.environ.items():
        if name.startswith("NIX_PYTHON") or name == "HERMES_BUNDLED_PLUGINS":
            env[name] = value
    return env


def qualify_target_telegram(
    plugins_dir: Path | None = None, timeout: float = PROBE_TIMEOUT_SECONDS
) -> dict[str, Any]:
    """Run the behavioral probe in a child process and return its verdict."""
    with tempfile.TemporaryDirectory(prefix="finite-telegram-gate-") as name:
        scratch = Path(name)
        (scratch / "home").mkdir()
        (scratch / "hermes").mkdir()
        command = [sys.executable, "-B", str(Path(__file__).resolve()), "probe"]
        if plugins_dir is not None:
            command += ["--plugins-dir", str(plugins_dir)]
        try:
            completed = subprocess.run(
                command,
                env=_child_env(scratch),
                cwd=scratch,
                capture_output=True,
                text=True,
                timeout=timeout,
            )
        except subprocess.TimeoutExpired:
            return {"qualified": False, "reason": "probe-timed-out"}
    lines = [line for line in completed.stdout.splitlines() if line.startswith("GATE=")]
    if completed.returncode != 0 or not lines:
        return {
            "qualified": False,
            "reason": f"probe-failed-exit-{completed.returncode}",
            "stderr_tail": completed.stderr[-2000:].replace(PROBE_TOKEN, "<probe>"),
        }
    return json.loads(lines[-1].removeprefix("GATE="))


def fingerprint(material: dict[str, Any]) -> str:
    rendered = json.dumps(material, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(rendered.encode("utf-8")).hexdigest()


def check_no_other_poller(
    token: str, base_url: str = "https://api.telegram.org/bot", timeout: float = 20
) -> dict[str, Any]:
    """One webhook-info and one offset-less getUpdates; report codes only."""
    import urllib.error
    import urllib.request

    def call(method: str, query: str = "") -> tuple[int, Any]:
        url = f"{base_url}{token}/{method}{query}"
        request = urllib.request.Request(url, method="GET")
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                return response.status, json.loads(response.read() or b"{}")
        except urllib.error.HTTPError as exc:
            return exc.code, None
        except Exception:
            # The exception text can carry the URL, and so the token.
            return 0, None

    webhook_status, webhook = call("getWebhookInfo")
    updates_status, updates = call("getUpdates", "?timeout=0&limit=1")
    info = (webhook or {}).get("result") if isinstance(webhook, dict) else None
    result = {
        "webhook_info_status": webhook_status,
        "webhook_set": bool(isinstance(info, dict) and info.get("url")),
        "pending_update_count": info.get("pending_update_count")
        if isinstance(info, dict)
        else None,
        "get_updates_status": updates_status,
        "other_poller_detected": updates_status == 409,
    }
    result["safe_to_start"] = (
        webhook_status == 200
        and not result["webhook_set"]
        and updates_status == 200
        and isinstance(updates, dict)
        and updates.get("ok") is True
    )
    return result


# --------------------------------------------------------------------------
# Child side: runs inside the target Hermes interpreter


def _probe_main(plugins_dir: str | None) -> int:
    import asyncio
    import importlib.metadata
    import importlib.util
    import threading
    import time
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
    from urllib.parse import parse_qs

    if plugins_dir is None:
        from hermes_cli.plugins import get_bundled_plugins_dir

        plugins_dir = str(get_bundled_plugins_dir())
    package_dir = Path(plugins_dir) / "platforms" / "telegram"
    adapter_path = (package_dir / "adapter.py").resolve()
    package_name = "finite_telegram_gate_adapter"
    spec = importlib.util.spec_from_file_location(
        package_name,
        package_dir / "__init__.py",
        submodule_search_locations=[str(package_dir)],
    )
    if spec is None or spec.loader is None:
        raise GateError("target Telegram adapter package cannot be loaded")
    package = importlib.util.module_from_spec(spec)
    sys.modules[package_name] = package
    spec.loader.exec_module(package)
    adapter_module = sys.modules[f"{package_name}.adapter"]
    from gateway.config import PlatformConfig

    class FakeBotApi:
        def __init__(self) -> None:
            self.lock = threading.Condition()
            self.reset()
            fake = self

            class Handler(BaseHTTPRequestHandler):
                def log_message(self, *args: Any) -> None:
                    return None

                def do_GET(self) -> None:
                    self.handle_call()

                def do_POST(self) -> None:
                    self.handle_call()

                def handle_call(self) -> None:
                    length = int(self.headers.get("content-length") or 0)
                    raw = self.rfile.read(length) if length else b""
                    kind = self.headers.get("content-type", "")
                    params: dict[str, Any] = {}
                    if raw and "json" in kind:
                        params = json.loads(raw)
                    elif raw and "form" in kind:
                        params = {k: v[0] for k, v in parse_qs(raw.decode()).items()}
                    method = self.path.rstrip("/").rsplit("/", 1)[-1]
                    status, body = fake.dispatch(method, params)
                    data = json.dumps(body).encode()
                    self.send_response(status)
                    self.send_header("content-type", "application/json")
                    self.send_header("content-length", str(len(data)))
                    self.end_headers()
                    self.wfile.write(data)

            self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
            threading.Thread(target=self.server.serve_forever, daemon=True).start()

        def reset(self) -> None:
            with self.lock:
                self.drop_requests = 0
                self.updates_ok = 0
                self.conflicts_left = 0
                self.conflicts_served = 0

        @property
        def base_url(self) -> str:
            return f"http://127.0.0.1:{self.server.server_address[1]}/bot"

        def dispatch(self, method: str, params: dict[str, Any]) -> tuple[int, Any]:
            if method == "getMe":
                return 200, {
                    "ok": True,
                    "result": {
                        "id": 1000000001,
                        "is_bot": True,
                        "first_name": "Gate",
                        "username": "finite_gate_bot",
                    },
                }
            if method in ("deleteWebhook", "setWebhook"):
                if str(params.get("drop_pending_updates", "")).lower() in ("true", "1"):
                    with self.lock:
                        self.drop_requests += 1
                return 200, {"ok": True, "result": True}
            if method == "getUpdates":
                with self.lock:
                    if self.conflicts_left > 0:
                        self.conflicts_left -= 1
                        self.conflicts_served += 1
                        return 409, {
                            "ok": False,
                            "error_code": 409,
                            "description": "Conflict: terminated by other getUpdates "
                            "request; make sure that only one bot instance is running",
                        }
                time.sleep(0.05)
                with self.lock:
                    self.updates_ok += 1
                    self.lock.notify_all()
                return 200, {"ok": True, "result": []}
            if method == "getWebhookInfo":
                return 200, {
                    "ok": True,
                    "result": {
                        "url": "",
                        "has_custom_certificate": False,
                        "pending_update_count": 0,
                    },
                }
            if method == "getMyCommands":
                return 200, {"ok": True, "result": []}
            return 200, {"ok": True, "result": True}

        def wait(self, predicate, timeout: float) -> bool:
            deadline = time.time() + timeout
            with self.lock:
                while not predicate():
                    remaining = deadline - time.time()
                    if remaining <= 0:
                        return False
                    self.lock.wait(min(0.2, remaining))
            return True

    fake = FakeBotApi()
    real_sleep = asyncio.sleep

    async def capped_sleep(delay: float, result: Any = None) -> Any:
        return await real_sleep(min(delay, 0.1), result)

    async def run_case(preserve: bool) -> dict[str, Any]:
        fake.reset()
        adapter = adapter_module.TelegramAdapter(
            PlatformConfig(
                enabled=True,
                token=PROBE_TOKEN,
                extra={"base_url": fake.base_url, "preserve_pending_updates": preserve},
            )
        )
        case: dict[str, Any] = {}
        try:
            connected = await asyncio.wait_for(adapter.connect(), 60)
            case["connected"] = bool(connected)
            case["cold_start_polled"] = await asyncio.to_thread(
                fake.wait, lambda: fake.updates_ok >= 1, 30
            )
            case["cold_start_drop_requests"] = fake.drop_requests
            before_drop = fake.drop_requests
            polled_before = fake.updates_ok
            with fake.lock:
                fake.conflicts_left = 2
            asyncio.sleep = capped_sleep
            try:
                case["polling_resumed_after_conflicts"] = await asyncio.to_thread(
                    fake.wait,
                    lambda: (
                        fake.conflicts_served >= 2 and fake.updates_ok > polled_before
                    ),
                    60,
                )
            finally:
                asyncio.sleep = real_sleep
            case["conflicts_served"] = fake.conflicts_served
            case["conflict_drop_requests"] = fake.drop_requests - before_drop
            case["sites"] = dict(getattr(adapter, "_finite_preserve_pending_sites", {}))
        finally:
            try:
                await asyncio.wait_for(adapter.disconnect(), 30)
            except Exception:
                pass
        return case

    async def main() -> dict[str, Any]:
        return {
            "preserve": await run_case(True),
            "control": await run_case(False),
        }

    probes = asyncio.run(main())
    marker = getattr(adapter_module, "FINITE_TELEGRAM_PRESERVE_PENDING", None)
    preserve, control = probes["preserve"], probes["control"]
    reasons = []
    if marker != REQUIRED_CAPABILITY:
        reasons.append("capability-marker-missing-or-wrong-version")
    if not (preserve.get("connected") and preserve.get("cold_start_polled")):
        reasons.append("preserve-case-did-not-poll")
    if preserve.get("cold_start_drop_requests"):
        reasons.append("cold-start-drops-queue")
    if not preserve.get("polling_resumed_after_conflicts"):
        reasons.append("conflict-recovery-did-not-resume")
    if preserve.get("conflict_drop_requests"):
        reasons.append("conflict-retry-drops-queue")
    if any(preserve.get("sites", {}).get(site) is not False for site in REQUIRED_SITES):
        reasons.append("patched-call-sites-not-exercised")
    # The control proves the probe can see a drop at all.
    if not control.get("cold_start_drop_requests") or not control.get(
        "conflict_drop_requests"
    ):
        reasons.append("control-did-not-drop-probe-is-blind")
    material = {
        "capability": marker,
        "hermes_version": importlib.metadata.version("hermes-agent"),
        "adapter_path": str(adapter_path),
        "adapter_sha256": hashlib.sha256(adapter_path.read_bytes()).hexdigest(),
    }
    verdict = {
        "qualified": not reasons,
        "reasons": reasons,
        **material,
        "fingerprint": fingerprint(material),
        "probes": probes,
    }
    print("GATE=" + json.dumps(verdict, sort_keys=True))
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    probe = sub.add_parser("probe", help="internal: run inside the target Hermes")
    probe.add_argument("--plugins-dir")
    args = parser.parse_args(argv)
    if args.command == "probe":
        return _probe_main(args.plugins_dir)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())

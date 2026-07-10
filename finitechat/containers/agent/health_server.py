#!/usr/bin/env python3
"""Narrow health/contact endpoint for the Finite Agent Runtime."""

from __future__ import annotations

import json
import os
import subprocess
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

AGENT_HOME = Path(os.environ.get("FINITECHAT_HOME", "/data/agent"))
FINITECHAT_BIN = os.environ.get("FINITECHAT_BIN", "/usr/local/bin/finitechat")
HOST = os.environ.get("FINITE_AGENT_HTTP_HOST", "0.0.0.0")
PORT = int(os.environ.get("FINITE_AGENT_HTTP_PORT", "8080"))
BRIDGE_STATUS_FILE = AGENT_HOME / "hermes-bridge-status.json"


def identity() -> dict[str, Any]:
    config_path = AGENT_HOME / "config.json"
    if not config_path.exists():
        return {"ready": False, "error": "agent home is not initialized"}
    try:
        proc = subprocess.run(
            [FINITECHAT_BIN, "auth", "status"],
            capture_output=True,
            check=True,
            text=True,
            timeout=5,
        )
        value = json.loads(proc.stdout)
    except Exception as exc:
        return {"ready": False, "error": str(exc)}
    return {
        "ready": True,
        "npub": value.get("npub"),
        "account_id": value.get("account_id"),
    }


def bridge_status() -> dict[str, Any]:
    try:
        payload = json.loads(BRIDGE_STATUS_FILE.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return {"status": "starting", "ok": False}
    except Exception as exc:
        return {"status": "unavailable", "ok": False, "error": str(exc)}
    if not isinstance(payload, dict):
        return {
            "status": "unavailable",
            "ok": False,
            "error": "bridge status is not an object",
        }
    return payload


def runtime_health() -> dict[str, Any]:
    payload = identity()
    # `npub` remains the generic identity-health field. `agent_npub` is the
    # stable Finite Chat contact coordinate used by Hosted Web, Electron, and
    # native Devices to perform MLS Add + Welcome admission.
    payload["agent_npub"] = payload.get("npub")
    payload["bridge"] = bridge_status()
    return payload


def runtime_ready(payload: dict[str, Any]) -> bool:
    return bool(payload.get("ready")) and payload.get("bridge", {}).get("ok") is not False


class Handler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        if self.path == "/healthz":
            payload = runtime_health()
            self._write(200 if runtime_ready(payload) else 503, payload)
            return
        if self.path in {"/contact", "/invite"}:
            # `/invite` is a temporary URL-compatibility alias for already
            # recorded Runtime facts. It serves contact metadata only and does
            # not recreate the removed invite-session admission protocol.
            payload = runtime_health()
            self._write(200 if runtime_ready(payload) else 503, payload)
            return
        self._write(404, {"ready": False, "error": "not found"})

    def log_message(self, fmt: str, *args: object) -> None:
        return

    def _write(self, status: int, payload: dict[str, Any]) -> None:
        body = json.dumps(payload, sort_keys=True).encode("utf-8")
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def main() -> None:
    server = ThreadingHTTPServer((HOST, PORT), Handler)
    server.serve_forever()


if __name__ == "__main__":
    main()

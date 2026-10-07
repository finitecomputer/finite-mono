#!/usr/bin/env python3
"""Prove fresh chats survive real-Hermes interruption and empty-target restore."""

from __future__ import annotations

import argparse
import copy
import hashlib
import importlib.util
import io
import json
import os
import re
import shutil
import socket
import sqlite3
import subprocess
import tarfile
import tempfile
import threading
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
MONOREPO_ROOT = REPO_ROOT.parent
DURABLE_SMOKE_PATH = REPO_ROOT / "scripts" / "hermes-durable-home-docker-smoke.py"
DEFAULT_IMAGE = "finite-agent-chat-interruption-smoke"
# Repeated graceful stops qualify the release-then-disconnect race: a lease
# released while the adapter's inbound stream is still open could be
# re-leased onto it and stranded. Each cycle uses its own markers.
GRACEFUL_STOP_CASES = ["graceful-stop", "graceful-stop-repeat-2", "graceful-stop-repeat-3"]
DOCKER_HOST_ARGS = ["--add-host", "host.docker.internal:host-gateway"]
# Owned by finitechat-cli `HERMES_INBOX_FILE` under FINITECHAT_HOME, which
# `smoke.start_agent_container` places at /home/node/.finitechat/agent.
AGENT_HOME_MOUNT = "/home/node"
HERMES_INBOX_PATH = f"{AGENT_HOME_MOUNT}/.finitechat/agent/hermes-inbox.json"
DIAGNOSTIC_LOG_LINES = 120
DIAGNOSTIC_TEXT_LIMIT = 6000
DIAGNOSTIC_INBOX_EVENTS = 40
DIAGNOSTIC_PROVIDER_REQUESTS = 40
# Mirrors finitechat-cli `DEFAULT_HERMES_INBOX_LEASE_TTL_MILLIS` (45 minutes);
# a contract test pins the two together. The smoke never shortens it: the
# Agent must run the production TTL, and crash recovery is simulated only by
# backdating the two known leases on the stopped synthetic volume.
PRODUCTION_LEASE_TTL_MS = 45 * 60 * 1000
LEASE_TTL_ENV = "FINITECHAT_HERMES_LEASE_TTL_MILLIS"
SIMULATED_EXPIRY_MARGIN_MS = 60_000
# Tolerated host/daemon clock skew when checking a fresh lease's age.
LEASE_CLOCK_SKEW_MS = 60_000
FINAL_ACK_TIMEOUT_SECS = 60
# Mirrors pinned Hermes `run_agent` `_lease_ttl`; a contract test pins the two.
# Every turn on an existing session holds this cross-process lease in
# state.db, refreshed while the turn runs. A SIGKILLed gateway cannot release
# it, and a recycled PID in the new container can stop Hermes reclaiming it
# early, so it lasts until its own TTL. A real crash is redelivered only after
# the 45-minute inbox TTL, long after this lease lapses. The SIGKILL case
# waits it out in real time and never edits Hermes state.
HERMES_TURN_LEASE_TTL_SECS = 300
HERMES_TURN_LEASE_MARGIN_SECS = 15
HERMES_STATE_DIR = f"{AGENT_HOME_MOUNT}/.hermes"

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
        default=os.environ.get("FINITECHAT_SERVER_BIN") or shutil.which("finitechat-server") or "",
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


def run_nix_package_binary(package: str, binary: str) -> Path:
    if shutil.which("nix") is None:
        raise SmokeFailure(f"nix is required to resolve {binary}")
    result = subprocess.run(
        ["nix", "build", "--no-link", "--print-out-paths", f"{MONOREPO_ROOT}#{package}"],
        cwd=MONOREPO_ROOT,
        text=True,
        capture_output=True,
        timeout=600,
    )
    if result.returncode != 0:
        raise SmokeFailure(
            f"nix build failed for {package}\nstdout={result.stdout[-3000:]}\nstderr={result.stderr[-3000:]}"
        )
    outputs = [line.strip() for line in result.stdout.splitlines() if line.strip()]
    if len(outputs) != 1:
        raise SmokeFailure(f"expected one Nix output for {package}, got {outputs!r}")
    return Path(outputs[0]) / "bin" / binary


def resolve_server_binary(value: str) -> Path:
    if value:
        return Path(value).resolve()
    return run_nix_package_binary("finitechat-server", "finitechat-server")


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


def latest_user_text(payload: dict[str, Any]) -> str:
    messages = payload.get("messages")
    if isinstance(messages, list):
        for message in reversed(messages):
            if isinstance(message, dict) and message.get("role") == "user":
                return all_text(message.get("content"))
    return all_text(payload.get("input") or payload)


class FakeModelState:
    def __init__(self) -> None:
        self.condition = threading.Condition()
        self.seen: set[str] = set()
        self.released: set[str] = set()
        self.requests: list[dict[str, Any]] = []

    def observe(self, payload: dict[str, Any]) -> str | None:
        return self.record(payload)[1]

    def record(self, payload: dict[str, Any]) -> tuple[dict[str, Any], str | None]:
        text = all_text(payload.get("messages") or payload.get("input") or payload)
        matches = re.findall(r"FINITE_INTERRUPT_STALL:([a-z0-9-]+)", text)
        stall = matches[-1] if matches else None
        with self.condition:
            request = {
                "stream": payload.get("stream"),
                "stall": stall,
                "text": text,
                "latest_user_text": latest_user_text(payload),
                "received_at_ms": int(time.time() * 1000),
                "outcome": "pending",
            }
            self.requests.append(request)
            return request, stall

    def finish(self, request: dict[str, Any], outcome: str) -> None:
        with self.condition:
            request["outcome"] = outcome
            request["finished_at_ms"] = int(time.time() * 1000)

    def summary(self, *, limit: int = DIAGNOSTIC_PROVIDER_REQUESTS) -> dict[str, Any]:
        """Request metadata without prompt bodies: synthetic markers only."""
        with self.condition:
            requests = list(enumerate(self.requests))
        return {
            "count": len(requests),
            "requests": [
                {
                    "index": index,
                    "stream": request.get("stream"),
                    "stall": request.get("stall"),
                    "latest_user_reply_marker": reply_marker(
                        str(request.get("latest_user_text") or "")
                    ),
                    "received_at_ms": request.get("received_at_ms"),
                    "stall_entered_at_ms": request.get("stall_entered_at_ms"),
                    "stall_wait_finished_at_ms": request.get("stall_wait_finished_at_ms"),
                    "finished_at_ms": request.get("finished_at_ms"),
                    "outcome": request.get("outcome"),
                }
                for index, request in requests[-limit:]
            ],
        }

    def mark_seen(self, name: str, request: dict[str, Any]) -> None:
        with self.condition:
            if request.get("stream") is not True or request.get("stall") != name:
                raise SmokeFailure("only an SSE request can enter its stall barrier")
            request["stall_entered_at_ms"] = int(time.time() * 1000)
            self.seen.add(name)
            self.condition.notify_all()

    def require_stream_in_flight(self, name: str) -> dict[str, Any]:
        """Prove the actual open SSE barrier, not markers retained in prompt history."""
        with self.condition:
            candidates = [
                (index, request)
                for index, request in enumerate(self.requests)
                if request.get("stall") == name and request.get("stall_entered_at_ms") is not None
            ]
            if len(candidates) != 1:
                raise SmokeFailure(f"{name} requires exactly one entered SSE barrier")
            index, request = candidates[0]
            if (
                request.get("stream") is not True
                or request.get("outcome") != "pending"
                or request.get("stall_wait_finished_at_ms") is not None
                or name in self.released
            ):
                raise SmokeFailure(f"{name} SSE barrier is no longer in flight")
            return {
                "request_index": index,
                "stream": True,
                "stall": name,
                "entered_at_ms": request["stall_entered_at_ms"],
                "checked_at_ms": int(time.time() * 1000),
                "outcome": "pending",
            }

    def wait_seen(self, name: str, *, timeout: float = 60) -> None:
        deadline = time.monotonic() + timeout
        with self.condition:
            while name not in self.seen:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise SmokeFailure(f"fake model never observed stalled turn {name!r}")
                self.condition.wait(remaining)

    def wait_released(self, name: str, request: dict[str, Any], *, timeout: float = 120) -> None:
        deadline = time.monotonic() + timeout
        with self.condition:
            while name not in self.released:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    break
                self.condition.wait(remaining)
            request["stall_wait_finished_at_ms"] = int(time.time() * 1000)

    def release(self, name: str) -> None:
        with self.condition:
            self.released.add(name)
            self.condition.notify_all()


def reply_marker(text: str) -> str | None:
    matches = re.findall(r"Reply with exactly:\s*([^\n]+)", text)
    return matches[-1].strip().strip('"')[:120] if matches else None


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
            request, stall = state.record(payload)
            reply = expected_reply(payload)
            if payload.get("stream") is True:
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Cache-Control", "no-cache")
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.flush()
                if stall:
                    state.mark_seen(stall, request)
                    state.wait_released(stall, request)
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
                    state.finish(request, "streamed")
                except (BrokenPipeError, ConnectionResetError):
                    state.finish(request, "client_disconnected")
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
            state.finish(request, "responded")

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


def wait_existing_reply(
    *,
    image: str,
    volume: str,
    server_url: str,
    room_id: str,
    prompt_message_id: str,
    expected: str,
    env: dict[str, str],
) -> dict[str, Any]:
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
                    "prompt_message_id": prompt_message_id,
                    "reply_message_id": message.get("message_id"),
                    "reply_text": message.get("text"),
                }
    raise SmokeFailure(f"queued reply {expected!r} did not arrive")


def wait_durable_inbox_event(container: str, message_id: str, *, timeout: float = 30) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = smoke.run(
            [
                "docker",
                "exec",
                container,
                "/bin/sh",
                "-c",
                'test -f "$FINITECHAT_HOME/hermes-inbox.json" '
                '&& cat "$FINITECHAT_HOME/hermes-inbox.json" || printf \'{"events":[]}\'',
            ],
            timeout=30,
        )
        try:
            inbox = json.loads(result.stdout)
        except json.JSONDecodeError:
            inbox = {}
        if any(
            str(event.get("message_id") or event.get("event", {}).get("message_id") or "")
            == message_id
            for event in inbox.get("events") or []
            if isinstance(event, dict)
        ):
            return
        time.sleep(0.1)
    raise SmokeFailure(f"queued message {message_id} was not retained in the durable Hermes inbox")


def bounded_text(text: str, limit: int = DIAGNOSTIC_TEXT_LIMIT) -> str:
    return text if len(text) <= limit else "...(truncated)\n" + text[-limit:]


def summarize_hermes_inbox(
    inbox: dict[str, Any], *, now_ms: int, limit: int = DIAGNOSTIC_INBOX_EVENTS
) -> dict[str, Any]:
    """Lease state per entry; never the event payload (it carries message text)."""
    events = [event for event in inbox.get("events") or [] if isinstance(event, dict)]
    summarized = []
    for event in events[-limit:]:
        lease = event.get("lease")
        if not isinstance(lease, dict):
            lease = {}
        leased_at_ms = lease.get("leased_at_ms")
        summarized.append(
            {
                "key": event.get("key"),
                "seq": event.get("seq"),
                "message_id": event.get("message_id"),
                "created_at_ms": event.get("created_at_ms"),
                # Entries written before leases existed load as pending.
                "lease_state": lease.get("state", "pending"),
                "lease_id": lease.get("lease_id"),
                "leased_at_ms": leased_at_ms,
                "lease_age_ms": now_ms - leased_at_ms if isinstance(leased_at_ms, int) else None,
            }
        )
    acked = [entry for entry in inbox.get("acked") or [] if isinstance(entry, dict)]
    return {
        "event_count": len(events),
        "events": summarized,
        "cursors": inbox.get("cursors"),
        "acked_count": len(acked),
        "acked_recent": [
            {"key": entry.get("key"), "acked_at_ms": entry.get("acked_at_ms")}
            for entry in acked[-limit:]
        ],
    }


def inbox_key_message_id(key: Any) -> str:
    # finitechat-cli inbox keys are "room\x1fseq\x1fmessage_id".
    return str(key or "").rsplit("\x1f", 1)[-1]


def inbox_event_by_message_id(inbox: dict[str, Any], message_id: str) -> list[dict[str, Any]]:
    return [
        event
        for event in inbox.get("events") or []
        if isinstance(event, dict) and event.get("message_id") == message_id
    ]


def inbox_message_acked(inbox: dict[str, Any], message_id: str) -> bool:
    return any(
        isinstance(entry, dict) and inbox_key_message_id(entry.get("key")) == message_id
        for entry in inbox.get("acked") or []
    )


def lease_state(event: dict[str, Any]) -> str:
    lease = event.get("lease")
    return str(lease.get("state", "pending")) if isinstance(lease, dict) else "pending"


def require_graceful_inbox_released(
    inbox: dict[str, Any], *, message_ids: dict[str, str]
) -> dict[str, str]:
    """A graceful stop must leave no lease behind for restart to wait out.

    Both expected entries must be Pending: the fake provider is still stalled
    and the queued turn has never reached it, so neither turn can have
    completed. Accepting Acked here would conceal a dropped interrupted turn.
    """
    leased = [
        f"seq={event.get('seq')} lease_id={(event.get('lease') or {}).get('lease_id')}"
        for event in inbox.get("events") or []
        if isinstance(event, dict) and lease_state(event) == "leased"
    ]
    if leased:
        raise SmokeFailure(
            "graceful stop left inbox entries leased before restart: " + ", ".join(leased)
        )
    states: dict[str, str] = {}
    for role, message_id in message_ids.items():
        events = inbox_event_by_message_id(inbox, message_id)
        if len(events) > 1:
            raise SmokeFailure(f"{role} message {message_id} is duplicated in the inbox")
        if events:
            states[role] = lease_state(events[0])
        elif inbox_message_acked(inbox, message_id):
            states[role] = "acked"
        else:
            raise SmokeFailure(f"{role} message {message_id} vanished from the inbox")
    if states.get("queued") != "pending":
        raise SmokeFailure(f"queued follow-up was settled before it ran: {states}")
    if states.get("active") != "pending":
        raise SmokeFailure(f"stalled active turn was settled without completing: {states}")
    return states


def simulate_lease_expiry(
    inbox: dict[str, Any],
    *,
    message_ids: dict[str, str],
    now_ms: int,
    ttl_ms: int = PRODUCTION_LEASE_TTL_MS,
    margin_ms: int = SIMULATED_EXPIRY_MARGIN_MS,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """Backdate exactly the given fresh leases past the production TTL.

    Test fixture only, applied to a stopped synthetic volume after a proven
    SIGKILL: it stands in for 45 minutes of wall clock so the sidecar's own
    TTL sweep, unchanged, redelivers the crash-stranded entries. Every other
    field, entry and the entry order are left exactly as they were.
    """
    if len(set(message_ids.values())) != 2:
        raise SmokeFailure(f"expiry fixture needs exactly two distinct entries: {message_ids}")
    simulated = copy.deepcopy(inbox)
    records = []
    for role, message_id in message_ids.items():
        events = inbox_event_by_message_id(simulated, message_id)
        if len(events) != 1:
            raise SmokeFailure(f"{role} message {message_id} found {len(events)} times")
        lease = events[0].get("lease")
        if lease_state(events[0]) != "leased" or not isinstance(lease, dict):
            raise SmokeFailure(f"{role} message {message_id} is not leased after SIGKILL")
        leased_at_ms = lease.get("leased_at_ms")
        if not isinstance(leased_at_ms, int):
            raise SmokeFailure(f"{role} lease has no integer leased_at_ms")
        age_ms = now_ms - leased_at_ms
        if not -LEASE_CLOCK_SKEW_MS <= age_ms < ttl_ms:
            raise SmokeFailure(f"{role} lease age {age_ms}ms is not a fresh, unexpired lease")
        backdated = leased_at_ms - ttl_ms - margin_ms
        if backdated < 0:
            raise SmokeFailure(f"{role} lease cannot be backdated below zero")
        lease["leased_at_ms"] = backdated
        records.append(
            {
                "role": role,
                "seq": events[0].get("seq"),
                "message_id": message_id,
                "lease_id": lease.get("lease_id"),
                "original_leased_at_ms": leased_at_ms,
                "simulated_leased_at_ms": backdated,
                "original_age_ms": age_ms,
            }
        )
    require_only_leases_backdated(inbox, simulated, records)
    return simulated, records


def require_only_leases_backdated(
    original: dict[str, Any], simulated: dict[str, Any], records: list[dict[str, Any]]
) -> None:
    restored = copy.deepcopy(simulated)
    for record in records:
        for event in inbox_event_by_message_id(restored, record["message_id"]):
            lease = event.get("lease")
            if (
                not isinstance(lease, dict)
                or lease.get("leased_at_ms") != record["simulated_leased_at_ms"]
            ):
                raise SmokeFailure(f"{record['role']} lease was not backdated as recorded")
            lease["leased_at_ms"] = record["original_leased_at_ms"]
    if restored != original or json.dumps(restored) != json.dumps(original):
        raise SmokeFailure("expiry fixture changed more than the two recorded leased_at_ms values")


def sha256_text(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()


def write_stopped_inbox(
    *, image: str, home_volume: str, expected_sha256: str, fixture: Path
) -> None:
    """Replace the stopped Agent's inbox in place, only if it is unchanged.

    In-place `cat >` keeps the file's owner and 0600 mode; the compare-and-swap
    on the read digest refuses to write over anything but the inbox we read.
    """
    script = (
        "set -eu\n"
        f'f="{HERMES_INBOX_PATH}"\n'
        'test "$(sha256sum "$f" | cut -d" " -f1)" = "$EXPECTED_SHA256"\n'
        'cat /fixture/hermes-inbox.json > "$f"\n'
        'test "$(sha256sum "$f" | cut -d" " -f1)" = "$(sha256sum /fixture/hermes-inbox.json | cut -d" " -f1)"\n'
    )
    smoke.run(
        [
            "docker",
            "run",
            "--rm",
            "--network",
            "none",
            "--env",
            f"EXPECTED_SHA256={expected_sha256}",
            "--entrypoint",
            "/bin/sh",
            "--mount",
            f"type=volume,src={home_volume},dst={AGENT_HOME_MOUNT}",
            "--mount",
            f"type=bind,src={fixture},dst=/fixture/hermes-inbox.json,readonly",
            image,
            "-c",
            script,
        ],
        timeout=120,
    )


def read_stopped_turn_leases(*, image: str, home_volume: str) -> list[dict[str, Any]]:
    """List Hermes turn leases from a copy of the stopped Agent's state.db.

    The volume is mounted read-only; only a host-side copy is opened, so
    SQLite replays the killed gateway's WAL without touching the volume.
    """
    inspected = smoke.run(["docker", "volume", "inspect", home_volume], check=False, timeout=30)
    if inspected.returncode != 0:
        raise SmokeFailure(f"home volume {home_volume} does not exist")
    script = (
        f'cd "{HERMES_STATE_DIR}"\n'
        "set -- state.db\n"
        'if [ -e state.db-wal ]; then set -- "$@" state.db-wal; fi\n'
        'exec tar -cf - "$@"\n'
    )
    result = subprocess.run(
        [
            "docker",
            "run",
            "--rm",
            "--network",
            "none",
            "--entrypoint",
            "/bin/sh",
            "--mount",
            f"type=volume,src={home_volume},dst={AGENT_HOME_MOUNT},readonly",
            image,
            "-c",
            script,
        ],
        capture_output=True,
        timeout=120,
        check=False,
    )
    if result.returncode != 0:
        raise SmokeFailure(f"state.db read exited {result.returncode}: {result.stderr[-500:]!r}")
    with tempfile.TemporaryDirectory(prefix="finite-hermes-state-") as scratch:
        with tarfile.open(fileobj=io.BytesIO(result.stdout)) as archive:
            archive.extractall(scratch, filter="data")
        conn = sqlite3.connect(Path(scratch) / "state.db")
        try:
            rows = conn.execute(
                "SELECT conversation_id, holder, acquired_at, expires_at "
                "FROM session_turn_leases ORDER BY conversation_id"
            ).fetchall()
        finally:
            conn.close()
    return [
        {"conversation_id": row[0], "holder": row[1], "acquired_at": row[2], "expires_at": row[3]}
        for row in rows
    ]


def wait_turn_lease_expiry(
    exited_monotonic: float,
    *,
    ttl_secs: float = HERMES_TURN_LEASE_TTL_SECS,
    margin_secs: float = HERMES_TURN_LEASE_MARGIN_SECS,
    monotonic: Any = time.monotonic,
    sleep: Any = time.sleep,
) -> dict[str, Any]:
    """Sleep until a lease last refreshed at the proven exit has lapsed."""
    deadline = exited_monotonic + ttl_secs + margin_secs
    started = monotonic()
    while (remaining := deadline - monotonic()) > 0:
        sleep(remaining)
    finished = monotonic()
    return {
        "pinned_ttl_secs": ttl_secs,
        "margin_secs": margin_secs,
        "waited_secs": round(finished - started, 3),
        "since_exit_secs": round(finished - exited_monotonic, 3),
    }


def require_turn_leases_lapsed(
    before: list[dict[str, Any]],
    after: list[dict[str, Any]],
    *,
    exited_at: float,
    now: float,
    ttl_secs: float = HERMES_TURN_LEASE_TTL_SECS,
) -> list[dict[str, Any]]:
    """The killed gateway's turn leases lapse on their own clock before restart.

    `exited_at` is taken once exit 137 is proven, so no refresh can be later:
    every lease must expire by `exited_at + ttl_secs`, be unchanged while the
    Agent is stopped, and have expired by `now`.
    """
    if not before:
        raise SmokeFailure("SIGKILL left no Hermes turn lease; the case no longer covers it")
    if after != before:
        raise SmokeFailure(f"Hermes turn leases changed while stopped: {before} -> {after}")
    records = []
    for row in before:
        expires_at = float(row["expires_at"])
        if expires_at > exited_at + ttl_secs:
            raise SmokeFailure(f"turn lease outlives its TTL after the exit: {row}")
        if expires_at >= now:
            raise SmokeFailure(f"turn lease has not lapsed before restart: {row}")
        records.append(
            {
                "holder": row["holder"],
                "expired_secs_before_restart": round(now - expires_at, 3),
                "expires_secs_after_exit": round(expires_at - exited_at, 3),
            }
        )
    return records


def require_settled_out_of_inbox(
    inbox: dict[str, Any], *, message_ids: dict[str, str]
) -> dict[str, Any]:
    """After the replies, both entries are acked and nothing is left leased."""
    problems = []
    for role, message_id in message_ids.items():
        if inbox_event_by_message_id(inbox, message_id):
            problems.append(f"{role} still in inbox")
        elif not inbox_message_acked(inbox, message_id):
            problems.append(f"{role} missing from the acked ring")
    leased = [
        event.get("seq")
        for event in inbox.get("events") or []
        if isinstance(event, dict) and lease_state(event) == "leased"
    ]
    if leased:
        problems.append(f"leased seqs {leased}")
    if problems:
        raise SmokeFailure("inbox not settled after replies: " + "; ".join(problems))
    return {"acked": sorted(message_ids), "leased_entries": 0}


def require_restart_order(
    requests: list[dict[str, Any]], *, active_marker: str, queued_expected: str
) -> dict[str, Any]:
    """The queued follow-up reaches the model exactly once, after any rerun
    of the interrupted turn, and the interrupted turn reruns at most once."""
    active = [
        index
        for index, request in enumerate(requests)
        if active_marker in str(request.get("latest_user_text") or "")
    ]
    queued = [
        index
        for index, request in enumerate(requests)
        if queued_expected in str(request.get("latest_user_text") or "")
    ]
    if len(queued) != 1:
        raise SmokeFailure(f"queued follow-up reached the model {len(queued)} times")
    if len(active) > 1:
        raise SmokeFailure(f"interrupted turn reran {len(active)} times after restart")
    if active and active[0] > queued[0]:
        raise SmokeFailure("queued follow-up ran before the interrupted turn's rerun")
    return {"active_reruns": len(active), "queued_handoffs": len(queued)}


def require_acks_after_handoff(
    requests: list[dict[str, Any]],
    inbox: dict[str, Any],
    *,
    message_ids: dict[str, str],
    markers: dict[str, str],
) -> dict[str, int]:
    """Each entry the stop left durable reaches the model after restart, then is acked.

    Run 37428809877 acked both entries about 2s before any model request: the
    startup-restore gate had only queued them in memory. Returns how long
    after its handoff each entry was acked.
    """
    after_handoff_ms: dict[str, int] = {}
    for role, message_id in message_ids.items():
        acked_at = [
            ack_time
            for entry in inbox.get("acked") or []
            if isinstance(entry, dict)
            and inbox_key_message_id(entry.get("key")) == message_id
            and isinstance(ack_time := entry.get("acked_at_ms"), int)
        ]
        if not acked_at:
            raise SmokeFailure(f"{role} turn has no ack time in the acked ring")
        handoffs = [
            int(request.get("received_at_ms") or 0)
            for request in requests
            if markers[role] in str(request.get("latest_user_text") or "")
        ]
        if not handoffs:
            raise SmokeFailure(f"{role} turn was acked without reaching the model after restart")
        lead_ms = min(acked_at) - handoffs[0]
        if lead_ms < 0:
            raise SmokeFailure(f"{role} turn was acked {-lead_ms} ms before it reached the model")
        after_handoff_ms[role] = lead_ms
    return after_handoff_ms


def read_hermes_inbox(*, image: str, container: str, home_volume: str, live: bool) -> str:
    """Read the inbox without mutating it: exec into a live Agent, else mount read-only."""
    if live:
        command = ["docker", "exec", container, "cat", HERMES_INBOX_PATH]
    else:
        # `docker run --mount` would create a missing volume; never do that here.
        inspected = smoke.run(["docker", "volume", "inspect", home_volume], check=False, timeout=30)
        if inspected.returncode != 0:
            raise SmokeFailure(f"home volume {home_volume} does not exist")
        command = [
            "docker",
            "run",
            "--rm",
            "--network",
            "none",
            "--entrypoint",
            "cat",
            "--mount",
            f"type=volume,src={home_volume},dst={AGENT_HOME_MOUNT},readonly",
            image,
            HERMES_INBOX_PATH,
        ]
    result = smoke.run(command, check=False, timeout=60)
    if result.returncode != 0:
        raise SmokeFailure(f"inbox read exited {result.returncode}: {result.stderr[-500:]}")
    return result.stdout


def capture_diagnostics(
    point: str,
    *,
    image: str,
    container: str,
    home_volume: str,
    live: bool,
    model_state: FakeModelState,
) -> dict[str, Any]:
    """Best-effort snapshot; every part records its own error and never raises."""
    now_ms = int(time.time() * 1000)
    snapshot: dict[str, Any] = {"point": point, "captured_at_ms": now_ms}
    try:
        raw = read_hermes_inbox(
            image=image, container=container, home_volume=home_volume, live=live
        )
        snapshot["hermes_inbox"] = summarize_hermes_inbox(json.loads(raw), now_ms=now_ms)
    except Exception as exc:
        snapshot["hermes_inbox_error"] = bounded_text(f"{type(exc).__name__}: {exc}", 1000)
    try:
        snapshot["agent_log_tail"] = bounded_text(
            smoke.agent_log_tail(container, lines=DIAGNOSTIC_LOG_LINES)
        )
    except Exception as exc:
        snapshot["agent_log_error"] = bounded_text(f"{type(exc).__name__}: {exc}", 1000)
    try:
        snapshot["provider"] = model_state.summary()
    except Exception as exc:
        snapshot["provider_error"] = bounded_text(f"{type(exc).__name__}: {exc}", 1000)
    return snapshot


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
    server_bin = resolve_server_binary(args.server_bin)
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
            "lease_ttl_ms": PRODUCTION_LEASE_TTL_MS,
            "graceful_stop_cycles": GRACEFUL_STOP_CASES,
            "sigkill_lease_expiry": (
                "simulated: the two known leases are backdated past the production TTL "
                "on the stopped synthetic volume after a proven SIGKILL"
            ),
            "sigkill_hermes_turn_lease": (
                "waited in real time: the pinned Hermes turn lease TTL plus a margin "
                "after the proven SIGKILL, read-only state.db check before restart"
            ),
            "production_kata_task_and_stable_manifest_gate": False,
        },
    }

    report["image_id"] = smoke.run(
        ["docker", "image", "inspect", "--format", "{{.Id}}", image], timeout=30
    ).stdout.strip()

    def write_report() -> None:
        report_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")

    def set_stage(stage: str, case: dict[str, Any] | None = None) -> None:
        if case is not None:
            case["stage"] = stage
            stage = f"{case['name']}/{stage}"
        report["stage"] = stage
        write_report()

    def diagnose(point: str, *, live: bool) -> dict[str, Any]:
        return capture_diagnostics(
            point,
            image=image,
            container=name,
            home_volume=home_volume,
            live=live,
            model_state=model_state,
        )

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
            ["docker", "exec", name, "hermes", "--version"],
            timeout=30,
        )
        # Recorded as evidence only; the image↔pin equality is asserted where
        # the lock-derived stamp lives (Dockerfile build check and the
        # runtime-image workflow's in-container assert), never as a literal.
        report["coverage"]["real_hermes"] = parse_hermes_version(version_result.stdout)
        ttl_override = smoke.run(
            ["docker", "exec", name, "printenv", LEASE_TTL_ENV], check=False, timeout=30
        )
        if ttl_override.returncode == 0:
            raise SmokeFailure(f"the Agent must run the production lease TTL, not {LEASE_TTL_ENV}")
        return health

    def interrupt(case_name: str, *, kill: bool, restore: bool) -> None:
        case: dict[str, Any] = {
            "name": case_name,
            "signal": "SIGKILL" if kill else "SIGTERM",
            "empty_target_restore": restore,
            "status": "running",
            "diagnostics": [],
        }
        report["cases"].append(case)
        try:
            interrupt_case(case, case_name, kill=kill, restore=restore)
        except Exception:
            case["status"] = "failed"
            raise

    def interrupt_case(case: dict[str, Any], case_name: str, *, kill: bool, restore: bool) -> None:
        set_stage("send_stalled_turn", case)
        active_marker = f"FINITE_INTERRUPT_STALL:{case_name}"
        prompt = f"{active_marker} keep this turn open"
        active_sent = user_app(
            image=image,
            volume=user_volume,
            server_url=server_url,
            args=["send", "--room-id", room_id, "--text", prompt],
            env=env,
        )
        set_stage("wait_stall_seen", case)
        model_state.wait_seen(case_name)
        set_stage("send_queued", case)
        queued_expected = f"{case_name} queued follow-up ok"
        queued_prompt = f"Reply with exactly: {queued_expected}"
        queued_sent = user_app(
            image=image,
            volume=user_volume,
            server_url=server_url,
            args=["send", "--room-id", room_id, "--text", queued_prompt],
            env=env,
        )
        queued_message_id = smoke.first_matching_mine_message_id(queued_sent, queued_prompt)
        case["queued_prompt_message_id"] = queued_message_id
        message_ids = {
            "active": smoke.first_matching_mine_message_id(active_sent, prompt),
            "queued": queued_message_id,
        }
        case["active_prompt_message_id"] = message_ids["active"]
        set_stage("wait_durable_inbox", case)
        wait_durable_inbox_event(name, queued_message_id)
        if any(
            queued_expected in str(request.get("latest_user_text") or "")
            for request in model_state.requests
        ):
            raise SmokeFailure(
                f"{case_name} follow-up reached Hermes before the active turn released"
            )
        case.update(
            {
                "queued_before_restart": {
                    "prompt_message_id": queued_message_id,
                    "durable_unacked": True,
                    "handed_to_model": False,
                },
            }
        )
        case["diagnostics"].append(diagnose("before_stop", live=True))
        set_stage("check_streaming_path", case)
        case["provider_stream_barrier"] = model_state.require_stream_in_flight(case_name)
        case["provider_stream_in_flight"] = True
        set_stage("stop_container", case)
        if kill:
            smoke.run(["docker", "kill", "--signal", "KILL", name], timeout=30)
        else:
            smoke.run(["docker", "stop", "--time", "15", name], timeout=30)
        exit_code = int(
            smoke.run(
                ["docker", "inspect", "--format", "{{.State.ExitCode}}", name], timeout=30
            ).stdout.strip()
        )
        exited_monotonic, exited_at = time.monotonic(), time.time()
        if kill and exit_code != 137:
            raise SmokeFailure(f"{case_name} exited {exit_code}, expected SIGKILL exit 137")
        if not kill and exit_code == 137:
            raise SmokeFailure(f"{case_name} escalated to SIGKILL instead of stopping gracefully")
        case["container_exit_code"] = exit_code
        model_state.release(case_name)
        case["diagnostics"].append(diagnose("before_container_removal", live=False))
        if not kill:
            set_stage("check_stopped_inbox_released", case)
            stopped = json.loads(
                read_hermes_inbox(image=image, container=name, home_volume=home_volume, live=False)
            )
            case["stopped_inbox"] = {
                "lease_states": require_graceful_inbox_released(stopped, message_ids=message_ids),
                "leased_entries": 0,
            }
        set_stage("remove_container", case)
        smoke.docker_container_rm(name)
        if kill:
            set_stage("simulate_lease_expiry", case)
            case["simulated_lease_expiry"] = simulate_stopped_lease_expiry(case_name, message_ids)
            set_stage("wait_hermes_turn_lease_expiry", case)
            case["hermes_turn_lease_expiry"] = wait_stopped_turn_lease_expiry(
                exited_monotonic, exited_at
            )
        if restore:
            set_stage("archive_and_restore", case)
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
        set_stage("restart_agent", case)
        restart_request_index = len(model_state.requests)
        health = start_agent()
        status = smoke.wait_agent_room_connected(name, room_id, server_url)
        if health.get("npub") != agent_npub or status.get("room_id") != room_id:
            raise SmokeFailure(f"{case_name} changed the Agent identity or room")
        case["diagnostics"].append(diagnose("after_restart", live=True))
        set_stage("wait_queued_reply", case)
        queued_reply = wait_existing_reply(
            image=image,
            volume=user_volume,
            server_url=server_url,
            room_id=room_id,
            prompt_message_id=queued_message_id,
            expected=queued_expected,
            env=env,
        )
        queued_handoffs = sum(
            queued_expected in str(request.get("latest_user_text") or "")
            for request in model_state.requests
        )
        case["queued_model_handoffs_observed"] = queued_handoffs
        if queued_handoffs != 1:
            raise SmokeFailure(
                f"{case_name} queued follow-up reached the model {queued_handoffs} times"
            )
        case["restart_order"] = require_restart_order(
            model_state.requests[restart_request_index:],
            active_marker=active_marker,
            queued_expected=queued_expected,
        )
        case["queued_after_restart"] = {
            **queued_reply,
            "first_next_ordinary_turn": True,
            "model_handoffs": queued_handoffs,
        }
        set_stage("fresh_turns", case)
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
        set_stage("wait_inbox_settled", case)
        case["final_inbox"] = wait_inbox_settled(message_ids)
        case["restart_order"] = require_restart_order(
            model_state.requests[restart_request_index:],
            active_marker=active_marker,
            queued_expected=queued_expected,
        )
        set_stage("check_acks_after_handoff", case)
        case["acked_after_handoff_ms"] = require_acks_after_handoff(
            model_state.requests[restart_request_index:],
            json.loads(
                read_hermes_inbox(image=image, container=name, home_volume=home_volume, live=True)
            ),
            message_ids=message_ids,
            markers={"active": active_marker, "queued": queued_expected},
        )
        case["status"] = "passed"
        case.pop("stage", None)
        write_report()

    def simulate_stopped_lease_expiry(
        case_name: str, message_ids: dict[str, str]
    ) -> dict[str, Any]:
        original_raw = read_hermes_inbox(
            image=image, container=name, home_volume=home_volume, live=False
        )
        simulated, records = simulate_lease_expiry(
            json.loads(original_raw), message_ids=message_ids, now_ms=int(time.time() * 1000)
        )
        # The untouched inbox stays on the host for the run; only digests go
        # into the report because entries carry message text.
        original_copy = state_dir / f"{case_name}-hermes-inbox.original.json"
        original_copy.write_text(original_raw)
        fixture = state_dir / f"{case_name}-hermes-inbox.simulated.json"
        fixture_text = json.dumps(simulated, indent=2)
        fixture.write_text(fixture_text)
        write_stopped_inbox(
            image=image,
            home_volume=home_volume,
            expected_sha256=sha256_text(original_raw),
            fixture=fixture,
        )
        written = read_hermes_inbox(
            image=image, container=name, home_volume=home_volume, live=False
        )
        if json.loads(written) != simulated or sha256_text(written) != sha256_text(fixture_text):
            raise SmokeFailure(f"{case_name} expiry fixture did not land as written")
        return {
            "applied_after": "SIGKILL exit 137 and container removal",
            "production_ttl_ms": PRODUCTION_LEASE_TTL_MS,
            "margin_ms": SIMULATED_EXPIRY_MARGIN_MS,
            "entries": records,
            "original_sha256": sha256_text(original_raw),
            "simulated_sha256": sha256_text(fixture_text),
            "original_copy": str(original_copy),
        }

    def wait_stopped_turn_lease_expiry(exited_monotonic: float, exited_at: float) -> dict[str, Any]:
        before = read_stopped_turn_leases(image=image, home_volume=home_volume)
        waited = wait_turn_lease_expiry(exited_monotonic)
        after = read_stopped_turn_leases(image=image, home_volume=home_volume)
        leases = require_turn_leases_lapsed(before, after, exited_at=exited_at, now=time.time())
        return {**waited, "leases": leases}

    def wait_inbox_settled(message_ids: dict[str, str]) -> dict[str, Any]:
        deadline = time.monotonic() + FINAL_ACK_TIMEOUT_SECS
        while True:
            inbox = json.loads(
                read_hermes_inbox(image=image, container=name, home_volume=home_volume, live=True)
            )
            try:
                return require_settled_out_of_inbox(inbox, message_ids=message_ids)
            except SmokeFailure:
                if time.monotonic() >= deadline:
                    raise
            time.sleep(0.5)

    try:
        set_stage("start_finitechat_server")
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

        set_stage("start_agent")
        health = start_agent()
        agent_npub = str(health.get("npub") or "")
        account_id = health.get("account_id")
        if not agent_npub or not isinstance(account_id, str):
            raise SmokeFailure(f"agent health omitted identity: {health}")
        set_stage("create_room")
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

        for graceful_case in GRACEFUL_STOP_CASES:
            interrupt(graceful_case, kill=False, restore=False)
        interrupt("sigkill", kill=True, restore=False)
        interrupt("empty-target-restore", kill=False, restore=True)
        # Each case proves its own still-open SSE barrier immediately before stop.
        # Later non-streaming requests can legitimately retain its marker in history.
        report["provider_request_count"] = len(model_state.requests)
        report["status"] = "passed"
        report.pop("stage", None)
        write_report()
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0
    except Exception as exc:
        report["status"] = "failed"
        report["failure"] = str(exc)
        report["finitechat_server_log"] = (
            server_log.read_text(errors="replace")[-4000:] if server_log.exists() else ""
        )
        report["failure_stage"] = report.get("stage")
        report["failure_diagnostics"] = diagnose("on_failure", live=False)
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

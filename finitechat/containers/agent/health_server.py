#!/usr/bin/env python3
"""Small runtime health endpoint for container/Tinfoil probes."""

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
INVITE_FILE = AGENT_HOME / "current-invite.json"
ROOM_NAME = os.environ.get(
    "FINITECHAT_HERMES_ROOM_NAME",
    os.environ.get("FINITE_AGENT_NAME", "Finite Agent"),
)
# Hosted pairing is no-PIN: possession of the invite URL is admission, and
# this endpoint is public. Single-use is a product invariant, not config.
INVITE_MAX_JOINS = 1
# One hour covers a live onboarding session (user is at the dashboard right
# now) while bounding how long a scraped URL stays joinable. Expired
# unconsumed invites re-mint automatically, so a short TTL costs nothing.
INVITE_TTL_MS = int(os.environ.get("FINITE_AGENT_INVITE_TTL_MS", str(60 * 60 * 1000)))


def identity() -> dict[str, Any]:
    config_path = AGENT_HOME / "config.json"
    if not config_path.exists():
        return {"ready": False, "error": "agent home is not initialized"}
    try:
        proc = subprocess.run(
            [
                FINITECHAT_BIN,
                "auth",
                "status",
            ],
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


def finitechat_json(args: list[str]) -> dict[str, Any]:
    proc = subprocess.run(
        [FINITECHAT_BIN, *args],
        capture_output=True,
        check=True,
        text=True,
        timeout=10,
    )
    return json.loads(proc.stdout)


def read_cached_invite() -> dict[str, Any] | None:
    if not INVITE_FILE.exists():
        return None
    return json.loads(INVITE_FILE.read_text(encoding="utf-8"))


def write_cached_invite(payload: dict[str, Any]) -> None:
    INVITE_FILE.write_text(json.dumps(payload, indent=2), encoding="utf-8")


def mint_invite(room_id: str | None) -> dict[str, Any]:
    args = ["hermes", "--agent-home", str(AGENT_HOME), "invite"]
    if room_id is None:
        args.extend(["--room-name", ROOM_NAME])
    else:
        args.extend(["--room-id", room_id])
    args.extend(
        [
            "--max-joins",
            str(INVITE_MAX_JOINS),
            "--ttl-ms",
            str(INVITE_TTL_MS),
            "--json",
        ]
    )
    return finitechat_json(args)


def invite_status(url: str) -> dict[str, Any]:
    return finitechat_json(["hermes", "invite-status", "--url", url, "--json"])


def invite_base_payload(
    identity_payload: dict[str, Any], invite_payload: dict[str, Any]
) -> dict[str, Any]:
    return {
        "ready": True,
        "agent_npub": identity_payload.get("npub"),
        "account_id": identity_payload.get("account_id"),
        "room_id": invite_payload.get("room_id"),
        "invite_id": invite_payload.get("invite_id"),
    }


def paired_payload(
    identity_payload: dict[str, Any], invite_payload: dict[str, Any]
) -> dict[str, Any]:
    # No url: the dashboard treats a missing url as not-serving-an-invite,
    # and paired tells it why.
    payload = invite_base_payload(identity_payload, invite_payload)
    payload["paired"] = True
    payload["invite_state"] = "consumed"
    return payload


def open_payload(
    identity_payload: dict[str, Any],
    invite_payload: dict[str, Any],
    invite_state: str,
    expires_at_ms: int | None,
) -> dict[str, Any]:
    payload = invite_base_payload(identity_payload, invite_payload)
    payload["paired"] = False
    payload["invite_state"] = invite_state
    # url present means "joinable invite being served"; keep it absent (not
    # null) otherwise so consumers can key off the field existing.
    url = invite_payload.get("url")
    if url is not None:
        payload["url"] = url
    if expires_at_ms is not None:
        payload["expires_at_ms"] = expires_at_ms
    return payload


def invite() -> dict[str, Any]:
    identity_payload = identity()
    if not identity_payload.get("ready"):
        return identity_payload
    try:
        invite_payload = read_cached_invite()
        if invite_payload is None:
            invite_payload = mint_invite(room_id=None)
            write_cached_invite(invite_payload)
        if invite_payload.get("paired"):
            return paired_payload(identity_payload, invite_payload)
        try:
            status = invite_status(invite_payload["url"])
        except Exception:
            # The status probe failed (for example the room server is
            # unreachable). Serving the cached URL cannot re-open admission:
            # a consumed single-use invite is rejected server-side.
            return open_payload(
                identity_payload, invite_payload, invite_state="unknown", expires_at_ms=None
            )
        if status.get("consumed"):
            # Someone paired. Persist that verdict so /invite never re-mints
            # for this room, even if the server later forgets the session.
            invite_payload["paired"] = True
            write_cached_invite(invite_payload)
            return paired_payload(identity_payload, invite_payload)
        if status.get("expired"):
            # Nobody paired in time: mint a fresh invite for the same room.
            invite_payload = mint_invite(room_id=invite_payload.get("room_id"))
            write_cached_invite(invite_payload)
            try:
                status = invite_status(invite_payload["url"])
            except Exception:
                return open_payload(
                    identity_payload,
                    invite_payload,
                    invite_state="unknown",
                    expires_at_ms=None,
                )
        if status.get("state") == "not_found":
            # Unjoinable, but nothing proves it was consumed: fail closed
            # with no url instead of guessing (re-inviting is explicit).
            payload = invite_base_payload(identity_payload, invite_payload)
            payload["paired"] = False
            payload["invite_state"] = "not_found"
            return payload
        return open_payload(
            identity_payload,
            invite_payload,
            invite_state=str(status.get("state")),
            expires_at_ms=status.get("expires_at_ms"),
        )
    except Exception as exc:
        return {"ready": False, "error": f"invite unavailable: {exc}"}


class Handler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        if self.path == "/healthz":
            payload = identity()
            self._write(200 if payload["ready"] else 503, payload)
            return
        if self.path == "/invite":
            payload = invite()
            self._write(200 if payload["ready"] else 503, payload)
            return
        else:
            self._write(404, {"ready": False, "error": "not found"})
            return

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

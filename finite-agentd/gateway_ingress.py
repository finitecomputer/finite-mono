"""Authenticated, byte-preserving ingress to the runtime's Hermes listener.

The public proxy supplies the locally attributed principal on every connection.
Health/contact do not call this module unless the gateway prefix is requested.
"""

import hmac
import json
import select
import socket
import threading
from urllib.parse import parse_qs, urlsplit

SLOTS = threading.BoundedSemaphore(32)


def authorized(config, headers, query):
    if not isinstance(config, dict):
        return False
    expected = config.get("token") if config.get("enabled") is True else None
    if not isinstance(expected, str) or len(expected) != 64:
        return False
    values = [headers.get("X-Hermes-Session-Token", "")]
    authorization = headers.get("Authorization", "")
    if authorization.startswith("Bearer "):
        values.append(authorization[7:])
    values.extend(parse_qs(query).get("token", []))
    return any(hmac.compare_digest(value.encode(), expected.encode()) for value in values)


def forward(handler, agent_home):
    handler.close_connection = True
    path = urlsplit(handler.path)
    try:
        with (agent_home / "agentd/hosted-gateway.json").open() as config_file:
            config = json.loads(config_file.read(8192))
        with (agent_home / "config.json").open() as identity_file:
            principal = json.loads(identity_file.read(8192))["account_id"]
    except (OSError, ValueError, KeyError, TypeError):
        config = {}
        principal = None
    if not principal or handler.headers.get("X-Finite-Agent-Account-Id") != principal:
        handler.send_error(421, "Agent identity does not match")
        return
    if not authorized(config, handler.headers, path.query):
        handler.send_error(401, "Gateway access is disabled or the credential is invalid")
        return
    if not SLOTS.acquire(blocking=False):
        handler.send_error(503, "Gateway connection capacity reached")
        return
    try:
        try:
            upstream = socket.create_connection(("127.0.0.1", 9120), timeout=3)
        except OSError:
            handler.send_error(503, "Gateway is starting or unavailable")
            return
        with upstream:
            target = handler.path[len("/gateway") :]
            # The backend sees its original route, including its query string.
            # Suppress internal attribution and proxy-supplied forwarding headers.
            headers = [
                (key, value)
                for key, value in handler.headers.items()
                if key.lower()
                not in {
                    "connection",
                    "host",
                    "origin",
                    "authorization",
                    "x-hermes-session-token",
                    "x-finite-agent-account-id",
                    "forwarded",
                }
                and not key.lower().startswith("x-forwarded-")
            ]
            upgrade = handler.headers.get("Upgrade", "").lower() == "websocket"
            # Only after outer credential and identity checks, act as the
            # authenticated local client of Hermes' loopback-only listener.
            headers.extend(
                [
                    ("Host", "127.0.0.1:9120"),
                    ("X-Hermes-Session-Token", config["token"]),
                    ("Connection", "Upgrade" if upgrade else "close"),
                ]
            )
            head = f"{handler.command} {target} HTTP/1.1\r\n"
            head += "".join(f"{key}: {value}\r\n" for key, value in headers) + "\r\n"
            upstream.sendall(head.encode("latin-1"))
            upstream.settimeout(30)
            handler.connection.settimeout(30)
            # Handler uses rbufsize=0, so request bodies are still on the socket.
            # Copy the response headers and any immediately following WS frames
            # together; an HTTPResponse parser would buffer the first event.
            while True:
                ready, _, _ = select.select([handler.connection, upstream], [], [], 60)
                if not ready:
                    continue
                for source in ready:
                    data = source.recv(65536)
                    if not data:
                        return
                    (upstream if source is handler.connection else handler.connection).sendall(data)
    except (OSError, ValueError, KeyError, TypeError):
        # The response may already be streaming. Close without writing a second
        # HTTP response or logging a credential-bearing URL.
        return
    finally:
        SLOTS.release()

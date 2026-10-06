"""Point-in-time Chat turn idleness of one Kata Agent from its durable host root.

Reads only `<state_root>/agent` (the guest's `/data/agent`):

- `hermes-home/gateway_state.json`: Hermes `gateway_state` and `active_agents`,
  rewritten at every turn start and end (`_persist_active_agents`).
- `hermes-inbox.json` events: `lease.state` `pending` (queued; an absent lease
  loads as pending) or `leased` (in flight).
- `agentd-inbox.json` events and `hermes-running.json` messages.

The finitechat loaders default the three inbox/marker files to empty on
NotFound, so absence reads as empty only beneath a valid root with a running
gateway record. Idle requires `gateway_state == "running"`, `active_agents == 0`,
both inboxes empty and no running markers. Anything malformed, oversized,
symlinked, unreadable or not a regular file is unknown, never idle.

Only counts and ages are emitted: never Chat text, room, message or entry
ids, lease ids, or credentials. The observation is not atomic with any later
stop; a message that arrives after it can still be in flight at stop time.
"""
from __future__ import annotations

import errno
import json
import os
import stat
from datetime import datetime
from pathlib import Path
from typing import Any

MAX_STATE_BYTES = 32 * 1024 * 1024
GATEWAY_STATES = frozenset({"starting", "running", "degraded", "draining", "stopping",
                            "stopped", "startup_failed"})
BUSY_REASONS = ("active_agents", "inbox_pending", "inbox_leased", "agentd_inbox")
MISSING = object()  # an absent file; distinct from a parsed JSON null


class Unreadable(ValueError):
    """A fixed reason code; never carries file content or paths."""


def _open_dir(name: str, parent: int | None) -> int:
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    try:
        return os.open(name, flags, dir_fd=parent)
    except FileNotFoundError:
        raise Unreadable("missing") from None
    except OSError as error:
        raise Unreadable("symlink" if error.errno in (errno.ELOOP, errno.ENOTDIR) else "unreadable") from None


def _read_json(directory: int, name: str) -> Any:
    """Return parsed JSON, or MISSING when the file is absent."""
    try:
        fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
    except FileNotFoundError:
        return MISSING
    except OSError as error:
        raise Unreadable("symlink" if error.errno == errno.ELOOP else "unreadable") from None
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode):
            raise Unreadable("not_regular")
        if info.st_size > MAX_STATE_BYTES:
            raise Unreadable("oversize")
        chunks, size = [], 0
        while chunk := os.read(fd, 1024 * 1024):
            size += len(chunk)
            if size > MAX_STATE_BYTES:
                raise Unreadable("oversize")
            chunks.append(chunk)
    except OSError:
        raise Unreadable("unreadable") from None
    finally:
        os.close(fd)
    try:
        return json.loads(b"".join(chunks))
    except (ValueError, UnicodeDecodeError):
        raise Unreadable("malformed") from None


def _count(value: Any) -> int:
    if type(value) is not int or value < 0:
        raise Unreadable("malformed")
    return value


def _list(document: Any, key: str, **defaults: type) -> list[Any]:
    """Mirror the finitechat serde loaders: an absent file is empty, `{}` is
    empty (every field is serde(default)) and unknown fields are ignored, but
    a present null or wrongly typed field fails the whole load."""
    if document is MISSING:
        return []
    if not isinstance(document, dict):
        raise Unreadable("malformed")
    if "cursors" in defaults:
        cursors = document.get("cursors", {})
        if not isinstance(cursors, dict) or not all(
                type(value) is int and value >= 0 for value in cursors.values()):
            raise Unreadable("malformed")
    if "acked" in defaults:
        acked = document.get("acked", [])
        if not isinstance(acked, list) or not all(
                isinstance(item, dict) and isinstance(item.get("key"), str)
                and type(item.get("acked_at_ms")) is int and item["acked_at_ms"] >= 0
                for item in acked):
            raise Unreadable("malformed")
    value = document.get(key, [])
    if not isinstance(value, list) or not all(isinstance(item, dict) for item in value):
        raise Unreadable("malformed")
    return value


def _age_s(now_ms: int, then_ms: int) -> int:
    return max(0, (now_ms - then_ms) // 1000)


def _gateway(document: Any, now_ms: int) -> dict[str, Any]:
    if document is MISSING:
        raise Unreadable("missing")
    if not isinstance(document, dict) or not isinstance(document.get("gateway_state"), str):
        raise Unreadable("malformed")
    state = document["gateway_state"]
    updated_age_s = None
    try:
        updated = datetime.fromisoformat(document["updated_at"].replace("Z", "+00:00"))
        if updated.tzinfo is not None:
            updated_age_s = _age_s(now_ms, int(updated.timestamp() * 1000))
    except (KeyError, AttributeError, ValueError, TypeError):
        pass  # informational only: an idle gateway need not rewrite this
    return {"state": state if state in GATEWAY_STATES else "unrecognized",
            "active_agents": _count(document.get("active_agents")),
            "updated_age_s": updated_age_s}


def _hermes_inbox(document: Any, now_ms: int) -> dict[str, Any]:
    pending: list[int] = []
    leased: list[int] = []
    for event in _list(document, "events", cursors=dict, acked=list):
        lease = event.get("lease", {"state": "pending"})
        if not isinstance(lease, dict):
            raise Unreadable("malformed")
        if lease.get("state") == "pending":
            pending.append(_age_s(now_ms, _count(event.get("created_at_ms"))))
        elif lease.get("state") == "leased" and isinstance(lease.get("lease_id"), str):
            leased.append(_age_s(now_ms, _count(lease.get("leased_at_ms"))))
        else:
            raise Unreadable("unknown_lease")
    return {"present": document is not MISSING, "pending": len(pending), "leased": len(leased),
            "oldest_pending_age_s": max(pending, default=None),
            "oldest_lease_age_s": max(leased, default=None),
            "newest_lease_age_s": min(leased, default=None)}


def observe(state_root: Path, now_ms: int) -> dict[str, Any]:
    """Read and classify one validated `<work_root>/kata/<runtime>` root."""
    reasons: list[str] = []
    result: dict[str, Any] = {"gateway": None, "hermes_inbox": None,
                              "agentd_inbox": None, "running_markers": None}
    root = agent = home = None
    try:
        try:
            root = _open_dir(str(state_root), None)
            agent = _open_dir("agent", root)
            home = _open_dir("hermes-home", agent)
        except Unreadable as error:
            return {**result, "verdict": "unknown", "reasons": [f"agent_root_{error}"]}
        readers = (
            ("gateway", home, "hermes-home/gateway_state.json", _gateway),
            ("hermes_inbox", agent, "hermes-inbox.json", _hermes_inbox),
            ("agentd_inbox", agent, "agentd-inbox.json",
             lambda doc, _: {"present": doc is not MISSING, "events": len(_list(doc, "events", cursors=dict))}),
            ("running_markers", agent, "hermes-running.json",
             lambda doc, _: {"present": doc is not MISSING, "messages": len(_list(doc, "messages"))}),
        )
        for key, directory, relative, parse in readers:
            try:
                result[key] = parse(_read_json(directory, Path(relative).name), now_ms)
            except Unreadable as error:
                reasons.append(f"{key}_{error}")
    finally:
        for fd in (home, agent, root):
            if fd is not None:
                os.close(fd)
    return {**result, **classify(result, reasons)}


def classify(result: dict[str, Any], reasons: list[str]) -> dict[str, Any]:
    gateway = result["gateway"]
    if gateway is not None and gateway["state"] != "running":
        reasons.append("gateway_not_running")
    signals = {
        "active_agents": gateway and gateway["active_agents"],
        "inbox_pending": result["hermes_inbox"] and result["hermes_inbox"]["pending"],
        "inbox_leased": result["hermes_inbox"] and result["hermes_inbox"]["leased"],
        "agentd_inbox": result["agentd_inbox"] and result["agentd_inbox"]["events"],
        "running_markers": result["running_markers"] and result["running_markers"]["messages"],
    }
    reasons += [name for name, value in signals.items() if value]
    if any(reason not in signals for reason in reasons):
        verdict = "unknown"
    elif any(reason in BUSY_REASONS for reason in reasons):
        verdict = "busy"
    elif reasons:
        verdict = "unfinished_markers"
    else:
        verdict = "idle"
    return {"verdict": verdict, "reasons": reasons}

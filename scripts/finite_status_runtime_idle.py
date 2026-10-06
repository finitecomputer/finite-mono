"""Point-in-time Chat turn idleness of one Kata Agent from its durable host root.

Reads only `<state_root>/agent` (the guest's `/data/agent`):

- `hermes-home/gateway_state.json`: Hermes `gateway_state` and `active_agents`,
  rewritten at every turn start and end (`_persist_active_agents`).
- `hermes-inbox.json` events: `lease.state` `pending` (queued; an absent lease
  loads as pending) or `leased` (in flight).
- `agentd-inbox.json` events and `hermes-running.json` messages.
- `hermes-inbox.json` `acked` ring: the newest `acked_at_ms`. A /bg or /btw
  command entry is acked when its child is launched, so the child is visible
  to neither `active_agents` nor the inbox; a recent ack reads busy.
- `hermes-home/gateway.pid` mtime (created O_EXCL once per gateway process)
  and a private scratch copy of `hermes-home/state.db` (+ `-wal`, never
  `-shm`): /bg children run an AIAgent with session id `bg_*` whose row stays
  `ended_at IS NULL` until the child finishes, then is ended just before the
  result is delivered. Open rows started before the current gateway process
  are dead children of an earlier process and only reported.

The finitechat loaders default the three inbox/marker files to empty on
NotFound, so absence reads as empty only beneath a valid root with a running
gateway record. Idle requires `gateway_state == "running"`, `active_agents == 0`,
both inboxes empty, no running markers, no inbox entry acked within
QUIET_AFTER_ACK_S, no open `bg_*` session of the current gateway process and
none ended within BACKGROUND_DELIVERY_S. Anything malformed, oversized,
symlinked, unreadable or not a regular file is unknown, never idle.

Only counts and ages are emitted: never Chat text, room, message or entry
ids, lease ids, or credentials. The observation is not atomic with any later
stop; a message that arrives after it can still be in flight at stop time.
"""
from __future__ import annotations

import errno
import json
import os
import shutil
import stat
import tempfile
from datetime import datetime
from pathlib import Path
from typing import Any

MAX_STATE_BYTES = 32 * 1024 * 1024
MAX_SESSION_DB_BYTES = 4 * 1024 * 1024 * 1024
# A /btw one-shot answer is bounded at 180 s and one Finite Private request at
# ~720 s by the limiter; a /bg child is covered by its session row instead.
QUIET_AFTER_ACK_S = 30 * 60
# A /bg row is ended before its result is sent (30 s per send).
BACKGROUND_DELIVERY_S = 5 * 60
GATEWAY_STATES = frozenset({"starting", "running", "degraded", "draining", "stopping",
                            "stopped", "startup_failed"})
BUSY_REASONS = ("active_agents", "inbox_pending", "inbox_leased", "agentd_inbox",
                "recent_ack", "background_sessions")
MISSING = object()  # an absent file; distinct from a parsed JSON null
COPY_CHUNK_BYTES = 1024 * 1024
BACKGROUND_SQL = """
SELECT COALESCE(SUM(ended_at IS NULL AND started_at >= ?1), 0),
       COALESCE(SUM(ended_at IS NULL AND started_at < ?1), 0),
       COALESCE(SUM(ended_at IS NOT NULL AND ended_at >= ?2), 0)
FROM sessions WHERE id GLOB 'bg_*'
"""


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
    acked = [_age_s(now_ms, item["acked_at_ms"])
             for item in ([] if document is MISSING else document.get("acked", []))]
    return {"present": document is not MISSING, "pending": len(pending), "leased": len(leased),
            "oldest_pending_age_s": max(pending, default=None),
            "oldest_lease_age_s": max(leased, default=None),
            "newest_lease_age_s": min(leased, default=None),
            "newest_ack_age_s": min(acked, default=None)}


def _identity(info: os.stat_result) -> tuple[int, int, int]:
    return info.st_ino, info.st_size, info.st_mtime_ns


def _copy_stable(directory: int, name: str, target: Path, required: bool) -> tuple[int, int, int] | None:
    """Copy one file beneath `directory` and return the (ino, size, mtime_ns)
    it was stable at; None when an optional file is absent."""
    try:
        fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
    except FileNotFoundError:
        if required:
            raise Unreadable("missing") from None
        return None
    except OSError as error:
        raise Unreadable("symlink" if error.errno == errno.ELOOP else "unreadable") from None
    try:
        before = os.fstat(fd)
        if not stat.S_ISREG(before.st_mode):
            raise Unreadable("not_regular")
        if before.st_size > MAX_SESSION_DB_BYTES:
            raise Unreadable("oversize")
        out = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        try:
            while chunk := os.read(fd, COPY_CHUNK_BYTES):
                pending = memoryview(chunk)
                while pending:  # a write may be short, e.g. when the temp filesystem fills
                    written = os.write(out, pending)
                    if written <= 0:
                        raise Unreadable("unreadable")
                    pending = pending[written:]
            copied = os.fstat(out).st_size
        finally:
            os.close(out)
        after = os.fstat(fd)
        current = os.stat(name, dir_fd=directory, follow_symlinks=False)
    except FileNotFoundError:
        raise Unreadable("unstable") from None  # replaced or checkpointed away mid-copy
    except OSError:
        raise Unreadable("unreadable") from None
    finally:
        os.close(fd)
    if {_identity(info) for info in (before, after, current)} != {_identity(before)}:
        raise Unreadable("unstable")
    if copied != before.st_size:  # a short copy must never read as an older, valid state
        raise Unreadable("unreadable")
    return _identity(before)


def _background(home: int, now_ms: int) -> dict[str, Any]:
    """Count /bg sessions from a scratch copy; never opens the live database."""
    try:
        pid = os.stat("gateway.pid", dir_fd=home, follow_symlinks=False)
    except FileNotFoundError:
        raise Unreadable("pid_missing") from None
    except OSError:
        raise Unreadable("unreadable") from None
    if not stat.S_ISREG(pid.st_mode):
        raise Unreadable("not_regular")
    try:
        import sqlite3
    except ImportError:
        raise Unreadable("sqlite_unavailable") from None
    started_s = pid.st_mtime_ns / 1e9
    for _attempt in range(3):
        scratch = Path(tempfile.mkdtemp(prefix="finite-status-idle."))
        try:
            database = _copy_stable(home, "state.db", scratch / "state.db", required=True)
            _copy_stable(home, "state.db-wal", scratch / "state.db-wal", required=False)
            # Only a checkpoint writes state.db. One between the two copies, then a WAL
            # reset, truncation or close, pairs the old db copy with a WAL that no longer
            # holds its newest rows, so the db must still be the one that was copied.
            try:
                unchanged = _identity(os.stat("state.db", dir_fd=home, follow_symlinks=False)) == database
            except OSError:
                unchanged = False
            if not unchanged:
                raise Unreadable("unstable")
            connection = sqlite3.connect(scratch / "state.db")  # the copy, so WAL recovery stays private
            try:
                live, stale, ended = connection.execute(
                    BACKGROUND_SQL, (started_s, now_ms / 1000 - BACKGROUND_DELIVERY_S)).fetchone()
            finally:
                connection.close()
            return {"gateway_started_age_s": _age_s(now_ms, pid.st_mtime_ns // 1_000_000),
                    "open": live, "stale_open": stale, "recently_ended": ended}
        except Unreadable as error:
            if str(error) != "unstable":  # only a file changed mid-copy is retried
                raise
        except sqlite3.Error:
            raise Unreadable("malformed") from None
        finally:
            shutil.rmtree(scratch, ignore_errors=True)
    raise Unreadable("unstable")


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
        try:
            background = _background(home, now_ms)
        except Unreadable as error:
            reasons.append(f"background_{error}")
        else:
            if result["gateway"] is not None:
                result["gateway"]["background"] = background
    finally:
        for fd in (home, agent, root):
            if fd is not None:
                os.close(fd)
    return {**result, **classify(result, reasons)}


def classify(result: dict[str, Any], reasons: list[str]) -> dict[str, Any]:
    gateway = result["gateway"]
    if gateway is not None and gateway["state"] != "running":
        reasons.append("gateway_not_running")
    inbox = result["hermes_inbox"]
    background = gateway and gateway.get("background")
    signals = {
        "active_agents": gateway and gateway["active_agents"],
        "inbox_pending": inbox and inbox["pending"],
        "inbox_leased": inbox and inbox["leased"],
        "agentd_inbox": result["agentd_inbox"] and result["agentd_inbox"]["events"],
        "running_markers": result["running_markers"] and result["running_markers"]["messages"],
        "recent_ack": inbox and inbox["newest_ack_age_s"] is not None
                      and inbox["newest_ack_age_s"] < QUIET_AFTER_ACK_S,
        "background_sessions": background and background["open"] + background["recently_ended"],
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

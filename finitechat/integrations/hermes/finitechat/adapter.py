"""Finite Chat platform plugin for Hermes.

The adapter is intentionally thin: Hermes callbacks become JSON bridge
requests, and the finitechat daemon/CLI owns validation, cursoring, storage,
encryption, and attachment materialization.
"""

from __future__ import annotations

import asyncio
import builtins
import contextlib
import contextvars
import functools
import hashlib
import json
import logging
import os
import re
import shlex
import shutil
import stat
import sys
import threading
import time
import types
import urllib.error
import urllib.parse
import urllib.request
import weakref
from collections import deque
from pathlib import Path
from typing import Any, NamedTuple

from gateway.config import HomeChannel, Platform, PlatformConfig
from gateway.platforms.base import (
    BasePlatformAdapter,
    MessageEvent,
    MessageType,
    SendResult,
    build_session_key,
)

logger = logging.getLogger(__name__)

FINITE_PLATFORM_NAME = "finitechat"
LOCAL_ENV_FILE = "finitechat.env"
DEFAULT_POLL_LIMIT = 10
DEFAULT_POLL_TIMEOUT_SECS = 20
DEFAULT_ACTIVITY_REFRESH_SECS = 10.0
ACTIVE_TURN_POLL_TIMEOUT_MILLIS = 100
DEFAULT_SERVICE_ADDR = "127.0.0.1:0"
SERVICE_READY_FILE = "hermes-service.json"
BRIDGE_STATUS_FILE = "hermes-bridge-status.json"
SERVICE_START_TIMEOUT_SECS = 5.0
STREAM_RECONNECT_BACKOFF_SECS = 2.0
STREAM_RECONNECT_MAX_BACKOFF_SECS = 30.0
SERVICE_TRANSPORT_RETRY_SECS = 0.1
ACTIVITY_CONTROL_TIMEOUT_SECS = 1.5
PROCESSING_ACTIVITY_TTL_MILLIS = 15 * 1000
ADMISSION_RECHECK_SECS = 0.05
ADMISSION_RETRY_SECS = 1.0
ADMISSION_MAX_RETRY_SECS = 30.0
DEFAULT_FINITE_PRIVATE_CONTROL_URL = "https://finite.computer/api/core/v1/finite-private"
FINITE_PRIVATE_CONTROL_TIMEOUT_SECS = 5
FINITECHAT_HOME_CHANNEL_ENV = "FINITECHAT_HOME_CHANNEL"
FINITE_ACCOUNT_ID_PATTERN = re.compile(r"[0-9a-f]{64}")
REQUESTER_CONTEXT_DIR = "requester-context-v1"
REQUESTER_CONTEXT_V2_DIR = "requester-context-v2"
REQUESTER_CONTEXT_TTL_SECS = 15 * 60
REQUESTER_CONTEXT_VERSION = 1
REQUESTER_CONTEXT_V2_VERSION = 2
# Opt-in Brain requester-lease diagnostic (FIN-117). Normal images leave it
# unset. Only a non-production diagnostic canary image, scoped to one Agent
# Runtime, sets it (infra/runbooks/runtime-image.md). Its records go to a
# private directory under /tmp, outside the durable /data chat state.
REQUESTER_DIAGNOSTICS_ENV = "FINITECHAT_REQUESTER_DIAGNOSTICS"
REQUESTER_DIAGNOSTICS_DIR = Path("/tmp/finitechat-requester-diagnostics")
REQUESTER_DIAGNOSTICS_FILE = "trace.log"
# One owner per process, kept in sys.modules so plugin rediscovery, which
# evicts and re-imports this module, finds the same worker and sink.
_REQUESTER_DIAGNOSTICS_OWNER_KEY = "_finitechat_requester_diagnostics_owner"
_REQUESTER_DIAGNOSTICS_QUEUE_LIMIT = 256
_REQUESTER_DIAGNOSTICS_MAX_BYTES = 16 * 1024 * 1024
_REQUESTER_DIAGNOSTICS_IDLE_SECS = (0.05, 1.0)
_REQUESTER_DIAGNOSTICS_REOPEN_SECS = 5.0
_AUTHENTICATED_FINITE_TURN_USER: contextvars.ContextVar[str | None] = contextvars.ContextVar(
    "finitechat_authenticated_turn_user", default=None
)
_AUTHENTICATED_FINITE_REQUESTER_CONTEXT: contextvars.ContextVar[tuple[str, str] | None] = (
    contextvars.ContextVar("finitechat_authenticated_requester_context", default=None)
)
APPROVAL_CONTROL_TEXT = frozenset(
    {
        "approve",
        "yes",
        "ok",
        "okay",
        "confirm",
        "y",
        "👍",
        "deny",
        "no",
        "reject",
        "cancel",
        "n",
        "👎",
        "always",
        "approve always",
        "always approve",
        "session",
        "approve session",
        "session approve",
    }
)

# Pinned Hermes does not pass a semantic progress flag to platform adapters.
# Pin its real registry prefix -> tool-name pairs and friendly prefix -> verb
# pairs instead of accepting the unsafe cross-product of any tool emoji and
# any tool-looking prose. Variation selectors are removed before matching.
HERMES_018_RAW_TOOL_NAMES_BY_PREFIX = {
    "video": frozenset({"xai_video_edit", "xai_video_extend"}),
    "⌨": frozenset({"browser_press", "browser_type"}),
    "⏰": frozenset({"cronjob"}),
    "⏸": frozenset({"kanban_block"}),
    "▶": frozenset({"kanban_unblock"}),
    "◀": frozenset({"browser_back"}),
    "⚙": frozenset(
        {
            "computer_use",
            "discord",
            "discord_admin",
            "process",
            "project_create",
            "project_list",
            "project_switch",
        }
    ),
    "✉": frozenset({"feishu_drive_add_comment", "feishu_drive_reply_comment", "yb_send_dm"}),
    "✍": frozenset({"write_file"}),
    "✔": frozenset({"kanban_complete"}),
    "❓": frozenset({"clarify"}),
    "\u2795": frozenset({"kanban_create"}),
    "🌐": frozenset({"browser_navigate"}),
    "🎨": frozenset({"image_generate", "yb_send_sticker"}),
    "🎬": frozenset({"video_analyze", "video_generate"}),
    "🏠": frozenset({"ha_call_service", "ha_get_state", "ha_list_entities", "ha_list_services"}),
    "🐍": frozenset({"execute_code"}),
    "🐦": frozenset({"x_search"}),
    "👁": frozenset({"browser_vision", "vision_analyze"}),
    "👆": frozenset({"browser_click"}),
    "👥": frozenset({"yb_query_group_info"}),
    "💓": frozenset({"kanban_heartbeat"}),
    "💬": frozenset(
        {
            "browser_dialog",
            "feishu_drive_list_comment_replies",
            "feishu_drive_list_comments",
            "kanban_comment",
        }
    ),
    "💻": frozenset({"terminal"}),
    "📄": frozenset({"feishu_doc_read", "web_extract"}),
    "📋": frozenset({"kanban_list", "kanban_show", "todo", "yb_query_group_members"}),
    "📖": frozenset({"read_file"}),
    "📚": frozenset({"skill_view", "skills_list"}),
    "📜": frozenset({"browser_scroll"}),
    "📝": frozenset({"skill_manage"}),
    "📸": frozenset({"browser_snapshot"}),
    "🔀": frozenset({"delegate_task"}),
    "🔊": frozenset({"text_to_speech"}),
    "🔍": frozenset({"session_search", "web_search", "yb_search_sticker"}),
    "🔎": frozenset({"search_files"}),
    "🔗": frozenset({"kanban_link"}),
    "🔧": frozenset({"patch"}),
    "🖥": frozenset({"browser_console", "close_terminal", "read_terminal"}),
    "🖼": frozenset({"browser_get_images"}),
    "🧠": frozenset({"memory"}),
    "🧪": frozenset({"browser_cdp"}),
}
HERMES_018_FRIENDLY_TOOL_PREFIXES = frozenset(
    {
        ("🔍", "Searching the web"),
        ("📄", "Reading"),
        ("🌐", "Browsing"),
        ("👆", "Clicking"),
        ("⌨", "Typing"),
        ("📖", "Reading"),
        ("✍", "Writing"),
        ("🔧", "Editing"),
        ("🔎", "Searching files"),
        ("💻", "Running"),
        ("🐍", "Running code"),
        ("🎨", "Generating image"),
        ("🎬", "Generating video"),
        ("🔊", "Generating speech"),
        ("👁", "Looking at the image"),
        ("🔍", "Searching past sessions"),
        ("📚", "Reading skill"),
        ("📚", "Listing skills"),
        ("📝", "Updating skill"),
        ("🔀", "Delegating"),
        ("⏰", "Scheduling"),
        ("❓", "Asking"),
        ("🧠", "Updating memory"),
        ("📋", "Updating tasks"),
    }
)
HERMES_018_RAW_TOOL_PROGRESS_PATTERN = re.compile(
    r"^(?P<name>[a-z][a-z0-9_.-]*)(?:\([^\n]*\)|:\s*(?:\"|')|\.\.\.)"
)


def _load_local_env_defaults(path: Path | None = None) -> None:
    env_path = path or Path(__file__).with_name(LOCAL_ENV_FILE)
    try:
        raw = env_path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return
    except OSError as exc:
        logger.warning("[finitechat] could not read %s: %s", env_path, exc)
        return

    for line in raw.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or "=" not in stripped:
            continue
        key, value = stripped.split("=", 1)
        key = key.strip()
        # FINITE_HOME pins the shared Finite identity location (hosted
        # runtimes); everything else must be finitechat-namespaced.
        if not key.startswith("FINITECHAT_") and key != "FINITE_HOME":
            continue
        os.environ.setdefault(key, value.strip())


_load_local_env_defaults()


def _requester_sink_open(directory: str, name: str) -> int | None:
    """Open the trace file only if it is this user's private regular file.

    The worker creates the directory with mode 0700. A directory or file that
    is a symlink, belongs to another user, or grants any group or other
    access is refused, and so is a file with a second name (a hard link).
    Nothing is written to a refused sink. The worker changes no permissions
    on a path it did not create. The checks run on each open, on the opened
    descriptors; an open descriptor keeps its file.
    """
    uid = os.geteuid()
    try:
        os.mkdir(directory, 0o700)
    except FileExistsError:
        pass
    except OSError:
        return None
    try:
        listed = os.lstat(directory)
        if not stat.S_ISDIR(listed.st_mode) or listed.st_uid != uid or listed.st_mode & 0o077:
            return None
        dir_fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    except OSError:
        return None
    try:
        opened = os.fstat(dir_fd)
        if (
            (opened.st_dev, opened.st_ino) != (listed.st_dev, listed.st_ino)
            or not stat.S_ISDIR(opened.st_mode)
            or opened.st_uid != uid
            or opened.st_mode & 0o077
        ):
            return None
        fd = os.open(
            name,
            os.O_WRONLY | os.O_APPEND | os.O_CREAT | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC,
            0o600,
            dir_fd=dir_fd,
        )
    except OSError:
        return None
    finally:
        os.close(dir_fd)
    try:
        info = os.fstat(fd)
        if (
            stat.S_ISREG(info.st_mode)
            and info.st_uid == uid
            and info.st_nlink == 1
            and not info.st_mode & 0o077
        ):
            return fd
    except OSError:
        pass
    os.close(fd)
    return None


def _requester_sink_write(fd: int, line: bytes, cap: int) -> bool:
    """Append one line unless it would take the file past the cap.

    The cap is measured on the file itself, so a full file from an earlier
    process accepts nothing and a restart gets no new allowance.
    """
    if os.fstat(fd).st_size + len(line) > cap:
        return False
    os.write(fd, line)
    return True


def _requester_diagnostics_worker(owner: Any) -> None:
    """Drain every registered ring into the one sink until the owner stops."""
    fd: int | None = None
    reopen_at = 0.0
    pid = os.getpid()
    idle, max_idle = owner.idle_secs
    try:
        while not owner.stopped:
            owner.busy = True
            handled = 0
            with owner.lock:
                owner.entries = [entry for entry in owner.entries if entry[0]() is not None]
                entries = list(owner.entries)
            for ref, marker in entries:
                producer = ref()
                ring = producer._ring if producer is not None else None
                producer = None
                while ring:
                    try:
                        at, stage, gate = ring.popleft()
                    except IndexError:
                        break
                    handled += 1
                    try:
                        if fd is None and time.monotonic() >= reopen_at:
                            fd = owner.open_sink(owner.directory, owner.name)
                            if fd is None:
                                reopen_at = time.monotonic() + owner.reopen_secs
                        if fd is None:
                            continue
                        stamp = time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(at))
                        line = (
                            f"{stamp}.{int(at * 1000) % 1000:03d}Z [DEBUG-fbrain-requester] "
                            f"stage={stage} gate={gate} pid={pid} marker={marker:x}\n"
                        )
                        try:
                            owner.write_line(fd, line.encode("ascii"), owner.max_bytes)
                        except OSError:
                            with contextlib.suppress(OSError):
                                os.close(fd)
                            fd = None
                            reopen_at = time.monotonic() + owner.reopen_secs
                    except Exception:
                        pass
            owner.busy = False
            if handled:
                idle = owner.idle_secs[0]
            else:
                time.sleep(idle)
                idle = min(idle * 2, max_idle)
    finally:
        owner.busy = False
        if fd is not None:
            with contextlib.suppress(OSError):
                os.close(fd)


def _requester_diagnostics_after_fork(owner: Any) -> None:
    # The child touches no inherited lock: it only reads the entry list and
    # switches every registered producer off.
    owner.forked = True
    owner.worker = None
    for ref, _ in owner.entries:
        producer = ref()
        if producer is not None:
            producer.after_fork_in_child()


def _isolated(function: Any, **names: Any) -> Any:
    """Rebind a function to minimal globals so it keeps no module instance alive."""
    return types.FunctionType(
        function.__code__, {"__builtins__": builtins, **names}, function.__name__
    )


def _requester_diagnostics_owner() -> Any:
    """The process's one owner of the diagnostic worker and sink.

    It survives plugin rediscovery. It holds each module instance's producer
    only by weak reference, and its code runs with minimal globals, so it
    keeps no module instance alive longer than that instance's ring needs.
    """
    existing = sys.modules.get(_REQUESTER_DIAGNOSTICS_OWNER_KEY)
    if existing is not None:
        return existing
    owner: Any = types.ModuleType(_REQUESTER_DIAGNOSTICS_OWNER_KEY)
    owner.lock = threading.Lock()
    owner.entries = []
    owner.worker = None
    owner.forked = False
    owner.stopped = False
    owner.busy = False
    owner.directory = str(REQUESTER_DIAGNOSTICS_DIR)
    owner.name = REQUESTER_DIAGNOSTICS_FILE
    owner.max_bytes = _REQUESTER_DIAGNOSTICS_MAX_BYTES
    owner.reopen_secs = _REQUESTER_DIAGNOSTICS_REOPEN_SECS
    owner.idle_secs = _REQUESTER_DIAGNOSTICS_IDLE_SECS
    owner.open_sink = _isolated(_requester_sink_open, os=os, stat=stat)
    owner.write_line = _isolated(_requester_sink_write, os=os)
    owner.run = _isolated(_requester_diagnostics_worker, os=os, time=time, contextlib=contextlib)
    owner = sys.modules.setdefault(_REQUESTER_DIAGNOSTICS_OWNER_KEY, owner)
    if hasattr(os, "register_at_fork") and not getattr(owner, "fork_registered", False):
        owner.fork_registered = True
        os.register_at_fork(
            after_in_child=functools.partial(_isolated(_requester_diagnostics_after_fork), owner)
        )
    return owner


class _RequesterDiagnostics:
    """Record fixed requester-lease reason codes without touching chat delivery.

    A turn or tool hook reads the clock and appends one fixed record
    (timestamp, stage, gate) to this module instance's bounded ring.
    `deque.append` takes no Python-level lock, and the producer never starts
    a thread or calls `logging`. The process's one owner runs the only worker,
    which drains every registered ring into the one sink. If the worker is
    missing or stalled, the ring overwrites its oldest record and chat
    carries on. A forked child has the diagnostic off. The marker is the
    identity of this module instance's ContextVar object, which tells a
    reloaded hook module apart from the one the connected adapter uses. It is
    never a user, session, tool-call or credential value.
    """

    def __init__(self) -> None:
        self._ring: deque[tuple[float, str, str]] = deque(maxlen=_REQUESTER_DIAGNOSTICS_QUEUE_LIMIT)
        self._enabled = False
        self._started = False
        self._forked = False
        self._owner: Any = None

    def start(self) -> None:
        """Turn the diagnostic on when the flag is set. Runs at registration.

        Registration is the only place a worker starts, outside every turn
        and hook path. Later calls on this instance do nothing.
        """
        if self._started or self._forked:
            return
        self._started = True
        if os.environ.get(REQUESTER_DIAGNOSTICS_ENV) != "1":
            return
        try:
            owner = _requester_diagnostics_owner()
            if owner.forked:
                return
            self._owner = owner
            self._enabled = True
            with owner.lock:
                owner.entries.append((weakref.ref(self), id(_AUTHENTICATED_FINITE_TURN_USER)))
                worker = owner.worker
                if worker is None or not worker.is_alive():
                    worker = threading.Thread(
                        target=owner.run,
                        args=(owner,),
                        name="finitechat-requester-diagnostics",
                        daemon=True,
                    )
                    worker.start()
                    owner.worker = worker
        except Exception:
            # Without a worker the ring keeps only the newest records.
            return

    def emit(self, stage: str, gate: str = "") -> None:
        # The clock read and this append are the producer's whole work.
        if self._enabled:
            self._ring.append((time.time(), stage, gate))

    def flush(self, timeout: float) -> bool:
        """Wait until the worker has handled every record. Used by tests."""
        deadline = time.monotonic() + timeout
        while self._ring or (self._owner is not None and self._owner.busy):
            if time.monotonic() >= deadline:
                return False
            time.sleep(0.01)
        return True

    def after_fork_in_child(self) -> None:
        # The child gets a fresh ring and no worker; it touches nothing the
        # parent's worker may hold.
        self._forked = True
        self._enabled = False
        self._ring = deque(maxlen=_REQUESTER_DIAGNOSTICS_QUEUE_LIMIT)


_REQUESTER_DIAGNOSTICS = _RequesterDiagnostics()


def _requester_diagnostic(stage: str, gate: str = "") -> None:
    try:
        _REQUESTER_DIAGNOSTICS.emit(stage, gate)
    except Exception:
        # This optional observation must never change turn or hook behavior.
        pass


class _RequesterContextBroker:
    """Lease authenticated Finite sender context to turn-local subprocesses.

    Hermes already isolates the session variables with ContextVars and copies
    the active values into terminal subprocesses. The small file lease lets
    `fsite` distinguish that live binding from arbitrary or stale environment
    text without teaching Sites about Chat or Hermes.
    """

    def __init__(self, root: Path | None = None) -> None:
        self.root = root or _requester_context_root()
        self.root_v2 = self.root.parent / REQUESTER_CONTEXT_V2_DIR
        self._lock = threading.Lock()
        self._leases: dict[str, dict[str, tuple[int, int]]] = {}
        self._clear_on_start()
        _requester_diagnostic("broker_created")

    def before_tool_call(self, **kwargs: Any) -> None:
        if str(kwargs.get("tool_name") or "") != "terminal":
            return
        session_key, user_id = _active_finite_session(diagnostic_stage="pre_tool")
        if session_key is None or user_id is None:
            return
        lease_id = _requester_context_lease_id(kwargs)
        now = int(time.time())
        with self._lock:
            self._prune(now)
            session_leases = self._leases.setdefault(session_key, {})
            count, _ = session_leases.get(lease_id, (0, 0))
            session_leases[lease_id] = (
                count + 1,
                now + REQUESTER_CONTEXT_TTL_SECS,
            )
            written = self._write(
                session_key=session_key,
                user_id=user_id,
                expires_at_unix=now + REQUESTER_CONTEXT_TTL_SECS,
            )
        _requester_diagnostic(*written)

    def after_tool_call(self, **kwargs: Any) -> None:
        if str(kwargs.get("tool_name") or "") != "terminal":
            return
        session_key, _ = _active_finite_session(diagnostic_stage="post_tool")
        if session_key is None:
            return
        lease_id = _requester_context_lease_id(kwargs)
        removed: bool | None = None
        with self._lock:
            session_leases = self._leases.get(session_key)
            if session_leases is None:
                removed = self._remove(session_key)
            else:
                count, expires_at = session_leases.get(lease_id, (0, 0))
                if count <= 1:
                    session_leases.pop(lease_id, None)
                else:
                    session_leases[lease_id] = (count - 1, expires_at)
                if not session_leases:
                    self._leases.pop(session_key, None)
                    removed = self._remove(session_key)
        if removed is not None:
            _requester_diagnostic("removed" if removed else "remove_failed")

    def _clear_on_start(self) -> None:
        try:
            self.root.mkdir(mode=0o700, parents=True, exist_ok=True)
            self.root.chmod(0o700)
            for root in (self.root, self.root_v2):
                root.mkdir(mode=0o700, parents=True, exist_ok=True)
                root.chmod(0o700)
                for path in root.iterdir():
                    if path.is_file() or path.is_symlink():
                        path.unlink(missing_ok=True)
        except OSError as exc:
            logger.warning("[finitechat] could not reset requester context leases: %s", exc)

    def _prune(self, now: int) -> None:
        for leases in self._leases.values():
            for lease_id, (_, expires_at) in list(leases.items()):
                if expires_at <= now:
                    leases.pop(lease_id, None)
        expired_keys = [session_key for session_key, leases in self._leases.items() if not leases]
        for session_key in expired_keys:
            self._leases.pop(session_key, None)
            self._remove(session_key)
        try:
            for path in (*self.root.glob("*.json"), *self.root_v2.glob("*.json")):
                try:
                    payload = json.loads(path.read_text(encoding="utf-8"))
                    expires_at = int(payload.get("expires_at_unix") or 0)
                except (OSError, ValueError, TypeError, json.JSONDecodeError):
                    expires_at = 0
                if expires_at <= now:
                    path.unlink(missing_ok=True)
        except OSError:
            pass

    def _write(self, *, session_key: str, user_id: str, expires_at_unix: int) -> tuple[str, str]:
        """Write the v1 lease, then the v2 lease when available.

        Returns the diagnostic stage and the lease formats it covers, for the
        caller to report after it leaves the broker lock.
        """
        written = ("write_failed", "v1")
        try:
            self.root.mkdir(mode=0o700, parents=True, exist_ok=True)
            final_path = self.root / _requester_context_filename(session_key)
            temp_path = self.root / f".{final_path.name}.{os.getpid()}.tmp"
            payload = {
                "version": REQUESTER_CONTEXT_VERSION,
                "session_key": session_key,
                "platform": FINITE_PLATFORM_NAME,
                "requesting_user_id": user_id,
                "expires_at_unix": expires_at_unix,
            }
            with temp_path.open("w", encoding="utf-8") as handle:
                os.chmod(temp_path, 0o600)
                json.dump(payload, handle, separators=(",", ":"), sort_keys=True)
                handle.write("\n")
                handle.flush()
                os.fsync(handle.fileno())
            temp_path.replace(final_path)
            written = ("written", "v1")
            requester_context = _AUTHENTICATED_FINITE_REQUESTER_CONTEXT.get()
            if requester_context is not None:
                written = ("write_failed", "v2")
                email, sites_assertion = requester_context
                self._write_v2(
                    session_key=session_key,
                    user_id=user_id,
                    email=email,
                    sites_assertion=sites_assertion,
                    expires_at_unix=expires_at_unix,
                )
                written = ("written", "v1_v2")
        except OSError as exc:
            logger.warning("[finitechat] could not write requester context lease: %s", exc)
        return written

    def _write_v2(
        self,
        *,
        session_key: str,
        user_id: str,
        email: str,
        sites_assertion: str,
        expires_at_unix: int,
    ) -> None:
        self.root_v2.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.root_v2.chmod(0o700)
        final_path = self.root_v2 / _requester_context_filename(session_key)
        temp_path = self.root_v2 / f".{final_path.name}.{os.getpid()}.tmp"
        payload = {
            "version": REQUESTER_CONTEXT_V2_VERSION,
            "session_key": session_key,
            "platform": FINITE_PLATFORM_NAME,
            "requesting_user_id": user_id,
            "owner_email": email,
            "hosted_requester_assertion": sites_assertion,
            "expires_at_unix": expires_at_unix,
        }
        with temp_path.open("w", encoding="utf-8") as handle:
            os.chmod(temp_path, 0o600)
            json.dump(payload, handle, separators=(",", ":"), sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        temp_path.replace(final_path)

    def _remove(self, session_key: str) -> bool:
        removed = True
        for root in (self.root, self.root_v2):
            try:
                (root / _requester_context_filename(session_key)).unlink(missing_ok=True)
            except OSError:
                removed = False
        return removed


def _requester_context_root() -> Path:
    finite_home = str(os.getenv("FINITE_HOME") or "").strip()
    root = Path(finite_home).expanduser() if finite_home else Path.home() / ".finite"
    return root / REQUESTER_CONTEXT_DIR


def _requester_context_filename(session_key: str) -> str:
    digest = hashlib.sha256(session_key.encode("utf-8")).hexdigest()
    return f"{digest}.json"


def _requester_context_lease_id(kwargs: dict[str, Any]) -> str:
    return ":".join(
        str(kwargs.get(name) or "")
        for name in ("tool_call_id", "task_id", "turn_id", "api_request_id")
    )


def _active_finite_session(*, diagnostic_stage: str = "") -> tuple[str | None, str | None]:
    try:
        from gateway.session_context import get_session_env
    except ImportError:
        if diagnostic_stage:
            _requester_diagnostic(diagnostic_stage, "session_api_unavailable")
        return None, None
    platform = str(get_session_env("HERMES_SESSION_PLATFORM", "") or "").strip()
    session_key = str(get_session_env("HERMES_SESSION_KEY", "") or "").strip()
    user_id = str(get_session_env("HERMES_SESSION_USER_ID", "") or "").strip()
    authenticated_turn_user = _AUTHENTICATED_FINITE_TURN_USER.get()
    if diagnostic_stage and _REQUESTER_DIAGNOSTICS._enabled:
        _requester_diagnostic(
            diagnostic_stage,
            _requester_gate(platform, session_key, user_id, authenticated_turn_user),
        )
    if (
        # Pinned Hermes maps plugin platforms that are not enum members to
        # LOCAL. The adapter-owned ContextVar below is the Finite marker;
        # the platform value is retained only as a fail-closed shape check.
        platform not in {FINITE_PLATFORM_NAME, Platform.LOCAL.value}
        or not session_key
        or FINITE_ACCOUNT_ID_PATTERN.fullmatch(user_id) is None
        or authenticated_turn_user != user_id
    ):
        return None, None
    return session_key, user_id


def _requester_gate(
    platform: str, session_key: str, user_id: str, authenticated_turn_user: str | None
) -> str:
    """Name the first `_active_finite_session` check that rejects, as a fixed code."""
    if platform not in {FINITE_PLATFORM_NAME, Platform.LOCAL.value}:
        return "foreign_platform"
    if not session_key:
        return "missing_session"
    if FINITE_ACCOUNT_ID_PATTERN.fullmatch(user_id) is None:
        return "invalid_user"
    if authenticated_turn_user is None:
        return "missing_turn"
    if authenticated_turn_user != user_id:
        return "sender_mismatch"
    return "accepted"


def _authenticated_requester_for_event(event: MessageEvent) -> str | None:
    if bool(getattr(event, "internal", False)):
        return None
    raw_message = getattr(event, "raw_message", None)
    if not isinstance(raw_message, dict):
        return None
    raw_source = raw_message.get("source")
    if not isinstance(raw_source, dict):
        return None
    authenticated_user_id = str(raw_source.get("user_id") or "").strip()
    source_user_id = str(getattr(getattr(event, "source", None), "user_id", "") or "").strip()
    if (
        FINITE_ACCOUNT_ID_PATTERN.fullmatch(authenticated_user_id) is None
        or source_user_id != authenticated_user_id
    ):
        return None
    return authenticated_user_id


def _authenticated_requester_context_for_event(
    event: MessageEvent,
) -> tuple[str, str] | None:
    if _authenticated_requester_for_event(event) is None:
        return None
    raw_message = getattr(event, "raw_message", None)
    if not isinstance(raw_message, dict):
        return None
    email = str(raw_message.get("requester_email") or "").strip()
    assertion = str(raw_message.get("sites_requester_assertion") or "").strip()
    if (
        not email
        or len(email.encode("utf-8")) > 320
        or not assertion
        or len(assertion.encode("utf-8")) > 512
        or any(character.isspace() for character in assertion)
    ):
        return None
    return email, assertion


def check_requirements() -> bool:
    return bool(_resolve_finitechat_command(""))


def validate_config(config: PlatformConfig) -> bool:
    extra = getattr(config, "extra", {}) or {}
    return bool(extra.get("home") or os.getenv("FINITECHAT_HOME"))


def is_connected(config: PlatformConfig) -> bool:
    return validate_config(config) and check_requirements()


class FiniteChatAdapter(BasePlatformAdapter):
    """Bridge Finite Chat messages to Hermes through the resident service."""

    MAX_MESSAGE_LENGTH = 12000
    SUPPORTS_MESSAGE_EDITING = False

    def __init__(self, config: PlatformConfig):
        super().__init__(config, _finite_platform())
        extra = getattr(config, "extra", {}) or {}
        self.home = str(extra.get("home") or os.getenv("FINITECHAT_HOME") or "").strip()
        # Shared with the module-level post_tool_call hook so fbrain's
        # markers and this adapter's sends observe one pending set.
        self._brain_approval_filings = _BRAIN_APPROVAL_FILINGS
        # Optional room filter; by default the adapter serves every room the
        # Agent Principal has joined through MLS Add + Welcome.
        self.room_id = str(extra.get("room_id") or os.getenv("FINITECHAT_ROOM_ID") or "").strip()
        self.poll_timeout_secs = _bounded_int(
            extra.get("poll_timeout_secs") or os.getenv("FINITECHAT_HERMES_POLL_TIMEOUT_SECS"),
            DEFAULT_POLL_TIMEOUT_SECS,
            minimum=1,
            maximum=60,
        )
        self.poll_limit = _bounded_int(
            extra.get("poll_limit") or os.getenv("FINITECHAT_HERMES_POLL_LIMIT"),
            DEFAULT_POLL_LIMIT,
            minimum=1,
            maximum=32,
        )
        self.activity_refresh_secs = float(
            _bounded_int(
                extra.get("activity_refresh_secs")
                or os.getenv("FINITECHAT_HERMES_ACTIVITY_REFRESH_SECS"),
                int(DEFAULT_ACTIVITY_REFRESH_SECS),
                minimum=5,
                maximum=120,
            )
        )
        self.service_url = (
            str(extra.get("service_url") or os.getenv("FINITECHAT_HERMES_SERVICE_URL") or "")
            .strip()
            .rstrip("/")
        )
        self.service_addr = str(
            extra.get("service_addr")
            or os.getenv("FINITECHAT_HERMES_SERVICE_ADDR")
            or DEFAULT_SERVICE_ADDR
        ).strip()
        self.inbound_stream = _bounded_bool(
            extra.get("inbound_stream")
            if "inbound_stream" in extra
            else os.getenv("FINITECHAT_HERMES_INBOUND_STREAM"),
            default=False,
        )
        self._poll_task: asyncio.Task | None = None
        self._service_proc: asyncio.subprocess.Process | None = None
        self._service_ready_file: Path | None = None
        self._finitechat_cmd = _resolve_finitechat_command(str(extra.get("finitechat_bin") or ""))
        self._finitechat_lock = asyncio.Lock()
        self._home_channel_hydrated = False
        # In-flight state, reply/edit routing, and delivered-event dedup are all
        # owned by the Rust sidecar now (ownership audit O1/O2): the sidecar
        # leases an inbox entry on delivery, resolves reply/edit scope from its
        # own store, and keeps a recently-acked ring for idempotency. The
        # adapter keeps no route table, dedup set, or SQLite state of its own.
        #
        # The Rust inbox is the durable queue. While a Hermes session is busy
        # the adapter retains delivered leases in arrival order per session.
        # Releasing later entries lets an in-transit stream batch overtake them
        # when the session becomes idle. These ordered maps are ephemeral lease
        # holders, not another durable queue; shutdown releases them and a crash
        # leaves recovery to the sidecar's existing lease expiry.
        self._deferred_admissions: dict[
            str, dict[str, tuple[MessageEvent, str, Any, str, str]]
        ] = {}
        self._admission_tasks: dict[str, asyncio.Task] = {}
        # Per-turn marker (not a dedup store): the key of an event currently
        # being admitted. The turn's completion hook clears it when it settles
        # the sidecar lease; if it is still set after handle_message returns the
        # event was consumed inline by a busy session (no background turn fires
        # the hook), so the inline path acks it exactly once.
        self._inflight_admissions: set[str] = set()
        # A user's /stop, /new or /reset ends the session's in-flight turn and
        # everything sent before it. Hermes reports that cancellation with the
        # same CANCELLED outcome as shutdown, so the adapter marks the turn
        # tasks the command cancels (settled as consumed, never redelivered)
        # and keeps the command's room sequence per session so pre-command
        # events still cycling through the durable inbox are acked unrun. The
        # boundaries live for the process: clearing one on idle would let an
        # entry whose lease expires later replay work the user stopped.
        # While a command runs, its session maps to the events the gateway
        # took from the Hermes pending slot (non-text queued behind the turn),
        # which the stop handler discards without settling their leases.
        self._user_interrupting_sessions: dict[str, list[MessageEvent]] = {}
        self._user_cancelled_tasks: weakref.WeakSet[asyncio.Task] = weakref.WeakSet()
        self._user_interrupt_boundaries: dict[str, tuple[str, int]] = {}

    async def _process_message_background(
        self,
        event: MessageEvent,
        session_key: str,
    ) -> None:
        """Bind the authenticated Finite sender to this exact queued turn.

        Hermes starts a fresh background task for every queued follow-up. The
        event remains the authenticated source of truth even when Hermes
        reuses a cached agent session, while ContextVar propagation carries
        this marker into that turn's tool thread.
        """
        requester = _authenticated_requester_for_event(event)
        token = _AUTHENTICATED_FINITE_TURN_USER.set(requester)
        requester_context = _authenticated_requester_context_for_event(event)
        context_token = _AUTHENTICATED_FINITE_REQUESTER_CONTEXT.set(requester_context)
        try:
            _requester_diagnostic("turn", "authenticated" if requester else "unauthenticated")
            await super()._process_message_background(event, session_key)
        finally:
            _AUTHENTICATED_FINITE_REQUESTER_CONTEXT.reset(context_token)
            _AUTHENTICATED_FINITE_TURN_USER.reset(token)

    async def _dispatch_active_session_command(
        self,
        event: MessageEvent,
        session_key: str,
        cmd: str,
    ) -> None:
        """Make a busy-session /stop, /new or /reset final for Finite delivery.

        Hermes clears its own queued follow-up for these commands, but the
        Finite admission head and the inbox entries behind it live outside
        that queue, and the cancelled turn's lease would otherwise be released
        for redelivery as a brand-new run. Everything this session sent before
        the command is settled as consumed; later messages run normally.
        Settling is durable ack, not the in-memory boundary alone, so a
        gateway restart inside the lease window cannot resurrect a stopped
        message.
        """
        await self._interrupt_admissions(session_key, event)
        dequeued: list[MessageEvent] = []
        self._user_interrupting_sessions[session_key] = dequeued
        try:
            await super()._dispatch_active_session_command(event, session_key, cmd)
        finally:
            self._user_interrupting_sessions.pop(session_key, None)
            for pending in dequeued:
                await self._settle_user_interrupted_pending(session_key, pending)

    def get_pending_message(self, session_key: str) -> MessageEvent | None:
        pending = super().get_pending_message(session_key)
        dequeued = self._user_interrupting_sessions.get(session_key)
        if pending is not None and dequeued is not None:
            dequeued.append(pending)
        return pending

    async def cancel_session_processing(
        self,
        session_key: str,
        *,
        release_guard: bool = True,
        discard_pending: bool = True,
    ) -> None:
        if session_key in self._user_interrupting_sessions:
            task = self._session_tasks.get(session_key)
            if task is not None and not task.done():
                self._user_cancelled_tasks.add(task)
        await super().cancel_session_processing(
            session_key,
            release_guard=release_guard,
            discard_pending=discard_pending,
        )

    async def connect(self, is_reconnect: bool = False, **_: Any) -> bool:
        if not self.home:
            logger.error("[finitechat] FINITECHAT_HOME is required (agent home directory)")
            return False
        if not self._finitechat_cmd:
            logger.error("[finitechat] finitechat CLI is not configured")
            return False

        await self._ensure_service()
        await self._recover_interrupted_turns()
        self._mark_connected()
        self._write_bridge_status("connected")
        if self.inbound_stream:
            self._poll_task = asyncio.create_task(self._stream_loop())
        else:
            self._poll_task = asyncio.create_task(self._poll_loop())
        logger.info(
            "[finitechat] connected (home=%s%s%s%s)",
            self.home,
            f", room filter={self.room_id}" if self.room_id else "",
            ", inbound stream=on" if self.inbound_stream else "",
            ", reconnect" if is_reconnect else "",
        )
        return True

    async def _recover_interrupted_turns(self) -> None:
        result = await self._finitechat_json("recover", {}, timeout=60)
        if not result.ok:
            logger.warning("[finitechat] could not recover interrupted turns: %s", result.error)
            return
        recovered = result.data.get("recovered") or 0
        if recovered:
            logger.info("[finitechat] recovered %s interrupted Hermes turn(s)", recovered)

    async def disconnect(self) -> None:
        if self._poll_task:
            self._poll_task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await self._poll_task
            self._poll_task = None
        await self._cancel_admission_tasks()
        await self._stop_service()
        await self.cancel_background_tasks()
        self._mark_disconnected()
        self._write_bridge_status("disconnected")
        logger.info("[finitechat] disconnected")

    async def on_processing_complete(self, event: MessageEvent, outcome: Any) -> None:
        """Settle the event's inbox lease, then surface any claimed quota notice.

        Pinned Hermes invokes this hook once after the final response (including
        streamed delivery), and not after each model/tool subcall. Core claims a
        threshold before returning it, so delivery is intentionally at-most-once:
        a process crash between the Core response and Finite Chat send can omit a
        transient notice, while the dashboard continues to show authoritative state.
        """
        outcome_name = str(getattr(outcome, "value", getattr(outcome, "name", outcome))).lower()
        await self._settle_event_ack(event, outcome_name)
        if outcome_name != "success":
            return
        status = await asyncio.to_thread(_finite_private_control_request, "usage", "GET")
        if not isinstance(status, dict):
            return
        notice = status.get("notice")
        if not isinstance(notice, dict):
            return
        message = str(notice.get("message") or "").strip()
        if not message:
            return
        raw_message = event.raw_message if isinstance(event.raw_message, dict) else {}
        metadata = self._route_metadata(
            _string_or_none(raw_message.get("conversation_id")),
            _string_or_none(raw_message.get("segment_id")),
        )
        result = await self.send(
            chat_id=str(getattr(event.source, "chat_id", "") or raw_message.get("room_id") or ""),
            content=message,
            metadata=metadata,
        )
        if not result.success:
            logger.warning(
                "[finitechat] could not deliver Finite Private usage notice: %s", result.error
            )

    async def send(
        self,
        chat_id: str,
        content: str,
        reply_to: str | None = None,
        metadata: dict[str, Any] | None = None,
    ) -> SendResult:
        payload = self._send_payload(chat_id, content, reply_to, metadata)
        drained = self._attach_brain_approval_metadata(payload)
        result = await self._finitechat_json("send", payload, timeout=30)
        if not result.ok:
            # `retryable` is the sidecar's decision, carried verbatim from the
            # envelope; nothing here reads the message text.
            return SendResult(success=False, error=result.error, retryable=result.retryable)
        self._finish_brain_approval_drain(drained)
        message_id = str(result.data.get("message_id") or result.data.get("id") or "") or None
        return SendResult(
            success=True,
            message_id=message_id,
            raw_response=result.data,
        )

    async def send_clarify(
        self,
        chat_id: str,
        question: str,
        choices: list | None,
        clarify_id: str,
        session_key: str,
        metadata: dict[str, Any] | None = None,
    ) -> SendResult:
        """Project Hermes clarification onto one exact ordinary Chat route."""
        meta = self._message_metadata(metadata)
        room_id = self._room_id(chat_id)
        conversation_id, segment_id, thread_id = self._route_fields_from_metadata(meta)
        if conversation_id is None and segment_id is None and thread_id is None:
            error = (
                "Hermes clarification requires an exact Finite Chat topic and chat; "
                "refusing Home or active-chat fallback"
            )
            logger.warning("[finitechat] %s (session=%s)", error, session_key)
            return SendResult(success=False, error=error, retryable=False)

        # Hermes owns the pending request and the prompt format. Finite only
        # pins its delivery route and keeps the prompt on the ordinary message
        # rail, bypassing the legacy emoji/prose presentation inference. The
        # sidecar resolves a bare thread id and, when it matches nothing, logs
        # a warned Home fallback so the clarification still reaches a visible
        # surface.
        meta["_finitechat_kind"] = "message"
        meta["_finitechat_status"] = "complete"
        if conversation_id is not None:
            meta["conversation_id"] = conversation_id
            meta["segment_id"] = segment_id if segment_id is not None else thread_id
        elif thread_id is not None:
            meta["thread_id"] = thread_id
        elif segment_id is not None:
            meta["segment_id"] = segment_id
        return await super().send_clarify(
            chat_id=room_id,
            question=question,
            choices=choices,
            clarify_id=clarify_id,
            session_key=session_key,
            metadata=meta,
        )

    async def edit_message(
        self,
        chat_id: str,
        message_id: str,
        content: str,
        *,
        finalize: bool = False,
    ) -> SendResult:
        # The sidecar owns the original message's route and kind: it looks the
        # message up by (room_id, message_id) in its running-turn file, so the
        # adapter no longer remembers outbound message routes or kinds. Route
        # and kind fields left unset here are filled in on the sidecar.
        payload = {
            "room_id": self._room_id(chat_id),
            "conversation_id": None,
            "segment_id": None,
            "message_id": str(message_id),
            "text": str(content),
            "kind": "message",
            "status": "complete" if finalize else "running",
            "finalize": bool(finalize),
            "metadata": {},
        }
        drained = self._attach_brain_approval_metadata(payload) if finalize else []
        result = await self._finitechat_json("edit", payload, timeout=30)
        if not result.ok:
            return SendResult(success=False, error=result.error, retryable=result.retryable)
        self._finish_brain_approval_drain(drained)
        edited_message_id = str(result.data.get("message_id") or message_id)
        return SendResult(
            success=True,
            message_id=edited_message_id,
            raw_response=result.data,
        )

    async def send_typing(self, chat_id: str, metadata=None) -> None:
        payload = self._activity_payload(chat_id, metadata, action="set")
        await self._run_activity_control(
            "set",
            self._finitechat_json("activity", payload, timeout=15),
        )

    async def stop_typing(self, chat_id: str, metadata=None) -> None:
        if metadata is None:
            # Hermes performs a room-only cleanup after cancelling the typing
            # task. Finite activity is scoped to an exact topic/chat route, and
            # guessing here can clear a different concurrent turn. The exact
            # _keep_typing task owns its matching clear in finally instead.
            return
        payload = self._activity_payload(chat_id, metadata, action="clear")
        await self._run_activity_control(
            "clear",
            self._finitechat_json("activity", payload, timeout=15),
        )

    async def _keep_typing(
        self,
        chat_id: str,
        interval: float = DEFAULT_ACTIVITY_REFRESH_SECS,
        metadata=None,
        stop_event: asyncio.Event | None = None,
    ) -> None:
        refresh_secs = self.activity_refresh_secs if self.activity_refresh_secs > 0 else interval
        send_timeout = max(0.25, min(ACTIVITY_CONTROL_TIMEOUT_SECS, refresh_secs - 0.25))
        try:
            while True:
                if stop_event is not None and stop_event.is_set():
                    return
                try:
                    await asyncio.wait_for(
                        self.send_typing(chat_id, metadata=metadata),
                        timeout=send_timeout,
                    )
                except TimeoutError:
                    pass
                except asyncio.CancelledError:
                    raise
                except Exception as exc:
                    logger.debug("[finitechat] activity refresh failed: %s", exc)
                if stop_event is None:
                    await asyncio.sleep(refresh_secs)
                    continue
                loop = asyncio.get_running_loop()
                deadline = loop.time() + refresh_secs
                while not stop_event.is_set():
                    remaining = deadline - loop.time()
                    if remaining <= 0:
                        break
                    # Polling avoids leaving an Event.wait task behind when
                    # Hermes cancels the refresh task during turn shutdown.
                    await asyncio.sleep(min(0.25, remaining))
                if stop_event.is_set():
                    return
        except asyncio.CancelledError:
            pass
        finally:
            # An empty mapping still denotes the exact unscoped Home route;
            # None is reserved for Hermes' later room-only cleanup call.
            await self.stop_typing(chat_id, metadata=metadata if metadata is not None else {})

    async def send_image(
        self,
        chat_id: str,
        image_url: str,
        caption: str | None = None,
        reply_to: str | None = None,
        metadata: dict[str, Any] | None = None,
    ) -> SendResult:
        return await self._send_media(
            chat_id,
            caption or "",
            {"kind": "image", "url": image_url, "name": caption or "image", "mime_type": "image/*"},
            reply_to=reply_to,
            metadata=metadata,
        )

    async def send_image_file(
        self,
        chat_id: str,
        image_path: str,
        caption: str | None = None,
        reply_to: str | None = None,
        metadata: dict[str, Any] | None = None,
    ) -> SendResult:
        return await self._send_media(
            chat_id,
            caption or "",
            _local_attachment(image_path, "image"),
            reply_to=reply_to,
            metadata=metadata,
        )

    async def send_video(
        self,
        chat_id: str,
        video_path: str,
        caption: str | None = None,
        reply_to: str | None = None,
        metadata: dict[str, Any] | None = None,
    ) -> SendResult:
        return await self._send_media(
            chat_id,
            caption or "",
            _local_attachment(video_path, "video"),
            reply_to=reply_to,
            metadata=metadata,
        )

    async def send_voice(
        self,
        chat_id: str,
        audio_path: str,
        metadata: dict[str, Any] | None = None,
    ) -> SendResult:
        return await self._send_media(
            chat_id,
            "",
            _local_attachment(audio_path, "audio"),
            metadata=metadata,
        )

    async def send_document(
        self,
        chat_id: str,
        file_path: str,
        caption: str | None = None,
        reply_to: str | None = None,
        metadata: dict[str, Any] | None = None,
    ) -> SendResult:
        return await self._send_media(
            chat_id,
            caption or "",
            _local_attachment(file_path, "file"),
            reply_to=reply_to,
            metadata=metadata,
        )

    @staticmethod
    def extract_local_files(content: str):
        return [], content

    async def get_chat_info(self, chat_id: str) -> dict[str, Any]:
        room_id = self._room_id(chat_id)
        return {"id": room_id, "name": room_id, "type": "finite"}

    async def _poll_loop(self) -> None:
        while self.is_connected:
            if not await self._poll_once():
                await asyncio.sleep(2.0)

    async def _poll_once(self) -> bool:
        result = await self._finitechat_json(
            "poll",
            self._inbound_request_payload(),
            timeout=self.poll_timeout_secs + 15,
        )
        if not result.ok:
            logger.warning("[finitechat] poll failed: %s", result.error)
            self._write_bridge_status("poll_error", result.error)
            return False
        await self._process_poll_payload(result.data)
        self._write_bridge_status("connected")
        return True

    async def _stream_loop(self) -> None:
        reconnect_attempt = 0
        while self.is_connected:
            if not self.service_url and not await self._ensure_service():
                error = "resident Hermes service is unavailable"
                logger.warning("[finitechat] %s; waiting to reconnect stream", error)
                self._write_bridge_status("stream_error", error)
                await asyncio.sleep(_stream_reconnect_delay(reconnect_attempt))
                reconnect_attempt += 1
                continue
            loop = asyncio.get_running_loop()
            queue: asyncio.Queue[_FiniteChatResult] = asyncio.Queue()
            stop_event = threading.Event()
            service_url = self.service_url
            worker = threading.Thread(
                target=_finitechat_service_stream_worker,
                args=(
                    service_url,
                    self._inbound_request_payload(),
                    self.poll_timeout_secs + 15,
                    loop,
                    queue,
                    stop_event,
                ),
                daemon=True,
            )
            worker.start()
            try:
                while self.is_connected and self.service_url == service_url:
                    result = await queue.get()
                    if result.ok:
                        reconnect_attempt = 0
                        await self._process_inbound_records(result.data.get("records") or [])
                        self._write_bridge_status("connected")
                        continue
                    logger.warning("[finitechat] inbound stream failed: %s", result.error)
                    self._write_bridge_status("stream_error", result.error)
                    # A service process supervised by this adapter may need to
                    # be rediscovered or restarted. An externally supervised
                    # service keeps its stable URL and is retried in place.
                    if result.transport_error and self._service_proc is not None:
                        self.service_url = ""
                    break
            finally:
                stop_event.set()
                await asyncio.to_thread(worker.join, 0.5)
            if not self.is_connected:
                break
            await asyncio.sleep(_stream_reconnect_delay(reconnect_attempt))
            reconnect_attempt += 1

    def _inbound_request_payload(self) -> dict[str, Any]:
        timeout_millis = self.poll_timeout_secs * 1000
        if self._has_active_turn():
            timeout_millis = min(timeout_millis, ACTIVE_TURN_POLL_TIMEOUT_MILLIS)
        payload: dict[str, Any] = {
            "limit": self.poll_limit,
            "timeout_millis": timeout_millis,
        }
        if self.room_id:
            payload["room_id"] = self.room_id
        return payload

    async def _process_poll_payload(self, data: dict[str, Any]) -> None:
        for account in data.get("joined") or []:
            logger.info("[finitechat] verified joiner admitted: %s", account)
        for raw_event in data.get("events") or []:
            await self._dispatch_raw_event(raw_event)

    async def _process_inbound_records(self, records: list[Any]) -> None:
        for raw_record in records:
            if not isinstance(raw_record, dict):
                continue
            record_type = str(raw_record.get("type") or "")
            if record_type == "joined":
                logger.info(
                    "[finitechat] verified joiner admitted: %s", raw_record.get("account_id")
                )
                continue
            if record_type == "event":
                raw_event = raw_record.get("event")
            elif record_type:
                logger.debug("[finitechat] ignored non-message inbound record type %s", record_type)
                continue
            else:
                raw_event = raw_record
            await self._dispatch_raw_event(raw_event)

    async def _dispatch_raw_event(self, raw_event: Any) -> None:
        try:
            await self._handle_finitechat_event(raw_event)
        except Exception as exc:
            logger.error("[finitechat] failed to dispatch event: %s", exc, exc_info=True)

    async def _handle_finitechat_event(self, raw_event: dict[str, Any]) -> None:
        if not isinstance(raw_event, dict):
            return
        room_id = str(raw_event.get("room_id") or self.room_id)
        if self.room_id and room_id != self.room_id:
            logger.warning("[finitechat] ignored event for filtered room %s", room_id)
            return
        seq = raw_event.get("seq")
        message_id = str(raw_event.get("message_id") or "")
        if not message_id:
            logger.warning("[finitechat] ignored event without message_id")
            return
        event_key = _adapter_event_key(room_id, seq, message_id)
        if any(event_key in admissions for admissions in self._deferred_admissions.values()):
            # A lease can expire while its turn is still queued or running.
            # Keep the existing holder and order; its completion settles the
            # renewed lease too (the protocol settles by event identity).
            return
        # The sidecar's recently-acked ring owns durable deduplication. The
        # checks above only coalesce renewed leases while this process still
        # holds the same event; they retain no completed-event history.

        raw_source = raw_event.get("source")
        source_data: dict[str, Any] = raw_source if isinstance(raw_source, dict) else {}
        authenticated_user_id = _string_or_none(source_data.get("user_id"))
        conversation_id = _string_or_none(raw_event.get("conversation_id"))
        segment_id = _string_or_none(raw_event.get("segment_id"))
        source_thread_id = (
            segment_id or _string_or_none(source_data.get("thread_id")) or conversation_id
        )
        raw_attachments = raw_event.get("attachments")
        attachments: list[Any] = raw_attachments if isinstance(raw_attachments, list) else []
        media_urls, media_types = _event_media(attachments)
        source = self.build_source(
            chat_id=str(source_data.get("chat_id") or room_id),
            chat_name=_string_or_none(source_data.get("chat_name")),
            chat_type=str(source_data.get("chat_type") or "dm"),
            user_id=authenticated_user_id or "finite-user",
            user_name=_string_or_none(source_data.get("user_name")),
            thread_id=source_thread_id,
            chat_topic=_string_or_none(source_data.get("chat_topic")),
            user_id_alt=_string_or_none(source_data.get("user_id_alt")),
            chat_id_alt=_string_or_none(source_data.get("chat_id_alt")),
            is_bot=bool(source_data.get("is_bot") or False),
        )
        event = MessageEvent(
            text=str(raw_event.get("text") or ""),
            message_type=_message_type(str(raw_event.get("message_type") or ""), media_types),
            source=source,
            raw_message=raw_event,
            message_id=message_id,
            platform_update_id=seq if isinstance(seq, int) else None,
            media_urls=media_urls,
            media_types=media_types,
            reply_to_message_id=_string_or_none(raw_event.get("reply_to_message_id")),
            reply_to_text=_string_or_none(raw_event.get("reply_to_text")),
            auto_skill=raw_event.get("auto_skill"),
            channel_prompt=_finite_sender_channel_prompt(
                authenticated_user_id,
                raw_event.get("channel_prompt"),
            ),
            internal=bool(raw_event.get("internal") or False),
        )
        session_key = build_session_key(
            event.source,
            group_sessions_per_user=self.config.extra.get("group_sessions_per_user", True),
            thread_sessions_per_user=self.config.extra.get("thread_sessions_per_user", False),
        )
        if event_key in self._inflight_admissions:
            if self._session_is_active(session_key):
                return
            # A lost completion hook must not suppress recovery forever once
            # there is no active session owner to settle the original lease.
            self._inflight_admissions.discard(event_key)
        if self._precedes_user_interrupt(session_key, room_id, seq):
            # Sent before this session's latest /stop, /new or /reset and
            # redelivered afterwards (a released later event, or an expired
            # lease): the command already ended it, so settle it unrun.
            logger.info("[finitechat] discarded %s/%s sent before a user interrupt", room_id, seq)
            await self._ack_finitechat_event(room_id, seq, message_id)
            return
        if session_key in self._deferred_admissions and event.get_command() in {
            "stop",
            "new",
            "reset",
        }:
            # The queue can be waiting on its next handoff with no active turn.
            # Hermes's busy-command path alone cannot cover this idle window.
            await self._interrupt_admissions(session_key, event)
        if self._should_defer_admission(event, session_key):
            self._defer_admission(
                session_key,
                event,
                room_id,
                seq,
                message_id,
                event_key or "",
            )
            return

        await self._admit_finitechat_event(
            event,
            room_id,
            seq,
            message_id,
            event_key or "",
        )

    async def _admit_finitechat_event(
        self,
        event: MessageEvent,
        room_id: str,
        seq: Any,
        message_id: str,
        event_key: str,
    ) -> None:
        raw_event = event.raw_message if isinstance(event.raw_message, dict) else {}
        conversation_id = _string_or_none(raw_event.get("conversation_id"))
        segment_id = _string_or_none(raw_event.get("segment_id"))
        session_key = build_session_key(
            event.source,
            group_sessions_per_user=self.config.extra.get("group_sessions_per_user", True),
            thread_sessions_per_user=self.config.extra.get("thread_sessions_per_user", False),
        )
        activity_metadata = self._route_metadata(conversation_id, segment_id)
        activity_set = False
        # The sidecar leased this entry on delivery. Its lease is settled only
        # by the turn: the completion hook acks on success, failure, or a user
        # /stop, and a shutdown-cancelled turn releases it. A turn that fails
        # synchronously before completion is released here so the sidecar
        # redelivers it whole.
        try:
            await self._hydrate_hermes_home_channel_if_needed()
            activity_set = await self._set_processing_activity(room_id, activity_metadata)
            session_active = self._session_is_active(session_key)
            if event_key:
                self._inflight_admissions.add(event_key)
            await self.handle_message(event)
        except asyncio.CancelledError:
            self._inflight_admissions.discard(event_key)
            # The activity RPC can have completed remotely before cancellation
            # reaches its caller. Clear even if it did not return success yet.
            await self._clear_processing_activity(room_id, activity_metadata)
            raise
        except Exception:
            if event_key:
                self._inflight_admissions.discard(event_key)
            if activity_set:
                await self._clear_processing_activity(room_id, activity_metadata)
            await self._release_finitechat_event(room_id, seq, message_id)
            raise
        queued_for_later = getattr(self, "_pending_messages", {}).get(session_key) is event
        if (
            event_key
            and event_key in self._inflight_admissions
            and session_active
            and not queued_for_later
        ):
            # Events consumed inline by a busy session (slash-command bypass,
            # busy-session handlers) never pass through the background turn that
            # fires the completion hook, so ack here — exactly once. Every other
            # event is acked (or released) by the completion hook.
            self._inflight_admissions.discard(event_key)
            await self._ack_finitechat_event(room_id, seq, message_id)

    async def _hydrate_hermes_home_channel_if_needed(self) -> None:
        if self._home_channel_hydrated:
            return
        if _string_or_none(os.getenv(FINITECHAT_HOME_CHANNEL_ENV)):
            self._home_channel_hydrated = True
            return
        if getattr(self.config, "home_channel", None) is not None:
            self._home_channel_hydrated = True
            return

        result = await self._finitechat_json("home-channel-show", {}, timeout=5)
        if not result.ok:
            return
        metadata = result.data.get("home_channel")
        if not isinstance(metadata, dict):
            return
        room_id = _string_or_none(metadata.get("room_id"))
        if room_id is None:
            return

        try:
            _save_hermes_home_channel_env(room_id)
        except Exception as exc:
            logger.debug("[finitechat] Could not hydrate Hermes home channel: %s", exc)
            return
        self.config.home_channel = HomeChannel(
            platform=_finite_platform(),
            chat_id=room_id,
            name="Finite Chat",
        )
        self._home_channel_hydrated = True

    def _should_defer_admission(self, event: MessageEvent, session_key: str) -> bool:
        # Media needs the same durable admission as text. Hermes can merge
        # busy-session media into one pending event and run it recursively
        # inside the current turn, without a completion hook for each lease.
        # Admit each event as its own turn so its hook alone settles it.
        if event.internal:
            return False
        if (event.text or "").lstrip().startswith("/"):
            return False
        if self._is_immediate_text_control(event, session_key):
            return False
        return session_key in self._deferred_admissions or self._session_is_active(session_key)

    @staticmethod
    def _is_immediate_text_control(event: MessageEvent, session_key: str) -> bool:
        try:
            from tools import clarify_gateway

            if (
                clarify_gateway.get_pending_for_session(
                    session_key,
                    include_choice_prompts=True,
                )
                is not None
            ):
                return True
        except Exception:
            pass

        if (event.text or "").strip().lower() not in APPROVAL_CONTROL_TEXT:
            return False
        try:
            from tools.approval import has_blocking_approval

            return bool(has_blocking_approval(session_key))
        except Exception:
            return False

    def _session_is_active(self, session_key: str) -> bool:
        if session_key not in self._active_sessions:
            return False
        self._heal_stale_session_lock(session_key)
        return session_key in self._active_sessions

    def _defer_admission(
        self,
        session_key: str,
        event: MessageEvent,
        room_id: str,
        seq: Any,
        message_id: str,
        event_key: str,
    ) -> None:
        admissions = self._deferred_admissions.setdefault(session_key, {})
        admissions[event_key] = (
            event,
            room_id,
            seq,
            message_id,
            event_key,
        )
        if session_key in self._admission_tasks:
            return
        task = asyncio.create_task(self._admit_when_session_idle(session_key))
        self._admission_tasks[session_key] = task

    async def _admit_when_session_idle(self, session_key: str) -> None:
        retry_delay = ADMISSION_RETRY_SECS
        try:
            while admissions := self._deferred_admissions.get(session_key):
                if self._session_is_active(session_key):
                    owner = self._session_tasks.get(session_key)
                    if owner is not None and not owner.done():
                        await asyncio.wait({owner}, timeout=ADMISSION_RECHECK_SECS)
                    else:
                        await asyncio.sleep(ADMISSION_RECHECK_SECS)
                    continue
                event, room_id, seq, message_id, event_key = next(iter(admissions.values()))
                try:
                    await self._admit_finitechat_event(
                        event,
                        room_id,
                        seq,
                        message_id,
                        event_key,
                    )
                except Exception:
                    # Handoff failure may release the lease. Retain its place
                    # until retry succeeds; a redelivery coalesces with this
                    # holder instead of letting later entries overtake it.
                    logger.exception("[finitechat] deferred handoff failed for %s", session_key)
                    await asyncio.sleep(retry_delay)
                    retry_delay = min(retry_delay * 2, ADMISSION_MAX_RETRY_SECS)
                    continue
                admissions.pop(event_key, None)
                retry_delay = ADMISSION_RETRY_SECS
        except asyncio.CancelledError:
            raise
        finally:
            current = asyncio.current_task()
            if self._admission_tasks.get(session_key) is current:
                self._admission_tasks.pop(session_key, None)
                self._deferred_admissions.pop(session_key, None)

    def _precedes_user_interrupt(self, session_key: str, room_id: str, seq: Any) -> bool:
        boundary = self._user_interrupt_boundaries.get(session_key)
        if boundary is None or not isinstance(seq, int):
            return False
        boundary_room_id, boundary_seq = boundary
        return room_id == boundary_room_id and seq < boundary_seq

    async def _settle_user_interrupted_pending(self, session_key: str, event: MessageEvent) -> None:
        raw_message = event.raw_message if isinstance(event.raw_message, dict) else {}
        room_id = str(raw_message.get("room_id") or self.room_id)
        seq = raw_message.get("seq")
        message_id = str(raw_message.get("message_id") or "")
        if not message_id:
            return
        event_key = _adapter_event_key(room_id, seq, message_id)
        if event_key:
            self._inflight_admissions.discard(event_key)
        if self._precedes_user_interrupt(session_key, room_id, seq):
            await self._ack_finitechat_event(room_id, seq, message_id)
        else:
            # Inbound dispatch is serial, so nothing sent after the command
            # can reach the slot while it runs; never drop one if it does.
            await self._release_finitechat_event(room_id, seq, message_id)

    async def _interrupt_admissions(self, session_key: str, event: MessageEvent) -> None:
        raw_message = event.raw_message if isinstance(event.raw_message, dict) else {}
        seq = raw_message.get("seq")
        if isinstance(seq, int):
            room_id = str(raw_message.get("room_id") or self.room_id)
            self._user_interrupt_boundaries[session_key] = (room_id, seq)
        await self._discard_deferred_admission(session_key)

    async def _discard_deferred_admission(self, session_key: str) -> None:
        task = self._admission_tasks.pop(session_key, None)
        admissions = self._deferred_admissions.pop(session_key, {})
        if task is not None and not task.done():
            task.cancel()
            await asyncio.gather(task, return_exceptions=True)
        for _event, room_id, seq, message_id, _event_key in admissions.values():
            await self._ack_finitechat_event(room_id, seq, message_id)

    async def _cancel_admission_tasks(self) -> None:
        admissions = list(self._deferred_admissions.values())
        tasks = list(self._admission_tasks.values())
        for task in tasks:
            task.cancel()
        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)
        self._admission_tasks.clear()
        self._deferred_admissions.clear()
        for queue in admissions:
            for _event, room_id, seq, message_id, _event_key in queue.values():
                await self._release_finitechat_event(room_id, seq, message_id)

    async def _set_processing_activity(
        self,
        room_id: str,
        metadata: dict[str, Any] | None,
    ) -> bool:
        payload = self._activity_payload(room_id, metadata, action="set")
        payload["expires_in_millis"] = PROCESSING_ACTIVITY_TTL_MILLIS
        return await self._run_activity_control(
            "set",
            self._finitechat_json("activity", payload, timeout=15),
        )

    async def _clear_processing_activity(
        self,
        room_id: str,
        metadata: dict[str, Any] | None,
    ) -> None:
        await self.stop_typing(room_id, metadata=metadata if metadata is not None else {})

    async def _run_activity_control(self, action: str, operation: Any) -> bool:
        try:
            result = await asyncio.wait_for(operation, timeout=ACTIVITY_CONTROL_TIMEOUT_SECS)
            return bool(getattr(result, "ok", True))
        except TimeoutError:
            logger.debug("[finitechat] timed out during activity %s", action)
        except Exception as exc:
            logger.debug("[finitechat] activity %s failed: %s", action, exc)
        return False

    async def _ack_finitechat_event(self, room_id: str, seq: Any, message_id: str) -> None:
        if not isinstance(seq, int):
            return
        ack = await self._finitechat_json(
            "ack",
            {"room_id": room_id, "seq": seq, "message_id": message_id},
            timeout=15,
        )
        if not ack.ok:
            logger.warning("[finitechat] failed to ack %s/%s: %s", room_id, seq, ack.error)

    async def _release_finitechat_event(self, room_id: str, seq: Any, message_id: str) -> None:
        if not isinstance(seq, int):
            return
        result = await self._finitechat_json(
            "release",
            {"room_id": room_id, "seq": seq, "message_id": message_id},
            timeout=15,
        )
        if not result.ok:
            logger.warning("[finitechat] failed to release %s/%s: %s", room_id, seq, result.error)

    async def _settle_event_ack(self, event: MessageEvent, outcome_name: str) -> None:
        """Settle the sidecar's inbox lease once the event's turn has run.

        The completion hook fires exactly once per background turn. A turn
        cancelled by shutdown or recovery releases the lease so the sidecar
        redelivers the entry whole; a turn the user cancelled with /stop, /new
        or /reset is acked, because redelivering it would restart the very work
        the user stopped. Success or failure acks too (a failed turn still ran
        to completion and answered the user, so re-running it on redelivery
        would be wrong). Ack and release are both idempotent on the sidecar.
        """
        raw_message = event.raw_message if isinstance(event.raw_message, dict) else {}
        room_id = str(raw_message.get("room_id") or self.room_id)
        seq = raw_message.get("seq")
        message_id = str(raw_message.get("message_id") or "")
        if not message_id:
            return
        # Claim the in-flight marker so the inline-admission path does not also
        # ack this event once its background turn's completion hook has fired.
        event_key = _adapter_event_key(room_id, seq, message_id)
        if event_key:
            self._inflight_admissions.discard(event_key)
        if outcome_name == "cancelled" and asyncio.current_task() not in self._user_cancelled_tasks:
            await self._release_finitechat_event(room_id, seq, message_id)
            return
        await self._ack_finitechat_event(room_id, seq, message_id)

    @staticmethod
    def _route_metadata(
        conversation_id: str | None,
        segment_id: str | None,
    ) -> dict[str, str] | None:
        metadata: dict[str, str] = {}
        if conversation_id:
            metadata["conversation_id"] = conversation_id
        if segment_id:
            metadata["segment_id"] = segment_id
            metadata["thread_id"] = segment_id
        return metadata or None

    async def _send_media(
        self,
        chat_id: str,
        body: str,
        attachment: dict[str, Any],
        *,
        reply_to: str | None = None,
        metadata: dict[str, Any] | None = None,
    ) -> SendResult:
        meta = self._message_metadata(metadata)
        attachments = list(meta.get("attachments") or [])
        attachments.append(attachment)
        meta["attachments"] = attachments
        meta["_finitechat_kind"] = "media"
        return await self.send(chat_id=chat_id, content=body, reply_to=reply_to, metadata=meta)

    def _attach_brain_approval_metadata(self, payload: dict[str, Any]) -> list[dict[str, Any]]:
        """Ride unreported brain approval filings on a final user-visible
        delivery, as the reference-only `metadata.approve` envelope.

        Only final deliveries (kind message, status complete) carry the
        question: commentary, tool progress, and streaming partials never do.
        Returns the drained filings; the caller marks them reported once the
        delivery is accepted.
        """
        if str(payload.get("kind")) != "message" or str(payload.get("status")) != "complete":
            return []
        filings = self._brain_approval_filings.take_pending()
        if not filings:
            return []
        meta = payload.setdefault("metadata", {})
        if not isinstance(meta, dict) or "approve" in meta:
            return []
        meta["approve"] = _brain_approval_metadata(filings)
        return filings

    def _finish_brain_approval_drain(self, drained: list[dict[str, Any]]) -> None:
        if not drained:
            return
        self._brain_approval_filings.mark_reported({filing["requestId"] for filing in drained})

    def _send_payload(
        self,
        chat_id: str,
        content: str,
        reply_to: str | None,
        metadata: dict[str, Any] | None,
    ) -> dict[str, Any]:
        meta = self._message_metadata(metadata)
        room_id = self._room_id(chat_id)
        conversation_id, segment_id, thread_id = self._route_fields_from_metadata(meta)
        attachments = meta.pop("attachments", [])
        explicit_kind = meta.pop("_finitechat_kind", None)
        explicit_status = meta.pop("_finitechat_status", None)
        kind = str(explicit_kind or ("media" if attachments else _infer_finitechat_kind(content)))
        status = str(explicit_status or _infer_finitechat_status(content))
        return {
            "room_id": room_id,
            "conversation_id": conversation_id,
            "segment_id": segment_id,
            "thread_id": thread_id,
            "text": str(content),
            "kind": kind,
            "status": status,
            "attachments": attachments if isinstance(attachments, list) else [],
            "reply_to_message_id": reply_to,
            "metadata": meta,
        }

    def _activity_payload(
        self,
        chat_id: str,
        metadata: dict[str, Any] | None,
        *,
        action: str,
        conversation_id: str | None = None,
        segment_id: str | None = None,
    ) -> dict[str, Any]:
        meta = self._message_metadata(metadata)
        room_id = self._room_id(chat_id)
        metadata_conversation_id, metadata_segment_id, metadata_thread_id = (
            self._route_fields_from_metadata(meta)
        )
        resolved_conversation_id = (
            conversation_id if conversation_id is not None else metadata_conversation_id
        )
        resolved_segment_id = segment_id if segment_id is not None else metadata_segment_id
        return {
            "room_id": room_id,
            "conversation_id": resolved_conversation_id,
            "segment_id": resolved_segment_id,
            # Only hand a thread_id to the sidecar when there is no explicit
            # route to resolve against; the sidecar resolves it from its store.
            "thread_id": metadata_thread_id
            if resolved_conversation_id is None and resolved_segment_id is None
            else None,
            "activity_kind": "working",
            "activity_id": None,
            "action": action,
            "payload": None,
            "expires_in_millis": 60 * 1000,
        }

    def _has_active_turn(self) -> bool:
        active_sessions = getattr(self, "_active_sessions", None)
        return bool(active_sessions)

    def _room_id(self, chat_id: str | None) -> str:
        return str(chat_id or self.room_id).strip() or self.room_id

    @staticmethod
    def _route_fields_from_metadata(
        metadata: dict[str, Any] | None,
    ) -> tuple[str | None, str | None, str | None]:
        """Split send/activity metadata into (conversation_id, segment_id,
        thread_id). An explicit Topic/Chat route is passed through as-is; when
        only a Hermes thread id is present it is handed to the sidecar, which
        owns resolving it against its store (ownership audit O2). The adapter
        keeps no route table and never resolves scope itself."""
        if not isinstance(metadata, dict):
            return None, None, None
        thread_id = _string_or_none(metadata.pop("thread_id", None))
        conversation_id = _string_or_none(metadata.pop("conversation_id", None))
        segment_id = _string_or_none(metadata.pop("segment_id", None)) or _string_or_none(
            metadata.pop("chat_id", None)
        )
        if conversation_id is not None:
            # Explicit Topic authority; treat a bare thread id as the chat.
            if segment_id is None and thread_id is not None:
                segment_id = thread_id
            return conversation_id, segment_id, None
        return conversation_id, segment_id, thread_id

    @staticmethod
    def _message_metadata(metadata: dict[str, Any] | None) -> dict[str, Any]:
        if isinstance(metadata, dict):
            return dict(metadata)
        return {}

    async def _finitechat_json(
        self,
        action: str,
        payload: dict[str, Any],
        *,
        timeout: int,
    ) -> _FiniteChatResult:
        if self._service_proc is not None and self._service_proc.returncode is not None:
            self.service_url = ""
            await self._ensure_service()
        if self.inbound_stream and not self.service_url:
            await self._ensure_service()
        if self.service_url:
            result = await asyncio.to_thread(
                _finitechat_service_json,
                self.service_url,
                action,
                payload,
                timeout,
            )
            if result.ok or not result.transport_error:
                return result
            await asyncio.sleep(SERVICE_TRANSPORT_RETRY_SECS)
            retry_result = await asyncio.to_thread(
                _finitechat_service_json,
                self.service_url,
                action,
                payload,
                timeout,
            )
            if retry_result.ok or not retry_result.transport_error:
                return retry_result
            result = retry_result
            action_detail = ""
            if action == "activity" and isinstance(payload.get("action"), str):
                action_detail = f"/{payload['action']}"
            logger.warning(
                "[finitechat] Hermes service unavailable during %s%s (%s)%s",
                action,
                action_detail,
                result.error,
                "; strict stream mode will retry the resident service"
                if self.inbound_stream
                else "; falling back to finitechat CLI",
            )
            if self.inbound_stream:
                return result
        if self.inbound_stream:
            return _FiniteChatResult(
                False,
                {},
                "resident Hermes service is unavailable in strict stream mode",
                True,
                True,
            )
        if not self._finitechat_cmd:
            return _FiniteChatResult(False, {}, "finitechat CLI is not configured", False)
        command = [*self._finitechat_cmd, "hermes"]
        if self.home:
            command += ["--agent-home", self.home]
        command += [action, "--json"]
        try:
            stdin = json.dumps(payload, ensure_ascii=False).encode("utf-8") + b"\n"
            async with self._finitechat_lock:
                proc = await asyncio.create_subprocess_exec(
                    *command,
                    env=os.environ.copy(),
                    stdin=asyncio.subprocess.PIPE,
                    stdout=asyncio.subprocess.PIPE,
                    stderr=asyncio.subprocess.PIPE,
                )
                stdout, stderr = await asyncio.wait_for(proc.communicate(stdin), timeout=timeout)
        except TimeoutError:
            return _FiniteChatResult(False, {}, "finitechat timed out", True)
        except FileNotFoundError as exc:
            return _FiniteChatResult(False, {}, str(exc), False)
        except Exception as exc:
            return _FiniteChatResult(False, {}, str(exc), True)

        stdout_text = stdout.decode("utf-8", errors="replace").strip()
        stderr_text = stderr.decode("utf-8", errors="replace").strip()
        if proc.returncode != 0:
            error = _service_error(stderr_text or stdout_text)
            return _FiniteChatResult(
                False,
                {},
                error.message or f"finitechat exited {proc.returncode}",
                error.retryable,
                error_kind=error.kind,
            )
        if not stdout_text:
            return _FiniteChatResult(True, {}, None, False)
        try:
            return _FiniteChatResult(True, json.loads(stdout_text), None, False)
        except json.JSONDecodeError as exc:
            try:
                return _FiniteChatResult(
                    True, json.loads(stdout_text.splitlines()[-1]), None, False
                )
            except json.JSONDecodeError:
                return _FiniteChatResult(
                    False, {}, f"finitechat returned invalid JSON: {exc}", False
                )

    async def _ensure_service(self) -> bool:
        if self.service_url:
            healthy = await asyncio.to_thread(_finitechat_service_health, self.service_url, 2)
            if healthy:
                return True
            if self._service_proc is None:
                return False
            self.service_url = ""
        if self._service_proc is not None and self._service_proc.returncode is None:
            if self._service_ready_file is not None:
                started = _read_service_ready_file(self._service_ready_file)
                if started.get("url"):
                    candidate_url = str(started["url"]).rstrip("/")
                    healthy = await asyncio.to_thread(_finitechat_service_health, candidate_url, 2)
                    if healthy:
                        self.service_url = candidate_url
                        logger.info("[finitechat] Hermes service ready at %s", self.service_url)
                        return True
            return bool(self.service_url)
        if not self.home or not self._finitechat_cmd:
            return False

        ready_file = Path(self.home) / SERVICE_READY_FILE
        self._service_ready_file = ready_file
        with contextlib.suppress(FileNotFoundError):
            ready_file.unlink()
        command = [
            *self._finitechat_cmd,
            "hermes",
            "--agent-home",
            self.home,
            "serve",
            "--addr",
            self.service_addr,
            "--ready-file",
            str(ready_file),
            "--json",
        ]
        try:
            self._service_proc = await asyncio.create_subprocess_exec(
                *command,
                env=os.environ.copy(),
                stdout=asyncio.subprocess.DEVNULL,
                stderr=asyncio.subprocess.DEVNULL,
            )
        except Exception as exc:
            logger.warning("[finitechat] could not start Hermes service: %s", exc)
            return False

        deadline = asyncio.get_running_loop().time() + SERVICE_START_TIMEOUT_SECS
        while asyncio.get_running_loop().time() < deadline:
            if self._service_proc.returncode is not None:
                logger.warning(
                    "[finitechat] Hermes service exited during startup (%s)",
                    self._service_proc.returncode,
                )
                self._service_proc = None
                self._service_ready_file = None
                return False
            started = _read_service_ready_file(ready_file)
            if started.get("url"):
                candidate_url = str(started["url"]).rstrip("/")
                healthy = await asyncio.to_thread(_finitechat_service_health, candidate_url, 2)
                if healthy:
                    self.service_url = candidate_url
                    logger.info("[finitechat] Hermes service ready at %s", self.service_url)
                    return True
            await asyncio.sleep(0.05)

        if self.inbound_stream:
            logger.warning("[finitechat] Hermes service did not become ready; will retry")
        else:
            logger.warning("[finitechat] Hermes service did not become ready; using CLI bridge")
        return False

    async def _stop_service(self) -> None:
        proc = self._service_proc
        self._service_proc = None
        self._service_ready_file = None
        if proc is None or proc.returncode is not None:
            return
        proc.terminate()
        try:
            await asyncio.wait_for(proc.wait(), timeout=2.0)
        except TimeoutError:
            proc.kill()
            await proc.wait()

    def _write_bridge_status(self, status: str, error: str | None = None) -> None:
        if not self.home:
            return
        payload: dict[str, Any] = {
            "status": status,
            "ok": not status.endswith("_error"),
            "updated_at_ms": int(time.time() * 1000),
            "inbound_stream": bool(self.inbound_stream),
        }
        if self.service_url:
            payload["service_url"] = self.service_url
        if error:
            payload["error"] = str(error)
        path = Path(self.home) / BRIDGE_STATUS_FILE
        tmp_path = path.with_suffix(f"{path.suffix}.tmp")
        try:
            path.parent.mkdir(parents=True, exist_ok=True)
            tmp_path.write_text(json.dumps(payload, sort_keys=True), encoding="utf-8")
            os.replace(tmp_path, path)
        except OSError as exc:
            logger.debug("[finitechat] could not write bridge status: %s", exc)


class _FiniteChatResult:
    def __init__(
        self,
        ok: bool,
        data: dict[str, Any],
        error: str | None,
        retryable: bool,
        transport_error: bool = False,
        *,
        error_kind: str | None = None,
    ):
        self.ok = ok
        self.data = data
        self.error = error
        self.retryable = retryable
        self.transport_error = transport_error
        self.error_kind = error_kind


def _resolve_finitechat_command(configured: str) -> list[str]:
    raw = str(
        configured or os.getenv("FINITECHAT_HERMES_BIN") or os.getenv("FINITECHAT_BIN") or ""
    ).strip()
    if raw:
        return shlex.split(raw)
    for name in ("finitechat",):
        path = shutil.which(name)
        if path:
            return [path]
    return []


def _finitechat_service_json(
    service_url: str,
    action: str,
    payload: dict[str, Any],
    timeout: int,
) -> _FiniteChatResult:
    encoded_action = urllib.parse.quote(str(action), safe="")
    request = urllib.request.Request(
        f"{service_url}/v1/hermes/{encoded_action}",
        data=json.dumps(payload, ensure_ascii=False).encode("utf-8"),
        headers={"Accept": "application/json", "Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = response.read().decode("utf-8", errors="replace").strip()
    except urllib.error.HTTPError as exc:
        error = _service_error(exc.read().decode("utf-8", errors="replace").strip())
        return _FiniteChatResult(
            False,
            {},
            error.message or f"finitechat service returned HTTP {exc.code}",
            error.retryable,
            False,
            error_kind=error.kind,
        )
    except TimeoutError as exc:
        return _FiniteChatResult(False, {}, str(exc), True, False)
    except (urllib.error.URLError, OSError) as exc:
        return _FiniteChatResult(False, {}, str(exc), True, True)

    if not body:
        return _FiniteChatResult(True, {}, None, False)
    try:
        return _FiniteChatResult(True, json.loads(body), None, False)
    except json.JSONDecodeError as exc:
        return _FiniteChatResult(
            False, {}, f"finitechat service returned invalid JSON: {exc}", False
        )


def _finitechat_service_stream_worker(
    service_url: str,
    payload: dict[str, Any],
    timeout: int,
    loop: asyncio.AbstractEventLoop,
    queue: asyncio.Queue,
    stop_event: threading.Event,
) -> None:
    query = urllib.parse.urlencode(
        {key: value for key, value in payload.items() if value is not None and value != ""}
    )
    url = f"{service_url}/v1/hermes/inbound"
    if query:
        url = f"{url}?{query}"
    request = urllib.request.Request(
        url,
        headers={"Accept": "application/x-ndjson, application/json"},
        method="GET",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            # Opening the streaming response is itself the liveness proof. An
            # idle room may produce only blank heartbeat lines for hours, so
            # waiting for a chat record before clearing a prior stream error
            # leaves the runtime falsely unhealthy after the server recovers.
            _put_stream_result(
                loop,
                queue,
                _FiniteChatResult(
                    True,
                    {"records": [{"type": "connected"}]},
                    None,
                    False,
                    False,
                ),
            )
            while not stop_event.is_set():
                raw_line = response.readline()
                if not raw_line:
                    _put_stream_result(
                        loop,
                        queue,
                        _FiniteChatResult(False, {}, "finitechat inbound stream ended", True, True),
                    )
                    return
                stripped = raw_line.decode("utf-8", errors="replace").strip()
                if not stripped:
                    continue
                try:
                    record = json.loads(stripped)
                except json.JSONDecodeError as exc:
                    _put_stream_result(
                        loop,
                        queue,
                        _FiniteChatResult(
                            False,
                            {},
                            f"finitechat inbound stream returned invalid JSON: {exc}",
                            False,
                            False,
                        ),
                    )
                    return
                if isinstance(record, dict) and record.get("type") == "error":
                    error = str(record.get("error") or "finitechat inbound stream failed")
                    _put_stream_result(
                        loop,
                        queue,
                        _FiniteChatResult(False, {}, error, True, False),
                    )
                    return
                _put_stream_result(
                    loop,
                    queue,
                    _FiniteChatResult(True, {"records": [record]}, None, False, False),
                )
    except urllib.error.HTTPError as exc:
        error = _service_error(exc.read().decode("utf-8", errors="replace").strip())
        _put_stream_result(
            loop,
            queue,
            _FiniteChatResult(
                False,
                {},
                error.message or f"finitechat service returned HTTP {exc.code}",
                error.retryable,
                False,
                error_kind=error.kind,
            ),
        )
    except TimeoutError as exc:
        _put_stream_result(loop, queue, _FiniteChatResult(False, {}, str(exc), True, False))
    except (urllib.error.URLError, OSError) as exc:
        _put_stream_result(loop, queue, _FiniteChatResult(False, {}, str(exc), True, True))


def _put_stream_result(
    loop: asyncio.AbstractEventLoop,
    queue: asyncio.Queue,
    result: _FiniteChatResult,
) -> None:
    with contextlib.suppress(RuntimeError):
        loop.call_soon_threadsafe(queue.put_nowait, result)


class _ServiceError(NamedTuple):
    """The sidecar's structured error, as read from an HTTP error body or
    the ``--json`` CLI stderr line: ``{"error", "error_kind", "retryable"}``.

    ``retryable`` is taken from the sidecar verbatim; Python never infers it
    from the message text. A body that is not that shape (a crash, a signal,
    an HTML or empty response) did not come from the sidecar's error path,
    so it is reported verbatim with ``retryable=False``: the transport-level
    failures that are genuinely transient (timeouts, refused connections) are
    classified by exception type before any body is parsed, and retrying a
    request whose outcome is unknown risks duplicating a delivery.
    """

    message: str | None
    kind: str | None
    retryable: bool


def _service_error(body: str) -> _ServiceError:
    data = _parse_service_error_json(body)
    if data is None:
        return _ServiceError(body or None, None, False)
    error = data.get("error")
    kind = data.get("error_kind")
    return _ServiceError(
        str(error) if error else (body or None),
        str(kind) if isinstance(kind, str) and kind else None,
        data.get("retryable") is True,
    )


def _parse_service_error_json(body: str) -> dict[str, Any] | None:
    """The whole body, else its last line (the CLI may log above the JSON line)."""
    for candidate in (body, body.rsplit("\n", 1)[-1].strip()):
        try:
            data = json.loads(candidate)
        except json.JSONDecodeError:
            continue
        if isinstance(data, dict):
            return data
    return None


def _finitechat_service_health(service_url: str, timeout: int) -> bool:
    try:
        request = urllib.request.Request(
            f"{service_url.rstrip('/')}/healthz",
            headers={"Accept": "application/json"},
            method="GET",
        )
        with urllib.request.urlopen(request, timeout=timeout) as response:
            data = json.loads(response.read().decode("utf-8", errors="replace"))
    except Exception:
        return False
    return isinstance(data, dict) and data.get("status") == "ok"


def _adapter_event_key(room_id: str, seq: Any, message_id: str) -> str | None:
    if not isinstance(seq, int):
        return None
    return f"{room_id}\x1f{seq}\x1f{message_id}"


def _read_service_ready_file(path: Path) -> dict[str, Any]:
    try:
        raw = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return {}
    except OSError as exc:
        logger.warning("[finitechat] could not read Hermes service ready file %s: %s", path, exc)
        return {}
    try:
        data = json.loads(raw)
    except json.JSONDecodeError:
        return {}
    return data if isinstance(data, dict) else {}


def _event_media(attachments: list[Any]) -> tuple[list[str], list[str]]:
    urls: list[str] = []
    types: list[str] = []
    for item in attachments:
        if not isinstance(item, dict):
            continue
        local_path = _string_or_none(item.get("path"))
        # Blob URLs point at encrypted bytes. If the resident sidecar could
        # not materialize a verified local path, retain the blob only in the
        # raw message for UI recovery/resend and deliver the caption as text.
        media_ref = local_path
        if not media_ref and not isinstance(item.get("blob"), dict):
            media_ref = _string_or_none(item.get("url"))
        if not media_ref:
            continue
        urls.append(media_ref)
        types.append(
            _string_or_none(item.get("mime_type"))
            or _string_or_none(item.get("mimeType"))
            or "application/octet-stream"
        )
    return urls, types


def _message_type(raw: str, media_types: list[str]) -> MessageType:
    value = raw.strip()
    if value == "command":
        return MessageType.COMMAND
    if value == "sticker":
        return MessageType.STICKER
    if value == "location":
        return MessageType.LOCATION
    if not media_types:
        return MessageType.TEXT
    first = media_types[0]
    if first.startswith("image/"):
        return MessageType.PHOTO
    if first.startswith("video/"):
        return MessageType.VIDEO
    if first.startswith("audio/"):
        return MessageType.AUDIO
    return MessageType.DOCUMENT


def _infer_finitechat_kind(content: str) -> str:
    text = str(content or "").strip()
    if not text:
        return "message"
    if text == "Hermes is working":
        return "status"
    lines = text.splitlines()
    first_line = lines[0].lstrip()
    first_parts = first_line.split(maxsplit=1)
    first_token = first_parts[0].replace("\ufe0f", "")
    progress_label = first_parts[1].strip() if len(first_parts) > 1 else ""
    friendly_progress = any(
        first_token == prefix and (progress_label == verb or progress_label.startswith(f"{verb} "))
        for prefix, verb in HERMES_018_FRIENDLY_TOOL_PREFIXES
    )
    raw_match = HERMES_018_RAW_TOOL_PROGRESS_PATTERN.match(progress_label)
    raw_tool_name = raw_match.group("name") if raw_match else None
    known_raw_progress = raw_tool_name in HERMES_018_RAW_TOOL_NAMES_BY_PREFIX.get(first_token, ())
    # Third-party Hermes tools without a registry emoji use the gateway's
    # default gear. Keep that one extension point while retaining the strict
    # icon/name pairs for every pinned built-in.
    custom_default_progress = first_token == "⚙" and raw_tool_name is not None
    terminal_code_block = (
        first_token == "💻"
        and progress_label == "terminal"
        and len(lines) > 1
        and lines[1].lstrip().startswith("```")
    )
    if friendly_progress or known_raw_progress or custom_default_progress or terminal_code_block:
        return "tool"
    return "message"


def _infer_finitechat_status(content: str) -> str:
    return "running" if "▉" in str(content or "") else "complete"


BRAIN_APPROVAL_FILED_MARKER = re.compile(
    r"finite-brain-approval-filed"
    r" brain=(?P<brain>[a-z0-9][a-z0-9_-]{0,127})"
    r" request=(?P<request>[A-Za-z0-9][A-Za-z0-9_-]{0,127})"
)
# Tool results can be large; bound the scan so a huge output never stalls the hook.
BRAIN_APPROVAL_MARKER_SCAN_BYTES = 2 * 1024 * 1024
# fbrain's approval requests expire after 15 minutes; a filing older than
# that must not ride a much later delivery.
BRAIN_APPROVAL_FILING_MAX_AGE_SECS = 20 * 60


def _tool_result_text(result: Any) -> str:
    """Best-effort text of a terminal tool result, never raising."""
    try:
        if result is None:
            return ""
        if isinstance(result, str):
            text = result
            try:
                result = json.loads(result)
            except ValueError:
                return text
        if isinstance(result, dict):
            for key in ("output", "stdout", "result", "text", "content"):
                value = result.get(key)
                if isinstance(value, str):
                    return value
            return json.dumps(result)
        return str(result)
    except Exception:  # observer hook must never raise
        return ""


class _BrainApprovalFilings:
    """In-memory bridge from fbrain's stdout marker to the next final delivery.

    ``fbrain`` prints one ``finite-brain-approval-filed brain=<id>
    request=<id>`` trailer line per filed approval request; the terminal
    tool's ``post_tool_call`` result carries that output here. The agent's
    next final user-visible delivery drains the filings as the
    reference-only ``metadata.approve`` envelope.

    Everything degrades to "no card": marker absent, result unparsable, hook
    never fired, filing stale, or the runtime restarted before the final
    delivery — the request stays durable server-side and
    ``fbrain approvals list`` remains the authoritative fallback. No
    filesystem or config-directory coupling, and no chat send is ever
    affected by this broker.
    """

    def __init__(self, max_pending: int = 64) -> None:
        self._lock = threading.Lock()
        self._pending: list[dict[str, Any]] = []
        self._max_pending = max(1, max_pending)

    def after_tool_call(self, **kwargs: Any) -> None:
        try:
            if str(kwargs.get("tool_name") or "") != "terminal":
                return
            text = _tool_result_text(kwargs.get("result"))[:BRAIN_APPROVAL_MARKER_SCAN_BYTES]
            if not text:
                return
            found = [
                {"brainId": match["brain"], "requestId": match["request"], "filedAt": time.time()}
                for match in BRAIN_APPROVAL_FILED_MARKER.finditer(text)
            ]
            if not found:
                return
            with self._lock:
                known = {filing["requestId"] for filing in self._pending}
                for filing in found:
                    if filing["requestId"] not in known:
                        self._pending.append(filing)
                        known.add(filing["requestId"])
                del self._pending[: -self._max_pending]
        except Exception:  # observer hook must never raise
            logger.debug("brain approval marker scan failed", exc_info=True)

    def take_pending(self) -> list[dict[str, Any]]:
        """Snapshot the filings that may ride the next final delivery."""
        now = time.time()
        with self._lock:
            self._pending = [
                filing
                for filing in self._pending
                if now - float(filing.get("filedAt", 0)) <= BRAIN_APPROVAL_FILING_MAX_AGE_SECS
            ]
            return [dict(filing) for filing in self._pending]

    def mark_reported(self, request_ids: set[str]) -> None:
        if not request_ids:
            return
        with self._lock:
            self._pending = [
                filing for filing in self._pending if filing["requestId"] not in request_ids
            ]


def _brain_approval_metadata(requests: list[dict[str, Any]]) -> dict[str, Any]:
    """The reference-only approve envelope: no approval payload ever rides."""
    return {
        "service": "brain",
        "requests": [
            {"brainId": request["brainId"], "requestId": request["requestId"]}
            for request in requests
        ],
    }


def _local_attachment(path: str, kind: str) -> dict[str, Any]:
    local_path = Path(path)
    return {
        "kind": kind,
        "path": str(local_path),
        "name": local_path.name or kind,
        "mime_type": _mime_type_for_path(local_path),
    }


def _mime_type_for_path(path: Path) -> str:
    suffix = path.suffix.lower()
    return {
        ".png": "image/png",
        ".jpg": "image/jpeg",
        ".jpeg": "image/jpeg",
        ".webp": "image/webp",
        ".gif": "image/gif",
        ".svg": "image/svg+xml",
        ".mp3": "audio/mpeg",
        ".wav": "audio/wav",
        ".ogg": "audio/ogg",
        ".opus": "audio/ogg",
        ".mp4": "video/mp4",
        ".mov": "video/quicktime",
        ".webm": "video/webm",
        ".pdf": "application/pdf",
        ".txt": "text/plain",
        ".md": "text/markdown",
        ".json": "application/json",
    }.get(suffix, "application/octet-stream")


def _string_or_none(value: Any) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _finite_sender_channel_prompt(user_id: str | None, channel_prompt: Any) -> str | None:
    existing = _string_or_none(channel_prompt)
    if user_id is None or FINITE_ACCOUNT_ID_PATTERN.fullmatch(user_id) is None:
        return existing
    sender_context = (
        "Authenticated Finite Chat sender metadata for this turn: "
        f"event.source.user_id is `{user_id}`. Use this exact identifier only when a "
        "Finite tool or skill explicitly requests event.source.user_id; never substitute "
        "a display name."
    )
    if existing is None:
        return sender_context
    return f"{sender_context}\n\n{existing}"


def _bounded_int(value: Any, default: int, *, minimum: int, maximum: int) -> int:
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        parsed = default
    return max(minimum, min(maximum, parsed))


def _bounded_bool(value: Any, *, default: bool) -> bool:
    if value is None:
        return default
    if isinstance(value, bool):
        return value
    text = str(value).strip().lower()
    if not text:
        return default
    if text in {"1", "true", "yes", "on"}:
        return True
    if text in {"0", "false", "no", "off"}:
        return False
    return default


def _stream_reconnect_delay(attempt: int) -> float:
    exponent = max(0, min(int(attempt), 4))
    return min(
        STREAM_RECONNECT_BACKOFF_SECS * (2**exponent),
        STREAM_RECONNECT_MAX_BACKOFF_SECS,
    )


def _finite_private_control_request(path: str, method: str) -> dict[str, Any] | None:
    api_key = os.getenv("FINITE_PRIVATE_API_KEY", "").strip()
    if not api_key:
        return None
    base_url = (
        os.getenv("FINITE_PRIVATE_CONTROL_URL", "").strip() or DEFAULT_FINITE_PRIVATE_CONTROL_URL
    ).rstrip("/")
    request = urllib.request.Request(
        f"{base_url}/{path.lstrip('/')}",
        method=method,
        data=b"{}" if method != "GET" else None,
        headers={
            "Authorization": f"Bearer {api_key}",
            "Accept": "application/json",
            "Content-Type": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(
            request, timeout=FINITE_PRIVATE_CONTROL_TIMEOUT_SECS
        ) as response:
            payload = json.loads(response.read().decode("utf-8"))
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError, UnicodeDecodeError) as exc:
        logger.debug("[finitechat] Finite Private control request failed: %s", exc)
        return None
    return payload if isinstance(payload, dict) else None


def _save_hermes_home_channel_env(room_id: str) -> None:
    from hermes_cli.config import save_env_value

    save_env_value(FINITECHAT_HOME_CHANNEL_ENV, room_id)


def _finite_platform() -> Platform:
    try:
        return Platform(FINITE_PLATFORM_NAME)
    except ValueError:
        return Platform.LOCAL


_BRAIN_APPROVAL_FILINGS = _BrainApprovalFilings()


def register(ctx) -> None:
    _REQUESTER_DIAGNOSTICS.start()
    register_hook = getattr(ctx, "register_hook", None)
    if callable(register_hook):
        requester_context = _RequesterContextBroker()
        register_hook("pre_tool_call", requester_context.before_tool_call)
        register_hook("post_tool_call", requester_context.after_tool_call)
        register_hook("post_tool_call", _BRAIN_APPROVAL_FILINGS.after_tool_call)
        _requester_diagnostic("hooks_registered")
    ctx.register_platform(
        name=FINITE_PLATFORM_NAME,
        label="Finite Chat",
        adapter_factory=lambda cfg: FiniteChatAdapter(cfg),
        check_fn=check_requirements,
        validate_config=validate_config,
        is_connected=is_connected,
        required_env=["FINITECHAT_HOME"],
        install_hint=(
            "Install the finitechat binary, run `finitechat hermes "
            "init --server URL`, then install this plugin with "
            "`finitechat hermes install`."
        ),
        allowed_users_env="FINITECHAT_ALLOWED_USERS",
        allow_all_env="FINITECHAT_ALLOW_ALL_USERS",
        cron_deliver_env_var=FINITECHAT_HOME_CHANNEL_ENV,
        max_message_length=FiniteChatAdapter.MAX_MESSAGE_LENGTH,
        allow_update_command=True,
        platform_hint=(
            "You are chatting through Finite Chat. The room is the delivery "
            "boundary and the thread is the conversation/topic. Use normal markdown. "
            "You can send files natively: to deliver a file to the user, include "
            "MEDIA:/absolute/path/to/file in your response. Images appear inline; "
            "audio and video use native media; documents, spreadsheets, archives, "
            "and other files arrive as downloadable attachments. Do not tell the user "
            "that Finite Chat cannot send file attachments."
        ),
    )

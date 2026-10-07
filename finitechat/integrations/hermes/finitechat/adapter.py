"""Finite Chat platform plugin for Hermes.

The adapter is intentionally thin: Hermes callbacks become JSON bridge
requests, and the finitechat daemon/CLI owns validation, cursoring, storage,
encryption, and attachment materialization.
"""

from __future__ import annotations

import asyncio
import contextlib
import contextvars
import copy
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
from collections.abc import Awaitable
from dataclasses import dataclass, field
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
# The readers' size limit and the name they derive from a session key.
REQUESTER_CONTEXT_MAX_BYTES = 4096
_REQUESTER_CONTEXT_NAME = re.compile(r"[0-9a-f]{64}\.json")
_REQUESTER_STATE_MODULE = "_finitechat_requester_state_v1"


def _requester_state() -> Any:
    """Process-lifetime requester data shared by every load of this plugin.

    A forced plugin rediscovery imports this file again under the same
    `hermes_plugins.*` name, so a running adapter can set its turn marker in
    one copy while hooks registered by the next copy read another. Only data
    lives here, outside that namespace: the markers, lock and lease counts.
    """
    state = sys.modules.get(_REQUESTER_STATE_MODULE)
    if state is None:
        candidate: Any = types.ModuleType(_REQUESTER_STATE_MODULE)
        candidate.turn_user = contextvars.ContextVar(
            "finitechat_authenticated_turn_user", default=None
        )
        candidate.requester_context = contextvars.ContextVar(
            "finitechat_authenticated_requester_context", default=None
        )
        candidate.lock = threading.Lock()
        # Resolved lease root -> session key -> lease ID -> (count, expiry).
        candidate.leases = {}
        # Resolved lease roots whose startup cleanup has completed.
        candidate.started_roots = set()
        # Resolved lease root -> (stale lease paths a reader could still
        # accept, or None when the lease directories could not be listed;
        # the reason). Present only while startup cleanup is incomplete.
        candidate.quarantine = {}
        state = sys.modules.setdefault(_REQUESTER_STATE_MODULE, candidate)
    return state


_AUTHENTICATED_FINITE_TURN_USER: contextvars.ContextVar[str | None] = _requester_state().turn_user
_AUTHENTICATED_FINITE_REQUESTER_CONTEXT: contextvars.ContextVar[tuple[str, str] | None] = (
    _requester_state().requester_context
)


@dataclass(eq=False)
class _RetryRewind:
    """The live transcript a /retry rewound, just before and just after."""

    store: Any
    session_id: str
    before: list[Any]
    after: list[Any]


@dataclass(eq=False)
class _FiniteTurn:
    """One background turn, as its completion hook needs to settle it."""

    event: MessageEvent
    session_key: str
    # The pinned gateway command the event dispatches as, or None.
    command: str | None
    # Neither a gateway command nor a reply to a pending prompt.
    ordinary: bool
    # The event text as handed over. The pinned gateway rewrites a command
    # it turns into this message's model turn (/queue, /plan, ...).
    text: str
    # The session guard the base adapter runs this turn under, and the run
    # generation bound to it when the turn began. The gateway binds a fresh
    # generation to that guard before each agent run.
    guard: Any
    run_generation: Any
    # Busy-session /steer entries the turn's running agent accepted. They
    # are settled with the turn, so a stopped turn's steer is redelivered.
    riders: list[tuple[str, Any, str]] = field(default_factory=list)
    # Whether the gateway's message handler received this turn's event,
    # whether it returned, and whether it returned before ``stop()`` began.
    dispatched: bool = False
    answered: bool = False
    answered_before_stop: bool = False
    # Set once the completion hook decides how to settle; no rider joins after.
    settled: bool = False
    # That decision: release (True) or ack, once any /retry rewind is
    # settled, the entries still to deliver it to, and the task delivering it.
    release: bool | None = None
    undelivered: list[tuple[str, Any, str]] = field(default_factory=list)
    settlement: asyncio.Task[None] | None = None
    # The transcript this turn's /retry rewound before it re-sent the message,
    # and whether settlement has closed it to a rewind still on its way.
    retry_rewind: _RetryRewind | None = None
    retry_closed: bool = False
    launch: _Launch | None = None
    # The /goal entry a queued kickoff or continuation's turn settles.
    entry: tuple[str, Any, str] | None = None

    def answered_by_gateway(self) -> bool:
        """Hermes carried out this turn's command or prompt reply itself.

        Read from what the gateway's handler did, not from the text alone. A
        command Hermes rewrote into the model's input is model work. Otherwise
        a control command, or a reply to a pending clarify or approval
        prompt, counts once the handler has received it: its effect may be
        under way, so a stop then leaves it unrun at worst, for the user to
        resend. A command that can start model work counts only once the
        handler has returned: /goal then saved the goal and queued its kickoff
        as separate work, /blueprint scheduled its job, and /bg or /btw
        started its child. /retry never counts: it re-sends the last message
        as model work.
        """
        if self.ordinary or self.command == "retry" or (self.event.text or "") != self.text:
            return False
        if self.command in _HERMES_TURN_COMMANDS or self.command in _CHILD_COMMANDS:
            return self.answered
        return self.dispatched


_FINITE_TURN: contextvars.ContextVar[_FiniteTurn | None] = contextvars.ContextVar(
    "finitechat_turn", default=None
)

# Pinned commands whose handler replies once it starts a child task, which
# sends the result itself, and the coroutines those children run.
_CHILD_COMMANDS = frozenset({"bg", "btw"})
_LAUNCH_CHILD_QUALNAMES = (
    "._run_background_task",
    "._handle_btw_command.<locals>._run_side_question",
)
_GOAL_ENDING_ARGS = frozenset({"pause", "clear", "stop", "done"})
LAUNCH_DELIVERY_GRACE_SECS = 1.5
LAUNCH_CANCEL_WAIT_SECS = 0.5
# The pinned adapter teardown default, for an adapter without a gateway.
SETTLE_TIMEOUT_SECS = 5.0
# What disconnect() leaves of that budget for stopping the sidecar.
SERVICE_STOP_RESERVE_SECS = 0.5
TURN_OWNERS_DIR = "hermes-turn-owners"


@dataclass(eq=False)
class _Launch:
    """A /bg, /btw or /goal set|resume dispatch and the work it started past its reply.

    Until that work is done the command's inbox entry stays leased, so a stop
    hands the command back and the rollout's idle check reads the chat busy.
    """

    entry: tuple[str, Any, str]
    key: str
    command: str
    session_key: str
    owned: bool = False
    children: list[asyncio.Task[Any]] = field(default_factory=list)
    queued: list[MessageEvent] = field(default_factory=list)
    queue_key: str = ""
    handed_off: bool = False
    sending: int = 0
    delivered: bool = False
    send_failure: str | None = None
    disconnecting: bool = False
    # Registered until the command's own settlement point has passed.
    command_done: bool = False
    released: bool | None = None

    @property
    def settled(self) -> bool:
        return self.released is not None

    def unfinished(self) -> bool:
        return bool(self.queued) or any(not child.done() for child in self.children)

    def track(self, task: asyncio.Task[Any] | None) -> bool:
        return task is not None and task in self.children

    def record_send(self, result: Any) -> None:
        if getattr(result, "success", False):
            self.delivered = True
        elif self.send_failure != "retryable":
            self.send_failure = "retryable" if getattr(result, "retryable", True) else "final"


def _is_launch_child(
    task: asyncio.Task[Any], launch: _Launch, context: contextvars.ContextVar[_Launch | None]
) -> bool:
    """Created while the handler ran (its context carries ``launch``) and a pinned child.
    Without Python 3.12's ``Task.get_context`` none is found, and the command is acked."""
    get_context = getattr(task, "get_context", None)
    if task.done() or get_context is None or get_context().get(context) is not launch:
        return False
    qualname = str(getattr(task.get_coro(), "__qualname__", ""))
    return qualname.endswith(_LAUNCH_CHILD_QUALNAMES)


def _time_left(deadline: float | None, cap: float | None = None) -> float | None:
    """Seconds until a ``time.monotonic()`` deadline, at most ``cap``; None waits for all."""
    if deadline is None:
        return cap
    left = max(0.0, deadline - time.monotonic())
    return left if cap is None else min(cap, left)


async def _await_until(deadline: float | None, work: Awaitable[Any]) -> None:
    """Await ``work`` until a ``time.monotonic()`` deadline, then cancel it; None awaits it all.
    Work that would start after the deadline never runs."""
    with contextlib.suppress(TimeoutError):
        await asyncio.wait_for(work, _time_left(deadline))


def _write_turn_owner_file(path: Path, entry: tuple[str, Any, str] | None) -> None:
    try:
        if entry is None:
            path.unlink(missing_ok=True)
            return
        path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        room_id, seq, message_id = entry
        temp_path = path.with_name(f".{path.name}.{os.getpid()}.tmp")
        with temp_path.open("w", encoding="utf-8") as handle:
            os.chmod(temp_path, 0o600)
            record = {"version": 1, "room_id": room_id, "seq": seq, "message_id": message_id}
            json.dump(record, handle, sort_keys=True)
            handle.flush()
            os.fsync(handle.fileno())
        temp_path.replace(path)
    except OSError as exc:
        logger.warning("[finitechat] could not record a chat's turn owner: %s", exc)


# Pinned gateway commands that make their message a model turn in an idle
# session: /blueprint (when a blueprint matches), /init, /learn, /moa, /plan,
# /queue and /steer rewrite it into the agent's input, /retry re-sends the
# last message, and /goal queues a kickoff turn. A draining gateway refuses
# that turn, so they wait out a drain like ordinary work. A pinned test
# derives this set from the gateway's dispatch.
_HERMES_TURN_COMMANDS = frozenset(
    {"blueprint", "goal", "init", "learn", "moa", "plan", "queue", "retry", "steer"}
)
# Pinned registry commands the gateway has no dispatch branch for. It hands
# them to the model as an ordinary turn, so the adapter treats them as
# ordinary text. A pinned test derives this set too.
_HERMES_MODEL_TEXT_COMMANDS = frozenset({"curator"})
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


class _RequesterContextBroker:
    """Lease authenticated Finite sender context to turn-local subprocesses.

    Hermes already isolates the session variables with ContextVars and copies
    the active values into terminal subprocesses. The small file lease lets
    `fsite` distinguish that live binding from arbitrary or stale environment
    text without teaching Sites about Chat or Hermes.

    Every broker for one root shares that root's lease counts, so a broker
    from a reloaded plugin can finish a call an earlier one started. A broker
    writes and removes only the files of calls this process holds: any process
    that registers this plugin with the same FINITE_HOME may construct one
    while a turn holds a lease, and a registration that fails takes the
    terminal gate below with it. Readers refuse expired files. Leases
    left by a killed gateway are removed when the next gateway connects; see
    `_clear_requester_leases_at_gateway_start`. Until that cleanup completes,
    `before_tool_call` retries it and blocks terminal calls in the sessions it
    could not clean. The state module name is
    versioned: change its data shape only under a new name, since a running
    process keeps the first shape it published.
    """

    def __init__(self, root: Path | None = None) -> None:
        self.root = root or _requester_context_root()
        self.root_v2 = self.root.parent / REQUESTER_CONTEXT_V2_DIR
        state = _requester_state()
        self._lock = state.lock
        with self._lock:
            try:
                for root in (self.root, self.root_v2):
                    root.mkdir(mode=0o700, parents=True, exist_ok=True)
                    root.chmod(0o700)
            except OSError as exc:
                logger.warning("[finitechat] could not prepare requester context leases: %s", exc)
            self._leases: dict[str, dict[str, tuple[int, int]]] = state.leases.setdefault(
                os.path.realpath(self.root), {}
            )
            self._prune(int(time.time()))

    def before_tool_call(self, **kwargs: Any) -> dict[str, str] | None:
        if str(kwargs.get("tool_name") or "") != "terminal":
            return None
        blocked = _stale_requester_lease_block(self.root)
        if blocked is not None:
            return blocked
        session_key, user_id = _active_finite_session()
        if session_key is None or user_id is None:
            return None
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
            self._write(
                session_key=session_key,
                user_id=user_id,
                expires_at_unix=now + REQUESTER_CONTEXT_TTL_SECS,
            )
        return None

    def after_tool_call(self, **kwargs: Any) -> None:
        if str(kwargs.get("tool_name") or "") != "terminal":
            return
        session_key, _ = _active_finite_session()
        if session_key is None:
            return
        lease_id = _requester_context_lease_id(kwargs)
        with self._lock:
            session_leases = self._leases.get(session_key)
            if session_leases is None:
                # No call this process holds; the file may be another process's.
                return
            count, expires_at = session_leases.get(lease_id, (0, 0))
            if count <= 1:
                session_leases.pop(lease_id, None)
            else:
                session_leases[lease_id] = (count - 1, expires_at)
            if not session_leases:
                self._leases.pop(session_key, None)
                self._remove(session_key)

    def _prune(self, now: int) -> None:
        for leases in self._leases.values():
            for lease_id, (_, expires_at) in list(leases.items()):
                if expires_at <= now:
                    leases.pop(lease_id, None)
        expired_keys = [session_key for session_key, leases in self._leases.items() if not leases]
        for session_key in expired_keys:
            self._leases.pop(session_key, None)
            self._remove(session_key)

    def _write(self, *, session_key: str, user_id: str, expires_at_unix: int) -> None:
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
            requester_context = _AUTHENTICATED_FINITE_REQUESTER_CONTEXT.get()
            if requester_context is not None:
                email, sites_assertion = requester_context
                self._write_v2(
                    session_key=session_key,
                    user_id=user_id,
                    email=email,
                    sites_assertion=sites_assertion,
                    expires_at_unix=expires_at_unix,
                )
        except OSError as exc:
            logger.warning("[finitechat] could not write requester context lease: %s", exc)

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

    def _remove(self, session_key: str) -> None:
        for root in (self.root, self.root_v2):
            with contextlib.suppress(OSError):
                (root / _requester_context_filename(session_key)).unlink(missing_ok=True)


def _clear_requester_leases_at_gateway_start() -> None:
    """Remove leases a previous gateway left for this FINITE_HOME, once per process.

    A gateway killed mid-call never runs its post hook, and the readers check
    only session, user and expiry, so a later turn in that session could use
    the dead call's lease. Pinned Hermes takes the gateway lock for its
    HERMES_HOME before connecting any adapter, and the Finite Runtime runs one
    gateway per Agent with its own HERMES_HOME and FINITE_HOME. So at this
    gateway's first connect, a lease this process does not hold belongs to a
    process that is gone. Two Hermes homes sharing one FINITE_HOME is not a
    supported layout: a second gateway's start would end the first's live
    calls, which then fail closed as "requester context unavailable".

    A first connect can fail before reaching this; Hermes then retries with a
    new adapter and `is_reconnect=True`. So every connect calls this, and the
    shared state records the root only once cleanup completes. Later connects
    and plugin reloads skip it.

    Cleanup never fails the connect, so Chat stays up. A stale lease that
    cannot be removed and has not expired is quarantined instead: the broker
    blocks terminal calls in its session (every session when the directories
    cannot be listed) and retries the removal on each terminal call and
    connect until it succeeds or the lease expires.
    """
    root = _requester_context_root()
    state = _requester_state()
    resolved = os.path.realpath(root)
    with state.lock:
        if resolved in state.started_roots:
            return
        _sweep_stale_requester_leases(state, root, resolved)
        stale = state.quarantine.get(resolved)
    if stale is not None:
        logger.error(
            "[finitechat] could not clear requester leases a previous gateway left under %s "
            "(%s); terminal calls in the affected Finite Chat sessions are blocked until "
            "the files can be removed or expire. Fix the permissions or remove the files.",
            root.parent,
            stale[1],
        )


def _sweep_stale_requester_leases(state: Any, root: Path, resolved: str) -> None:
    """Remove leftover leases this process does not hold; the caller holds state.lock.

    Records the root as started when no lease a reader could accept remains,
    otherwise quarantines what remains. A retry revisits only the quarantined
    files, so it never removes a lease written after the first sweep.
    """
    held = {_requester_context_filename(key) for key in state.leases.get(resolved, {})}
    previous = state.quarantine.get(resolved)
    if previous is not None and previous[0] is not None:
        candidates = [Path(path) for path in previous[0]]
    else:
        candidates = []
        for directory in (root, root.parent / REQUESTER_CONTEXT_V2_DIR):
            try:
                candidates.extend(directory.iterdir())
            except FileNotFoundError:
                continue
            except OSError as exc:
                state.quarantine[resolved] = (None, f"cannot list {directory}: {exc.strerror}")
                return
    now = int(time.time())
    stale: dict[str, str] = {}
    for path in candidates:
        if path.name not in held:
            reason = _remove_stale_lease(path, now)
            if reason is not None:
                stale[str(path)] = reason
    if stale:
        more = f" and {len(stale) - 1} more" if len(stale) > 1 else ""
        state.quarantine[resolved] = (frozenset(stale), min(stale.values()) + more)
        return
    if state.quarantine.pop(resolved, None) is not None:
        logger.warning("[finitechat] cleared the requester leases a previous gateway left")
    state.started_roots.add(resolved)


def _remove_stale_lease(path: Path, now: int) -> str | None:
    """Remove one leftover file; return why a reader could still accept it, if so.

    Only files and symlinks are removed; directories are left alone. A file
    that cannot be removed is harmless only when it has a reader's lease name
    and is a regular file whose integer expiry has passed. Anything else
    unreadable, malformed or unexpired is treated as live.
    """
    try:
        mode = path.lstat().st_mode
        if stat.S_ISREG(mode) or stat.S_ISLNK(mode):
            path.unlink()
        return None
    except FileNotFoundError:
        return None
    except OSError as exc:
        if _REQUESTER_CONTEXT_NAME.fullmatch(path.name) is None or _lease_expired(path, now):
            return None
        return f"cannot remove {path}: {exc.strerror}"


def _lease_expired(path: Path, now: int) -> bool:
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    except OSError:
        return False
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_size > REQUESTER_CONTEXT_MAX_BYTES:
            return False
        payload = json.loads(os.read(fd, REQUESTER_CONTEXT_MAX_BYTES + 1))
    except (OSError, ValueError, RecursionError):
        return False
    finally:
        os.close(fd)
    expires_at = payload.get("expires_at_unix") if isinstance(payload, dict) else None
    return type(expires_at) is int and 0 <= expires_at <= now


def _stale_requester_lease_block(root: Path) -> dict[str, str] | None:
    """Return a block directive for a terminal call that could use a stale lease.

    The session's own platform and key decide, not the authenticated-turn
    marker: an internal turn carries the session and user of an earlier
    request, and the readers check nothing else. Hermes treats a pre-tool
    hook that raises as allowing the call, so any failure here blocks.
    """
    scope = "Other sessions are not affected."
    try:
        state = _requester_state()
        resolved = os.path.realpath(root)
        with state.lock:
            if resolved not in state.quarantine:
                return None
            _sweep_stale_requester_leases(state, root, resolved)
            stale = state.quarantine.get(resolved)
        if stale is None:
            return None
        from gateway.session_context import get_session_env

        platform = str(get_session_env("HERMES_SESSION_PLATFORM", "") or "").strip()
        session_key = str(get_session_env("HERMES_SESSION_KEY", "") or "").strip()
        if platform not in {FINITE_PLATFORM_NAME, Platform.LOCAL.value} or not session_key:
            return None
        paths, reason = stale
        if paths is None:
            scope = "Every Finite Chat session is affected."
        elif _requester_context_filename(session_key) not in {Path(path).name for path in paths}:
            return None
    except Exception as exc:
        logger.exception("[finitechat] could not check for stale requester leases")
        reason = f"the check failed: {exc}"
        scope = "Other sessions may be affected too."
    return {
        "action": "block",
        "message": (
            "Terminal is unavailable in this Finite Chat session: a requester lease left "
            f"by a previous gateway could not be cleared ({reason}). Commands here could "
            f"otherwise act for that earlier request. {scope} This clears by itself once "
            "the file can be removed or expires; an operator can remove it or fix the "
            "directory permissions."
        ),
    }


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


def _active_finite_session() -> tuple[str | None, str | None]:
    try:
        from gateway.session_context import get_session_env
    except ImportError:
        return None, None
    platform = str(get_session_env("HERMES_SESSION_PLATFORM", "") or "").strip()
    session_key = str(get_session_env("HERMES_SESSION_KEY", "") or "").strip()
    user_id = str(get_session_env("HERMES_SESSION_USER_ID", "") or "").strip()
    authenticated_turn_user = _AUTHENTICATED_FINITE_TURN_USER.get()
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
        # Set once this adapter begins stopping its service. A settlement still
        # running then must not start another: nothing would stop it, and it
        # would hold the store's writer lease from the next gateway's service.
        # Hermes reconnects with a new adapter, which starts its own.
        self._service_stopped = False
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
        # The background turn each session is running, which an accepted
        # /steer is settled with.
        self._running_turns: dict[str, _FiniteTurn] = {}
        # Events a draining gateway refused after handoff. A redelivery waits
        # out the drain even if it is a command the adapter would hand over.
        # The drain ends in stop(), so this lives no longer than the process.
        self._drain_refused: set[str] = set()
        # The rewind of a /retry entry whose re-sent message Hermes queued
        # behind a reserved slot, with the entry, for the queued turn that
        # settles it. Disconnect settles any whose queued turn never ran.
        self._retry_rewinds: dict[str, tuple[_RetryRewind, tuple[str, Any, str]]] = {}
        # Settlements in flight. Each outlives a cancellation of its turn.
        self._settlements: set[asyncio.Task[None]] = set()
        self._launches: dict[str, _Launch] = {}
        # Per adapter: plugin rediscovery can import this module again.
        self._launch_context: contextvars.ContextVar[_Launch | None] = contextvars.ContextVar(
            "finitechat_launch", default=None
        )
        # Owned /goal entries (key -> session), and those a later control ended.
        self._goal_entries: dict[str, str] = {}
        self._final_entries: set[str] = set()
        self._owner_writes: dict[str, asyncio.Future[None]] = {}

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
        # The pinned base turn runs under the guard handle_message installed.
        guard = self._active_sessions.get(session_key)
        command = _gateway_command(event)
        if command == "retry":
            _watch_retry_rewinds(self._gateway_session_store())
        turn = _FiniteTurn(
            event=event,
            session_key=session_key,
            command=command,
            ordinary=command is None and not self._is_immediate_text_control(event, session_key),
            text=event.text or "",
            guard=guard,
            run_generation=getattr(guard, "_hermes_run_generation", None),
        )
        launch = self._take_queued_launch(event)
        if launch is not None:
            turn.entry = launch.entry
            self._inflight_admissions.add(launch.key)
        room_id, seq, message_id = turn.entry or _inbox_entry(event, self.room_id)
        owner = (room_id, seq, message_id) if message_id and isinstance(seq, int) else None
        if owner is not None or not self._gateway_draining():
            # A turn the draining gateway refuses never runs, so it cannot take
            # the chat's recovery from an inbox turn the stop released.
            self._record_turn_owner(session_key, owner)
        turn_token = _FINITE_TURN.set(turn)
        self._running_turns[session_key] = turn
        try:
            await super()._process_message_background(event, session_key)
        finally:
            if self._running_turns.get(session_key) is turn:
                del self._running_turns[session_key]
            _FINITE_TURN.reset(turn_token)
            _AUTHENTICATED_FINITE_REQUESTER_CONTEXT.reset(context_token)
            _AUTHENTICATED_FINITE_TURN_USER.reset(token)
            if not turn.settled:
                # Hermes queued the event for a later turn, which settles it.
                # Steers its agent took are released to run as their own turn.
                turn.settled = True
                for room_id, seq, message_id in turn.riders:
                    self._inflight_admissions.discard(
                        _adapter_event_key(room_id, seq, message_id) or ""
                    )
                    await self._release_finitechat_event(room_id, seq, message_id)

    def set_message_handler(self, handler: Any) -> None:
        """Record when the gateway's handler receives and returns each turn's event.

        Settlement reads a command's progress from these: a stop can cancel
        the turn before the base adapter calls the handler, or while the
        handler is still running. ``stop()`` marks the gateway stopping before
        it interrupts running turns, so a handler that returned before then
        finished its work uninterrupted.

        It also records each /bg, /btw and /goal launch (``_Launch``).
        """

        async def handle(event: MessageEvent) -> Any:
            turn = _FINITE_TURN.get()
            if turn is not None and turn.event is not event:
                turn = None
            command = _gateway_command(event)
            launch = self._begin_launch(event, command)
            ends_goal = command == "goal" and _goal_args(event) in _GOAL_ENDING_ARGS
            if turn is None and launch is None and not ends_goal:
                return await handler(event)
            if turn is not None:
                turn.dispatched = True
                turn.launch = launch
            token = self._launch_context.set(launch)
            try:
                response = await handler(event)
            finally:
                self._launch_context.reset(token)
                if launch is not None:
                    self._own_launch_work(launch)
            if ends_goal:
                self._end_goal_launches(self._event_session_key(event))
            if turn is not None:
                turn.answered = True
                turn.answered_before_stop = not self._gateway_stopping()
            return response

        super().set_message_handler(handle)

    async def handle_message(self, event: MessageEvent) -> None:
        """Decline Hermes's auto-resume turn where the inbox owns recovery.

        The inbox redelivers a turn it owns whole, so a resume would run it
        twice; the session stays marked, so the redelivery gets the recovery
        note. A turn with no inbox entry, like a goal continuation, resumes.
        """
        if _is_hermes_resume_event(event) and await self._inbox_owns_latest_turn(
            self._event_session_key(event)
        ):
            logger.info(
                "[finitechat] declined Hermes auto-resume for chat %s; "
                "the Finite inbox redelivers interrupted turns",
                getattr(event.source, "chat_id", None),
            )
            return
        await super().handle_message(event)

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
        elif pending is not None:
            self._hand_queued_launch_to_turn(pending, session_key)
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
        _clear_requester_leases_at_gateway_start()
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
        settle_by = self._settle_deadline()
        if self._poll_task:
            self._poll_task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await self._poll_task
            self._poll_task = None
        await self._cancel_admission_tasks(settle_by)
        # Hermes's stop has done this already; its fatal-adapter path has not.
        await self.cancel_background_tasks()
        await self._settle_orphaned_retry_rewinds(settle_by)
        await self._settle_launches_at_disconnect(settle_by)
        await self._finish_settlements(settle_by)
        await self._stop_service()
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
        launch = self._child_launch()
        if launch is None:
            return await self._send_message(chat_id, content, reply_to, metadata)
        launch.sending += 1
        try:
            result = await self._send_message(chat_id, content, reply_to, metadata)
        except Exception:
            launch.send_failure = "retryable"
            raise
        finally:
            launch.sending -= 1
        launch.record_send(result)
        return result

    async def _send_message(
        self,
        chat_id: str,
        content: str,
        reply_to: str | None,
        metadata: dict[str, Any] | None,
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
                await self._release_stranded_stream_events(queue)
            if not self.is_connected:
                break
            await asyncio.sleep(_stream_reconnect_delay(reconnect_attempt))
            reconnect_attempt += 1

    async def _release_stranded_stream_events(self, queue: asyncio.Queue) -> None:
        """Release events the stream worker queued after consumption stopped.

        Hermes cancels background turns before ``disconnect()`` cancels this
        loop, so a lease a cancelled turn released can be re-leased onto the
        stream in between. Every delivered event holds a sidecar lease; one
        left unconsumed would stay leased until the lease TTL.
        """
        while not queue.empty():
            result = queue.get_nowait()
            if not result.ok:
                continue
            for raw_record in result.data.get("records") or []:
                if not isinstance(raw_record, dict):
                    continue
                record_type = str(raw_record.get("type") or "")
                raw_event = raw_record.get("event") if record_type == "event" else None
                if not record_type:
                    raw_event = raw_record
                if not isinstance(raw_event, dict) or not raw_event.get("message_id"):
                    continue
                await self._release_finitechat_event(
                    str(raw_event.get("room_id") or self.room_id),
                    raw_event.get("seq"),
                    str(raw_event["message_id"]),
                )

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
        if self._launch_coalesces(event_key):
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
        session_key = self._event_session_key(event)
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
        if not self._should_defer_admission(event, session_key, event_key or "") and (
            await self._admit_finitechat_event(
                event,
                room_id,
                seq,
                message_id,
                event_key or "",
            )
        ):
            return
        self._defer_admission(
            session_key,
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
    ) -> bool:
        """Hand the event to Hermes; False if it must wait in the ordered queue."""
        raw_event = event.raw_message if isinstance(event.raw_message, dict) else {}
        conversation_id = _string_or_none(raw_event.get("conversation_id"))
        segment_id = _string_or_none(raw_event.get("segment_id"))
        session_key = self._event_session_key(event)
        activity_metadata = self._route_metadata(conversation_id, segment_id)
        activity_set = False
        session_active = False
        held = False
        steered_turn: _FiniteTurn | None = None
        # The sidecar leased this entry on delivery. Its lease is settled only
        # by the turn: the completion hook acks on success, failure, or a user
        # /stop, and a shutdown-cancelled turn releases it. A turn that fails
        # synchronously before completion is released here so the sidecar
        # redelivers it whole.
        try:
            await self._hydrate_hermes_home_channel_if_needed()
            activity_set = await self._set_processing_activity(room_id, activity_metadata)
            # A drain, or another turn in this session, can begin during the
            # awaits above. Nothing awaits between this check and
            # handle_message handing the event over; a drain that begins
            # after the handoff is settled by the hook.
            held = self._must_wait(event, session_key, event_key)
            if not held:
                session_active = self._session_is_active(session_key)
                if session_active and _gateway_command(event) == "steer":
                    steered_turn = self._running_turns.get(session_key)
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
        if held:
            if activity_set:
                await self._clear_processing_activity(room_id, activity_metadata)
            return False
        if (
            event_key
            and event_key in self._inflight_admissions
            and session_active
            and not self._hermes_queued(session_key, event)
        ):
            # Events consumed inline by a busy session (slash-command bypass,
            # busy-session handlers) never pass through the background turn that
            # fires the completion hook, so ack here — exactly once. Every other
            # event is acked (or released) by the completion hook. A /steer
            # the running agent took is part of that turn and settles with it,
            # and a /bg or /btw with the child it started.
            self._inflight_admissions.discard(event_key)
            launch = self._launches.get(event_key)
            if not self._launch_takes_entry(launch) and not self._ride_with_turn(
                steered_turn, room_id, seq, message_id
            ):
                await self._ack_finitechat_event(room_id, seq, message_id)
        return True

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

    def _should_defer_admission(
        self, event: MessageEvent, session_key: str, event_key: str
    ) -> bool:
        # Media needs the same durable admission as text. Hermes can merge
        # busy-session media into one pending event and run it recursively
        # inside the current turn, without a completion hook for each lease.
        # Admit each event as its own turn so its hook alone settles it.
        if event.internal:
            return False
        return self._must_wait(
            event, session_key, event_key, behind=session_key in self._deferred_admissions
        )

    def _must_wait(
        self,
        event: MessageEvent,
        session_key: str,
        event_key: str,
        *,
        behind: bool = False,
    ) -> bool:
        """The event waits in the session's ordered queue instead of reaching Hermes.

        Ordinary work waits behind a busy turn (or the queue, when ``behind``)
        and runs as its own turn. So do /queue, and a /steer the running
        agent cannot take: Hermes would keep either as a new in-memory event,
        which a stop or drain drops. Other gateway commands and replies to a
        pending clarify or approval prompt stay live. While the gateway drains
        it refuses any new model turn, so ordinary work and every command in
        ``_HERMES_TURN_COMMANDS`` wait for the restart; a /steer into a
        running turn still reaches it.

        The base adapter runs a command inline while the session's guard is
        set. If no gateway turn is running then, as while the previous turn
        finishes after its ack, Hermes would run a command in
        ``_HERMES_TURN_COMMANDS`` to completion there, outside any background
        turn and its settlement. Such a command waits for its own turn. With a
        gateway turn running, Hermes answers it with its busy-command policy.
        A /goal control form starts no turn, so it stays live there: a goal
        loop keeps its chat busy between its turns, and a held /goal pause
        would wait out the whole loop.
        """
        if self._gateway_startup_restoring():
            # Commands included: the gate queues every non-internal event.
            return True
        if self._is_immediate_text_control(event, session_key):
            return False
        command = _gateway_command(event)
        steers_running_turn = command == "steer" and self._running_agent_takes_steer(session_key)
        if self._gateway_draining():
            return not steers_running_turn and (
                command is None
                or command in _HERMES_TURN_COMMANDS
                or event_key in self._drain_refused
            )
        next_turn = command in (None, "queue") or (command == "steer" and not steers_running_turn)
        if next_turn:
            return behind or self._session_is_active(session_key)
        return (
            command in _HERMES_TURN_COMMANDS
            and not (command == "goal" and _goal_control(event))
            and self._session_is_active(session_key)
            and not self._gateway_turn_running(session_key)
        )

    def _gateway_turn_running(self, session_key: str) -> bool:
        """The gateway holds a running turn (or one being set up) for the session."""
        runner = getattr(self, "gateway_runner", None)
        running = getattr(runner, "_is_session_running", None)
        return not callable(running) or bool(running(session_key))

    def _running_agent_takes_steer(self, session_key: str) -> bool:
        """The session's running agent can take a /steer into its current turn.

        Otherwise (no turn, the agent still starting, or no steer support) the
        pinned gateway would queue the steer for the next turn in memory.
        """
        if not self._session_is_active(session_key):
            return False
        runner = getattr(self, "gateway_runner", None)
        peek = getattr(runner, "_peek_session_state", None)
        state = peek(session_key) if callable(peek) else None
        agent = getattr(getattr(state, "turn", None), "agent", None)
        return callable(getattr(agent, "steer", None))

    def _gateway_draining(self) -> bool:
        """The pinned gateway refuses new turns while it drains to stop or restart.

        It replies with a refusal that the completion hook sees as a successful
        turn, so a handed-off event would be acked without running. Held
        leases are released when the adapter disconnects.
        """
        return bool(getattr(getattr(self, "gateway_runner", None), "_draining", False))

    def _gateway_startup_restoring(self) -> bool:
        """The pinned gateway's startup-restore gate is closed.

        While it is closed, ``_handle_message`` queues inbound events in
        memory and returns, so the completion hook would ack an event before
        it reaches the model, and a stop or crash before the queue drains
        would lose it. The gate's replay also bypasses this adapter's ordered
        admission. Held leases wait for the gate to open, or are released
        when the adapter disconnects.
        """
        runner = getattr(self, "gateway_runner", None)
        return bool(getattr(runner, "_startup_restore_in_progress", False))

    def _gateway_stopping(self) -> bool:
        """``stop()`` has begun, so turns still running are being interrupted.

        Only ``stop()`` clears ``_running`` while draining. A restart drain
        keeps it set while running turns finish normally.
        """
        runner = getattr(self, "gateway_runner", None)
        return self._gateway_draining() and not getattr(runner, "_running", True)

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

    def _event_session_key(self, event: MessageEvent) -> str:
        return build_session_key(
            event.source,
            group_sessions_per_user=self.config.extra.get("group_sessions_per_user", True),
            thread_sessions_per_user=self.config.extra.get("thread_sessions_per_user", False),
        )

    def _hermes_queued(self, session_key: str, event: MessageEvent) -> bool:
        """Hermes queued this inbox entry for a later turn, which settles it.

        The gateway queues a busy or reserved session's event as is, or, for
        /queue, as a new event that carries the same inbox record.
        """
        pending = getattr(self, "_pending_messages", {}).get(session_key)
        if pending is None:
            return False
        return pending is event or (
            isinstance(event.raw_message, dict) and pending.raw_message is event.raw_message
        )

    @staticmethod
    def _ride_with_turn(turn: _FiniteTurn | None, room_id: str, seq: Any, message_id: str) -> bool:
        """Settle an accepted /steer with the inbox turn it steered, if still running."""
        if turn is None or turn.settled or not isinstance(turn.event.raw_message, dict):
            return False
        turn.riders.append((room_id, seq, message_id))
        return True

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
                if self._gateway_draining() or self._gateway_startup_restoring():
                    await asyncio.sleep(ADMISSION_RECHECK_SECS)
                    continue
                if self._session_is_active(session_key):
                    owner = self._session_tasks.get(session_key)
                    if owner is not None and not owner.done():
                        await asyncio.wait({owner}, timeout=ADMISSION_RECHECK_SECS)
                    else:
                        await asyncio.sleep(ADMISSION_RECHECK_SECS)
                    continue
                event, room_id, seq, message_id, event_key = next(iter(admissions.values()))
                try:
                    if not await self._admit_finitechat_event(
                        event,
                        room_id,
                        seq,
                        message_id,
                        event_key,
                    ):
                        # A drain began during the handoff. The head keeps
                        # its place and the checks above hold it.
                        continue
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
            for launch in list(self._launches.values()):
                if (
                    launch.queued
                    and launch.session_key == session_key
                    and self._precedes_user_interrupt(session_key, launch.entry[0], launch.entry[1])
                ):
                    self._end_queued_launch(launch)
        await self._discard_deferred_admission(session_key)

    async def _discard_deferred_admission(self, session_key: str) -> None:
        task = self._admission_tasks.pop(session_key, None)
        admissions = self._deferred_admissions.pop(session_key, {})
        if task is not None and not task.done():
            task.cancel()
            await asyncio.gather(task, return_exceptions=True)
        for _event, room_id, seq, message_id, _event_key in admissions.values():
            await self._ack_finitechat_event(room_id, seq, message_id)

    async def _cancel_admission_tasks(self, settle_by: float | None = None) -> None:
        """Cancel the deferred admissions, then release their entries in order.

        Releases end at ``settle_by``; an entry not released by then keeps its
        lease until it expires.
        """
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
                await _await_until(
                    settle_by, self._release_finitechat_event(room_id, seq, message_id)
                )

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

    async def _ack_finitechat_event(self, room_id: str, seq: Any, message_id: str) -> bool:
        """Ack an inbox entry; True once the sidecar has it."""
        if not isinstance(seq, int):
            return True
        ack = await self._finitechat_json(
            "ack",
            {"room_id": room_id, "seq": seq, "message_id": message_id},
            timeout=15,
        )
        if not ack.ok:
            logger.warning("[finitechat] failed to ack %s/%s: %s", room_id, seq, ack.error)
        return ack.ok

    async def _release_finitechat_event(self, room_id: str, seq: Any, message_id: str) -> bool:
        """Release an inbox entry for redelivery; True once the sidecar has it."""
        if not isinstance(seq, int):
            return True
        result = await self._finitechat_json(
            "release",
            {"room_id": room_id, "seq": seq, "message_id": message_id},
            timeout=15,
        )
        if not result.ok:
            logger.warning("[finitechat] failed to release %s/%s: %s", room_id, seq, result.error)
        return result.ok

    async def _settle_event_ack(self, event: MessageEvent, outcome_name: str) -> None:
        """Settle the sidecar's inbox lease once the event's turn has run.

        Steers the turn's agent took settle with it. A gateway that queues the
        event behind a reserved or busy session slot returns at once; the
        queued turn settles it.

        Two kinds of turn are acked whatever happens around them. A turn the
        user ended with /stop, /new or /reset is acked, because redelivering
        it would restart the very work the user stopped. The base adapter can
        finish a stopped turn before it marks the task cancelled, so the
        session's interrupt boundary decides too. A command Hermes carried out
        itself (see ``_FiniteTurn.answered_by_gateway``) is acked too, even
        when a stop cancels it, because running it again after the restart
        would repeat its effect.

        Model work (ordinary text, a command Hermes rewrote into the model's
        input or has not carried out yet, and /retry) is released when a
        shutdown cancels it, when its handler returns after ``stop()`` begins,
        or when a draining gateway refused it, so it runs after the restart.
        ``stop()`` interrupts running turns cooperatively
        (``restart_drain_timeout`` defaults to 0s) and reports them as SUCCESS
        or FAILURE, never CANCELLED; a turn that completed in that window may
        run once more. A turn whose handler returned before ``stop()`` began
        finished uninterrupted and is acked, even if this hook comes later,
        unless the stop cancels it first. Any other success or failure acks: a failed turn still
        ran and answered the user. A /retry rewinds the transcript before it
        re-sends, so its rewind is undone first unless the re-sent message
        took effect.

        The decision is made once per turn. The pinned base adapter runs this
        hook a second time, as CANCELLED, when a shutdown cancels the turn's
        task after the hook began: in the usage call after the ack, or while
        the settlement is still on its way. Deciding again could reverse the
        first decision, and the sidecar's ack removes an entry a release has
        just returned to Pending. So the settlement runs as its own task,
        which the cancellation does not reach, and the second run joins it.
        """
        turn = _FINITE_TURN.get()
        if turn is not None and turn.event is not event:
            turn = None
        if turn is not None and turn.entry is not None:
            room_id, seq, message_id = turn.entry
        else:
            room_id, seq, message_id = _inbox_entry(event, self.room_id)
        if not message_id and (turn is None or not turn.riders):
            return
        event_key = _adapter_event_key(room_id, seq, message_id) if message_id else None
        if turn is not None and turn.settlement is not None:
            await self._join_settlement(turn)
            return
        if turn is not None and self._launch_takes_entry(turn.launch):
            if event_key:
                self._inflight_admissions.discard(event_key)
            return
        if (
            turn is not None
            and outcome_name != "cancelled"
            and self._hermes_queued(turn.session_key, event)
        ):
            if turn.retry_rewind is not None and event_key:
                self._retry_rewinds[event_key] = (turn.retry_rewind, (room_id, seq, message_id))
            return
        # Claim the in-flight marker so the inline-admission path does not also
        # ack this event once its background turn's completion hook has fired.
        if event_key:
            self._inflight_admissions.discard(event_key)
        queued_rewind = self._retry_rewinds.pop(event_key, None) if event_key else None
        rewind = queued_rewind[0] if queued_rewind is not None else None
        if turn is not None and turn.retry_rewind is not None:
            rewind = turn.retry_rewind
        user_ended = asyncio.current_task() in self._user_cancelled_tasks or (
            bool(self._user_interrupt_boundaries)
            and self._precedes_user_interrupt(
                turn.session_key if turn is not None else self._event_session_key(event),
                room_id,
                seq,
            )
        )
        model_started = turn is not None and self._model_run_started(turn)
        final = user_ended or (
            turn is not None and not model_started and turn.answered_by_gateway()
        )
        refused = not final and turn is not None and self._gateway_draining() and not model_started
        if refused and event_key:
            self._drain_refused.add(event_key)
        interrupted_by_stop = self._gateway_stopping() and not (
            turn is not None and turn.answered_before_stop
        )
        release = not final and (outcome_name == "cancelled" or interrupted_by_stop or refused)
        retry = rewind is not None or (turn is not None and turn.command == "retry")
        entries = [(room_id, seq, message_id)] if message_id else []
        if turn is not None:
            turn.settled = True
            entries.extend(turn.riders)
            for rider in turn.riders:
                self._inflight_admissions.discard(_adapter_event_key(*rider) or "")
        settlement = asyncio.ensure_future(
            self._settle_turn(
                turn, entries, rewind, release=release, model_started=model_started, retry=retry
            )
        )
        self._settlements.add(settlement)
        settlement.add_done_callback(self._settlements.discard)
        if turn is not None:
            turn.settlement = settlement
        await asyncio.shield(settlement)

    async def _settle_turn(
        self,
        turn: _FiniteTurn | None,
        entries: list[tuple[str, Any, str]],
        rewind: _RetryRewind | None,
        *,
        release: bool,
        model_started: bool,
        retry: bool,
    ) -> None:
        """Settle ``entries`` one way, once any /retry rewind is settled.

        A released entry runs again whichever turn starts next, so the chat's
        recovery is the inbox's; that is recorded first.
        """
        try:
            if retry:
                release = await self._settle_retry_rewind(
                    turn, rewind, release=release, model_started=model_started
                )
            if turn is not None:
                turn.release = release
                turn.undelivered = entries
                redelivered = [
                    entry
                    for entry in entries
                    if _adapter_event_key(*entry) not in self._final_entries
                ]
                if release and redelivered:
                    await self._record_turn_owner(turn.session_key, redelivered[0])
            await self._deliver_settlement(entries, release=release)
        except Exception:
            logger.exception("[finitechat] could not settle %s", entries[0])

    async def _join_settlement(self, turn: _FiniteTurn) -> None:
        """Wait out the turn's settlement, then resend it to entries the sidecar missed."""
        if turn.settlement is not None:
            await asyncio.shield(turn.settlement)
        if turn.release is not None and turn.undelivered:
            await self._deliver_settlement(turn.undelivered, release=turn.release)

    async def _deliver_settlement(
        self, entries: list[tuple[str, Any, str]], *, release: bool
    ) -> None:
        """Ack or release each entry; one stays listed until the sidecar has it.

        The sidecar's ack and release are idempotent, so resending one is safe.
        An entry a later /goal control made final is acked either way.
        """
        for entry in list(entries):
            key = _adapter_event_key(*entry)
            final = key in self._final_entries
            if release and not final:
                settled = await self._release_finitechat_event(*entry)
            else:
                settled = await self._ack_finitechat_event(*entry)
            if settled:
                entries.remove(entry)
                if not release or final:
                    self._goal_entries.pop(key or "", None)
                    self._final_entries.discard(key or "")

    async def _settle_orphaned_retry_rewinds(self, settle_by: float | None = None) -> None:
        """Settle each /retry whose re-sent message Hermes queued and then dropped.

        Hermes queues the re-sent message behind a reserved session slot, and
        a stop clears that queue, so the turn that would settle the entry
        never runs and its lease would outlive the restart with the rewind in
        place. The rewind is undone under the same no-overwrite check and the
        entry released, as for a refused re-send. A rewind whose queued turn
        is still running is left to that turn.

        Each rewind is undone whatever the time, but its sidecar call ends at
        ``settle_by``. An entry not settled by then keeps its lease until it
        expires, as when the sidecar is down.
        """
        running = {
            _adapter_event_key(*_inbox_entry(turn.event, self.room_id))
            for turn in self._running_turns.values()
        }
        for event_key in [key for key in self._retry_rewinds if key not in running]:
            rewind, entry = self._retry_rewinds.pop(event_key)
            release = await self._settle_retry_rewind(
                None, rewind, release=True, model_started=False
            )
            await _await_until(settle_by, self._deliver_settlement([entry], release=release))

    def _model_run_started(self, turn: _FiniteTurn) -> bool:
        """The gateway started an agent run for this turn's work.

        Before each agent run the pinned gateway binds a fresh run generation
        to the turn's session guard, so a turn it refused or answered leaves
        that guard as the turn found it. The pinned gateway refuses a new
        model turn only while ``_draining`` is set, which stays set until the
        process stops; a turn that ran and finished during the drain was not
        refused.
        """
        guard = (
            turn.guard if turn.guard is not None else self._active_sessions.get(turn.session_key)
        )
        return getattr(guard, "_hermes_run_generation", None) != turn.run_generation

    async def _settle_retry_rewind(
        self,
        turn: _FiniteTurn | None,
        rewind: _RetryRewind | None,
        *,
        release: bool,
        model_started: bool,
    ) -> bool:
        """Undo a /retry's rewind that its re-sent message never used; True to release.

        The pinned /retry rewinds the transcript, then re-sends the last
        message as a new turn, which a draining gateway refuses and a stop
        can cancel. A redelivered /retry would then rewind once more and
        retry an older message. So the entry is released only once the
        transcript is as it was before the rewind, or the re-sent message's
        own agent run has written to it since (that run is the redelivery's
        to retry). The rewind is undone only while the transcript is exactly
        what it left. If it cannot be undone, the entry is acked: the user
        sees the refusal and nothing is rewound twice.

        A stop can cancel the turn while its rewind is still running in a
        worker thread. Settlement closes the turn under the store's
        transcript lock, so that rewind is either recorded first or refused.
        The turn's settlement task runs this once; a cancellation of the turn
        does not reach it.
        """
        store = rewind.store if rewind is not None else self._gateway_session_store()
        if not _watching_retry_rewinds(store):
            # A rewind could have gone unrecorded.
            return False

        def settle() -> str:
            with store._get_transcript_drain_lock():
                recorded = rewind
                if turn is not None:
                    turn.retry_closed = True
                    recorded = turn.retry_rewind or recorded
                if recorded is None:
                    return "unchanged"
                if model_started and not release:
                    return "kept"
                return _restore_retry_rewind(recorded)

        try:
            outcome = await asyncio.to_thread(settle)
        except Exception:
            logger.exception("[finitechat] could not check a /retry rewind")
            outcome = "failed"
        if outcome in ("unchanged", "restored") or (outcome == "changed" and model_started):
            return release
        if release:
            logger.warning(
                "[finitechat] /retry rewind could not be undone (%s); acking it "
                "so a redelivery does not rewind again",
                outcome,
            )
        return False

    def _turn_owner_path(self, session_key: str) -> Path | None:
        if not self.home:
            return None
        digest = hashlib.sha256(session_key.encode("utf-8")).hexdigest()
        return Path(self.home) / TURN_OWNERS_DIR / f"{digest}.json"

    def _record_turn_owner(
        self, session_key: str, entry: tuple[str, Any, str] | None
    ) -> asyncio.Future[None]:
        """Record durably, in order per chat, whether the inbox owns its latest turn.
        It outlives the ack: crash recovery marks every recently active chat for
        resume, and one whose last turn the inbox settled has nothing to resume."""
        write = asyncio.ensure_future(
            self._write_turn_owner(
                self._owner_writes.get(session_key), self._turn_owner_path(session_key), entry
            )
        )
        self._owner_writes[session_key] = write
        write.add_done_callback(lambda done: self._forget_owner_write(session_key, done))
        return write

    def _forget_owner_write(self, session_key: str, write: asyncio.Future[None]) -> None:
        if self._owner_writes.get(session_key) is write:
            del self._owner_writes[session_key]

    @staticmethod
    async def _write_turn_owner(
        previous: asyncio.Future[None] | None,
        path: Path | None,
        entry: tuple[str, Any, str] | None,
    ) -> None:
        if previous is not None:
            await asyncio.wait({previous})
        if path is not None:
            await asyncio.to_thread(_write_turn_owner_file, path, entry)

    async def _inbox_owns_latest_turn(self, session_key: str) -> bool:
        """The inbox owned this chat's latest turn; a chat with no record was last
        served by a release that left its interrupted turns to Hermes's resume."""
        path = self._turn_owner_path(session_key)
        if path is None:
            return True
        pending = self._owner_writes.get(session_key)
        if pending is not None:
            await asyncio.wait({pending})
        return await asyncio.to_thread(path.exists)

    def _begin_launch(self, event: MessageEvent, command: str | None) -> _Launch | None:
        if command not in _CHILD_COMMANDS and (command != "goal" or _goal_control(event)):
            return None
        room_id, seq, message_id = _inbox_entry(event, self.room_id)
        key = _adapter_event_key(room_id, seq, message_id) if message_id else None
        if key is None:
            return None
        if command == "goal":
            self._watch_goal_kickoffs()
        return _Launch(
            entry=(room_id, seq, message_id),
            key=key,
            command=command,
            session_key=self._event_session_key(event),
        )

    def _own_launch_work(self, launch: _Launch) -> None:
        """The pinned handlers start each child last with no await before they
        return, so every child is still pending here."""
        launch.children = [
            task
            for task in asyncio.all_tasks()
            if _is_launch_child(task, launch, self._launch_context)
        ]
        launch.owned = bool(launch.children or launch.queued)
        if not launch.owned:
            return
        self._launches[launch.key] = launch
        if launch.queued:
            self._goal_entries[launch.key] = launch.session_key
        for child in launch.children:
            child.add_done_callback(lambda _child, launch=launch: self._child_finished(launch))

    def _watch_goal_kickoffs(self) -> None:
        """Record the event the pinned /goal handler queues through ``_enqueue_fifo``,
        without the inbox record, which would change the kickoff's requester. The
        launch is read from the adapter argument, which serves any adapter instance."""
        runner = getattr(self, "gateway_runner", None)
        enqueue = getattr(runner, "_enqueue_fifo", None)
        if (
            runner is None
            or not callable(enqueue)
            or getattr(runner, "_finitechat_watches_goal_queue", False)
        ):
            return

        def enqueue_fifo(*args: Any, **kwargs: Any) -> Any:
            adapter = args[2] if len(args) > 2 else kwargs.get("adapter")
            context = getattr(adapter, "_launch_context", None)
            launch = context.get() if isinstance(context, contextvars.ContextVar) else None
            queued = args[1] if len(args) > 1 else kwargs.get("queued_event")
            if launch is not None and launch.command == "goal" and isinstance(queued, MessageEvent):
                launch.queued.append(queued)
                launch.queue_key = str(args[0] if args else kwargs.get("session_key") or "")
            return enqueue(*args, **kwargs)

        runner._enqueue_fifo = enqueue_fifo
        runner._finitechat_watches_goal_queue = True

    def _take_queued_launch(self, event: MessageEvent) -> _Launch | None:
        for launch in self._launches.values():
            if any(queued is event for queued in launch.queued):
                launch.queued = [queued for queued in launch.queued if queued is not event]
                if not launch.queued:
                    launch.handed_off = True
                    if launch.command_done:
                        del self._launches[launch.key]
                return launch
        return None

    def _hand_queued_launch_to_turn(self, event: MessageEvent, session_key: str) -> None:
        """The gateway runs a queued /goal event inside the turn it is finishing,
        which settles the entry with its own; a draining gateway discards it."""
        launch = self._take_queued_launch(event)
        if launch is None:
            return
        turn = _FINITE_TURN.get()
        if turn is None or turn.session_key != session_key:
            turn = self._running_turns.get(session_key)
        if self._gateway_draining() or turn is None or turn.settled:
            self._settle_launch(launch, release=True)
            return
        turn.riders.append(launch.entry)
        self._inflight_admissions.add(launch.key)

    def _end_goal_launches(self, session_key: str) -> None:
        """Make earlier /goal entries in the chat final, so a restart cannot revive the goal."""
        for key, goal_session in self._goal_entries.items():
            if goal_session == session_key:
                self._final_entries.add(key)
        for launch in list(self._launches.values()):
            if launch.queued and launch.session_key == session_key:
                self._end_queued_launch(launch)

    def _end_queued_launch(self, launch: _Launch) -> None:
        launch.queued = []
        launch.handed_off = True
        self._settle_launch(launch, release=False)

    def _queued_alive(self, launch: _Launch) -> bool:
        queue: list[Any] = list(getattr(self, "_pending_messages", {}).values())
        runner = getattr(self, "gateway_runner", None)
        peek = getattr(runner, "_peek_session_state", None)
        state = peek(launch.queue_key) if callable(peek) and launch.queue_key else None
        queue += list(getattr(getattr(state, "conversation", None), "queued_events", None) or [])
        return any(queued is event for queued in launch.queued for event in queue)

    def _launch_takes_entry(self, launch: _Launch | None) -> bool:
        """At the command's own settlement point: its launch settles the entry instead."""
        if launch is None or not launch.owned:
            return False
        launch.command_done = True
        if (launch.settled or launch.handed_off) and self._launches.get(launch.key) is launch:
            del self._launches[launch.key]
        return True

    def _launch_coalesces(self, event_key: str | None) -> bool:
        """A lease-expiry redelivery joins its launch's work while that work runs."""
        launch = self._launches.get(event_key) if event_key else None
        if launch is None:
            return False
        if any(not child.done() for child in launch.children) or (
            launch.queued and self._queued_alive(launch)
        ):
            return True
        del self._launches[launch.key]
        return False

    def _child_finished(self, launch: _Launch) -> None:
        """Once any of the result reached the chat, rerunning would repeat finished
        work, so the entry is acked; a retryable failure before that keeps it
        leased until its lease expires or the adapter disconnects."""
        if launch.settled or launch.disconnecting or launch.unfinished():
            return
        where = (launch.command, launch.entry[0], launch.entry[1])
        for child in launch.children:
            if not child.cancelled() and child.exception() is not None:
                logger.warning("[finitechat] /%s child for %s/%s failed", *where)
        if launch.delivered or launch.send_failure == "final":
            if launch.send_failure is not None:
                logger.warning(
                    "[finitechat] /%s result for %s/%s was not fully delivered; acking it",
                    *where,
                )
            self._settle_launch(launch, release=False)
        elif any(child.cancelled() for child in launch.children):
            self._settle_launch(launch, release=True)
        else:
            logger.warning(
                "[finitechat] /%s result for %s/%s was not delivered; its entry stays "
                "leased for redelivery",
                *where,
            )

    def _settle_launch(self, launch: _Launch, *, release: bool) -> None:
        launch.released = release
        if launch.command_done and self._launches.get(launch.key) is launch:
            del self._launches[launch.key]
        settlement = asyncio.ensure_future(
            self._deliver_settlement([launch.entry], release=release)
        )
        self._settlements.add(settlement)
        settlement.add_done_callback(self._settlements.discard)

    async def _settle_launches_at_disconnect(self, settle_by: float | None) -> None:
        """Before the gateway cancels its background tasks. A child already sending
        gets a short grace; the rest are cancelled before their entries are released."""
        launches = [launch for launch in self._launches.values() if not launch.settled]
        sending = [
            child
            for launch in launches
            if launch.sending
            for child in launch.children
            if not child.done()
        ]
        if sending:
            await asyncio.wait(sending, timeout=_time_left(settle_by, LAUNCH_DELIVERY_GRACE_SECS))
        cancelled: list[asyncio.Task[Any]] = []
        for launch in launches:
            if launch.settled or not launch.unfinished():
                continue
            launch.disconnecting = True
            for child in launch.children:
                if not child.done():
                    child.cancel()
                    cancelled.append(child)
        if cancelled:
            await asyncio.wait(cancelled, timeout=_time_left(settle_by, LAUNCH_CANCEL_WAIT_SECS))
        for launch in launches:
            if not launch.settled and not launch.handed_off:
                self._settle_launch(launch, release=not launch.delivered)

    def _settle_deadline(self) -> float | None:
        """When disconnect() stops waiting to settle; None waits for every settlement.

        Hermes abandons disconnect() once its adapter teardown timeout has passed
        since the call (0 waits without bound), so settling ends while the sidecar
        stop still fits. An entry not settled by then keeps its lease until it expires.
        """
        budget = getattr(
            getattr(self, "gateway_runner", None), "_adapter_disconnect_timeout_secs", None
        )
        value = budget() if callable(budget) else None
        timeout = float(value) if isinstance(value, (int, float)) else SETTLE_TIMEOUT_SECS
        if timeout <= 0:
            return None
        return time.monotonic() + timeout - SERVICE_STOP_RESERVE_SECS

    async def _finish_settlements(self, settle_by: float | None) -> None:
        """Wait for the acks, releases and owner writes in flight, until ``settle_by``.

        One still running then is left to finish after the sidecar stops; an
        entry it has not settled keeps its lease until it expires.
        """
        pending = {*self._settlements, *self._owner_writes.values()}
        if pending:
            await asyncio.wait(pending, timeout=_time_left(settle_by))

    def _child_launch(self) -> _Launch | None:
        launch = self._launch_context.get()
        if launch is None or not launch.track(asyncio.current_task()):
            return None
        return launch

    def _gateway_session_store(self) -> Any:
        return getattr(getattr(self, "gateway_runner", None), "session_store", None)

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
        if self._service_stopped:
            return False
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
        if self._service_stopped:
            # The stop began while this process started, and found none to stop.
            await self._stop_service()
            return False

        deadline = asyncio.get_running_loop().time() + SERVICE_START_TIMEOUT_SECS
        while asyncio.get_running_loop().time() < deadline:
            if self._service_stopped:
                return False
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
        self._service_stopped = True
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


def _inbox_entry(event: MessageEvent, default_room_id: Any) -> tuple[str, Any, str]:
    """The inbox entry an event came from: room id, seq and message id."""
    raw_message = event.raw_message if isinstance(event.raw_message, dict) else {}
    return (
        str(raw_message.get("room_id") or default_room_id),
        raw_message.get("seq"),
        str(raw_message.get("message_id") or ""),
    )


def _adapter_event_key(room_id: str, seq: Any, message_id: str) -> str | None:
    if not isinstance(seq, int):
        return None
    return f"{room_id}\x1f{seq}\x1f{message_id}"


def _gateway_command(event: MessageEvent) -> str | None:
    """The pinned gateway command ``event`` dispatches as; None for ordinary work.

    This is the pinned base adapter's own rule for what bypasses a busy
    session: a registry command, named canonically (``/q`` is ``queue``,
    ``/reset`` is ``new``), after it rewrites a DM ``restart the gateway`` to
    ``/restart``. Unknown slash words and path-like text such as ``/usr/bin/x``
    are ordinary work, and so are the registry commands the gateway hands to
    the model as text. The probe is a copy, so the base adapter still
    rewrites the event itself.
    """
    try:
        from gateway.platforms.base import coerce_plaintext_gateway_command
        from hermes_cli.commands import resolve_command
    except ImportError:
        # Gateway test doubles only; the pinned runtime ships both.
        return event.get_command()
    probe = copy.copy(event)
    if getattr(probe, "allow_gateway_control", True):
        coerce_plaintext_gateway_command(probe)
    command = probe.get_command()
    definition = resolve_command(command) if command else None
    if definition is None or definition.name in _HERMES_MODEL_TEXT_COMMANDS:
        return None
    return definition.name


# /goal arguments the pinned handler answers without queueing goal work.
# "resume" queues a continuation; any other text sets (or drafts) a goal.
_GOAL_CONTROL_ARGS = frozenset(
    {"", "status", "show", "pause", "clear", "stop", "done", "wait", "unwait", "gate"}
)


def _goal_args(event: MessageEvent) -> str:
    return (event.get_command_args() or "").strip().lower()


def _goal_control(event: MessageEvent) -> bool:
    """``event``'s /goal is a control form: it reads or changes goal state only.

    The pinned idle ``_handle_goal_command`` grammar, which the gateway runs
    in a finished turn's tail: the whole stripped, lowercased argument, or
    ``wait <pid>`` and ``gate <subcommand>``. A pinned test checks it against
    the handler.
    """
    args = (event.get_command_args() or "").strip().lower()
    return args in _GOAL_CONTROL_ARGS or args.startswith(("wait ", "gate "))


def _watch_retry_rewinds(store: Any) -> None:
    """Record the transcript each Finite /retry rewinds, to undo it if unused.

    Integration hook: the pinned ``_handle_retry_command`` rewinds the live
    transcript (``rewrite_transcript``, or ``rewind_session`` for a
    compaction carrier) and only then re-sends the last message through the
    drain check, so a drain or stop between the two loses that message.
    This wraps the two methods on the gateway's session store; outside a
    Finite /retry turn they run unchanged. Inside one, the first successful
    call records the live transcript just before and just after it, under
    the store's transcript lock that both methods take.
    """
    if store is None or getattr(store, "_finitechat_watches_retry", False):
        return
    names = (
        "rewrite_transcript",
        "rewind_session",
        "load_transcript",
        "_get_transcript_drain_lock",
    )
    if not all(callable(getattr(store, name, None)) for name in names):
        # Settlement then acks a /retry it cannot prove left the transcript whole.
        logger.warning("[finitechat] session store lacks the /retry hooks; /retry is unwatched")
        return
    for name, refused in (("rewrite_transcript", False), ("rewind_session", None)):
        setattr(store, name, _recording_retry_rewind(store, getattr(store, name), refused))
    store._finitechat_watches_retry = True


def _watching_retry_rewinds(store: Any) -> bool:
    return bool(getattr(store, "_finitechat_watches_retry", False))


def _recording_retry_rewind(store: Any, rewind: Any, refused: Any) -> Any:
    def call(session_id: str, *args: Any, **kwargs: Any) -> Any:
        # The store runs in a worker thread that carries the turn's context.
        turn = _FINITE_TURN.get()
        if turn is None or turn.command != "retry" or turn.retry_rewind is not None:
            return rewind(session_id, *args, **kwargs)
        with store._get_transcript_drain_lock():
            if turn.retry_closed:
                # The turn has settled; /retry reports the transcript unchanged.
                return refused
            before = store.load_transcript(session_id)
            result = rewind(session_id, *args, **kwargs)
            if result:
                turn.retry_rewind = _RetryRewind(
                    store, session_id, before, store.load_transcript(session_id)
                )
            return result

    return call


def _restore_retry_rewind(rewind: _RetryRewind) -> str:
    """Put back the transcript a /retry rewound, if nothing has written to it since.

    The check and the write share the store's transcript lock, which its
    transcript writers take, and the write rejects a turn lease held by
    another process. No agent run of the session is active while the
    /retry's turn settles. "changed" leaves the transcript alone.
    """
    store = rewind.store
    with store._get_transcript_drain_lock():
        if store.load_transcript(rewind.session_id) != rewind.after:
            return "changed"
        restored = type(store).rewrite_transcript(
            store,
            rewind.session_id,
            rewind.before,
            active_only=True,
            reject_active_turn_lease=True,
        )
    return "restored" if restored else "failed"


def _is_hermes_resume_event(event: MessageEvent) -> bool:
    """The exact shape of the pinned ``_schedule_resume_pending_sessions`` event.

    Every Finite event carries its inbox record as ``raw_message``. Other
    internal gateway events (background completions, wakeups, plugin
    injections, handoffs) carry text or routing metadata.
    """
    return bool(
        event.internal
        and not (event.text or "").strip()
        and event.raw_message is None
        and event.message_id is None
        and not event.media_urls
        and not event.metadata
    )


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
    register_hook = getattr(ctx, "register_hook", None)
    if callable(register_hook):
        requester_context = _RequesterContextBroker()
        register_hook("pre_tool_call", requester_context.before_tool_call)
        register_hook("post_tool_call", requester_context.after_tool_call)
        register_hook("post_tool_call", _BRAIN_APPROVAL_FILINGS.after_tool_call)
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

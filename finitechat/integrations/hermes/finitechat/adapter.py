"""Finite Chat platform plugin for Hermes.

The adapter is intentionally thin: Hermes callbacks become JSON bridge
requests, and the finitechat daemon/CLI owns validation, cursoring, storage,
encryption, and attachment materialization.
"""

from __future__ import annotations

import asyncio
import contextlib
import contextvars
import hashlib
import json
import logging
import os
import re
import shlex
import shutil
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any

from gateway.config import Platform, PlatformConfig
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
MAX_DELIVERED_EVENT_KEYS = 256
MAX_OUTBOUND_MESSAGE_ROUTES = 256
STREAM_RECONNECT_BACKOFF_SECS = 2.0
STREAM_RECONNECT_MAX_BACKOFF_SECS = 30.0
SERVICE_TRANSPORT_RETRY_SECS = 0.1
ACTIVITY_CONTROL_TIMEOUT_SECS = 1.5
PROCESSING_ACTIVITY_TTL_MILLIS = 15 * 1000
ADMISSION_RECHECK_SECS = 0.05
DEFAULT_FINITE_PRIVATE_CONTROL_URL = "https://finite.computer/api/core/v1/finite-private"
FINITE_PRIVATE_CONTROL_TIMEOUT_SECS = 5
FINITE_ACCOUNT_ID_PATTERN = re.compile(r"[0-9a-f]{64}")
REQUESTER_CONTEXT_DIR = "requester-context-v1"
REQUESTER_CONTEXT_TTL_SECS = 15 * 60
REQUESTER_CONTEXT_VERSION = 1
_AUTHENTICATED_FINITE_TURN_USER: contextvars.ContextVar[str | None] = contextvars.ContextVar(
    "finitechat_authenticated_turn_user", default=None
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

# Hermes 0.18.2 does not pass a semantic progress flag to platform adapters.
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
    """

    def __init__(self, root: Path | None = None) -> None:
        self.root = root or _requester_context_root()
        self._lock = threading.Lock()
        self._leases: dict[str, dict[str, tuple[int, int]]] = {}
        self._clear_on_start()

    def before_tool_call(self, **kwargs: Any) -> None:
        if str(kwargs.get("tool_name") or "") != "terminal":
            return
        session_key, user_id = _active_finite_session()
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
            self._write(
                session_key=session_key,
                user_id=user_id,
                expires_at_unix=now + REQUESTER_CONTEXT_TTL_SECS,
            )

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
                self._remove(session_key)
                return
            count, expires_at = session_leases.get(lease_id, (0, 0))
            if count <= 1:
                session_leases.pop(lease_id, None)
            else:
                session_leases[lease_id] = (count - 1, expires_at)
            if not session_leases:
                self._leases.pop(session_key, None)
                self._remove(session_key)

    def _clear_on_start(self) -> None:
        try:
            self.root.mkdir(mode=0o700, parents=True, exist_ok=True)
            self.root.chmod(0o700)
            for path in self.root.iterdir():
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
            for path in self.root.glob("*.json"):
                try:
                    payload = json.loads(path.read_text(encoding="utf-8"))
                    expires_at = int(payload.get("expires_at_unix") or 0)
                except (OSError, ValueError, TypeError, json.JSONDecodeError):
                    expires_at = 0
                if expires_at <= now:
                    path.unlink(missing_ok=True)
        except OSError:
            pass

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
        except OSError as exc:
            logger.warning("[finitechat] could not write requester context lease: %s", exc)

    def _remove(self, session_key: str) -> None:
        with contextlib.suppress(OSError):
            (self.root / _requester_context_filename(session_key)).unlink(missing_ok=True)


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
        # Hermes 0.18.2 maps plugin platforms that are not enum members to
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
        self._delivered_event_keys: set[str] = set()
        self._delivered_event_order: list[str] = []
        self._typing_paused: set[str] = set()
        self._active_activity_routes: set[tuple[str, str | None, str | None]] = set()
        self._outbound_message_conversations: dict[str, str | None] = {}
        self._outbound_message_segments: dict[str, str | None] = {}
        self._outbound_message_kinds: dict[str, str] = {}
        self._outbound_message_order: list[str] = []
        self._inbound_chat_routes: dict[tuple[str, str], tuple[str | None, str | None]] = {}
        # The Rust inbox is the durable queue. Keep at most its first blocked
        # ordinary text event per Hermes session in memory while the current
        # owner task finishes. Later events remain only in the inbox and are
        # redelivered after this head event is ACKed.
        self._deferred_admissions: dict[str, tuple[MessageEvent, str, Any, str, str]] = {}
        self._admission_tasks: dict[str, asyncio.Task] = {}

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
        try:
            await super()._process_message_background(event, session_key)
        finally:
            _AUTHENTICATED_FINITE_TURN_USER.reset(token)

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
        """Surface a claimed quota notice after Hermes' final response delivery.

        Hermes 0.18.2 invokes this hook once after the final response (including
        streamed delivery), and not after each model/tool subcall. Core claims a
        threshold before returning it, so delivery is intentionally at-most-once:
        a process crash between the Core response and Finite Chat send can omit a
        transient notice, while the dashboard continues to show authoritative state.
        """
        outcome_name = str(getattr(outcome, "value", getattr(outcome, "name", outcome))).lower()
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
        result = await self._finitechat_json("send", payload, timeout=30)
        if not result.ok:
            return SendResult(success=False, error=result.error, retryable=result.retryable)
        message_id = str(result.data.get("message_id") or result.data.get("id") or "") or None
        if message_id:
            self._remember_outbound_message_route(
                message_id,
                payload["conversation_id"],
                payload.get("segment_id"),
                str(payload["kind"]),
            )
        return SendResult(
            success=True,
            message_id=message_id,
            raw_response=result.data,
        )

    async def edit_message(
        self,
        chat_id: str,
        message_id: str,
        content: str,
        *,
        finalize: bool = False,
    ) -> SendResult:
        conversation_id = self._outbound_message_conversations.get(str(message_id))
        segment_id = self._outbound_message_segments.get(str(message_id))
        kind = self._outbound_message_kinds.get(str(message_id), "message")
        payload = {
            "room_id": self._room_id(chat_id),
            "conversation_id": conversation_id,
            "segment_id": segment_id,
            "message_id": str(message_id),
            "text": str(content),
            "kind": kind,
            "status": "complete" if finalize else "running",
            "finalize": bool(finalize),
            "metadata": {},
        }
        result = await self._finitechat_json("edit", payload, timeout=30)
        if not result.ok:
            return SendResult(success=False, error=result.error, retryable=result.retryable)
        edited_message_id = str(result.data.get("message_id") or message_id)
        self._remember_outbound_message_route(str(message_id), conversation_id, segment_id, kind)
        if edited_message_id:
            self._remember_outbound_message_route(
                edited_message_id, conversation_id, segment_id, kind
            )
        return SendResult(
            success=True,
            message_id=edited_message_id,
            raw_response=result.data,
        )

    async def send_typing(self, chat_id: str, metadata=None) -> None:
        payload = self._activity_payload(chat_id, metadata, action="set")
        route = self._activity_route(payload)
        if await self._run_activity_control(
            "set",
            self._finitechat_json("activity", payload, timeout=15),
        ):
            self._active_activity_routes.add(route)

    async def stop_typing(self, chat_id: str, metadata=None) -> None:
        if metadata is None:
            # Hermes performs a room-only cleanup after cancelling the typing
            # task. Finite activity is scoped to an exact topic/chat route, and
            # guessing here can clear a different concurrent turn. The exact
            # _keep_typing task owns its matching clear in finally instead.
            return
        payload = self._activity_payload(chat_id, metadata, action="clear")
        route = self._activity_route(payload)
        if await self._run_activity_control(
            "clear",
            self._finitechat_json("activity", payload, timeout=15),
        ):
            self._active_activity_routes.discard(route)

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
                if chat_id not in self._typing_paused:
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
            self._typing_paused.discard(chat_id)

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
        if event_key and event_key in self._delivered_event_keys:
            await self._ack_finitechat_event(room_id, seq, message_id)
            return

        raw_source = raw_event.get("source")
        source_data: dict[str, Any] = raw_source if isinstance(raw_source, dict) else {}
        authenticated_user_id = _string_or_none(source_data.get("user_id"))
        conversation_id = _string_or_none(raw_event.get("conversation_id"))
        segment_id = _string_or_none(raw_event.get("segment_id"))
        source_thread_id = (
            segment_id or _string_or_none(source_data.get("thread_id")) or conversation_id
        )
        self._remember_inbound_chat_route(
            room_id,
            source_thread_id,
            conversation_id,
            segment_id,
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
        activity_metadata = self._route_metadata(conversation_id, segment_id)
        activity_set = await self._set_processing_activity(room_id, activity_metadata)
        try:
            await self.handle_message(event)
            if event_key:
                self._remember_delivered_event(event_key)
            await self._ack_finitechat_event(room_id, seq, message_id)
        except Exception:
            if activity_set:
                await self._clear_processing_activity(room_id, activity_metadata)
            raise

    def _should_defer_admission(self, event: MessageEvent, session_key: str) -> bool:
        if event.message_type != MessageType.TEXT or event.internal:
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
        if session_key in self._deferred_admissions:
            return
        self._deferred_admissions[session_key] = (
            event,
            room_id,
            seq,
            message_id,
            event_key,
        )
        task = asyncio.create_task(self._admit_when_session_idle(session_key))
        self._admission_tasks[session_key] = task

    async def _admit_when_session_idle(self, session_key: str) -> None:
        try:
            while self._session_is_active(session_key):
                admission = self._deferred_admissions.get(session_key)
                if admission is None:
                    return
                owner = self._session_tasks.get(session_key)
                if owner is not None and not owner.done():
                    await asyncio.wait({owner}, timeout=ADMISSION_RECHECK_SECS)
                else:
                    await asyncio.sleep(ADMISSION_RECHECK_SECS)

            admission = self._deferred_admissions.get(session_key)
            if admission is None:
                return
            event, room_id, seq, message_id, event_key = admission
            await self._admit_finitechat_event(
                event,
                room_id,
                seq,
                message_id,
                event_key,
            )
        except asyncio.CancelledError:
            raise
        except Exception as exc:
            # The inbox still owns the unacknowledged event. Dropping only the
            # ephemeral gate lets the next stream delivery retry it.
            logger.error(
                "[finitechat] deferred admission failed for session %s: %s",
                session_key,
                exc,
                exc_info=True,
            )
        finally:
            current = asyncio.current_task()
            if self._admission_tasks.get(session_key) is current:
                self._admission_tasks.pop(session_key, None)
                self._deferred_admissions.pop(session_key, None)

    async def _cancel_admission_tasks(self) -> None:
        tasks = list(self._admission_tasks.values())
        for task in tasks:
            task.cancel()
        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)
        self._admission_tasks.clear()
        self._deferred_admissions.clear()

    async def _set_processing_activity(
        self,
        room_id: str,
        metadata: dict[str, Any] | None,
    ) -> bool:
        payload = self._activity_payload(room_id, metadata, action="set")
        payload["expires_in_millis"] = PROCESSING_ACTIVITY_TTL_MILLIS
        activity_set = await self._run_activity_control(
            "set",
            self._finitechat_json("activity", payload, timeout=15),
        )
        if activity_set:
            self._active_activity_routes.add(self._activity_route(payload))
        return activity_set

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

    def _remember_delivered_event(self, event_key: str) -> None:
        if event_key in self._delivered_event_keys:
            return
        self._delivered_event_keys.add(event_key)
        self._delivered_event_order.append(event_key)
        while len(self._delivered_event_order) > MAX_DELIVERED_EVENT_KEYS:
            evicted = self._delivered_event_order.pop(0)
            self._delivered_event_keys.discard(evicted)

    def _remember_inbound_chat_route(
        self,
        room_id: str,
        thread_id: str | None,
        conversation_id: str | None,
        segment_id: str | None,
    ) -> None:
        if not thread_id or (conversation_id is None and segment_id is None):
            return
        self._inbound_chat_routes[(room_id, thread_id)] = (conversation_id, segment_id)

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

    def _remember_outbound_message_route(
        self,
        message_id: str,
        conversation_id: str | None,
        segment_id: str | None,
        kind: str | None = None,
    ) -> None:
        if message_id in self._outbound_message_conversations:
            self._outbound_message_conversations[message_id] = conversation_id
            self._outbound_message_segments[message_id] = segment_id
            if kind:
                self._outbound_message_kinds[message_id] = kind
            return
        self._outbound_message_conversations[message_id] = conversation_id
        self._outbound_message_segments[message_id] = segment_id
        if kind:
            self._outbound_message_kinds[message_id] = kind
        self._outbound_message_order.append(message_id)
        while len(self._outbound_message_order) > MAX_OUTBOUND_MESSAGE_ROUTES:
            evicted = self._outbound_message_order.pop(0)
            self._outbound_message_conversations.pop(evicted, None)
            self._outbound_message_segments.pop(evicted, None)
            self._outbound_message_kinds.pop(evicted, None)

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

    def _send_payload(
        self,
        chat_id: str,
        content: str,
        reply_to: str | None,
        metadata: dict[str, Any] | None,
    ) -> dict[str, Any]:
        meta = self._message_metadata(metadata)
        room_id = self._room_id(chat_id)
        conversation_id, segment_id = self._route_from_metadata(room_id, meta)
        attachments = meta.pop("attachments", [])
        explicit_kind = meta.pop("_finitechat_kind", None)
        explicit_status = meta.pop("_finitechat_status", None)
        kind = str(explicit_kind or ("media" if attachments else _infer_finitechat_kind(content)))
        status = str(explicit_status or _infer_finitechat_status(content))
        return {
            "room_id": room_id,
            "conversation_id": conversation_id,
            "segment_id": segment_id,
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
        metadata_conversation_id, metadata_segment_id = self._route_from_metadata(room_id, meta)
        return {
            "room_id": room_id,
            "conversation_id": conversation_id
            if conversation_id is not None
            else metadata_conversation_id,
            "segment_id": segment_id if segment_id is not None else metadata_segment_id,
            "activity_kind": "working",
            "activity_id": None,
            "action": action,
            "payload": None,
            "expires_in_millis": 60 * 1000,
        }

    @staticmethod
    def _activity_route(payload: dict[str, Any]) -> tuple[str, str | None, str | None]:
        return (
            str(payload["room_id"]),
            _string_or_none(payload.get("conversation_id")),
            _string_or_none(payload.get("segment_id")),
        )

    def _has_active_turn(self) -> bool:
        active_sessions = getattr(self, "_active_sessions", None)
        return bool(active_sessions)

    def _room_id(self, chat_id: str | None) -> str:
        return str(chat_id or self.room_id).strip() or self.room_id

    def _route_from_metadata(
        self,
        room_id: str,
        metadata: dict[str, Any] | None,
    ) -> tuple[str | None, str | None]:
        if not isinstance(metadata, dict):
            return None, None
        thread_id = _string_or_none(metadata.pop("thread_id", None))
        conversation_id = _string_or_none(metadata.pop("conversation_id", None))
        segment_id = _string_or_none(metadata.pop("segment_id", None)) or _string_or_none(
            metadata.pop("chat_id", None)
        )
        route_key = segment_id or thread_id
        remembered_route = (
            self._inbound_chat_routes.get((room_id, route_key)) if route_key else None
        )
        if remembered_route is not None:
            remembered_conversation_id, remembered_segment_id = remembered_route
            if conversation_id is None:
                conversation_id = remembered_conversation_id
            if segment_id is None:
                segment_id = remembered_segment_id
        elif conversation_id is not None:
            # An explicit Finite conversation makes a generic Hermes thread a
            # valid segment hint. A thread on its own is not a Finite topic id.
            if segment_id is None and thread_id is not None:
                segment_id = thread_id
        else:
            # Unknown Hermes thread/chat identifiers stay unscoped so Core can
            # apply the canonical Home/Home-chat fallback. Promoting them into
            # conversation ids manufactures phantom top-level topics.
            segment_id = None
        return conversation_id, segment_id

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
            command += ["--home", self.home]
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
            return _FiniteChatResult(
                False,
                {},
                stderr_text or stdout_text or f"finitechat exited {proc.returncode}",
                _is_retryable_cli_error(stderr_text or stdout_text),
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
            "--home",
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
    ):
        self.ok = ok
        self.data = data
        self.error = error
        self.retryable = retryable
        self.transport_error = transport_error


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
        body = exc.read().decode("utf-8", errors="replace").strip()
        return _FiniteChatResult(
            False,
            {},
            _service_error_body(body) or f"finitechat service returned HTTP {exc.code}",
            _is_retryable_cli_error(body),
            False,
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
        body = exc.read().decode("utf-8", errors="replace").strip()
        _put_stream_result(
            loop,
            queue,
            _FiniteChatResult(
                False,
                {},
                _service_error_body(body) or f"finitechat service returned HTTP {exc.code}",
                _is_retryable_cli_error(body),
                False,
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


def _service_error_body(body: str) -> str | None:
    try:
        data = json.loads(body)
    except json.JSONDecodeError:
        return body or None
    if isinstance(data, dict):
        error = data.get("error")
        if error:
            return str(error)
    return body or None


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


def _is_retryable_cli_error(message: str) -> bool:
    lowered = message.lower()
    return any(token in lowered for token in ("timed out", "connection", "temporarily", "busy"))


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


def _finite_platform() -> Platform:
    try:
        return Platform(FINITE_PLATFORM_NAME)
    except ValueError:
        return Platform.LOCAL


def register(ctx) -> None:
    register_hook = getattr(ctx, "register_hook", None)
    if callable(register_hook):
        requester_context = _RequesterContextBroker()
        register_hook("pre_tool_call", requester_context.before_tool_call)
        register_hook("post_tool_call", requester_context.after_tool_call)
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

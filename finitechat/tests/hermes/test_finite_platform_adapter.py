import asyncio
import importlib.util
import json
import os
import sys
import tempfile
import types
import unittest
from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path
from typing import Any, cast
from unittest.mock import patch

REPO_ROOT = Path(__file__).resolve().parents[2]
ADAPTER_PATH = REPO_ROOT / "integrations" / "hermes" / "finitechat" / "adapter.py"
GATEWAY_MODULE_NAMES = (
    "gateway",
    "gateway.config",
    "gateway.platforms",
    "gateway.platforms.base",
    "gateway.session_context",
)


class Platform(Enum):
    FINITECHAT = "finitechat"
    LOCAL = "local"


@dataclass
class PlatformConfig:
    enabled: bool = True
    extra: dict[str, Any] = field(default_factory=dict)


class MessageType(Enum):
    TEXT = "text"
    LOCATION = "location"
    PHOTO = "photo"
    VIDEO = "video"
    AUDIO = "audio"
    VOICE = "voice"
    DOCUMENT = "document"
    STICKER = "sticker"
    COMMAND = "command"


@dataclass
class MessageEvent:
    text: str
    message_type: MessageType = MessageType.TEXT
    source: Any = None
    raw_message: Any = None
    message_id: str | None = None
    platform_update_id: int | None = None
    media_urls: list[str] = field(default_factory=list)
    media_types: list[str] = field(default_factory=list)
    reply_to_message_id: str | None = None
    reply_to_text: str | None = None
    auto_skill: Any = None
    channel_prompt: str | None = None
    internal: bool = False


@dataclass
class SendResult:
    success: bool
    message_id: str | None = None
    error: str | None = None
    raw_response: Any = None
    retryable: bool = False


class BasePlatformAdapter:
    def __init__(self, config: PlatformConfig, platform: Platform):
        self.config = config
        self.platform = platform
        self._connected = False
        self._active_sessions: dict[str, asyncio.Event] = {}
        self._session_tasks: dict[str, asyncio.Task] = {}
        self.handled_messages: list[MessageEvent] = []

    @property
    def is_connected(self):
        return self._connected

    def _mark_connected(self):
        self._connected = True

    def _mark_disconnected(self):
        self._connected = False

    async def cancel_background_tasks(self):
        return None

    def build_source(self, **kwargs):
        kwargs.setdefault("platform", self.platform)
        return types.SimpleNamespace(**kwargs)

    async def handle_message(self, event: MessageEvent) -> None:
        self.handled_messages.append(event)

    async def _process_message_background(self, event: MessageEvent, session_key: str) -> None:
        self.handled_messages.append(event)

    def _heal_stale_session_lock(self, session_key: str) -> bool:
        task = self._session_tasks.get(session_key)
        if task is None or not task.done():
            return False
        self._active_sessions.pop(session_key, None)
        self._session_tasks.pop(session_key, None)
        return True


def build_session_key(
    source,
    group_sessions_per_user: bool = True,
    thread_sessions_per_user: bool = False,
):
    del group_sessions_per_user, thread_sessions_per_user
    thread = f":{source.thread_id}" if source.thread_id else ""
    return f"agent:main:{source.platform.value}:{source.chat_type}:{source.chat_id}{thread}"


def install_gateway_stubs() -> None:
    gateway = types.ModuleType("gateway")
    config = types.ModuleType("gateway.config")
    platforms = types.ModuleType("gateway.platforms")
    base = types.ModuleType("gateway.platforms.base")
    session_context = types.ModuleType("gateway.session_context")

    config_module = cast(Any, config)
    base_module = cast(Any, base)
    config_module.Platform = Platform
    config_module.PlatformConfig = PlatformConfig
    base_module.BasePlatformAdapter = BasePlatformAdapter
    base_module.MessageEvent = MessageEvent
    base_module.MessageType = MessageType
    base_module.SendResult = SendResult
    base_module.build_session_key = build_session_key
    session_context_module = cast(Any, session_context)
    session_context_module.values = {}
    session_context_module.get_session_env = lambda name, default="": (
        session_context_module.values.get(name, default)
    )

    sys.modules["gateway"] = gateway
    sys.modules["gateway.config"] = config
    sys.modules["gateway.platforms"] = platforms
    sys.modules["gateway.platforms.base"] = base
    sys.modules["gateway.session_context"] = session_context


def load_adapter_module():
    install_gateway_stubs()
    module_name = "finite_platform_adapter_under_test"
    sys.modules.pop(module_name, None)
    spec = importlib.util.spec_from_file_location(module_name, ADAPTER_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load adapter from {ADAPTER_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


class MockPluginContext:
    def __init__(self):
        self.registered: list[dict[str, Any]] = []
        self.registered_hooks: dict[str, Any] = {}
        self.registered_tools: list[dict[str, Any]] = []
        self.registered_commands: list[dict[str, Any]] = []

    def register_platform(self, **kwargs):
        self.registered.append(kwargs)

    def register_hook(self, name, callback):
        self.registered_hooks[name] = callback

    def register_tool(self, **kwargs):
        self.registered_tools.append(kwargs)

    def register_command(self, name, **kwargs):
        self.registered_commands.append({"name": name, **kwargs})


class FinitePlatformAdapterTests(unittest.TestCase):
    def setUp(self):
        self.original_gateway_modules = {
            name: sys.modules.get(name) for name in GATEWAY_MODULE_NAMES
        }
        self.module = load_adapter_module()

    def tearDown(self):
        for name, module in self.original_gateway_modules.items():
            if module is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = module

    def adapter(self, room_id: str | None = "room-agent-1"):
        extra = {"home": "/tmp/finite-agent-home", "finitechat_bin": "/bin/echo"}
        if room_id:
            extra["room_id"] = room_id
        return self.module.FiniteChatAdapter(PlatformConfig(extra=extra))

    def test_register_exposes_finitechat_platform_contract(self):
        ctx = MockPluginContext()
        with (
            tempfile.TemporaryDirectory() as finite_home,
            patch.dict(os.environ, {"FINITE_HOME": finite_home}),
        ):
            self.module.register(ctx)

        self.assertEqual(len(ctx.registered), 1)
        self.assertEqual(set(ctx.registered_hooks), {"pre_tool_call", "post_tool_call"})
        entry = ctx.registered[0]
        self.assertEqual(entry["name"], "finitechat")
        self.assertEqual(entry["label"], "Finite Chat")
        self.assertEqual(entry["required_env"], ["FINITECHAT_HOME"])
        self.assertEqual(entry["allowed_users_env"], "FINITECHAT_ALLOWED_USERS")
        self.assertEqual(ctx.registered_tools, [])
        self.assertEqual(ctx.registered_commands, [])
        self.assertEqual(
            entry["max_message_length"], self.module.FiniteChatAdapter.MAX_MESSAGE_LENGTH
        )
        self.assertTrue(callable(entry["adapter_factory"]))

    def test_register_advertises_generic_file_attachments_to_hermes(self):
        ctx = MockPluginContext()
        with (
            tempfile.TemporaryDirectory() as finite_home,
            patch.dict(os.environ, {"FINITE_HOME": finite_home}),
        ):
            self.module.register(ctx)

        hint = ctx.registered[0]["platform_hint"]
        self.assertIn("You can send files natively", hint)
        self.assertIn("MEDIA:/absolute/path/to/file", hint)
        self.assertIn("downloadable attachments", hint)
        self.assertIn("Do not tell the user", hint)

    def test_terminal_hook_leases_only_the_active_finite_sender_and_cleans_up(self):
        with tempfile.TemporaryDirectory() as finite_home:
            broker = self.module._RequesterContextBroker(Path(finite_home) / "contexts")
            session_context = cast(Any, sys.modules["gateway.session_context"])
            session_context.values = {
                "HERMES_SESSION_PLATFORM": "local",
                "HERMES_SESSION_KEY": "finitechat:room-a:thread-a",
                "HERMES_SESSION_USER_ID": "a1" * 32,
            }
            token = self.module._AUTHENTICATED_FINITE_TURN_USER.set("a1" * 32)
            try:
                hook = {"tool_name": "terminal", "tool_call_id": "call-a"}
                broker.before_tool_call(**hook)
                context_path = broker.root / self.module._requester_context_filename(
                    "finitechat:room-a:thread-a"
                )
                payload = json.loads(context_path.read_text(encoding="utf-8"))
                self.assertEqual(payload["requesting_user_id"], "a1" * 32)
                self.assertEqual(payload["platform"], "finitechat")

                broker.after_tool_call(**hook)
                self.assertFalse(context_path.exists())
            finally:
                self.module._AUTHENTICATED_FINITE_TURN_USER.reset(token)

            session_context.values["HERMES_SESSION_USER_ID"] = "b2" * 32
            token = self.module._AUTHENTICATED_FINITE_TURN_USER.set("b2" * 32)
            try:
                broker.before_tool_call(**hook)
                payload = json.loads(context_path.read_text(encoding="utf-8"))
                self.assertEqual(payload["requesting_user_id"], "b2" * 32)
            finally:
                broker.after_tool_call(**hook)
                self.module._AUTHENTICATED_FINITE_TURN_USER.reset(token)

    def test_non_finite_invalid_and_non_terminal_hooks_never_lease(self):
        with tempfile.TemporaryDirectory() as finite_home:
            broker = self.module._RequesterContextBroker(Path(finite_home) / "contexts")
            session_context = cast(Any, sys.modules["gateway.session_context"])
            valid = {
                "HERMES_SESSION_PLATFORM": "local",
                "HERMES_SESSION_KEY": "finitechat:room-a:thread-a",
                "HERMES_SESSION_USER_ID": "a1" * 32,
            }
            session_context.values = valid
            broker.before_tool_call(tool_name="terminal", tool_call_id="call-a")
            token = self.module._AUTHENTICATED_FINITE_TURN_USER.set("b2" * 32)
            try:
                broker.before_tool_call(tool_name="terminal", tool_call_id="call-a")
                session_context.values = {**valid, "HERMES_SESSION_PLATFORM": "telegram"}
                broker.before_tool_call(tool_name="terminal", tool_call_id="call-a")
                session_context.values = valid
                broker.before_tool_call(tool_name="read_file", tool_call_id="call-a")
            finally:
                self.module._AUTHENTICATED_FINITE_TURN_USER.reset(token)
            self.assertEqual(list(broker.root.glob("*.json")), [])

    def test_parallel_terminal_leases_are_reference_counted_and_restart_clears_them(self):
        with tempfile.TemporaryDirectory() as finite_home:
            root = Path(finite_home) / "contexts"
            broker = self.module._RequesterContextBroker(root)
            session_context = cast(Any, sys.modules["gateway.session_context"])
            session_context.values = {
                "HERMES_SESSION_PLATFORM": "local",
                "HERMES_SESSION_KEY": "finitechat:room-a:thread-a",
                "HERMES_SESSION_USER_ID": "a1" * 32,
            }
            token = self.module._AUTHENTICATED_FINITE_TURN_USER.set("a1" * 32)
            try:
                first = {"tool_name": "terminal", "tool_call_id": "call-a"}
                second = {"tool_name": "terminal", "tool_call_id": "call-b"}
                broker.before_tool_call(**first)
                broker.before_tool_call(**second)
                context_path = root / self.module._requester_context_filename(
                    "finitechat:room-a:thread-a"
                )
                broker.after_tool_call(**first)
                self.assertTrue(context_path.exists())
                broker.after_tool_call(**second)
                self.assertFalse(context_path.exists())

                broker.before_tool_call(**first)
                self.assertTrue(context_path.exists())
                self.module._RequesterContextBroker(root)
                self.assertFalse(context_path.exists())
            finally:
                broker.after_tool_call(**first)
                self.module._AUTHENTICATED_FINITE_TURN_USER.reset(token)

    def test_duplicate_hook_id_is_reference_counted(self):
        with tempfile.TemporaryDirectory() as finite_home:
            root = Path(finite_home) / "contexts"
            broker = self.module._RequesterContextBroker(root)
            session_context = cast(Any, sys.modules["gateway.session_context"])
            session_context.values = {
                "HERMES_SESSION_PLATFORM": "local",
                "HERMES_SESSION_KEY": "finitechat:room-a:thread-a",
                "HERMES_SESSION_USER_ID": "a1" * 32,
            }
            token = self.module._AUTHENTICATED_FINITE_TURN_USER.set("a1" * 32)
            try:
                hook = {"tool_name": "terminal", "tool_call_id": "duplicate"}
                context_path = root / self.module._requester_context_filename(
                    "finitechat:room-a:thread-a"
                )
                broker.before_tool_call(**hook)
                broker.before_tool_call(**hook)
                broker.after_tool_call(**hook)
                self.assertTrue(context_path.exists())
                broker.after_tool_call(**hook)
                self.assertFalse(context_path.exists())
            finally:
                self.module._AUTHENTICATED_FINITE_TURN_USER.reset(token)

    def test_expired_orphan_lease_cannot_keep_a_later_terminal_context_alive(self):
        with tempfile.TemporaryDirectory() as finite_home:
            root = Path(finite_home) / "contexts"
            broker = self.module._RequesterContextBroker(root)
            session_context = cast(Any, sys.modules["gateway.session_context"])
            session_context.values = {
                "HERMES_SESSION_PLATFORM": "local",
                "HERMES_SESSION_KEY": "finitechat:room-a:thread-a",
                "HERMES_SESSION_USER_ID": "a1" * 32,
            }
            token = self.module._AUTHENTICATED_FINITE_TURN_USER.set("a1" * 32)
            original_time = self.module.time.time
            try:
                self.module.time.time = lambda: 100
                broker.before_tool_call(tool_name="terminal", tool_call_id="orphan")
                self.module.time.time = lambda: 101 + self.module.REQUESTER_CONTEXT_TTL_SECS
                current = {"tool_name": "terminal", "tool_call_id": "current"}
                broker.before_tool_call(**current)
                broker.after_tool_call(**current)
            finally:
                self.module.time.time = original_time
                self.module._AUTHENTICATED_FINITE_TURN_USER.reset(token)
            context_path = root / self.module._requester_context_filename(
                "finitechat:room-a:thread-a"
            )
            self.assertFalse(context_path.exists())

    def test_background_turn_wrapper_marks_only_authenticated_finite_events(self):
        module = cast(Any, self.module)
        adapter = self.adapter()
        observed: list[str | None] = []
        original = BasePlatformAdapter._process_message_background

        async def observe(self, event, session_key):
            _ = (self, event, session_key)
            observed.append(module._AUTHENTICATED_FINITE_TURN_USER.get())

        BasePlatformAdapter._process_message_background = observe
        authenticated = MessageEvent(
            text="publish",
            source=types.SimpleNamespace(user_id="a1" * 32),
            raw_message={"source": {"user_id": "a1" * 32}},
        )
        internal = MessageEvent(
            text="background completion",
            source=types.SimpleNamespace(user_id="a1" * 32),
            raw_message={"source": {"user_id": "a1" * 32}},
            internal=True,
        )
        try:
            asyncio.run(adapter._process_message_background(authenticated, "session"))
            asyncio.run(adapter._process_message_background(internal, "session"))
        finally:
            BasePlatformAdapter._process_message_background = original
        self.assertEqual(observed, ["a1" * 32, None])
        self.assertIsNone(module._AUTHENTICATED_FINITE_TURN_USER.get())

    def test_adapter_disables_edit_streaming_for_ios_rendering_compatibility(self):
        self.assertFalse(self.module.FiniteChatAdapter.SUPPORTS_MESSAGE_EDITING)

    def test_post_turn_notice_uses_the_exact_inbound_conversation_and_chat(self):
        module = cast(Any, self.module)
        adapter = self.adapter()
        calls = []
        adapter._finitechat_json = self._record_json(calls)
        original = module._finite_private_control_request
        module._finite_private_control_request = lambda path, method: {
            "notice": {
                "thresholdRemainingPercent": 25,
                "message": (
                    "You have 25% remaining. Your usage resets at 2026-07-21T18:00:00Z (in 5h)."
                ),
            }
        }
        event = MessageEvent(
            text="hello",
            source=types.SimpleNamespace(chat_id="room-agent-1"),
            raw_message={
                "room_id": "room-agent-1",
                "conversation_id": "topic-build",
                "segment_id": "chat-build-1",
            },
        )
        try:
            asyncio.run(
                adapter.on_processing_complete(
                    event,
                    types.SimpleNamespace(value="success"),
                )
            )
        finally:
            module._finite_private_control_request = original
        self.assertEqual([call[0] for call in calls], ["send"])
        payload = calls[0][1]
        self.assertEqual(payload["conversation_id"], "topic-build")
        self.assertEqual(payload["segment_id"], "chat-build-1")
        self.assertIn("2026-07-21T18:00:00Z", payload["text"])

    def test_failed_or_cancelled_turn_does_not_claim_a_usage_notice(self):
        module = cast(Any, self.module)
        adapter = self.adapter()
        called = []
        original = module._finite_private_control_request
        module._finite_private_control_request = lambda path, method: called.append((path, method))
        event = MessageEvent(text="hello", source=types.SimpleNamespace(chat_id="room-agent-1"))
        try:
            for outcome in ("failure", "cancelled"):
                asyncio.run(
                    adapter.on_processing_complete(
                        event,
                        types.SimpleNamespace(value=outcome),
                    )
                )
        finally:
            module._finite_private_control_request = original
        self.assertEqual(called, [])

    def test_check_requirements_uses_finitechat_bin_not_finitecomputer(self):
        old_value = os.environ.get("FINITECHAT_BIN")
        os.environ["FINITECHAT_BIN"] = "/bin/echo"
        try:
            self.assertTrue(self.module.check_requirements())
        finally:
            if old_value is None:
                os.environ.pop("FINITECHAT_BIN", None)
            else:
                os.environ["FINITECHAT_BIN"] = old_value

    def test_connect_accepts_hermes_018_reconnect_keyword(self):
        adapter = self.adapter()

        async def noop():
            return None

        async def idle_loop():
            return None

        adapter._ensure_service = noop
        adapter._recover_interrupted_turns = noop
        adapter._poll_loop = idle_loop

        self.assertTrue(asyncio.run(adapter.connect(is_reconnect=True)))

    def test_stream_env_uses_strict_loop_even_before_service_is_ready(self):
        old_stream = os.environ.get("FINITECHAT_HERMES_INBOUND_STREAM")
        os.environ["FINITECHAT_HERMES_INBOUND_STREAM"] = "1"
        try:
            adapter = self.module.FiniteChatAdapter(
                PlatformConfig(
                    extra={
                        "home": "/tmp/finite-agent-home",
                        "finitechat_bin": "/bin/false",
                    }
                )
            )
        finally:
            if old_stream is None:
                os.environ.pop("FINITECHAT_HERMES_INBOUND_STREAM", None)
            else:
                os.environ["FINITECHAT_HERMES_INBOUND_STREAM"] = old_stream
        calls = []

        async def service_not_ready():
            calls.append("ensure")
            return False

        async def noop():
            return None

        async def stream_loop():
            calls.append("stream")
            adapter._mark_disconnected()

        async def poll_loop():
            calls.append("poll")
            adapter._mark_disconnected()

        adapter._ensure_service = service_not_ready
        adapter._recover_interrupted_turns = noop
        adapter._stream_loop = stream_loop
        adapter._poll_loop = poll_loop

        async def run_connect():
            connected = await adapter.connect()
            await adapter._poll_task
            return connected

        self.assertTrue(asyncio.run(run_connect()))
        self.assertEqual(calls, ["ensure", "stream"])

    def test_local_env_file_supplies_defaults_without_overriding_process_env(self):
        old_home = os.environ.pop("FINITECHAT_HOME", None)
        old_finite_home = os.environ.pop("FINITE_HOME", None)
        old_bin = os.environ.get("FINITECHAT_BIN")
        os.environ["FINITECHAT_BIN"] = "/env/bin/finitechat"
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                env_path = Path(temp_dir) / self.module.LOCAL_ENV_FILE
                env_path.write_text(
                    "FINITECHAT_HOME=/agent/home\n"
                    "FINITECHAT_BIN=/installed/bin/finitechat\n"
                    "FINITE_HOME=/agent/home\n"
                    "OTHER_SECRET=ignored\n",
                    encoding="utf-8",
                )
                self.module._load_local_env_defaults(env_path)

            self.assertEqual(os.environ["FINITECHAT_HOME"], "/agent/home")
            self.assertEqual(os.environ["FINITECHAT_BIN"], "/env/bin/finitechat")
            self.assertEqual(os.environ["FINITE_HOME"], "/agent/home")
            self.assertIsNone(os.environ.get("OTHER_SECRET"))
        finally:
            if old_home is None:
                os.environ.pop("FINITECHAT_HOME", None)
            else:
                os.environ["FINITECHAT_HOME"] = old_home
            if old_finite_home is None:
                os.environ.pop("FINITE_HOME", None)
            else:
                os.environ["FINITE_HOME"] = old_finite_home
            if old_bin is None:
                os.environ.pop("FINITECHAT_BIN", None)
            else:
                os.environ["FINITECHAT_BIN"] = old_bin

    def test_send_preserves_hermes_route_metadata_and_notify_boolean(self):
        adapter = self.adapter()
        calls = []

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            return self.module._FiniteChatResult(True, {"message_id": "out-1"}, None, False)

        adapter._finitechat_json = fake_json
        result = asyncio.run(
            adapter.send(
                "room-agent-1",
                "hello",
                reply_to="msg-0",
                metadata={
                    "conversation_id": "topic-build",
                    "thread_id": "chat-build-1",
                    "priority": "low",
                    "notify": True,
                },
            )
        )

        self.assertTrue(result.success)
        self.assertEqual(result.message_id, "out-1")
        self.assertEqual(calls[0][0], "send")
        payload = calls[0][1]
        self.assertEqual(payload["room_id"], "room-agent-1")
        self.assertEqual(payload["conversation_id"], "topic-build")
        self.assertEqual(payload["segment_id"], "chat-build-1")
        self.assertEqual(payload["reply_to_message_id"], "msg-0")
        self.assertEqual(payload["kind"], "message")
        self.assertEqual(payload["status"], "complete")
        self.assertEqual(payload["metadata"], {"priority": "low", "notify": True})

    def test_edit_reuses_thread_route_from_original_send(self):
        adapter = self.adapter()
        calls = []

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            if action == "send":
                return self.module._FiniteChatResult(True, {"message_id": "out-1"}, None, False)
            return self.module._FiniteChatResult(True, {"message_id": "edit-1"}, None, False)

        adapter._finitechat_json = fake_json

        send_result = asyncio.run(
            adapter.send(
                "room-agent-1",
                "running draft",
                metadata={
                    "conversation_id": "topic-build",
                    "thread_id": "chat-build-1",
                    "_finitechat_kind": "tool",
                },
            )
        )
        edit_result = asyncio.run(
            adapter.edit_message(
                "room-agent-1",
                "out-1",
                "final answer",
                finalize=True,
            )
        )

        self.assertTrue(send_result.success)
        self.assertTrue(edit_result.success)
        self.assertEqual(edit_result.message_id, "edit-1")
        self.assertEqual([call[0] for call in calls], ["send", "edit"])
        self.assertEqual(calls[1][1]["conversation_id"], "topic-build")
        self.assertEqual(calls[1][1]["segment_id"], "chat-build-1")
        self.assertEqual(calls[1][1]["message_id"], "out-1")
        self.assertEqual(calls[1][1]["kind"], "tool")
        self.assertEqual(calls[1][1]["status"], "complete")
        self.assertTrue(calls[1][1]["finalize"])
        self.assertEqual(adapter._outbound_message_conversations["out-1"], "topic-build")
        self.assertEqual(adapter._outbound_message_conversations["edit-1"], "topic-build")
        self.assertEqual(adapter._outbound_message_segments["out-1"], "chat-build-1")
        self.assertEqual(adapter._outbound_message_segments["edit-1"], "chat-build-1")
        self.assertEqual(adapter._outbound_message_kinds["out-1"], "tool")
        self.assertEqual(adapter._outbound_message_kinds["edit-1"], "tool")

    def test_media_send_uses_typed_attachment_payload(self):
        adapter = self.adapter()
        calls = []

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            return self.module._FiniteChatResult(True, {"message_id": "media-1"}, None, False)

        adapter._finitechat_json = fake_json
        result = asyncio.run(
            adapter.send_document(
                "room-agent-1",
                "/tmp/report.pdf",
                caption="report",
                metadata={"conversation_id": "topic-docs", "thread_id": "chat-docs-1"},
            )
        )

        self.assertTrue(result.success)
        payload = calls[0][1]
        self.assertEqual(payload["conversation_id"], "topic-docs")
        self.assertEqual(payload["segment_id"], "chat-docs-1")
        self.assertEqual(payload["kind"], "media")
        self.assertEqual(payload["attachments"][0]["kind"], "file")
        self.assertEqual(payload["attachments"][0]["mime_type"], "application/pdf")

    def test_generic_file_send_uses_downloadable_attachment_payload(self):
        adapter = self.adapter()
        calls = []

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            return self.module._FiniteChatResult(True, {"message_id": "archive-1"}, None, False)

        adapter._finitechat_json = fake_json
        result = asyncio.run(adapter.send_document("room-agent-1", "/tmp/export.zip"))

        self.assertTrue(result.success)
        payload = calls[0][1]
        self.assertEqual(payload["kind"], "media")
        self.assertEqual(payload["attachments"][0]["name"], "export.zip")
        self.assertEqual(payload["attachments"][0]["kind"], "file")
        self.assertEqual(payload["attachments"][0]["mime_type"], "application/octet-stream")

    def test_poll_event_maps_room_to_chat_and_conversation_to_thread_then_acks(self):
        adapter = self.adapter()
        calls = []

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            return self.module._FiniteChatResult(True, {}, None, False)

        adapter._finitechat_json = fake_json
        raw_event = {
            "room_id": "room-agent-1",
            "seq": 12,
            "message_id": "msg-12",
            "conversation_id": "topic-build",
            "segment_id": "chat-build-1",
            "text": "please build",
            "message_type": "text",
            "source": {
                "platform": "finitechat",
                "chat_id": "room-agent-1",
                "chat_type": "dm",
                "user_id": "alice",
                "user_name": "Alice",
                "thread_id": "topic-build",
                "chat_topic": "Builds",
            },
            "attachments": [
                {
                    "kind": "image",
                    "path": "/tmp/screenshot.png",
                    "name": "screenshot.png",
                    "mime_type": "image/png",
                }
            ],
            "reply_to_message_id": "msg-11",
            "reply_to_text": "previous",
            "auto_skill": "coding",
            "channel_prompt": "project prompt",
        }

        asyncio.run(adapter._handle_finitechat_event(raw_event))

        self.assertEqual(len(adapter.handled_messages), 1)
        event = adapter.handled_messages[0]
        self.assertEqual(event.text, "please build")
        self.assertEqual(event.channel_prompt, "project prompt")
        self.assertEqual(event.message_type, MessageType.PHOTO)
        self.assertEqual(event.source.chat_id, "room-agent-1")
        self.assertEqual(event.source.thread_id, "chat-build-1")
        self.assertEqual(event.source.chat_topic, "Builds")
        self.assertEqual(event.raw_message["conversation_id"], "topic-build")
        self.assertEqual(event.raw_message["segment_id"], "chat-build-1")
        self.assertEqual(event.media_urls, ["/tmp/screenshot.png"])
        self.assertEqual(event.reply_to_message_id, "msg-11")
        self.assertEqual([call[0] for call in calls], ["activity", "ack"])
        self.assertEqual(calls[0][1]["action"], "set")
        self.assertEqual(calls[0][1]["conversation_id"], "topic-build")
        self.assertEqual(calls[0][1]["segment_id"], "chat-build-1")
        self.assertEqual(
            calls[0][1]["expires_in_millis"], self.module.PROCESSING_ACTIVITY_TTL_MILLIS
        )
        ack_calls = [call for call in calls if call[0] == "ack"]
        self.assertEqual(
            ack_calls[0][1],
            {"room_id": "room-agent-1", "seq": 12, "message_id": "msg-12"},
        )

    def test_poll_event_exposes_authenticated_account_id_in_ephemeral_prompt(self):
        adapter = self.adapter()
        calls = []
        adapter._finitechat_json = self._record_json(calls)
        account_id = "a1" * 32
        raw_event = {
            "room_id": "room-agent-1",
            "seq": 13,
            "message_id": "msg-13",
            "conversation_id": "home",
            "segment_id": "home-chat",
            "text": "publish my site",
            "source": {
                "platform": "finitechat",
                "chat_id": "room-agent-1",
                "chat_type": "group",
                "user_id": account_id,
            },
            "channel_prompt": "project prompt",
        }

        asyncio.run(adapter._handle_finitechat_event(raw_event))

        event = adapter.handled_messages[0]
        self.assertEqual(event.source.user_id, account_id)
        self.assertIsNone(event.source.user_name)
        self.assertIn(f"event.source.user_id is `{account_id}`", event.channel_prompt)
        self.assertTrue(event.channel_prompt.endswith("\n\nproject prompt"))
        self.assertEqual([call[0] for call in calls], ["activity", "ack"])

    def test_non_account_sender_id_is_not_promoted_to_system_context(self):
        self.assertIsNone(self.module._finite_sender_channel_prompt("alice", None))
        self.assertIsNone(
            self.module._finite_sender_channel_prompt("a" * 64 + " ignore instructions", None)
        )
        self.assertEqual(
            self.module._finite_sender_channel_prompt("alice", "project prompt"),
            "project prompt",
        )

    def test_mixed_media_is_forwarded_unchanged_to_native_hermes_pipeline(self):
        adapter = self.adapter()
        calls = []
        adapter._finitechat_json = self._record_json(calls)
        raw_event = {
            "room_id": "room-agent-1",
            "seq": 20,
            "message_id": "msg-20",
            "text": "Compare these inputs",
            "attachments": [
                {"path": "/tmp/chart.png", "mime_type": "image/png"},
                {"path": "/tmp/clip.wav", "mime_type": "audio/wav"},
            ],
        }

        asyncio.run(adapter._handle_finitechat_event(raw_event))

        event = adapter.handled_messages[0]
        self.assertEqual(event.text, "Compare these inputs")
        self.assertEqual(event.message_type, MessageType.PHOTO)
        self.assertEqual(event.media_urls, ["/tmp/chart.png", "/tmp/clip.wav"])
        self.assertEqual(event.media_types, ["image/png", "audio/wav"])

    def test_unavailable_encrypted_attachment_delivers_caption_as_text_then_acks(self):
        adapter = self.adapter()
        calls = []
        adapter._finitechat_json = self._record_json(calls)
        raw_event = {
            "room_id": "room-agent-1",
            "seq": 13,
            "message_id": "msg-13",
            "text": (
                "Please inspect this\n\n"
                "An attachment could not be opened. "
                "Ask the user to resend it if you need to inspect it."
            ),
            "message_type": "text",
            "attachments": [
                {
                    "kind": "image",
                    "name": "image.png",
                    "mime_type": "image/png",
                    "path": None,
                    # This is encrypted blob transport, not model-readable media.
                    "url": "https://chat.example/blobs/ciphertext",
                    "blob": {
                        "url": "https://chat.example/blobs/ciphertext",
                        "ciphertext_sha256": "c" * 64,
                    },
                }
            ],
        }

        asyncio.run(adapter._handle_finitechat_event(raw_event))

        self.assertEqual(len(adapter.handled_messages), 1)
        event = adapter.handled_messages[0]
        self.assertTrue(event.text.startswith("Please inspect this"))
        self.assertEqual(event.message_type, MessageType.TEXT)
        self.assertEqual(event.media_urls, [])
        self.assertIn("blob", event.raw_message["attachments"][0])
        self.assertEqual([call[0] for call in calls], ["activity", "ack"])
        self.assertEqual(calls[-1][1]["message_id"], "msg-13")

    def test_group_poll_event_preserves_authenticated_sender_identity(self):
        adapter = self.adapter(room_id=None)
        calls = []
        adapter._finitechat_json = self._record_json(calls)

        raw_event = {
            "room_id": "room-group-1",
            "seq": 42,
            "message_id": "msg-42",
            "conversation_id": "topic-chat",
            "segment_id": "chat-group-1",
            "text": "hello group",
            "source": {
                "platform": "finitechat",
                "chat_id": "room-group-1",
                "chat_name": "Agent Camp",
                "chat_type": "group",
                "user_id": "npub1alice",
                "user_name": "Alice",
                "thread_id": "topic-chat",
                "chat_topic": "Agent Camp",
                "user_id_alt": "alice-phone",
                "chat_id_alt": "mls-group-id",
                "is_bot": False,
            },
        }

        asyncio.run(adapter._handle_finitechat_event(raw_event))

        self.assertEqual(len(adapter.handled_messages), 1)
        event = adapter.handled_messages[0]
        self.assertEqual(event.source.chat_id, "room-group-1")
        self.assertEqual(event.source.chat_name, "Agent Camp")
        self.assertEqual(event.source.chat_type, "group")
        self.assertEqual(event.source.user_id, "npub1alice")
        self.assertEqual(event.source.user_name, "Alice")
        self.assertEqual(event.source.user_id_alt, "alice-phone")
        self.assertEqual(event.source.chat_id_alt, "mls-group-id")
        self.assertFalse(event.source.is_bot)
        self.assertEqual(event.source.thread_id, "chat-group-1")
        self.assertEqual([call[0] for call in calls], ["activity", "ack"])
        self.assertEqual(calls[0][1]["conversation_id"], "topic-chat")
        self.assertEqual(calls[0][1]["segment_id"], "chat-group-1")
        ack_calls = [call for call in calls if call[0] == "ack"]
        self.assertEqual(ack_calls[0][1]["message_id"], "msg-42")

    def test_reply_uses_inbound_chat_thread_to_restore_topic_route(self):
        adapter = self.adapter()
        calls = []

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            if action == "send":
                return self.module._FiniteChatResult(True, {"message_id": "out-1"}, None, False)
            return self.module._FiniteChatResult(True, {}, None, False)

        adapter._finitechat_json = fake_json
        asyncio.run(
            adapter._handle_finitechat_event(
                {
                    "room_id": "room-agent-1",
                    "seq": 7,
                    "message_id": "msg-7",
                    "conversation_id": "topic-build",
                    "segment_id": "chat-build-1",
                    "text": "please build",
                }
            )
        )

        result = asyncio.run(
            adapter.send(
                "room-agent-1",
                "done",
                metadata={"thread_id": "chat-build-1"},
            )
        )

        self.assertTrue(result.success)
        send_payload = next(call[1] for call in calls if call[0] == "send")
        self.assertEqual(send_payload["conversation_id"], "topic-build")
        self.assertEqual(send_payload["segment_id"], "chat-build-1")

    def test_reply_restores_legacy_conversation_only_route(self):
        adapter = self.adapter()
        calls = []

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            if action == "send":
                return self.module._FiniteChatResult(
                    True, {"message_id": "out-legacy"}, None, False
                )
            return self.module._FiniteChatResult(True, {}, None, False)

        adapter._finitechat_json = fake_json
        asyncio.run(
            adapter._handle_finitechat_event(
                {
                    "room_id": "room-agent-1",
                    "seq": 8,
                    "message_id": "msg-8",
                    "conversation_id": "topic-legacy",
                    "text": "legacy topic message",
                }
            )
        )

        result = asyncio.run(
            adapter.send(
                "room-agent-1",
                "legacy reply",
                metadata={"thread_id": "topic-legacy"},
            )
        )

        self.assertTrue(result.success)
        send_payload = next(call[1] for call in calls if call[0] == "send")
        self.assertEqual(send_payload["conversation_id"], "topic-legacy")
        self.assertIsNone(send_payload["segment_id"])

    def test_unknown_hermes_thread_or_segment_stays_unscoped(self):
        adapter = self.adapter()
        calls = []

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            return self.module._FiniteChatResult(
                True, {"message_id": f"out-{len(calls)}"}, None, False
            )

        adapter._finitechat_json = fake_json
        asyncio.run(
            adapter.send(
                "room-agent-1",
                "thread reply",
                metadata={"thread_id": "hermes-session-random"},
            )
        )
        asyncio.run(
            adapter.send(
                "room-agent-1",
                "segment reply",
                metadata={"segment_id": "hermes-chat-random"},
            )
        )
        asyncio.run(
            adapter.send_typing(
                "room-agent-1",
                metadata={"thread_id": "hermes-session-random"},
            )
        )

        payloads = [call[1] for call in calls]
        self.assertEqual([payload["conversation_id"] for payload in payloads], [None, None, None])
        self.assertEqual([payload["segment_id"] for payload in payloads], [None, None, None])

    def test_duplicate_redelivery_is_acked_without_second_dispatch(self):
        adapter = self.adapter()
        calls = []

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            return self.module._FiniteChatResult(True, {}, None, False)

        adapter._finitechat_json = fake_json
        raw_event = {
            "room_id": "room-agent-1",
            "seq": 12,
            "message_id": "msg-12",
            "text": "please build",
        }

        asyncio.run(adapter._handle_finitechat_event(raw_event))
        asyncio.run(adapter._handle_finitechat_event(raw_event))

        self.assertEqual(len(adapter.handled_messages), 1)
        self.assertEqual([call[0] for call in calls], ["activity", "ack", "ack"])

    def test_ack_failure_retries_without_dispatching_duplicate(self):
        adapter = self.adapter()
        calls = []
        ack_attempts = 0

        async def fake_json(action, payload, *, timeout):
            nonlocal ack_attempts
            calls.append((action, payload, timeout))
            if action == "ack":
                ack_attempts += 1
            if action == "ack" and ack_attempts == 1:
                return self.module._FiniteChatResult(False, {}, "ack transport busy", True)
            return self.module._FiniteChatResult(True, {"acked": True}, None, False)

        adapter._finitechat_json = fake_json
        raw_event = {
            "room_id": "room-agent-1",
            "seq": 12,
            "message_id": "msg-12",
            "text": "please build",
        }

        asyncio.run(adapter._handle_finitechat_event(raw_event))
        asyncio.run(adapter._handle_finitechat_event(raw_event))

        self.assertEqual(len(adapter.handled_messages), 1)
        self.assertEqual([call[0] for call in calls], ["activity", "ack", "ack"])

    def test_busy_text_waits_unacked_then_admits_in_inbox_order(self):
        adapter = self.adapter()
        calls = []
        adapter._finitechat_json = self._record_json(calls)
        session_key = "agent:main:finitechat:dm:room-agent-1:chat-build-1"
        first = self._text_event(21, "msg-21", "queued first")
        second = self._text_event(22, "msg-22", "queued second")

        async def exercise():
            release = asyncio.Event()

            async def active_owner():
                await release.wait()
                adapter._active_sessions.pop(session_key, None)
                adapter._session_tasks.pop(session_key, None)

            owner = asyncio.create_task(active_owner())
            adapter._active_sessions[session_key] = asyncio.Event()
            adapter._session_tasks[session_key] = owner

            await adapter._handle_finitechat_event(first)
            for _ in range(20):
                await adapter._handle_finitechat_event(first)
                await adapter._handle_finitechat_event(second)

            self.assertEqual(adapter.handled_messages, [])
            self.assertEqual(calls, [])
            self.assertEqual(len(adapter._deferred_admissions), 1)
            self.assertEqual(len(adapter._admission_tasks), 1)
            admission_task = adapter._admission_tasks[session_key]

            release.set()
            await owner
            await admission_task

            self.assertEqual([event.text for event in adapter.handled_messages], ["queued first"])
            self.assertEqual([call[0] for call in calls], ["activity", "ack"])
            self.assertEqual(calls[-1][1]["message_id"], "msg-21")

            # The second event stayed only in the durable Rust inbox. Its next
            # redelivery becomes the following turn after the head is ACKed.
            await adapter._handle_finitechat_event(second)

        asyncio.run(exercise())

        self.assertEqual(
            [event.text for event in adapter.handled_messages],
            ["queued first", "queued second"],
        )
        self.assertEqual([call[0] for call in calls], ["activity", "ack", "activity", "ack"])
        self.assertEqual(calls[-1][1]["message_id"], "msg-22")

    def test_deferred_text_survives_adapter_restart_until_admission(self):
        first_adapter = self.adapter()
        first_calls = []
        first_adapter._finitechat_json = self._record_json(first_calls)
        session_key = "agent:main:finitechat:dm:room-agent-1:chat-build-1"
        queued = self._text_event(31, "msg-31", "survive restart")

        async def defer_then_stop():
            owner = asyncio.create_task(asyncio.Event().wait())
            first_adapter._active_sessions[session_key] = asyncio.Event()
            first_adapter._session_tasks[session_key] = owner
            await first_adapter._handle_finitechat_event(queued)
            self.assertEqual(first_calls, [])
            await first_adapter._cancel_admission_tasks()
            owner.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await owner

        asyncio.run(defer_then_stop())

        restarted = self.adapter()
        restarted_calls = []
        restarted._finitechat_json = self._record_json(restarted_calls)
        asyncio.run(restarted._handle_finitechat_event(queued))

        self.assertEqual(first_adapter.handled_messages, [])
        self.assertEqual(first_calls, [])
        self.assertEqual([event.text for event in restarted.handled_messages], ["survive restart"])
        self.assertEqual([call[0] for call in restarted_calls], ["activity", "ack"])

    def test_controls_bypass_busy_text_admission_gate(self):
        adapter = self.adapter()
        calls = []
        adapter._finitechat_json = self._record_json(calls)
        session_key = "agent:main:finitechat:dm:room-agent-1:chat-build-1"
        tools_module = types.ModuleType("tools")
        tools_module.__path__ = []
        clarify_module = types.ModuleType("tools.clarify_gateway")
        approval_module = types.ModuleType("tools.approval")
        clarify_pending = True
        approval_pending = True
        cast(Any, clarify_module).get_pending_for_session = lambda _key, include_choice_prompts: (
            object() if clarify_pending and include_choice_prompts else None
        )
        cast(Any, approval_module).has_blocking_approval = lambda _key: approval_pending
        cast(Any, tools_module).clarify_gateway = clarify_module
        previous_modules = {
            name: sys.modules.get(name)
            for name in ("tools", "tools.clarify_gateway", "tools.approval")
        }
        sys.modules["tools"] = tools_module
        sys.modules["tools.clarify_gateway"] = clarify_module
        sys.modules["tools.approval"] = approval_module

        async def exercise():
            nonlocal clarify_pending, approval_pending
            release = asyncio.Event()

            async def active_owner():
                await release.wait()
                adapter._active_sessions.pop(session_key, None)
                adapter._session_tasks.pop(session_key, None)

            owner = asyncio.create_task(active_owner())
            adapter._active_sessions[session_key] = asyncio.Event()
            adapter._session_tasks[session_key] = owner

            await adapter._handle_finitechat_event(self._text_event(41, "msg-41", "2"))
            clarify_pending = False
            await adapter._handle_finitechat_event(self._text_event(42, "msg-42", "yes"))
            approval_pending = False
            await adapter._handle_finitechat_event(self._text_event(43, "msg-43", "/stop"))
            await adapter._handle_finitechat_event(self._text_event(44, "msg-44", "ordinary"))

            self.assertEqual(
                [event.text for event in adapter.handled_messages],
                ["2", "yes", "/stop"],
            )
            self.assertEqual(len(adapter._deferred_admissions), 1)
            self.assertNotIn("msg-44", [call[1].get("message_id") for call in calls])

            admission_task = adapter._admission_tasks[session_key]
            release.set()
            await owner
            await admission_task

        try:
            asyncio.run(exercise())
        finally:
            for name, previous in previous_modules.items():
                if previous is None:
                    sys.modules.pop(name, None)
                else:
                    sys.modules[name] = previous

        self.assertEqual(
            [event.text for event in adapter.handled_messages],
            ["2", "yes", "/stop", "ordinary"],
        )
        acked = [call[1]["message_id"] for call in calls if call[0] == "ack"]
        self.assertEqual(acked, ["msg-41", "msg-42", "msg-43", "msg-44"])

    def test_active_session_does_not_block_another_session(self):
        adapter = self.adapter()
        calls = []
        adapter._finitechat_json = self._record_json(calls)
        active_key = "agent:main:finitechat:dm:room-agent-1:chat-a"

        async def exercise():
            owner = asyncio.create_task(asyncio.Event().wait())
            adapter._active_sessions[active_key] = asyncio.Event()
            adapter._session_tasks[active_key] = owner
            await adapter._handle_finitechat_event(
                self._text_event(51, "msg-51", "other session", segment_id="chat-b")
            )
            owner.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await owner

        asyncio.run(exercise())

        self.assertEqual([event.text for event in adapter.handled_messages], ["other session"])
        self.assertEqual([call[0] for call in calls], ["activity", "ack"])
        self.assertEqual(adapter._deferred_admissions, {})

    def test_failed_handoff_clears_processing_activity(self):
        adapter = self.adapter()
        calls = []
        adapter._finitechat_json = self._record_json(calls)

        async def fail_handle(_event):
            raise RuntimeError("handoff failed")

        adapter.handle_message = fail_handle
        raw_event = {
            "room_id": "room-agent-1",
            "seq": 12,
            "message_id": "msg-12",
            "text": "please build",
        }

        with self.assertRaises(RuntimeError):
            asyncio.run(adapter._handle_finitechat_event(raw_event))

        self.assertEqual([call[0] for call in calls], ["activity", "activity"])
        self.assertEqual(calls[0][1]["action"], "set")
        self.assertEqual(calls[1][1]["action"], "clear")

    def test_room_filter_drops_other_rooms_but_unfiltered_serves_all(self):
        filtered = self.adapter(room_id="room-agent-1")
        filtered_calls = []
        filtered._finitechat_json = self._record_json(filtered_calls)
        asyncio.run(
            filtered._handle_finitechat_event(
                {"room_id": "other-room", "seq": 1, "message_id": "msg-1", "text": "nope"}
            )
        )
        self.assertEqual(filtered.handled_messages, [])
        self.assertEqual(filtered_calls, [])

        unfiltered = self.adapter(room_id=None)
        unfiltered_calls = []
        unfiltered._finitechat_json = self._record_json(unfiltered_calls)
        asyncio.run(
            unfiltered._handle_finitechat_event(
                {"room_id": "any-room", "seq": 2, "message_id": "msg-2", "text": "hello"}
            )
        )
        self.assertEqual(len(unfiltered.handled_messages), 1)
        self.assertEqual(unfiltered.handled_messages[0].source.chat_id, "any-room")
        ack_calls = [call for call in unfiltered_calls if call[0] == "ack"]
        self.assertEqual(len(ack_calls), 1)
        self.assertEqual(
            ack_calls[0][1],
            {"room_id": "any-room", "seq": 2, "message_id": "msg-2"},
        )

    def test_home_is_required_and_room_is_optional(self):
        self.assertTrue(
            self.module.validate_config(PlatformConfig(extra={"home": "/tmp/finite-agent-home"}))
        )
        old_home = os.environ.pop("FINITECHAT_HOME", None)
        try:
            self.assertFalse(self.module.validate_config(PlatformConfig(extra={})))
        finally:
            if old_home is not None:
                os.environ["FINITECHAT_HOME"] = old_home

    def test_adapter_does_not_recreate_deleted_invite_sessions(self):
        self.assertFalse(hasattr(self.module.FiniteChatAdapter, "_surface_invite"))

    def _record_json(self, calls):
        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            return self.module._FiniteChatResult(True, {}, None, False)

        return fake_json

    @staticmethod
    def _text_event(
        seq: int,
        message_id: str,
        text: str,
        *,
        segment_id: str = "chat-build-1",
    ):
        return {
            "room_id": "room-agent-1",
            "seq": seq,
            "message_id": message_id,
            "conversation_id": "topic-build",
            "segment_id": segment_id,
            "text": text,
            "message_type": "text",
            "source": {
                "platform": "finitechat",
                "chat_id": "room-agent-1",
                "chat_type": "dm",
                "user_id": "alice",
            },
        }

    def test_send_infers_tool_status_when_hermes_metadata_is_missing(self):
        adapter = self.adapter()
        calls = []

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            return self.module._FiniteChatResult(True, {"message_id": "tool-1"}, None, False)

        adapter._finitechat_json = fake_json
        result = asyncio.run(adapter.send("room-agent-1", "💻 Running cargo test ▉"))

        self.assertTrue(result.success)
        self.assertEqual(calls[0][0], "send")
        self.assertEqual(calls[0][1]["kind"], "tool")
        self.assertEqual(calls[0][1]["status"], "running")

    def test_tool_kind_inference_covers_pinned_hermes_018_registry_pairs(self):
        for prefix, tool_names in self.module.HERMES_018_RAW_TOOL_NAMES_BY_PREFIX.items():
            for tool_name in tool_names:
                with self.subTest(prefix=prefix, tool_name=tool_name):
                    self.assertEqual(
                        self.module._infer_finitechat_kind(f'{prefix} {tool_name}: "preview"'),
                        "tool",
                    )

        self.assertEqual(
            self.module._infer_finitechat_kind('⚙️ third_party_tool: "preview"'),
            "tool",
        )

    def test_tool_kind_inference_covers_hermes_friendly_progress_labels(self):
        for prefix, verb in self.module.HERMES_018_FRIENDLY_TOOL_PREFIXES:
            content = f"{prefix} {verb} preview"
            with self.subTest(prefix=prefix, verb=verb):
                self.assertEqual(self.module._infer_finitechat_kind(content), "tool")
        self.assertEqual(
            self.module._infer_finitechat_kind("💻 terminal\n```\nnode --check app.js\n```"),
            "tool",
        )

    def test_tool_kind_inference_does_not_treat_arbitrary_emoji_as_progress(self):
        for content in (
            "🎉 shipped",
            "😀 hello",
            "plain assistant response",
            "💻not a tool",
            "✔ Done",
            "✔ Done...",
            "❓ What do you think?",
            "💬 Here is the result",
            "💬 Reading this carefully",
            "📝 Notes",
            "📖 Writing this up",
            "video finished",
        ):
            with self.subTest(content=content):
                self.assertEqual(self.module._infer_finitechat_kind(content), "message")

    def test_send_infers_status_kind_for_working_message(self):
        adapter = self.adapter()
        calls = []

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            return self.module._FiniteChatResult(True, {"message_id": "status-1"}, None, False)

        adapter._finitechat_json = fake_json
        result = asyncio.run(adapter.send("room-agent-1", "Hermes is working"))

        self.assertTrue(result.success)
        self.assertEqual(calls[0][1]["kind"], "status")
        self.assertEqual(calls[0][1]["status"], "complete")

    def test_typing_activity_uses_ephemeral_bridge_and_clears_same_thread_route(self):
        adapter = self.adapter()
        calls = []

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            return self.module._FiniteChatResult(True, {}, None, False)

        adapter._finitechat_json = fake_json
        asyncio.run(
            adapter.send_typing(
                "room-agent-1",
                metadata={"conversation_id": "topic-build", "thread_id": "chat-build-1"},
            )
        )
        asyncio.run(
            adapter.stop_typing(
                "room-agent-1",
                metadata={"conversation_id": "topic-build", "thread_id": "chat-build-1"},
            )
        )

        self.assertEqual(calls[0][0], "activity")
        self.assertEqual(calls[0][1]["action"], "set")
        self.assertEqual(calls[0][1]["conversation_id"], "topic-build")
        self.assertEqual(calls[0][1]["segment_id"], "chat-build-1")
        self.assertEqual(calls[0][1]["expires_in_millis"], 60 * 1000)
        self.assertEqual(calls[1][0], "activity")
        self.assertEqual(calls[1][1]["action"], "clear")
        self.assertEqual(calls[1][1]["conversation_id"], "topic-build")
        self.assertEqual(calls[1][1]["segment_id"], "chat-build-1")
        self.assertEqual(adapter._active_activity_routes, set())

    def test_quiet_turn_refreshes_before_the_fifteen_second_activity_lease(self):
        self.assertLess(
            self.module.DEFAULT_ACTIVITY_REFRESH_SECS * 1000,
            self.module.PROCESSING_ACTIVITY_TTL_MILLIS,
        )
        adapter = self.adapter()
        adapter.activity_refresh_secs = 0.01
        calls = []
        stop_event = asyncio.Event()

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            if payload["action"] == "set" and len(calls) == 3:
                stop_event.set()
            return self.module._FiniteChatResult(True, {}, None, False)

        adapter._finitechat_json = fake_json
        asyncio.run(
            adapter._keep_typing(
                "room-agent-1",
                metadata={"conversation_id": "topic-a", "thread_id": "chat-a"},
                stop_event=stop_event,
            )
        )

        self.assertEqual([call[1]["action"] for call in calls], ["set", "set", "set", "clear"])

    def test_concurrent_typing_activity_clears_only_the_matching_chat_route(self):
        adapter = self.adapter()
        calls = []
        adapter._finitechat_json = self._record_json(calls)

        async def exercise():
            await adapter.send_typing(
                "room-agent-1",
                metadata={"conversation_id": "topic-a", "thread_id": "chat-a"},
            )
            await adapter.send_typing(
                "room-agent-1",
                metadata={"conversation_id": "topic-b", "thread_id": "chat-b"},
            )
            await adapter.stop_typing(
                "room-agent-1",
                metadata={"conversation_id": "topic-a", "thread_id": "chat-a"},
            )
            self.assertEqual(
                adapter._active_activity_routes,
                {("room-agent-1", "topic-b", "chat-b")},
            )
            await adapter.stop_typing("room-agent-1")
            self.assertEqual(
                adapter._active_activity_routes,
                {("room-agent-1", "topic-b", "chat-b")},
            )
            await adapter.stop_typing(
                "room-agent-1",
                metadata={"conversation_id": "topic-b", "thread_id": "chat-b"},
            )

        asyncio.run(exercise())

        self.assertEqual(
            [
                (call[1]["action"], call[1]["conversation_id"], call[1]["segment_id"])
                for call in calls
            ],
            [
                ("set", "topic-a", "chat-a"),
                ("set", "topic-b", "chat-b"),
                ("clear", "topic-a", "chat-a"),
                ("clear", "topic-b", "chat-b"),
            ],
        )
        self.assertEqual(adapter._active_activity_routes, set())

    def test_typing_activity_timeout_does_not_stall_or_remember_failed_route(self):
        adapter = self.adapter()

        async def never_returns(_action, _payload, *, timeout):
            del timeout
            await asyncio.Event().wait()

        adapter._finitechat_json = never_returns
        module = cast(Any, self.module)
        original_timeout = module.ACTIVITY_CONTROL_TIMEOUT_SECS
        module.ACTIVITY_CONTROL_TIMEOUT_SECS = 0.01
        try:
            asyncio.run(
                adapter.send_typing(
                    "room-agent-1",
                    metadata={"conversation_id": "topic-a", "thread_id": "chat-a"},
                )
            )
        finally:
            module.ACTIVITY_CONTROL_TIMEOUT_SECS = original_timeout

        self.assertEqual(adapter._active_activity_routes, set())

    def test_keep_typing_sets_immediately_and_clears_exact_route_on_stop(self):
        adapter = self.adapter()
        calls = []
        stop_event = asyncio.Event()

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            if payload["action"] == "set":
                stop_event.set()
            return self.module._FiniteChatResult(True, {}, None, False)

        adapter._finitechat_json = fake_json
        asyncio.run(
            adapter._keep_typing(
                "room-agent-1",
                metadata={"conversation_id": "topic-a", "thread_id": "chat-a"},
                stop_event=stop_event,
            )
        )

        self.assertEqual([call[1]["action"] for call in calls], ["set", "clear"])
        self.assertEqual(calls[-1][1]["conversation_id"], "topic-a")
        self.assertEqual(calls[-1][1]["segment_id"], "chat-a")
        self.assertEqual(adapter._active_activity_routes, set())

    def test_keep_typing_clears_unscoped_home_route_without_room_guessing(self):
        adapter = self.adapter()
        calls = []
        stop_event = asyncio.Event()

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            if payload["action"] == "set":
                stop_event.set()
            return self.module._FiniteChatResult(True, {}, None, False)

        adapter._finitechat_json = fake_json
        asyncio.run(
            adapter._keep_typing(
                "room-agent-1",
                stop_event=stop_event,
            )
        )

        self.assertEqual([call[1]["action"] for call in calls], ["set", "clear"])
        self.assertIsNone(calls[-1][1]["conversation_id"])
        self.assertIsNone(calls[-1][1]["segment_id"])
        self.assertEqual(adapter._active_activity_routes, set())

    def test_keep_typing_honors_pause_and_cancels_without_waiting_for_interval(self):
        adapter = self.adapter()
        adapter.activity_refresh_secs = 30
        calls = []
        first_set = asyncio.Event()

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            if payload["action"] == "set":
                first_set.set()
            return self.module._FiniteChatResult(True, {}, None, False)

        async def exercise():
            adapter._finitechat_json = fake_json
            adapter._typing_paused.add("room-agent-1")
            task = asyncio.create_task(
                adapter._keep_typing(
                    "room-agent-1",
                    metadata={"conversation_id": "topic-a", "thread_id": "chat-a"},
                )
            )
            await asyncio.sleep(0.05)
            self.assertEqual(calls, [])
            adapter._typing_paused.discard("room-agent-1")
            task.cancel()
            await asyncio.wait_for(task, timeout=0.5)

        asyncio.run(exercise())

        self.assertFalse(first_set.is_set())
        self.assertEqual([call[1]["action"] for call in calls], ["clear"])
        self.assertEqual(calls[0][1]["conversation_id"], "topic-a")
        self.assertEqual(calls[0][1]["segment_id"], "chat-a")

    def test_poll_loop_uses_short_poll_while_agent_turn_is_active(self):
        adapter = self.adapter()
        adapter._mark_connected()
        adapter._active_sessions = {"room-agent-1": asyncio.Event()}
        calls = []

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            adapter._mark_disconnected()
            return self.module._FiniteChatResult(True, {"events": []}, None, False)

        adapter._finitechat_json = fake_json
        asyncio.run(adapter._poll_loop())

        self.assertEqual(calls[0][0], "poll")
        self.assertEqual(
            calls[0][1]["timeout_millis"],
            self.module.ACTIVE_TURN_POLL_TIMEOUT_MILLIS,
        )

    def test_poll_loop_continues_after_transient_poll_error(self):
        adapter = self.adapter()
        adapter._mark_connected()
        calls = []
        sleeps = []

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            if len(calls) == 1:
                return self.module._FiniteChatResult(False, {}, "server busy", True)
            adapter._mark_disconnected()
            return self.module._FiniteChatResult(True, {"events": []}, None, False)

        async def fake_sleep(delay):
            sleeps.append(delay)

        original_sleep = self.module.asyncio.sleep
        try:
            adapter._finitechat_json = fake_json
            self.module.asyncio.sleep = fake_sleep
            asyncio.run(adapter._poll_loop())
        finally:
            self.module.asyncio.sleep = original_sleep

        self.assertEqual([call[0] for call in calls], ["poll", "poll"])
        self.assertEqual(sleeps, [2.0])

    def test_finitechat_json_serializes_cli_access_per_adapter(self):
        adapter = self.adapter()
        original_create_subprocess_exec = self.module.asyncio.create_subprocess_exec
        active = 0
        max_active = 0

        class FakeProcess:
            returncode = 0

            async def communicate(self, stdin):
                nonlocal active, max_active
                active += 1
                max_active = max(max_active, active)
                await asyncio.sleep(0.01)
                active -= 1
                return b'{"accepted":true}', b""

        async def fake_create_subprocess_exec(*args, **kwargs):
            return FakeProcess()

        async def run_calls():
            return await asyncio.gather(
                adapter._finitechat_json("poll", {}, timeout=5),
                adapter._finitechat_json("send", {"text": "hello"}, timeout=5),
            )

        try:
            self.module.asyncio.create_subprocess_exec = fake_create_subprocess_exec
            results = asyncio.run(run_calls())
        finally:
            self.module.asyncio.create_subprocess_exec = original_create_subprocess_exec

        self.assertEqual([result.ok for result in results], [True, True])
        self.assertEqual(max_active, 1)

    def test_finitechat_json_prefers_configured_service_url(self):
        adapter = self.module.FiniteChatAdapter(
            PlatformConfig(
                extra={
                    "home": "/tmp/finite-agent-home",
                    "service_url": "http://127.0.0.1:9999",
                    "finitechat_bin": "/bin/false",
                }
            )
        )
        original_urlopen = self.module.urllib.request.urlopen
        captured = {}

        class FakeResponse:
            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, traceback):
                return False

            def read(self):
                return b'{"recovered":0}'

        def fake_urlopen(request, timeout):
            captured["url"] = request.full_url
            captured["body"] = request.data
            captured["timeout"] = timeout
            return FakeResponse()

        try:
            self.module.urllib.request.urlopen = fake_urlopen
            result = asyncio.run(adapter._finitechat_json("recover", {}, timeout=7))
        finally:
            self.module.urllib.request.urlopen = original_urlopen

        self.assertTrue(result.ok)
        self.assertEqual(result.data["recovered"], 0)
        self.assertEqual(captured["url"], "http://127.0.0.1:9999/v1/hermes/recover")
        self.assertEqual(captured["body"], b"{}")
        self.assertEqual(captured["timeout"], 7)

    def test_finitechat_json_retries_transient_service_transport_reset(self):
        adapter = self.module.FiniteChatAdapter(
            PlatformConfig(
                extra={
                    "home": "/tmp/finite-agent-home",
                    "service_url": "http://127.0.0.1:9999",
                    "finitechat_bin": "/bin/false",
                }
            )
        )
        original_urlopen = self.module.urllib.request.urlopen
        original_sleep = self.module.asyncio.sleep
        calls = []
        sleeps = []

        class FakeResponse:
            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, traceback):
                return False

            def read(self):
                return b'{"accepted":true}'

        def fake_urlopen(request, timeout):
            calls.append((request.full_url, timeout))
            if len(calls) == 1:
                raise self.module.urllib.error.URLError("connection reset")
            return FakeResponse()

        async def fake_sleep(delay):
            sleeps.append(delay)

        try:
            self.module.urllib.request.urlopen = fake_urlopen
            self.module.asyncio.sleep = fake_sleep
            result = asyncio.run(
                adapter._finitechat_json(
                    "activity",
                    {"room_id": "room-agent-1", "action": "clear"},
                    timeout=7,
                )
            )
        finally:
            self.module.urllib.request.urlopen = original_urlopen
            self.module.asyncio.sleep = original_sleep

        self.assertTrue(result.ok)
        self.assertEqual(result.data["accepted"], True)
        self.assertEqual(
            calls,
            [
                ("http://127.0.0.1:9999/v1/hermes/activity", 7),
                ("http://127.0.0.1:9999/v1/hermes/activity", 7),
            ],
        )
        self.assertEqual(sleeps, [self.module.SERVICE_TRANSPORT_RETRY_SECS])

    def test_finitechat_json_falls_back_to_cli_when_service_transport_fails(self):
        adapter = self.module.FiniteChatAdapter(
            PlatformConfig(
                extra={
                    "home": "/tmp/finite-agent-home",
                    "service_url": "http://127.0.0.1:9999",
                    "finitechat_bin": "/bin/finitechat",
                }
            )
        )
        original_urlopen = self.module.urllib.request.urlopen
        original_create_subprocess_exec = self.module.asyncio.create_subprocess_exec
        calls = []

        class FakeProcess:
            returncode = 0

            async def communicate(self, stdin):
                calls.append(stdin)
                return b'{"recovered":0}', b""

        async def fake_create_subprocess_exec(*args, **kwargs):
            calls.append(args)
            return FakeProcess()

        try:
            self.module.urllib.request.urlopen = lambda request, timeout: (_ for _ in ()).throw(
                self.module.urllib.error.URLError("service down")
            )
            self.module.asyncio.create_subprocess_exec = fake_create_subprocess_exec
            result = asyncio.run(adapter._finitechat_json("recover", {}, timeout=7))
        finally:
            self.module.urllib.request.urlopen = original_urlopen
            self.module.asyncio.create_subprocess_exec = original_create_subprocess_exec

        self.assertTrue(result.ok)
        self.assertEqual(result.data["recovered"], 0)
        self.assertEqual(calls[0][0:2], ("/bin/finitechat", "hermes"))
        self.assertEqual(calls[0][-2:], ("recover", "--json"))

    def test_strict_stream_service_failure_never_falls_back_to_cli(self):
        adapter = self.module.FiniteChatAdapter(
            PlatformConfig(
                extra={
                    "home": "/tmp/finite-agent-home",
                    "service_url": "http://127.0.0.1:9999",
                    "finitechat_bin": "/bin/finitechat",
                    "inbound_stream": True,
                }
            )
        )
        module = cast(Any, self.module)
        original_service_json = module._finitechat_service_json
        original_create_subprocess_exec = self.module.asyncio.create_subprocess_exec
        service_calls = []
        subprocess_calls = []

        def fake_service_json(service_url, action, payload, timeout):
            service_calls.append((service_url, action, payload, timeout))
            return self.module._FiniteChatResult(
                False,
                {},
                "connection reset",
                True,
                transport_error=True,
            )

        async def fake_create_subprocess_exec(*args, **kwargs):
            subprocess_calls.append((args, kwargs))
            raise AssertionError("strict stream mode must not execute a per-action CLI")

        try:
            module._finitechat_service_json = fake_service_json
            self.module.asyncio.create_subprocess_exec = fake_create_subprocess_exec
            result = asyncio.run(adapter._finitechat_json("send", {"text": "hello"}, timeout=7))
        finally:
            module._finitechat_service_json = original_service_json
            self.module.asyncio.create_subprocess_exec = original_create_subprocess_exec

        self.assertFalse(result.ok)
        self.assertTrue(result.retryable)
        self.assertTrue(result.transport_error)
        self.assertEqual(len(service_calls), 2)
        self.assertEqual(subprocess_calls, [])

    def test_finitechat_service_stream_worker_parses_ndjson_records(self):
        original_urlopen = self.module.urllib.request.urlopen
        captured = {}

        class FakeResponse:
            def __init__(self):
                self.lines = [
                    b'{"type":"joined","account_id":"alice"}\n',
                    (
                        b'{"type":"event","event":{"room_id":"room-agent-1",'
                        b'"seq":12,"message_id":"msg-12","text":"hello"}}\n'
                    ),
                ]

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, traceback):
                return False

            def readline(self):
                return self.lines.pop(0) if self.lines else b""

        def fake_urlopen(request, timeout):
            captured["url"] = request.full_url
            captured["timeout"] = timeout
            return FakeResponse()

        async def run_worker():
            loop = asyncio.get_running_loop()
            queue = asyncio.Queue()
            await asyncio.to_thread(
                self.module._finitechat_service_stream_worker,
                "http://127.0.0.1:9999",
                {"room_id": "room agent", "limit": 10, "timeout_millis": 1000},
                7,
                loop,
                queue,
                self.module.threading.Event(),
            )
            await asyncio.sleep(0)
            results = []
            while not queue.empty():
                results.append(await queue.get())
            return results

        try:
            self.module.urllib.request.urlopen = fake_urlopen
            results = asyncio.run(run_worker())
        finally:
            self.module.urllib.request.urlopen = original_urlopen

        self.assertTrue(results[0].ok)
        self.assertTrue(results[1].ok)
        self.assertTrue(results[2].ok)
        self.assertEqual(
            captured["url"],
            "http://127.0.0.1:9999/v1/hermes/inbound?"
            "room_id=room+agent&limit=10&timeout_millis=1000",
        )
        self.assertEqual(captured["timeout"], 7)
        self.assertEqual(results[0].data["records"][0]["type"], "connected")
        self.assertEqual(results[1].data["records"][0]["type"], "joined")
        self.assertEqual(results[2].data["records"][0]["event"]["message_id"], "msg-12")

    def test_finitechat_service_stream_worker_surfaces_sidecar_error_record(self):
        original_urlopen = self.module.urllib.request.urlopen

        class FakeResponse:
            def __init__(self):
                self.lines = [
                    b'{"type":"error","error":"attachment download failed"}\n',
                ]

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, traceback):
                return False

            def readline(self):
                return self.lines.pop(0) if self.lines else b""

        async def run_worker():
            loop = asyncio.get_running_loop()
            queue = asyncio.Queue()
            await asyncio.to_thread(
                self.module._finitechat_service_stream_worker,
                "http://127.0.0.1:9999",
                {"limit": 10, "timeout_millis": 1000},
                7,
                loop,
                queue,
                self.module.threading.Event(),
            )
            await asyncio.sleep(0)
            results = []
            while not queue.empty():
                results.append(await queue.get())
            return results

        try:
            self.module.urllib.request.urlopen = lambda request, timeout: FakeResponse()
            results = asyncio.run(run_worker())
        finally:
            self.module.urllib.request.urlopen = original_urlopen

        self.assertTrue(results[0].ok, "opening the stream is a liveness record")
        self.assertFalse(results[1].ok)
        self.assertEqual(results[1].error, "attachment download failed")
        self.assertTrue(results[1].retryable)
        self.assertFalse(results[1].transport_error)

    def test_stream_loop_consumes_inbound_records_and_acks_events(self):
        adapter = self.module.FiniteChatAdapter(
            PlatformConfig(
                extra={
                    "home": "/tmp/finite-agent-home",
                    "room_id": "room-agent-1",
                    "service_url": "http://127.0.0.1:9999",
                    "finitechat_bin": "/bin/false",
                    "inbound_stream": True,
                }
            )
        )
        adapter._mark_connected()
        module = cast(Any, self.module)
        original_worker = module._finitechat_service_stream_worker
        stream_calls = []
        ack_calls = []

        def fake_worker(service_url, payload, timeout, loop, queue, stop_event):
            stream_calls.append((service_url, payload, timeout))
            self.module._put_stream_result(
                loop,
                queue,
                self.module._FiniteChatResult(
                    True,
                    {
                        "records": [
                            {"type": "joined", "account_id": "alice"},
                            {
                                "type": "event",
                                "event": {
                                    "room_id": "room-agent-1",
                                    "seq": 12,
                                    "message_id": "msg-12",
                                    "text": "hello from stream",
                                },
                            },
                        ]
                    },
                    None,
                    False,
                ),
            )

        async def fake_json(action, payload, *, timeout):
            ack_calls.append((action, payload, timeout))
            adapter._mark_disconnected()
            return self.module._FiniteChatResult(True, {"acked": True}, None, False)

        try:
            module._finitechat_service_stream_worker = fake_worker
            adapter._finitechat_json = fake_json
            asyncio.run(adapter._stream_loop())
        finally:
            module._finitechat_service_stream_worker = original_worker

        self.assertEqual(stream_calls[0][0], "http://127.0.0.1:9999")
        self.assertEqual(stream_calls[0][1]["room_id"], "room-agent-1")
        self.assertEqual(len(adapter.handled_messages), 1)
        self.assertEqual(adapter.handled_messages[0].text, "hello from stream")
        message_acks = [call for call in ack_calls if call[0] == "ack"]
        self.assertEqual(message_acks[0][1]["message_id"], "msg-12")

    def test_stream_loop_skips_typed_receipt_records_without_dispatch_or_ack(self):
        adapter = self.module.FiniteChatAdapter(
            PlatformConfig(
                extra={
                    "home": "/tmp/finite-agent-home",
                    "room_id": "room-agent-1",
                    "service_url": "http://127.0.0.1:9999",
                    "finitechat_bin": "/bin/false",
                    "inbound_stream": True,
                }
            )
        )
        adapter._mark_connected()
        module = cast(Any, self.module)
        original_worker = module._finitechat_service_stream_worker
        calls = []

        def fake_worker(service_url, payload, timeout, loop, queue, stop_event):
            calls.append(("stream", payload))
            self.module._put_stream_result(
                loop,
                queue,
                self.module._FiniteChatResult(
                    True,
                    {
                        "records": [
                            {
                                "type": "receipt",
                                "room_id": "room-agent-1",
                                "seq": 13,
                                "message_id": "receipt-13",
                                "read_message_id": "msg-12",
                            },
                            {
                                "type": "event",
                                "event": {
                                    "room_id": "room-agent-1",
                                    "seq": 14,
                                    "message_id": "msg-14",
                                    "text": "real message",
                                },
                            },
                        ]
                    },
                    None,
                    False,
                ),
            )

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload))
            adapter._mark_disconnected()
            return self.module._FiniteChatResult(True, {"acked": True}, None, False)

        try:
            module._finitechat_service_stream_worker = fake_worker
            adapter._finitechat_json = fake_json
            asyncio.run(adapter._stream_loop())
        finally:
            module._finitechat_service_stream_worker = original_worker

        self.assertEqual(len(adapter.handled_messages), 1)
        self.assertEqual(adapter.handled_messages[0].text, "real message")
        ack_calls = [call for call in calls if call[0] == "ack"]
        self.assertEqual(len(ack_calls), 1)
        self.assertEqual(ack_calls[0][1]["message_id"], "msg-14")

    def _assert_stream_loop_reconnects_without_poll(self, error, transport_error):
        adapter = self.module.FiniteChatAdapter(
            PlatformConfig(
                extra={
                    "home": "/tmp/finite-agent-home",
                    "room_id": "room-agent-1",
                    "service_url": "http://127.0.0.1:9999",
                    "finitechat_bin": "/bin/false",
                    "inbound_stream": True,
                }
            )
        )
        adapter._mark_connected()
        module = cast(Any, self.module)
        original_worker = module._finitechat_service_stream_worker
        original_sleep = self.module.asyncio.sleep
        calls = []

        def fake_worker(service_url, payload, timeout, loop, queue, stop_event):
            calls.append(("stream", payload))
            if len([call for call in calls if call[0] == "stream"]) == 1:
                self.module._put_stream_result(
                    loop,
                    queue,
                    self.module._FiniteChatResult(
                        False,
                        {},
                        error,
                        True,
                        transport_error=transport_error,
                    ),
                )
                return
            self.module._put_stream_result(
                loop,
                queue,
                self.module._FiniteChatResult(
                    True,
                    {
                        "records": [
                            {
                                "type": "event",
                                "event": {
                                    "room_id": "room-agent-1",
                                    "seq": 15,
                                    "message_id": "msg-15",
                                    "text": "caught up after reconnect",
                                },
                            }
                        ]
                    },
                    None,
                    False,
                ),
            )

        async def fake_ensure_service():
            calls.append(("ensure", {}))
            return False

        async def fail_poll():
            raise AssertionError("strict stream mode must not poll")

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload))
            if action == "ack":
                adapter._mark_disconnected()
            return self.module._FiniteChatResult(True, {"acked": True}, None, False)

        async def fake_sleep(delay):
            calls.append(("sleep", {"delay": delay}))

        try:
            module._finitechat_service_stream_worker = fake_worker
            self.module.asyncio.sleep = fake_sleep
            adapter._ensure_service = fake_ensure_service
            adapter._poll_once = fail_poll
            adapter._finitechat_json = fake_json
            asyncio.run(adapter._stream_loop())
        finally:
            module._finitechat_service_stream_worker = original_worker
            self.module.asyncio.sleep = original_sleep

        self.assertEqual(
            [call[0] for call in calls],
            ["stream", "sleep", "stream", "activity", "ack"],
        )
        self.assertEqual(calls[1][1]["delay"], self.module.STREAM_RECONNECT_BACKOFF_SECS)
        self.assertEqual(adapter.service_url, "http://127.0.0.1:9999")
        self.assertEqual(len(adapter.handled_messages), 1)
        self.assertEqual(adapter.handled_messages[0].text, "caught up after reconnect")

    def test_stream_loop_reconnects_and_catches_up_without_poll_fallback(self):
        self._assert_stream_loop_reconnects_without_poll("connection reset", True)

    def test_stream_loop_retries_attachment_materialization_without_poll_or_early_ack(self):
        self._assert_stream_loop_reconnects_without_poll(
            "attachment is temporarily unavailable: server returned HTTP 503",
            False,
        )

    def test_stream_reconnect_backoff_is_bounded(self):
        self.assertEqual(
            [self.module._stream_reconnect_delay(attempt) for attempt in range(7)],
            [2.0, 4.0, 8.0, 16.0, 30.0, 30.0, 30.0],
        )

    def test_ensure_service_can_discover_late_ready_file_after_startup_timeout(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            adapter = self.module.FiniteChatAdapter(
                PlatformConfig(
                    extra={
                        "home": temp_dir,
                        "finitechat_bin": "/bin/finitechat",
                        "service_addr": "127.0.0.1:0",
                    }
                )
            )
            original_create_subprocess_exec = self.module.asyncio.create_subprocess_exec
            module = cast(Any, self.module)
            original_start_timeout = module.SERVICE_START_TIMEOUT_SECS
            original_health = module._finitechat_service_health
            calls = []
            health_calls = []

            class FakeProcess:
                returncode = None

            async def fake_create_subprocess_exec(*args, **kwargs):
                calls.append(args)
                return FakeProcess()

            def fake_health(service_url, timeout):
                health_calls.append((service_url, timeout))
                return True

            try:
                self.module.asyncio.create_subprocess_exec = fake_create_subprocess_exec
                module._finitechat_service_health = fake_health
                module.SERVICE_START_TIMEOUT_SECS = 0.0
                started = asyncio.run(adapter._ensure_service())
                Path(temp_dir, self.module.SERVICE_READY_FILE).write_text(
                    '{"url":"http://127.0.0.1:7777"}',
                    encoding="utf-8",
                )
                rediscovered = asyncio.run(adapter._ensure_service())
            finally:
                self.module.asyncio.create_subprocess_exec = original_create_subprocess_exec
                module._finitechat_service_health = original_health
                module.SERVICE_START_TIMEOUT_SECS = original_start_timeout

        self.assertFalse(started)
        self.assertTrue(rediscovered)
        self.assertEqual(adapter.service_url, "http://127.0.0.1:7777")
        self.assertEqual(health_calls, [("http://127.0.0.1:7777", 2)])
        self.assertEqual(len(calls), 1)

    def test_ensure_service_waits_for_health_after_ready_file(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            adapter = self.module.FiniteChatAdapter(
                PlatformConfig(
                    extra={
                        "home": temp_dir,
                        "finitechat_bin": "/bin/finitechat",
                        "service_addr": "127.0.0.1:0",
                    }
                )
            )
            original_create_subprocess_exec = self.module.asyncio.create_subprocess_exec
            original_sleep = self.module.asyncio.sleep
            module = cast(Any, self.module)
            original_start_timeout = module.SERVICE_START_TIMEOUT_SECS
            original_health = module._finitechat_service_health
            health_calls = []
            sleeps = []

            class FakeProcess:
                returncode = None

            async def fake_create_subprocess_exec(*args, **kwargs):
                ready_file = Path(args[args.index("--ready-file") + 1])
                ready_file.write_text('{"url":"http://127.0.0.1:7777"}', encoding="utf-8")
                return FakeProcess()

            def fake_health(service_url, timeout):
                health_calls.append((service_url, timeout))
                return len(health_calls) >= 2

            async def fake_sleep(delay):
                sleeps.append(delay)

            try:
                self.module.asyncio.create_subprocess_exec = fake_create_subprocess_exec
                self.module.asyncio.sleep = fake_sleep
                module._finitechat_service_health = fake_health
                module.SERVICE_START_TIMEOUT_SECS = 1.0
                started = asyncio.run(adapter._ensure_service())
            finally:
                self.module.asyncio.create_subprocess_exec = original_create_subprocess_exec
                self.module.asyncio.sleep = original_sleep
                module._finitechat_service_health = original_health
                module.SERVICE_START_TIMEOUT_SECS = original_start_timeout

        self.assertTrue(started)
        self.assertEqual(adapter.service_url, "http://127.0.0.1:7777")
        self.assertEqual(
            health_calls,
            [("http://127.0.0.1:7777", 2), ("http://127.0.0.1:7777", 2)],
        )
        self.assertEqual(sleeps, [0.05])

    def test_ensure_service_starts_finitechat_serve_and_reads_ready_file(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            adapter = self.module.FiniteChatAdapter(
                PlatformConfig(
                    extra={
                        "home": temp_dir,
                        "finitechat_bin": "/bin/finitechat",
                        "service_addr": "127.0.0.1:0",
                    }
                )
            )
            original_create_subprocess_exec = self.module.asyncio.create_subprocess_exec
            module = cast(Any, self.module)
            original_health = module._finitechat_service_health
            calls = []
            health_calls = []

            class FakeProcess:
                returncode = None
                terminated = False
                killed = False

                def terminate(self):
                    self.terminated = True
                    self.returncode = 0

                def kill(self):
                    self.killed = True
                    self.returncode = -9

                async def wait(self):
                    return self.returncode

            fake_process = FakeProcess()

            async def fake_create_subprocess_exec(*args, **kwargs):
                calls.append(args)
                ready_file = Path(args[args.index("--ready-file") + 1])
                ready_file.write_text('{"url":"http://127.0.0.1:7777"}', encoding="utf-8")
                return fake_process

            def fake_health(service_url, timeout):
                health_calls.append((service_url, timeout))
                return True

            try:
                self.module.asyncio.create_subprocess_exec = fake_create_subprocess_exec
                module._finitechat_service_health = fake_health
                started = asyncio.run(adapter._ensure_service())
                asyncio.run(adapter._stop_service())
            finally:
                self.module.asyncio.create_subprocess_exec = original_create_subprocess_exec
                module._finitechat_service_health = original_health

        self.assertTrue(started)
        self.assertEqual(adapter.service_url, "http://127.0.0.1:7777")
        self.assertEqual(health_calls, [("http://127.0.0.1:7777", 2)])
        self.assertEqual(calls[0][0:2], ("/bin/finitechat", "hermes"))
        self.assertIn("serve", calls[0])
        self.assertTrue(fake_process.terminated)


if __name__ == "__main__":
    unittest.main()

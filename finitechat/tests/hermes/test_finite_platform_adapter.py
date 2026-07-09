import asyncio
import importlib.util
import os
import sys
import tempfile
import types
import unittest
from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path
from typing import Any, cast

REPO_ROOT = Path(__file__).resolve().parents[2]
ADAPTER_PATH = REPO_ROOT / "integrations" / "hermes" / "finitechat" / "adapter.py"


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


def install_gateway_stubs() -> None:
    gateway = types.ModuleType("gateway")
    config = types.ModuleType("gateway.config")
    platforms = types.ModuleType("gateway.platforms")
    base = types.ModuleType("gateway.platforms.base")

    config_module = cast(Any, config)
    base_module = cast(Any, base)
    config_module.Platform = Platform
    config_module.PlatformConfig = PlatformConfig
    base_module.BasePlatformAdapter = BasePlatformAdapter
    base_module.MessageEvent = MessageEvent
    base_module.MessageType = MessageType
    base_module.SendResult = SendResult

    sys.modules["gateway"] = gateway
    sys.modules["gateway.config"] = config
    sys.modules["gateway.platforms"] = platforms
    sys.modules["gateway.platforms.base"] = base


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

    def register_platform(self, **kwargs):
        self.registered.append(kwargs)


class FinitePlatformAdapterTests(unittest.TestCase):
    def setUp(self):
        self.module = load_adapter_module()

    def adapter(self, room_id: str | None = "room-agent-1"):
        extra = {"home": "/tmp/finite-agent-home", "finitechat_bin": "/bin/echo"}
        if room_id:
            extra["room_id"] = room_id
        return self.module.FiniteChatAdapter(PlatformConfig(extra=extra))

    def test_register_exposes_finitechat_platform_contract(self):
        ctx = MockPluginContext()
        self.module.register(ctx)

        self.assertEqual(len(ctx.registered), 1)
        entry = ctx.registered[0]
        self.assertEqual(entry["name"], "finitechat")
        self.assertEqual(entry["label"], "Finite Chat")
        self.assertEqual(entry["required_env"], ["FINITECHAT_HOME"])
        self.assertEqual(entry["allowed_users_env"], "FINITECHAT_ALLOWED_USERS")
        self.assertEqual(
            entry["max_message_length"], self.module.FiniteChatAdapter.MAX_MESSAGE_LENGTH
        )
        self.assertTrue(callable(entry["adapter_factory"]))

    def test_adapter_disables_edit_streaming_for_ios_rendering_compatibility(self):
        self.assertFalse(self.module.FiniteChatAdapter.SUPPORTS_MESSAGE_EDITING)

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
        adapter._surface_invite = noop
        adapter._poll_loop = idle_loop

        self.assertTrue(asyncio.run(adapter.connect(is_reconnect=True)))

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

    def test_send_translates_hermes_room_thread_and_metadata_to_bridge_json(self):
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
        self.assertEqual(payload["metadata"], {"priority": "low"})

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
                metadata={"conversation_id": "topic-build", "thread_id": "chat-build-1"},
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
        self.assertEqual(calls[1][1]["status"], "complete")
        self.assertTrue(calls[1][1]["finalize"])
        self.assertEqual(adapter._outbound_message_conversations["out-1"], "topic-build")
        self.assertEqual(adapter._outbound_message_conversations["edit-1"], "topic-build")
        self.assertEqual(adapter._outbound_message_segments["out-1"], "chat-build-1")
        self.assertEqual(adapter._outbound_message_segments["edit-1"], "chat-build-1")

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

    def test_connect_surfaces_invite_qr_url(self):
        adapter = self.adapter(room_id=None)
        calls = []

        async def fake_json(action, payload, *, timeout):
            calls.append(action)
            if action == "invite":
                return self.module._FiniteChatResult(
                    True,
                    {
                        "qr": "█▀▀▀█ qr █▀▀▀█",
                        "url": "finite://join?v=1&s=http%3A%2F%2Fx&r=r&i=i&t=00&a=npub1q",
                        "pin": "123456",
                        "pin_window_seconds": 30,
                    },
                    None,
                    False,
                )
            return self.module._FiniteChatResult(True, {}, None, False)

        adapter._finitechat_json = fake_json
        asyncio.run(adapter._surface_invite())
        self.assertEqual(calls, ["invite"])

    def _record_json(self, calls):
        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            return self.module._FiniteChatResult(True, {}, None, False)

        return fake_json

    def test_send_infers_tool_status_when_hermes_metadata_is_missing(self):
        adapter = self.adapter()
        calls = []

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            return self.module._FiniteChatResult(True, {"message_id": "tool-1"}, None, False)

        adapter._finitechat_json = fake_json
        result = asyncio.run(adapter.send("room-agent-1", "💻 shell\ncargo test ▉"))

        self.assertTrue(result.success)
        self.assertEqual(calls[0][0], "send")
        self.assertEqual(calls[0][1]["kind"], "tool")
        self.assertEqual(calls[0][1]["status"], "running")

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
        asyncio.run(adapter.stop_typing("room-agent-1"))

        self.assertEqual(calls[0][0], "activity")
        self.assertEqual(calls[0][1]["action"], "set")
        self.assertEqual(calls[0][1]["conversation_id"], "topic-build")
        self.assertEqual(calls[0][1]["segment_id"], "chat-build-1")
        self.assertEqual(calls[0][1]["expires_in_millis"], 60 * 1000)
        self.assertEqual(calls[1][0], "activity")
        self.assertEqual(calls[1][1]["action"], "clear")
        self.assertEqual(calls[1][1]["conversation_id"], "topic-build")
        self.assertEqual(calls[1][1]["segment_id"], "chat-build-1")
        self.assertEqual(adapter._activity_conversations, {})
        self.assertEqual(adapter._activity_segments, {})

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
        self.assertEqual(
            captured["url"],
            "http://127.0.0.1:9999/v1/hermes/inbound?"
            "room_id=room+agent&limit=10&timeout_millis=1000",
        )
        self.assertEqual(captured["timeout"], 7)
        self.assertEqual(results[0].data["records"][0]["type"], "joined")
        self.assertEqual(results[1].data["records"][0]["event"]["message_id"], "msg-12")

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

    def test_stream_loop_falls_back_to_poll_after_stream_transport_error(self):
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
            self.module._put_stream_result(
                loop,
                queue,
                self.module._FiniteChatResult(
                    False,
                    {},
                    "connection reset",
                    True,
                    transport_error=True,
                ),
            )

        async def fake_ensure_service():
            calls.append(("ensure", {}))
            return False

        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload))
            adapter._mark_disconnected()
            return self.module._FiniteChatResult(True, {"events": []}, None, False)

        async def fake_sleep(delay):
            calls.append(("sleep", {"delay": delay}))

        try:
            module._finitechat_service_stream_worker = fake_worker
            self.module.asyncio.sleep = fake_sleep
            adapter._ensure_service = fake_ensure_service
            adapter._finitechat_json = fake_json
            asyncio.run(adapter._stream_loop())
        finally:
            module._finitechat_service_stream_worker = original_worker
            self.module.asyncio.sleep = original_sleep

        self.assertEqual([call[0] for call in calls], ["stream", "sleep", "ensure", "poll"])
        self.assertEqual(adapter.service_url, "")

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

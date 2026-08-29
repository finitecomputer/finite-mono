"""The completion hook must never ack-consume an event whose reply could not
be routed.

Covers the release-not-ack hardening for the sidecar's typed unknown-thread
error (`error_kind: "hermes"`, `retryable: false`): `_settle_event_ack`
releases such an event like a cancelled turn so redelivery retries later, and
a failing send is attributed to the exact inbox entry through the turn's
ContextVar binding.
"""

import asyncio
import importlib.util
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
GATEWAY_MODULE_NAMES = (
    "gateway",
    "gateway.config",
    "gateway.platforms",
    "gateway.platforms.base",
    "gateway.session_context",
)


class Platform(Enum):
    FINITECHAT = "finitechat"


@dataclass
class HomeChannel:
    platform: Platform
    chat_id: str
    name: str


@dataclass
class PlatformConfig:
    enabled: bool = True
    extra: dict[str, Any] = field(default_factory=dict)
    home_channel: HomeChannel | None = None


class MessageType(Enum):
    TEXT = "text"
    DOCUMENT = "document"


@dataclass
class MessageEvent:
    text: str
    message_type: MessageType = MessageType.TEXT
    source: Any = None
    raw_message: Any = None
    message_id: str | None = None
    platform_update_id: int | None = None


@dataclass
class SendResult:
    success: bool
    message_id: str | None = None
    error: str | None = None
    raw_response: Any = None
    retryable: bool = False


class BasePlatformAdapter:
    background_probe: Any = None

    def __init__(self, config: PlatformConfig, platform: Platform):
        self.config = config
        self.platform = platform

    async def handle_message(self, event: MessageEvent) -> None:
        del event

    async def _process_message_background(self, event: MessageEvent, session_key: str) -> None:
        del session_key
        if type(self).background_probe is not None:
            type(self).background_probe()

    async def cancel_background_tasks(self) -> None:
        return None


def install_gateway_stubs() -> None:
    gateway = types.ModuleType("gateway")
    config = types.ModuleType("gateway.config")
    platforms = types.ModuleType("gateway.platforms")
    base = types.ModuleType("gateway.platforms.base")
    session_context = types.ModuleType("gateway.session_context")

    config_module = cast(Any, config)
    config_module.HomeChannel = HomeChannel
    config_module.Platform = Platform
    config_module.PlatformConfig = PlatformConfig
    base_module = cast(Any, base)
    base_module.BasePlatformAdapter = BasePlatformAdapter
    base_module.MessageEvent = MessageEvent
    base_module.MessageType = MessageType
    base_module.SendResult = SendResult
    base_module.build_session_key = lambda source, **_: "session-key"
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
    module_name = "finite_platform_adapter_route_fallback_under_test"
    sys.modules.pop(module_name, None)
    spec = importlib.util.spec_from_file_location(module_name, ADAPTER_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load adapter from {ADAPTER_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


UNRESOLVABLE_ROUTE_ERROR = (
    'hermes: unknown thread_id "topic-archived-mid-session" for room room-agent-1 '
    "(send): no matching conversation or segment in the agent store"
)


class AdapterRouteFallbackSettlementTests(unittest.TestCase):
    def setUp(self):
        self.original_gateway_modules = {
            name: sys.modules.get(name) for name in GATEWAY_MODULE_NAMES
        }
        self.module = load_adapter_module()
        state_home = tempfile.TemporaryDirectory()
        self.addCleanup(state_home.cleanup)
        self.state_home = state_home.name

    def tearDown(self):
        for name, module in self.original_gateway_modules.items():
            if module is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = module

    def adapter(self):
        return self.module.FiniteChatAdapter(
            self.module.PlatformConfig(
                extra={"home": self.state_home, "finitechat_bin": "/bin/echo"}
            )
        )

    @staticmethod
    def inbound_event(seq: Any = 7, message_id: str = "m-7") -> MessageEvent:
        return MessageEvent(
            text="a user message that must stay answerable",
            source=types.SimpleNamespace(
                platform=Platform.FINITECHAT,
                chat_id="room-agent-1",
                chat_type="dm",
                thread_id=None,
            ),
            raw_message={
                "room_id": "room-agent-1",
                "seq": seq,
                "message_id": message_id,
            },
        )

    def recording_finitechat_json(self, adapter, send_result):
        """Records sidecar actions; `send` answers with `send_result`."""
        module = cast(Any, self.module)
        calls = []

        async def fake_json(action, payload, *, timeout):
            del timeout
            calls.append((action, payload))
            if action == "send":
                return send_result
            return module._FiniteChatResult(True, {}, None, False)

        adapter._finitechat_json = fake_json
        return calls

    def run_turn_and_settle(self, adapter, event, outcome_name):
        """One failing send inside a bound turn, then the completion hook."""

        async def exercise():
            outcome = await adapter.send(
                chat_id="room-agent-1",
                content="this reply cannot be routed",
                metadata={"thread_id": "topic-archived-mid-session"},
            )
            await adapter._settle_event_ack(event, outcome_name)
            return outcome

        token = cast(Any, self.module)._TURN_EVENT_KEY.set("room-agent-1\x1f7\x1fm-7")
        try:
            return asyncio.run(exercise())
        finally:
            cast(Any, self.module)._TURN_EVENT_KEY.reset(token)

    def test_unresolvable_route_failure_releases_instead_of_acks(self):
        module = cast(Any, self.module)
        adapter = self.adapter()
        calls = self.recording_finitechat_json(
            adapter,
            module._FiniteChatResult(
                False,
                {},
                UNRESOLVABLE_ROUTE_ERROR,
                False,
                False,
                error_kind="hermes",
            ),
        )

        result = self.run_turn_and_settle(adapter, self.inbound_event(), "failure")

        self.assertFalse(result.success)
        self.assertEqual(
            [action for action, _payload in calls],
            ["send", "release"],
            "the completion hook releases the unresolvable-route event instead of ack-consuming it",
        )
        release_payload = calls[1][1]
        self.assertEqual(release_payload["room_id"], "room-agent-1")
        self.assertEqual(release_payload["seq"], 7)
        self.assertEqual(release_payload["message_id"], "m-7")
        self.assertEqual(
            list(adapter._route_resolution_failures),
            [],
            "the failure marker is consumed when its event settles",
        )

    def test_release_on_route_error_holds_for_success_outcome_too(self):
        module = cast(Any, self.module)
        adapter = self.adapter()
        calls = self.recording_finitechat_json(
            adapter,
            module._FiniteChatResult(
                False,
                {},
                UNRESOLVABLE_ROUTE_ERROR,
                False,
                False,
                error_kind="hermes",
            ),
        )

        self.run_turn_and_settle(adapter, self.inbound_event(), "success")

        self.assertEqual([action for action, _payload in calls], ["send", "release"])

    def test_ordinary_failures_still_ack(self):
        module = cast(Any, self.module)
        adapter = self.adapter()
        calls = self.recording_finitechat_json(
            adapter,
            module._FiniteChatResult(False, {}, "server returned 500: busy", True),
        )

        self.run_turn_and_settle(adapter, self.inbound_event(), "failure")

        self.assertEqual(
            [action for action, _payload in calls],
            ["send", "ack"],
            "only unresolvable-reply-route errors release; ordinary failures ack",
        )
        self.assertEqual(list(adapter._route_resolution_failures), [])

    def test_route_failure_marker_correlates_by_event_key(self):
        module = cast(Any, self.module)
        adapter = self.adapter()
        calls = self.recording_finitechat_json(
            adapter,
            module._FiniteChatResult(
                False,
                {},
                UNRESOLVABLE_ROUTE_ERROR,
                False,
                False,
                error_kind="hermes",
            ),
        )

        async def exercise():
            outcome = await adapter.send(
                chat_id="room-agent-1",
                content="this reply cannot be routed",
                metadata=None,
            )
            # A different event settles while the failed send's marker is
            # outstanding: it must still be acked.
            await adapter._settle_event_ack(
                self.inbound_event(seq=8, message_id="m-8"),
                "failure",
            )
            return outcome

        token = module._TURN_EVENT_KEY.set("room-agent-1\x1f7\x1fm-7")
        try:
            asyncio.run(exercise())
        finally:
            module._TURN_EVENT_KEY.reset(token)

        self.assertEqual(
            [action for action, _payload in calls],
            ["send", "ack"],
            "unrelated events are never released because of another event's route failure",
        )
        self.assertIn(
            "room-agent-1\x1f7\x1fm-7",
            adapter._route_resolution_failures,
            "the failed send's marker stays outstanding for its own event",
        )

    def test_background_turn_binds_and_restores_the_event_key(self):
        module = cast(Any, self.module)
        adapter = self.adapter()
        probes = []

        def probe():
            probes.append(module._TURN_EVENT_KEY.get())

        event = self.inbound_event()
        BasePlatformAdapter.background_probe = probe
        try:

            async def exercise():
                await adapter._process_message_background(
                    event,
                    "agent:main:finitechat:dm:room-agent-1",
                )
                return module._TURN_EVENT_KEY.get()

            residual_after_turn = asyncio.run(exercise())
        finally:
            BasePlatformAdapter.background_probe = None

        expected_key = "room-agent-1\x1f7\x1fm-7"
        self.assertEqual(probes, [expected_key])
        self.assertIsNone(residual_after_turn, "the key binding does not outlive the turn")

    def test_unresolvable_route_error_signature_is_narrow(self):
        module = cast(Any, self.module)
        classify = module.FiniteChatAdapter._is_unresolvable_route_error

        matching = module._FiniteChatResult(
            False, {}, UNRESOLVABLE_ROUTE_ERROR, False, False, error_kind="hermes"
        )
        retryable_lookalike = module._FiniteChatResult(
            False, {}, UNRESOLVABLE_ROUTE_ERROR, True, False, error_kind="hermes"
        )
        other_kind = module._FiniteChatResult(
            False, {}, UNRESOLVABLE_ROUTE_ERROR, False, False, error_kind="runtime"
        )
        ordinary_hermes_failure = module._FiniteChatResult(
            False,
            {},
            "hermes: attachment could not be opened",
            False,
            False,
            error_kind="hermes",
        )

        self.assertTrue(classify(matching))
        self.assertFalse(classify(retryable_lookalike))
        self.assertFalse(classify(other_kind))
        self.assertFalse(classify(ordinary_hermes_failure))

    def test_remembered_route_failures_stay_bounded(self):
        module = cast(Any, self.module)
        adapter = self.adapter()
        limit = module.MAX_ROUTE_RESOLUTION_FAILURES

        for index in range(limit + 25):
            token = module._TURN_EVENT_KEY.set(f"room-agent-1\x1f{index}\x1fm-{index}")
            try:
                adapter._record_route_resolution_failure()
            finally:
                module._TURN_EVENT_KEY.reset(token)

        self.assertLessEqual(len(adapter._route_resolution_failures), limit)


if __name__ == "__main__":
    unittest.main()

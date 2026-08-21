import asyncio
import hashlib
import importlib.util
import os
import shlex
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from gateway.config import GatewayConfig, HomeChannel, Platform, PlatformConfig
from gateway.platforms.base import build_session_key
from gateway.run import GatewayRunner
from gateway.session import SessionSource, build_session_context, build_session_context_prompt
from gateway.session_context import get_session_env
from hermes_cli import plugins
from model_tools import handle_function_call

REPO_ROOT = Path(__file__).resolve().parents[2]
ADAPTER_PATH = REPO_ROOT / "integrations" / "hermes" / "finitechat" / "adapter.py"


class HookOnlyPluginContext:
    def __init__(self, manager):
        self.manager = manager

    def register_hook(self, name, callback):
        self.manager._hooks.setdefault(name, []).append(callback)

    def register_platform(self, **_kwargs):
        pass


def load_adapter_module():
    module_name = "finitechat_pinned_hook_adapter_under_test"
    sys.modules.pop(module_name, None)
    spec = importlib.util.spec_from_file_location(module_name, ADAPTER_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load adapter from {ADAPTER_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


PINNED_ADAPTER_MODULE = load_adapter_module()


class PinnedHermesSenderContextTests(unittest.TestCase):
    def test_cached_second_turn_uses_session_key_around_each_terminal_process(self):
        session_key = "finitechat:room-agent-1:home-chat"
        cached_session_id = "cached-agent-session"
        with tempfile.TemporaryDirectory() as finite_home:
            finite_home_path = Path(finite_home)
            context_name = hashlib.sha256(session_key.encode("utf-8")).hexdigest() + ".json"
            context_path = finite_home_path / "requester-context-v1" / context_name
            manager = plugins.PluginManager()
            previous_manager = plugins._plugin_manager
            plugins._plugin_manager = manager
            try:
                with patch.dict(os.environ, {"FINITE_HOME": finite_home}):
                    adapter = load_adapter_module()
                    adapter.register(HookOnlyPluginContext(manager))

                    runner = object.__new__(GatewayRunner)
                    runner.adapters = {}
                    for turn, account_id in enumerate(("a1" * 32, "b2" * 32), start=1):
                        source = SessionSource(
                            platform=adapter._finite_platform(),
                            chat_id="room-agent-1",
                            chat_type="group",
                            user_id=account_id,
                            thread_id="home-chat",
                        )
                        context = build_session_context(source, GatewayConfig())
                        context.session_key = session_key
                        context.session_id = cached_session_id
                        tokens = runner._set_session_env(context)
                        try:
                            self.assertEqual(get_session_env("HERMES_SESSION_KEY"), session_key)
                            self.assertEqual(get_session_env("HERMES_SESSION_PLATFORM"), "local")
                            # Gateway does not bind the cached agent's session ID
                            # on later turns; this is the production shape that
                            # the requester bridge must not depend on.
                            self.assertEqual(get_session_env("HERMES_SESSION_ID"), "")
                            observed_path = finite_home_path / f"observed-{turn}"
                            command = (
                                f'test "$HERMES_SESSION_KEY" = {shlex.quote(session_key)} '
                                f"&& test -f {shlex.quote(str(context_path))} "
                                f"&& grep -q {shlex.quote(account_id)} "
                                f"{shlex.quote(str(context_path))} "
                                f"&& printf observed > {shlex.quote(str(observed_path))}"
                            )
                            marker = adapter._AUTHENTICATED_FINITE_TURN_USER.set(account_id)
                            try:
                                result = handle_function_call(
                                    "terminal",
                                    {"command": command},
                                    task_id=f"task-{turn}",
                                    session_id=cached_session_id,
                                    tool_call_id=f"call-{turn}",
                                )
                            finally:
                                adapter._AUTHENTICATED_FINITE_TURN_USER.reset(marker)
                            self.assertTrue(observed_path.exists(), result)
                            self.assertEqual(observed_path.read_text(), "observed")
                            self.assertFalse(context_path.exists())

                            no_marker_path = finite_home_path / f"no-marker-{turn}"
                            no_marker_result = handle_function_call(
                                "terminal",
                                {
                                    "command": (
                                        f"test ! -e {shlex.quote(str(context_path))} "
                                        f"&& printf isolated > "
                                        f"{shlex.quote(str(no_marker_path))}"
                                    )
                                },
                                task_id=f"background-{turn}",
                                session_id=cached_session_id,
                                tool_call_id=f"background-call-{turn}",
                            )
                            self.assertTrue(no_marker_path.exists(), no_marker_result)
                        finally:
                            runner._clear_session_env(tokens)
            finally:
                plugins._plugin_manager = previous_manager

    def test_threaded_group_requires_per_turn_authenticated_sender_context(self):
        account_id = "a1" * 32
        source = SessionSource(
            platform=Platform.TELEGRAM,
            chat_id="room-agent-1",
            chat_type="group",
            user_id=account_id,
            user_name=None,
            thread_id="home-chat",
        )

        context = build_session_context(source, GatewayConfig())
        prompt = build_session_context_prompt(context)
        sender_prompt = (
            "Authenticated Finite Chat sender metadata for this turn: "
            f"event.source.user_id is `{account_id}`."
        )
        combined_prompt = f"{prompt}\n\n{sender_prompt}"

        self.assertTrue(context.shared_multi_user_session)
        self.assertNotIn(account_id, prompt)
        self.assertIn("Multi-user thread", prompt)
        self.assertIn(account_id, combined_prompt)


class PinnedHermesQueueAdmissionTests(unittest.IsolatedAsyncioTestCase):
    async def test_real_018_owner_task_blocks_ack_until_followup_turn_completes(self):
        module = PINNED_ADAPTER_MODULE
        agent_home = tempfile.TemporaryDirectory()
        self.addCleanup(agent_home.cleanup)
        adapter = module.FiniteChatAdapter(
            PlatformConfig(
                enabled=True,
                typing_indicator=False,
                home_channel=HomeChannel(
                    platform=module._finite_platform(),
                    chat_id="room-agent-1",
                    name="Finite Chat",
                ),
                extra={
                    "home": agent_home.name,
                    "finitechat_bin": "/bin/echo",
                },
            )
        )
        bridge_calls = []
        handler_events = []
        handler_started = asyncio.Event()
        finish_handler = asyncio.Event()

        async def fake_json(action, payload, *, timeout):
            bridge_calls.append((action, payload, timeout))
            return module._FiniteChatResult(True, {}, None, False)

        async def handler(event):
            handler_events.append(event)
            handler_started.set()
            await finish_handler.wait()
            return ""

        adapter._finitechat_json = fake_json
        adapter.set_message_handler(handler)
        source = adapter.build_source(
            chat_id="room-agent-1",
            chat_type="dm",
            user_id="alice",
            thread_id="chat-build-1",
        )
        session_key = build_session_key(source)
        owner_release = asyncio.Event()
        owner = asyncio.create_task(owner_release.wait())
        adapter._active_sessions[session_key] = asyncio.Event()
        adapter._session_tasks[session_key] = owner
        raw_event = {
            "room_id": "room-agent-1",
            "seq": 61,
            "message_id": "msg-61",
            "conversation_id": "topic-build",
            "segment_id": "chat-build-1",
            "text": "queued on real Hermes",
            "message_type": "text",
            "source": {
                "platform": "finitechat",
                "chat_id": "room-agent-1",
                "chat_type": "dm",
                "user_id": "alice",
            },
        }

        await adapter._handle_finitechat_event(raw_event)
        self.assertEqual(handler_events, [])
        self.assertEqual(bridge_calls, [])
        admission = adapter._admission_tasks[session_key]

        owner_release.set()
        await owner
        await asyncio.wait_for(handler_started.wait(), timeout=1)
        await asyncio.wait_for(admission, timeout=1)

        self.assertEqual([event.text for event in handler_events], ["queued on real Hermes"])
        # The turn owns the ack: dispatch alone must not ack the durable
        # inbox entry while the turn is still running.
        self.assertEqual([call[0] for call in bridge_calls], ["activity"])

        turn_task = adapter._session_tasks[session_key]
        finish_handler.set()
        await asyncio.wait_for(turn_task, timeout=5)

        # The processing activity is set first; the ack follows once the turn
        # completes. Later activity calls are the runtime's typing cleanup.
        self.assertEqual(bridge_calls[0][0], "activity")
        self.assertEqual(bridge_calls[0][1]["action"], "set")
        ack_calls = [call for call in bridge_calls if call[0] == "ack"]
        self.assertEqual(len(ack_calls), 1)
        self.assertEqual(ack_calls[0][1]["message_id"], "msg-61")

        await adapter.cancel_background_tasks()


class PinnedHermesClarificationTests(unittest.IsolatedAsyncioTestCase):
    @staticmethod
    def _adapter(home: str):
        module = PINNED_ADAPTER_MODULE
        return module.FiniteChatAdapter(
            PlatformConfig(
                enabled=True,
                typing_indicator=False,
                home_channel=HomeChannel(
                    platform=module._finite_platform(),
                    chat_id="room-agent-1",
                    name="Finite Chat",
                ),
                extra={
                    "home": home,
                    "finitechat_bin": "/bin/echo",
                },
            )
        )

    def _agent_home(self) -> str:
        agent_home = tempfile.TemporaryDirectory()
        self.addCleanup(agent_home.cleanup)
        return agent_home.name

    @staticmethod
    def _text_event(
        *,
        segment_id: str,
        seq: int,
        message_id: str,
        text: str,
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

    async def test_real_018_choice_reply_resumes_only_the_originating_active_chat(self):
        from tools import clarify_gateway

        module = PINNED_ADAPTER_MODULE
        adapter = self._adapter(self._agent_home())
        bridge_calls = []

        async def fake_json(action, payload, *, timeout):
            bridge_calls.append((action, payload, timeout))
            return module._FiniteChatResult(
                True,
                {"message_id": f"bridge-{len(bridge_calls)}"},
                None,
                False,
            )

        async def resolve_pending(event):
            session_key = build_session_key(event.source)
            clarify_gateway.resolve_text_response_for_session(session_key, event.text)
            return ""

        adapter._finitechat_json = fake_json
        adapter.set_message_handler(resolve_pending)
        session_a = build_session_key(
            adapter.build_source(
                chat_id="room-agent-1",
                chat_type="dm",
                user_id="alice",
                thread_id="chat-a",
            )
        )
        session_b = build_session_key(
            adapter.build_source(
                chat_id="room-agent-1",
                chat_type="dm",
                user_id="alice",
                thread_id="chat-b",
            )
        )
        request_id = "pinned-choice"
        clarify_gateway.register(
            clarify_id=request_id,
            session_key=session_a,
            question="Which environment should receive this change?",
            choices=["Staging", "Production"],
        )
        releases = {session_a: asyncio.Event(), session_b: asyncio.Event()}
        owners = {key: asyncio.create_task(release.wait()) for key, release in releases.items()}
        adapter._active_sessions.update({key: asyncio.Event() for key in releases})
        adapter._session_tasks.update(owners)
        try:
            sent = await adapter.send_clarify(
                chat_id="room-agent-1",
                question="Which environment should receive this change?",
                choices=["Staging", "Production"],
                clarify_id=request_id,
                session_key=session_a,
                metadata={
                    "conversation_id": "topic-build",
                    "segment_id": "chat-a",
                    "thread_id": "chat-a",
                },
            )
            self.assertTrue(sent.success)
            prompt = bridge_calls[0][1]
            self.assertEqual(prompt["kind"], "message")
            self.assertEqual(prompt["conversation_id"], "topic-build")
            self.assertEqual(prompt["segment_id"], "chat-a")
            self.assertIn("1. Staging", prompt["text"])
            self.assertIn("2. Production", prompt["text"])

            await adapter._handle_finitechat_event(
                self._text_event(
                    segment_id="chat-b",
                    seq=81,
                    message_id="answer-b",
                    text="2",
                )
            )
            self.assertNotIn(
                "answer-b",
                [
                    payload.get("message_id")
                    for action, payload, _ in bridge_calls
                    if action == "ack"
                ],
            )
            self.assertIsNotNone(
                clarify_gateway.get_pending_for_session(
                    session_a,
                    include_choice_prompts=True,
                )
            )

            await adapter._handle_finitechat_event(
                self._text_event(
                    segment_id="chat-a",
                    seq=82,
                    message_id="answer-a",
                    text="2",
                )
            )
            self.assertEqual(
                clarify_gateway.wait_for_response(request_id, timeout=0.1),
                "Production",
            )
            self.assertIn(
                "answer-a",
                [
                    payload.get("message_id")
                    for action, payload, _ in bridge_calls
                    if action == "ack"
                ],
            )
        finally:
            await adapter._cancel_admission_tasks()
            clarify_gateway.clear_session(session_a)
            for release in releases.values():
                release.set()
            await asyncio.gather(*owners.values(), return_exceptions=True)

    async def test_real_018_open_reply_survives_adapter_reconstruction(self):
        from tools import clarify_gateway

        module = PINNED_ADAPTER_MODULE
        agent_home = self._agent_home()
        first = self._adapter(agent_home)
        first_calls = []

        async def first_json(action, payload, *, timeout):
            first_calls.append((action, payload, timeout))
            return module._FiniteChatResult(True, {"message_id": "prompt-open"}, None, False)

        first._finitechat_json = first_json
        session_key = build_session_key(
            first.build_source(
                chat_id="room-agent-1",
                chat_type="dm",
                user_id="alice",
                thread_id="chat-open",
            )
        )
        request_id = "pinned-open"
        clarify_gateway.register(
            clarify_id=request_id,
            session_key=session_key,
            question="What should change?",
            choices=None,
        )
        owner_release = asyncio.Event()
        owner = asyncio.create_task(owner_release.wait())
        try:
            sent = await first.send_clarify(
                chat_id="room-agent-1",
                question="What should change?",
                choices=None,
                clarify_id=request_id,
                session_key=session_key,
                metadata={
                    "conversation_id": "topic-build",
                    "segment_id": "chat-open",
                    "thread_id": "chat-open",
                },
            )
            self.assertTrue(sent.success)
            self.assertEqual(first_calls[0][1]["text"], "❓ What should change?")
            self.assertEqual(first_calls[0][1]["kind"], "message")

            restarted = self._adapter(agent_home)
            restarted_calls = []

            async def restarted_json(action, payload, *, timeout):
                restarted_calls.append((action, payload, timeout))
                return module._FiniteChatResult(True, {"message_id": action}, None, False)

            async def resolve_pending(event):
                clarify_gateway.resolve_text_response_for_session(
                    build_session_key(event.source),
                    event.text,
                )
                return ""

            restarted._finitechat_json = restarted_json
            restarted.set_message_handler(resolve_pending)
            restarted._active_sessions[session_key] = asyncio.Event()
            restarted._session_tasks[session_key] = owner
            await restarted._handle_finitechat_event(
                self._text_event(
                    segment_id="chat-open",
                    seq=83,
                    message_id="answer-open",
                    text="Keep the existing colors.",
                )
            )

            self.assertEqual(
                clarify_gateway.wait_for_response(request_id, timeout=0.1),
                "Keep the existing colors.",
            )
            self.assertIn(
                "answer-open",
                [
                    payload.get("message_id")
                    for action, payload, _ in restarted_calls
                    if action == "ack"
                ],
            )
        finally:
            clarify_gateway.clear_session(session_key)
            owner_release.set()
            await owner


if __name__ == "__main__":
    unittest.main()

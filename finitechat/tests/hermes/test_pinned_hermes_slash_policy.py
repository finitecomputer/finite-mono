"""Restricted slash commands are refused by the adapter, against the pinned gateway.

Finite Chat hands every inbound event to Hermes through
``_handle_finitechat_event``. A restricted command (``/update``, ``/restart``,
``/loop`` and the rest of ``slash_policy.json``) must be answered there with a
durable refusal on the event's own route and settled, before Hermes sees it on
either the idle or the busy path. Everything else must reach Hermes unchanged.

These tests drive the real pinned ``MessageEvent``, command registry, plaintext
coercion, and ``BasePlatformAdapter`` dispatch. Only the sidecar and the turn
handler are simulated.
"""

import asyncio
import importlib.util
import json
import logging
import os
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any
from unittest.mock import patch

from gateway.config import GatewayConfig, PlatformConfig
from gateway.platforms import base as hermes_base
from gateway.platforms.base import MessageEvent, MessageType
from gateway.run import GatewayRunner
from gateway.session import SessionSource
from hermes_cli import commands as hermes_commands

REPO_ROOT = Path(__file__).resolve().parents[2]
PLUGIN_DIR = REPO_ROOT / "integrations" / "hermes" / "finitechat"
ADAPTER_PATH = PLUGIN_DIR / "adapter.py"
POLICY_MODULE_PATH = PLUGIN_DIR / "slash_policy.py"
POLICY_DATA_PATH = PLUGIN_DIR / "slash_policy.json"
ROOM_ID = "room-agent-1"
TIERS = {"suggested", "available", "not_recommended", "unlisted", "restricted"}
RESTRICTED = {
    "approvals",
    "blueprint",
    "bundles",
    "busy",
    "codex-runtime",
    "commands",
    "curator",
    "debug",
    "diff",
    "egress",
    "fast",
    "footer",
    "help",
    "init",
    "insights",
    "kanban",
    "loop",
    "memory",
    "moa",
    "platform",
    "profile",
    "reload-mcp",
    "restart",
    "rollback",
    "save",
    "sethome",
    "skills",
    "start",
    "suggestions",
    "title",
    "topic",
    "topup",
    "update",
    "verbose",
    "voice",
    "whoami",
    "yolo",
}
REFUSED_TEXT = {
    "/update": "update",
    "/UPDATE": "update",
    "/update@finite_bot": "update",
    "  /update now": "update",
    "/codex_runtime auto": "codex-runtime",
    "/reload_mcp": "reload-mcp",
    "/loop 30s /restart": "loop",
    "/proactive 5m check mail": "loop",
    "/restart": "restart",
    "restart hermes": "restart",
    "Please restart the gateway.": "restart",
}
PASSED_TEXT = (
    "hello there",
    "see https://example.com/a/b?c=d",
    "see /usr/bin for the binary",
    "a/b",
    "/usr/bin/env python3",
    "/stop",
    "/new",
    "/reset",
    "/queue then summarize it",
    "/q then summarize it",
    "/pause off",
    "/PAUSE Off",
    "/pause",
    "/approve",
    "/always",
    "/cancel",
    "/yes",
    "/no",
    "/my-custom-skill do the thing",
    "/status",
    "/model",
)


def load_module(name: str, path: Path) -> Any:
    sys.modules.pop(name, None)
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def load_policy_module() -> Any:
    return load_module("finitechat_slash_policy_under_test", POLICY_MODULE_PATH)


def message_event(text: str, *, chat_type: str = "dm", **kwargs: Any) -> MessageEvent:
    source = SessionSource(
        platform=hermes_base.Platform.LOCAL,
        chat_id=ROOM_ID,
        chat_type=chat_type,
        user_id="alice",
        thread_id="segment-1",
    )
    return MessageEvent(text=text, source=source, message_id="msg-1", **kwargs)


def raw_event(seq: int, text: str, *, chat_type: str = "dm") -> dict[str, Any]:
    return {
        "room_id": ROOM_ID,
        "seq": seq,
        "message_id": f"msg-{seq}",
        "conversation_id": "home",
        "segment_id": "segment-1",
        "text": text,
        "message_type": "text",
        "source": {
            "platform": "finitechat",
            "chat_id": ROOM_ID,
            "chat_type": chat_type,
            "user_id": "alice",
            "thread_id": "segment-1",
            "is_bot": False,
        },
        "attachments": [],
        "internal": False,
    }


def raw_photo(seq: int, caption: str) -> dict[str, Any]:
    raw = raw_event(seq, caption)
    raw["attachments"] = [
        {
            "kind": "image",
            "name": f"photo-{seq}.png",
            "path": f"/synthetic/photo-{seq}.png",
            "mime_type": "image/png",
        }
    ]
    return raw


class PolicyDataTests(unittest.TestCase):
    def setUp(self):
        self.policy = json.loads(POLICY_DATA_PATH.read_text(encoding="utf-8"))

    def test_every_pinned_gateway_command_has_a_tier(self):
        gateway_commands = {
            command.name
            for command in hermes_commands.COMMAND_REGISTRY
            if command.name in hermes_commands.GATEWAY_KNOWN_COMMANDS
        }
        missing = sorted(gateway_commands - set(self.policy))
        self.assertEqual(missing, [], "tier these commands in slash_policy.json")
        stale = sorted(set(self.policy) - gateway_commands)
        self.assertEqual(stale, [], "these entries are not pinned gateway commands")

    def test_every_gateway_alias_resolves_to_a_tiered_command(self):
        for name in sorted(hermes_commands.GATEWAY_KNOWN_COMMANDS):
            with self.subTest(name=name):
                command = hermes_commands.resolve_command(name)
                self.assertIn(getattr(command, "name", None), self.policy)

    def test_entries_are_plain_and_well_formed(self):
        for name, entry in self.policy.items():
            with self.subTest(name=name):
                self.assertLessEqual(set(entry), {"tier", "reason"})
                self.assertIn(entry["tier"], TIERS)
                if entry["tier"] in {"restricted", "not_recommended"}:
                    self.assertTrue(entry.get("reason", "").strip())
                self.assertNotIn("\u2014", entry.get("reason", ""))

    def test_restricted_set_matches_the_catalog(self):
        restricted = {name for name, entry in self.policy.items() if entry["tier"] == "restricted"}
        self.assertEqual(restricted, RESTRICTED)
        for control in ("stop", "new", "queue", "pause", "approve", "deny", "steer"):
            self.assertNotEqual(self.policy[control]["tier"], "restricted", control)


class EvaluateTests(unittest.TestCase):
    def setUp(self):
        self.policy = load_policy_module()

    def test_restricted_commands_are_refused_by_canonical_name(self):
        for text, expected in REFUSED_TEXT.items():
            if expected is None:
                continue
            with self.subTest(text=text):
                refusal = self.policy.evaluate(message_event(text))
                self.assertIsNotNone(refusal)
                self.assertEqual(refusal.command, expected)
                self.assertTrue(refusal.text.startswith(f"/{expected} isn't available"))
                self.assertNotIn("\u2014", refusal.text)

    def test_refusal_text_carries_the_catalog_reason(self):
        refusal = self.policy.evaluate(message_event("/update"))
        self.assertEqual(
            refusal.text,
            "/update isn't available in Finite chat. "
            "Finite manages your agent's software and restarts.",
        )
        generic = self.policy.evaluate(message_event("/rollback"))
        self.assertEqual(generic.text, "/rollback isn't available in Finite chat.")

    def test_evaluation_does_not_rewrite_the_event(self):
        event = message_event("restart hermes")
        self.assertIsNotNone(self.policy.evaluate(event))
        self.assertEqual(event.text, "restart hermes")

    def test_everything_else_passes(self):
        for text in PASSED_TEXT:
            with self.subTest(text=text):
                self.assertIsNone(self.policy.evaluate(message_event(text)))

    def test_group_plaintext_restart_is_ordinary_conversation(self):
        self.assertIsNone(self.policy.evaluate(message_event("restart hermes", chat_type="group")))

    def test_photo_caption_without_a_command_passes(self):
        event = message_event("what is in this picture?", message_type=MessageType.PHOTO)
        self.assertIsNone(self.policy.evaluate(event))

    def test_events_without_gateway_control_pass(self):
        event = message_event("/update", allow_gateway_control=False)
        self.assertIsNone(self.policy.evaluate(event))

    def test_untiered_gateway_command_fails_closed(self):
        with patch.dict(self.policy._policy(), {}, clear=True):
            refusal = self.policy.evaluate(message_event("/status"))
        self.assertIsNotNone(refusal)
        self.assertEqual(refusal.command, "status")

    def test_loop_is_refused_even_if_retiered(self):
        with patch.dict(self.policy._policy(), {"loop": {"tier": "available"}}):
            self.assertIsNotNone(self.policy.evaluate(message_event("/loop 5m /status")))

    def test_pause_off_passes_even_if_pause_is_restricted(self):
        restricted = {"tier": "restricted", "reason": "x"}
        with patch.dict(self.policy._policy(), {"pause": restricted}):
            self.assertIsNone(self.policy.evaluate(message_event("/pause off")))
            self.assertIsNotNone(self.policy.evaluate(message_event("/pause now")))

    def test_missing_coercion_helper_degrades_and_logs_once(self):
        with (
            patch.object(hermes_base, "coerce_plaintext_gateway_command", None),
            self.assertLogs(self.policy.logger, logging.WARNING) as logs,
        ):
            self.assertIsNone(self.policy.evaluate(message_event("restart hermes")))
            self.assertIsNone(self.policy.evaluate(message_event("restart hermes")))
            self.assertIsNotNone(self.policy.evaluate(message_event("/update")))
        self.assertEqual(len(logs.records), 1)

    def test_missing_registry_resolver_allows_and_logs_once(self):
        with (
            patch.object(hermes_commands, "resolve_command", None),
            self.assertLogs(self.policy.logger, logging.WARNING) as logs,
        ):
            self.assertIsNone(self.policy.evaluate(message_event("/update")))
            self.assertIsNone(self.policy.evaluate(message_event("/stop")))
        self.assertEqual(len(logs.records), 1)

    def test_unreadable_policy_allows_and_logs_once(self):
        with (
            tempfile.TemporaryDirectory() as scratch,
            patch.object(self.policy, "POLICY_PATH", Path(scratch) / "missing.json"),
            self.assertLogs(self.policy.logger, logging.ERROR) as logs,
        ):
            self.policy._policy.cache_clear()
            self.assertIsNone(self.policy.evaluate(message_event("/update")))
            self.assertIsNone(self.policy.evaluate(message_event("/stop")))
        self.assertEqual(len(logs.records), 1)


class PolicyHarness:
    """Real pinned gateway + adapter; simulated sidecar and turn handler."""

    def __init__(self, home: str):
        self.module = load_module("finitechat_pinned_slash_adapter_under_test", ADAPTER_PATH)
        config = PlatformConfig(enabled=True, extra={"home": home, "room_id": ROOM_ID})
        config.typing_indicator = False
        self.adapter = self.module.FiniteChatAdapter(config)
        self.adapter._home_channel_hydrated = True
        self.runner = GatewayRunner(GatewayConfig(sessions_dir=Path(home) / "sessions"))
        self.runner._session_db = None
        self.runner._persist_active_agents = lambda: None
        self.runner.adapters[self.adapter.platform] = self.adapter
        self.module._finite_private_control_request = lambda *_args: None

        self.inbox: dict[str, tuple[dict[str, Any], str]] = {}
        self.calls: list[tuple[str, dict[str, Any]]] = []
        self.admitted: list[str] = []
        self.handled: list[str] = []
        self.runs: list[str] = []
        self.started = asyncio.Event()
        self.adapter._finitechat_json = self._sidecar
        self.adapter.set_message_handler(self._handle)
        admit = self.adapter.handle_message

        async def recording_handle_message(event):
            self.admitted.append(event.text)
            return await admit(event)

        self.adapter.handle_message = recording_handle_message

    async def _sidecar(self, action, payload, *, timeout):
        del timeout
        self.calls.append((action, payload))
        if action in ("ack", "release"):
            raw, state = self.inbox[payload["message_id"]]
            if state == "leased":
                self.inbox[payload["message_id"]] = (
                    raw,
                    "acked" if action == "ack" else "pending",
                )
        data = {"message_id": f"reply-{len(self.calls)}"} if action == "send" else {}
        return self.module._FiniteChatResult(True, data, None, False)

    async def _handle(self, event):
        self.handled.append(event.text)
        session_key = self.runner._session_key_for_source(event.source)
        if (event.text or "").startswith("/stop"):
            return await self.runner._busy_stop_command(event, session_key, event.source)
        if (event.text or "").startswith("/"):
            return "command handled"
        self.runs.append(event.message_id)
        self.runner._begin_session_run_generation(session_key)

        class Agent:
            def hard_interrupt(self, message=None):
                del message

        self.runner._session_state(session_key).turn.agent = Agent()
        if event.text == "long running work":
            self.started.set()
            await asyncio.Event().wait()
        return "done"

    async def deliver(self, raw: dict[str, Any]) -> None:
        self.inbox[raw["message_id"]] = (raw, "leased")
        await self.adapter._handle_finitechat_event(raw)
        for _ in range(20):
            await asyncio.sleep(0)

    async def start_long_turn(self) -> None:
        await self.deliver(raw_event(1, "long running work"))
        await asyncio.wait_for(self.started.wait(), 2)

    def state(self, message_id: str) -> str:
        return self.inbox[message_id][1]

    async def wait_all_settled(self) -> None:
        async def settled():
            while any(state == "leased" for _raw, state in self.inbox.values()):
                await asyncio.sleep(0.01)

        await asyncio.wait_for(settled(), 5)

    def settlements(self, message_id: str) -> list[str]:
        return [
            action
            for action, payload in self.calls
            if action in ("ack", "release") and payload.get("message_id") == message_id
        ]

    def sends(self) -> list[dict[str, Any]]:
        return [payload for action, payload in self.calls if action == "send"]

    def refusals(self) -> list[dict[str, Any]]:
        return [send for send in self.sends() if "isn't available in Finite" in send["text"]]

    async def close(self) -> None:
        await self.adapter._cancel_admission_tasks()
        await self.adapter.cancel_background_tasks()
        self.runner.close_all_session_db_handles()
        self.runner.session_store.close_all_db_handles()
        self.runner._shutdown_executor()


class AdapterRefusalTests(unittest.TestCase):
    def run_scenario(self, scenario):
        with (
            tempfile.TemporaryDirectory(prefix="finite-slash-") as home,
            patch.dict(os.environ, {"HERMES_HOME": home}),
        ):

            async def main():
                harness = PolicyHarness(home)
                try:
                    await scenario(harness)
                finally:
                    await harness.close()

            asyncio.run(main())

    def assert_refused(self, h: PolicyHarness, seq: int, command: str) -> None:
        message_id = f"msg-{seq}"
        self.assertEqual(h.settlements(message_id), ["ack"], "settle the refusal exactly once")
        self.assertEqual(h.state(message_id), "acked")
        (refusal,) = [send for send in h.refusals() if send["reply_to_message_id"] == message_id]
        self.assertEqual(refusal["room_id"], ROOM_ID)
        self.assertEqual(refusal["conversation_id"], "home")
        self.assertEqual(refusal["segment_id"], "segment-1")
        self.assertEqual(refusal["kind"], "message")
        self.assertEqual(refusal["status"], "complete")
        self.assertTrue(refusal["text"].startswith(f"/{command} isn't available"))

    def test_idle_restricted_commands_are_refused_before_hermes(self):
        cases = [(text, command) for text, command in REFUSED_TEXT.items() if command]

        async def scenario(h: PolicyHarness):
            for seq, (text, command) in enumerate(cases, start=1):
                await h.deliver(raw_event(seq, text))
                with self.subTest(text=text):
                    self.assert_refused(h, seq, command)
            self.assertEqual(h.admitted, [])
            self.assertEqual(h.handled, [])
            self.assertEqual([action for action, _ in h.calls if action == "activity"], [])
            self.assertEqual(h.adapter._inflight_admissions, set())
            self.assertEqual(h.adapter._active_sessions, {})

        self.run_scenario(scenario)

    def test_busy_restricted_commands_are_refused_without_touching_the_turn(self):
        cases = [
            ("/update", "update"),
            ("/restart", "restart"),
            ("/codex_runtime auto", "codex-runtime"),
            ("/loop 30s /restart", "loop"),
            ("restart hermes", "restart"),
        ]

        async def scenario(h: PolicyHarness):
            await h.start_long_turn()
            activity_before = [action for action, _ in h.calls if action == "activity"]
            for seq, (text, command) in enumerate(cases, start=2):
                await h.deliver(raw_event(seq, text))
                with self.subTest(text=text):
                    self.assert_refused(h, seq, command)
            self.assertEqual(h.admitted, ["long running work"])
            self.assertEqual(h.handled, ["long running work"])
            self.assertEqual(h.runs, ["msg-1"])
            self.assertEqual(h.state("msg-1"), "leased", "the running turn keeps its lease")
            self.assertEqual(
                [action for action, _ in h.calls if action == "activity"], activity_before
            )
            self.assertEqual(h.adapter._deferred_admissions, {})

        self.run_scenario(scenario)

    def test_idle_ordinary_messages_and_allowed_commands_reach_hermes(self):
        texts = [
            "hello there",
            "see https://example.com/a/b?c=d",
            "see /usr/bin for the binary",
            "a/b",
            "/new",
            "/queue then summarize it",
            "/pause off",
            "/always",
            "/cancel",
            "/my-custom-skill do the thing",
        ]

        async def scenario(h: PolicyHarness):
            # Settle each entry before the next, so every one arrives idle and
            # /new does not discard the messages sent before it.
            for seq, text in enumerate(texts, start=1):
                await h.deliver(raw_event(seq, text))
                await h.wait_all_settled()
            await h.deliver(raw_photo(len(texts) + 1, "what is in this picture?"))
            await h.wait_all_settled()
            self.assertEqual(h.admitted, [*texts, "what is in this picture?"])
            self.assertEqual(h.refusals(), [])

        self.run_scenario(scenario)

    def test_busy_stop_still_takes_the_hardened_path(self):
        async def scenario(h: PolicyHarness):
            await h.start_long_turn()
            await h.deliver(raw_event(2, "/stop"))
            self.assertEqual(h.refusals(), [])
            self.assertIn("/stop", h.admitted)
            self.assertEqual(h.state("msg-1"), "acked")
            self.assertEqual(h.state("msg-2"), "acked")
            self.assertEqual(h.runs, ["msg-1"])

        self.run_scenario(scenario)

    def test_busy_new_still_takes_the_hardened_path(self):
        async def scenario(h: PolicyHarness):
            await h.start_long_turn()
            with patch.object(
                h.adapter,
                "_dispatch_active_session_command",
                wraps=h.adapter._dispatch_active_session_command,
            ) as dispatch:
                await h.deliver(raw_event(2, "/new"))
            self.assertEqual(h.refusals(), [])
            dispatch.assert_awaited_once()
            self.assertEqual(h.state("msg-1"), "acked")

        self.run_scenario(scenario)

    def test_busy_queue_and_pause_off_reach_hermes(self):
        async def scenario(h: PolicyHarness):
            await h.start_long_turn()
            await h.deliver(raw_event(2, "/queue then summarize it"))
            await h.deliver(raw_event(3, "/pause off"))
            self.assertEqual(h.refusals(), [])
            self.assertEqual(
                h.admitted,
                ["long running work", "/queue then summarize it", "/pause off"],
            )

        self.run_scenario(scenario)

    def test_failed_refusal_send_still_settles_once(self):
        async def scenario(h: PolicyHarness):
            sidecar = h._sidecar

            async def failing_send(action, payload, *, timeout):
                if action == "send":
                    h.calls.append((action, payload))
                    return h.module._FiniteChatResult(False, {}, "unavailable", True)
                return await sidecar(action, payload, timeout=timeout)

            h.adapter._finitechat_json = failing_send
            await h.deliver(raw_event(1, "/update"))
            self.assertEqual(h.settlements("msg-1"), ["ack"])
            self.assertEqual(h.admitted, [])

        self.run_scenario(scenario)


if __name__ == "__main__":
    unittest.main()

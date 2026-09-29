"""Restricted slash commands are refused by the adapter, against the pinned gateway.

Finite Chat hands every inbound event to Hermes through
``_handle_finitechat_event``. A restricted command (``/update``, ``/restart``,
``/loop`` and the rest of ``slash_policy.json``) must be answered there with an
ordinary reply on the event's own route, before Hermes sees it on either the
idle or the busy path. The inbox entry is acked only after the reply is sent;
anything else keeps it for redelivery, so a refusal can be seen twice. Owner ``quick_commands`` aliases resolve
before the check, and internally dispatched commands (a persisted ``/loop``
restored after a restart) meet the same policy. Everything else must reach
Hermes unchanged.

These tests drive the real pinned ``MessageEvent``, command registry, plaintext
coercion, quick-command expansion, loop watcher, and ``BasePlatformAdapter``
dispatch. Only the sidecar and, where noted, the turn handler are simulated.
"""

import os
import tempfile

# Importing gateway modules reads HERMES_HOME. Never let a run load ~/.hermes.
if not os.environ.get("HERMES_HOME"):
    os.environ["HERMES_HOME"] = tempfile.mkdtemp(prefix="finite-slash-hermes-home-")

import asyncio
import importlib.util
import json
import logging
import runpy
import shutil
import sys
import threading
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
from hermes_cli.loops import LoopManager, load_loop, save_loop

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
    "/loop resume": "loop",
    "/loop help": "loop",
    "/loop stop now": "loop",
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
    "/loop",
    "/loop stop",
    "/LOOP Stop",
    "/loop status",
    "/loop pause",
    "/loop clear",
    "/loop cancel",
    "/proactive stop",
)
QUICK_COMMANDS = {
    "again": {"type": "alias", "target": "/restart"},
    "diag": {"type": "alias", "target": "/debug"},
    "double": {"type": "alias", "target": "//restart"},
    "triple": {"type": "alias", "target": "///debug"},
    "repeat": {"type": "alias", "target": "/loop 30s /restart"},
    "up": {"type": "alias", "target": "update"},
    "chain": {"type": "alias", "target": "/again"},
    "stoploop": {"type": "alias", "target": "/loop stop"},
    "fine": {"type": "alias", "target": "/status"},
    "skill": {"type": "alias", "target": "/my-custom-skill"},
    "uptime": {"type": "exec", "command": "uptime"},
    "empty": {"type": "alias", "target": ""},
}


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
                if "reason" in entry:
                    self.assertEqual(entry["tier"], "restricted")
                    self.assertTrue(entry["reason"].strip())
                self.assertNotIn("\u2014", entry.get("reason", ""))

    def test_restricted_set_matches_the_catalog(self):
        restricted = {name for name, entry in self.policy.items() if entry["tier"] == "restricted"}
        self.assertEqual(restricted, RESTRICTED)
        for control in ("stop", "new", "queue", "pause", "approve", "deny", "steer"):
            self.assertNotEqual(self.policy[control]["tier"], "restricted", control)


class EvaluateTests(unittest.TestCase):
    def setUp(self):
        self.policy = load_policy_module()

    def evaluate(self, text: str, **kwargs: Any) -> Any:
        return self.policy.evaluate(message_event(text), quick_commands=kwargs.get("quick", {}))

    def test_restricted_commands_are_refused_by_canonical_name(self):
        for text, expected in REFUSED_TEXT.items():
            with self.subTest(text=text):
                refusal = self.evaluate(text)
                self.assertIsNotNone(refusal)
                self.assertEqual(refusal.command, expected)
                self.assertTrue(refusal.text.startswith(f"/{expected} isn't available"))
                self.assertNotIn("\u2014", refusal.text)

    def test_refusal_text_carries_the_catalog_reason(self):
        refusal = self.evaluate("/update")
        self.assertEqual(
            refusal.text,
            "/update isn't available in Finite chat. "
            "Finite manages your agent's software and restarts.",
        )
        generic = self.evaluate("/rollback")
        self.assertEqual(generic.text, "/rollback isn't available in Finite chat.")

    def test_evaluation_does_not_rewrite_the_event(self):
        event = message_event("restart hermes")
        self.assertIsNotNone(self.policy.evaluate(event, quick_commands={}))
        self.assertEqual(event.text, "restart hermes")
        alias = message_event("/again now")
        self.assertIsNotNone(self.policy.evaluate(alias, quick_commands=QUICK_COMMANDS))
        self.assertEqual(alias.text, "/again now")

    def test_everything_else_passes(self):
        for text in PASSED_TEXT:
            with self.subTest(text=text):
                self.assertIsNone(self.evaluate(text))

    def test_group_plaintext_restart_is_ordinary_conversation(self):
        event = message_event("restart hermes", chat_type="group")
        self.assertIsNone(self.policy.evaluate(event, quick_commands={}))

    def test_photo_caption_without_a_command_passes(self):
        event = message_event("what is in this picture?", message_type=MessageType.PHOTO)
        self.assertIsNone(self.policy.evaluate(event, quick_commands={}))

    def test_events_without_gateway_control_pass(self):
        event = message_event("/update", allow_gateway_control=False)
        self.assertIsNone(self.policy.evaluate(event, quick_commands={}))

    def test_quick_command_aliases_resolve_before_the_policy(self):
        refused = {
            "/again": "restart",
            "/diag local": "debug",
            "/double": "restart",
            "/triple local": "debug",
            "/repeat": "loop",
            "/up": "update",
            "/chain": "restart",
            "/AGAIN@finite_bot": "restart",
        }
        for text, expected in refused.items():
            with self.subTest(text=text):
                refusal = self.evaluate(text, quick=QUICK_COMMANDS)
                self.assertIsNotNone(refusal)
                self.assertEqual(refusal.command, expected)
        for text in ("/stoploop", "/fine", "/skill go", "/uptime", "/empty", "/unknown"):
            with self.subTest(text=text):
                self.assertIsNone(self.evaluate(text, quick=QUICK_COMMANDS))

    def test_registry_commands_are_never_shadowed_by_quick_commands(self):
        shadow = {"stop": {"type": "alias", "target": "/restart"}}
        self.assertIsNone(self.evaluate("/stop", quick=shadow))

    def test_alias_traversal_stops_where_the_gateway_stops(self):
        aliases = {
            "a": {"type": "alias", "target": "/b"},
            "b": {"type": "alias", "target": "/c"},
            "c": {"type": "alias", "target": "/restart"},
        }
        self.assertIsNone(self.evaluate("/a", quick=aliases))

    def test_quick_commands_come_from_the_live_gateway_runner(self):
        with tempfile.TemporaryDirectory() as home:
            runner = GatewayRunner(
                GatewayConfig(sessions_dir=Path(home) / "sessions", quick_commands=QUICK_COMMANDS)
            )
            try:
                refusal = self.policy.evaluate(message_event("/again"))
                self.assertIsNotNone(refusal)
                self.assertEqual(refusal.command, "restart")
            finally:
                runner.close_all_session_db_handles()
                runner.session_store.close_all_db_handles()
                runner._shutdown_executor()

    def test_untiered_gateway_command_fails_closed(self):
        with patch.dict(self.policy._policy(), {}, clear=True):
            refusal = self.evaluate("/status")
        self.assertIsNotNone(refusal)
        self.assertEqual(refusal.command, "status")

    def test_loop_creation_is_refused_even_if_retiered(self):
        with patch.dict(self.policy._policy(), {"loop": {"tier": "available"}}):
            self.assertIsNotNone(self.evaluate("/loop 5m /status"))
            self.assertIsNotNone(self.evaluate("/loop resume"))
            self.assertIsNone(self.evaluate("/loop stop"))

    def test_pause_resume_verbs_pass_even_if_pause_is_restricted(self):
        restricted = {"tier": "restricted", "reason": "x"}
        with patch.dict(self.policy._policy(), {"pause": restricted}):
            for verb in ("off", "resume", "stop", "disengage", "OFF", " Resume "):
                with self.subTest(verb=verb):
                    self.assertIsNone(self.evaluate(f"/pause {verb}"))
            self.assertIsNotNone(self.evaluate("/pause now"))
            self.assertIsNotNone(self.evaluate("/pause"))

    def test_missing_coercion_helper_degrades_and_logs_once(self):
        with (
            patch.object(hermes_base, "coerce_plaintext_gateway_command", None),
            self.assertLogs(self.policy.logger, logging.WARNING) as logs,
        ):
            self.assertIsNone(self.evaluate("restart hermes"))
            self.assertIsNone(self.evaluate("restart hermes"))
            self.assertIsNotNone(self.evaluate("/update"))
        self.assertEqual(len(logs.records), 1)

    def test_missing_registry_resolver_allows_and_logs_once(self):
        with (
            patch.object(hermes_commands, "resolve_command", None),
            self.assertLogs(self.policy.logger, logging.WARNING) as logs,
        ):
            self.assertIsNone(self.evaluate("/update"))
            self.assertIsNone(self.evaluate("/stop"))
        self.assertEqual(len(logs.records), 1)

    def assert_load_failure(self, content: str) -> None:
        with (
            tempfile.TemporaryDirectory() as scratch,
            self.assertLogs(self.policy.logger, logging.ERROR) as logs,
        ):
            path = Path(scratch) / "slash_policy.json"
            path.write_text(content, encoding="utf-8")
            with patch.object(self.policy, "POLICY_PATH", path):
                self.policy._policy.cache_clear()
                self.assertIsNone(self.policy._policy())
                self.assertIsNone(self.evaluate("/update"))
                self.assertIsNone(self.evaluate("/stop"))
                with self.assertRaises(RuntimeError):
                    self.policy.self_check()
            self.policy._policy.cache_clear()
        self.assertEqual(len(logs.records), 1)

    def test_unreadable_policy_is_a_load_failure(self):
        with (
            tempfile.TemporaryDirectory() as scratch,
            patch.object(self.policy, "POLICY_PATH", Path(scratch) / "missing.json"),
            self.assertLogs(self.policy.logger, logging.ERROR) as logs,
        ):
            self.policy._policy.cache_clear()
            self.assertIsNone(self.evaluate("/update"))
            self.assertIsNone(self.evaluate("/stop"))
        self.assertEqual(len(logs.records), 1)

    def test_empty_or_malformed_policy_is_a_load_failure(self):
        shipped = json.loads(POLICY_DATA_PATH.read_text(encoding="utf-8"))
        bad_tier = {**shipped, "status": {"tier": "sometimes"}}
        bad_reason = {**shipped, "update": {"tier": "restricted", "reason": 7}}
        extra_key = {**shipped, "status": {"tier": "suggested", "label": "x"}}
        for content in (
            "",
            "{}",
            "[]",
            "not json",
            json.dumps({"update": "restricted"}),
            json.dumps(bad_tier),
            json.dumps(bad_reason),
            json.dumps(extra_key),
        ):
            with self.subTest(content=content[:40]):
                self.assert_load_failure(content)

    def test_policy_that_restricts_an_escape_hatch_is_a_load_failure(self):
        shipped = json.loads(POLICY_DATA_PATH.read_text(encoding="utf-8"))
        for name in ("stop", "new", "approve", "deny"):
            with self.subTest(name=name, change="restricted"):
                restricted = {**shipped, name: {"tier": "restricted", "reason": "x"}}
                self.assert_load_failure(json.dumps(restricted))
            with self.subTest(name=name, change="missing"):
                missing = {key: value for key, value in shipped.items() if key != name}
                self.assert_load_failure(json.dumps(missing))

    def test_self_check_passes_for_the_shipped_policy(self):
        summary = self.policy.self_check()
        self.assertIn("68 commands", summary)
        self.assertIn(f"{len(RESTRICTED)} restricted", summary)


class RuntimeImageCheckTests(unittest.TestCase):
    """The runtime image runs this exact check (runtime-image.yml).

    ``runpy`` loads ``adapter.py`` outside its package, as the image check
    does; a missing or failing policy must fail the check, never pass it.
    """

    def run_image_check(self, plugin_dir: Path) -> Any:
        adapter = runpy.run_path(str(plugin_dir / "adapter.py"))
        policy = adapter["_SLASH_POLICY"]
        assert policy is not None, "slash policy did not load"
        return policy.self_check()

    def copy_plugin(self, scratch: str, *, skip: str) -> Path:
        plugin_dir = Path(scratch) / "finitechat"
        plugin_dir.mkdir()
        for name in ("adapter.py", "slash_policy.py", "slash_policy.json"):
            if name != skip:
                shutil.copy(PLUGIN_DIR / name, plugin_dir / name)
        return plugin_dir

    def test_shipped_plugin_passes_the_image_check(self):
        self.assertIn("68 commands", self.run_image_check(PLUGIN_DIR))

    def test_missing_policy_data_fails_the_image_check(self):
        with tempfile.TemporaryDirectory() as scratch:
            plugin_dir = self.copy_plugin(scratch, skip="slash_policy.json")
            with self.assertRaises(RuntimeError), self.assertLogs(level=logging.ERROR):
                self.run_image_check(plugin_dir)

    def test_missing_policy_module_fails_the_image_check(self):
        with tempfile.TemporaryDirectory() as scratch:
            plugin_dir = self.copy_plugin(scratch, skip="slash_policy.py")
            with self.assertRaises(AssertionError), self.assertLogs(level=logging.ERROR):
                self.run_image_check(plugin_dir)

    def test_image_workflow_runs_the_check(self):
        workflow = (REPO_ROOT.parent / ".github" / "workflows" / "runtime-image.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn('policy = adapter["_SLASH_POLICY"]', workflow)
        self.assertIn("policy.self_check()", workflow)


class PolicyHarness:
    """Real pinned gateway + adapter; simulated sidecar.

    With ``real_runner`` the Hermes ``GatewayRunner._handle_message`` pipeline
    handles admitted events (quick-command expansion, loop commands), with
    the restricted handlers replaced by sentinels. Otherwise a stand-in turn
    handler records what Hermes received.
    """

    def __init__(
        self,
        home: str,
        *,
        quick_commands: dict[str, Any] | None = None,
        real_runner: bool = False,
    ):
        self.module = load_module("finitechat_pinned_slash_adapter_under_test", ADAPTER_PATH)
        self.module.REFUSAL_RETRY_SECS = 0.01
        self.module.REFUSAL_MAX_RETRY_SECS = 0.04
        config = PlatformConfig(enabled=True, extra={"home": home, "room_id": ROOM_ID})
        config.typing_indicator = False
        self.adapter = self.module.FiniteChatAdapter(config)
        self.adapter._home_channel_hydrated = True
        self.runner = GatewayRunner(
            GatewayConfig(sessions_dir=Path(home) / "sessions", quick_commands=quick_commands or {})
        )
        self.runner._session_db = None
        self.runner._persist_active_agents = lambda: None
        self.runner.adapters[self.adapter.platform] = self.adapter
        self.module._finite_private_control_request = lambda *_args: None

        self.inbox: dict[str, tuple[dict[str, Any], str]] = {}
        self.calls: list[tuple[str, dict[str, Any]]] = []
        self.sent: list[dict[str, Any]] = []
        self.send_behaviors: list[str] = []
        self.ack_failures = 0
        self.send_started = asyncio.Event()
        self.send_gate = threading.Event()
        self.admitted: list[str] = []
        self.handled: list[str] = []
        self.sentinel_hits: list[tuple[str, str]] = []
        self.runs: list[str] = []
        self.started = asyncio.Event()
        self.adapter._finitechat_json = self._sidecar
        if real_runner:
            self._install_real_runner()
        else:
            self.adapter.set_message_handler(self._handle)
        admit = self.adapter.handle_message

        async def recording_handle_message(event):
            self.admitted.append(event.text)
            return await admit(event)

        self.adapter.handle_message = recording_handle_message

    def _install_real_runner(self) -> None:
        self.runner._is_user_authorized = lambda *_args, **_kwargs: True

        def sentinel(name):
            async def handler(event):
                self.sentinel_hits.append((name, event.text))
                return f"SENTINEL:{name}"

            return handler

        self.runner._handle_restart_command = sentinel("restart")
        self.runner._handle_debug_command = sentinel("debug")
        self.runner._handle_update_command = sentinel("update")
        self.runner._handle_loop_command = sentinel("loop")
        self.adapter.set_message_handler(self.runner._handle_message)
        self.adapter.set_session_store(self.runner.session_store)
        self.adapter.set_busy_session_handler(self.runner._handle_active_session_busy_message)

    async def _sidecar(self, action, payload, *, timeout):
        del timeout
        self.calls.append((action, payload))
        if action == "send":
            refusal = "isn't available in Finite" in payload["text"]
            behavior = self.send_behaviors.pop(0) if refusal and self.send_behaviors else "ok"
            if behavior in ("retryable", "permanent"):
                return self.module._FiniteChatResult(
                    False, {}, "rejected route", behavior == "retryable"
                )
            if behavior == "raise":
                raise RuntimeError("sidecar transport exploded")
            if behavior == "block":
                # Like the real transport: the send runs in a worker thread
                # that cancelling the awaiting task cannot stop.
                def send_in_worker():
                    self.send_gate.wait(5)
                    self.sent.append(payload)

                self.send_started.set()
                await asyncio.to_thread(send_in_worker)
            else:
                self.sent.append(payload)
            return self.module._FiniteChatResult(
                True, {"message_id": f"reply-{len(self.sent)}"}, None, False
            )
        if action == "ack" and self.ack_failures:
            self.ack_failures -= 1
            return self.module._FiniteChatResult(False, {}, "ack not committed", True)
        if action in ("ack", "release"):
            raw, state = self.inbox[payload["message_id"]]
            if state == "leased":
                self.inbox[payload["message_id"]] = (
                    raw,
                    "acked" if action == "ack" else "pending",
                )
        return self.module._FiniteChatResult(True, {}, None, False)

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

    async def deliver(self, raw: dict[str, Any], *, settle: bool = True) -> None:
        self.inbox[raw["message_id"]] = (raw, "leased")
        await self.adapter._handle_finitechat_event(raw)
        for _ in range(20):
            await asyncio.sleep(0)
        if settle:
            await self.settle_refusals()

    async def settle_refusals(self) -> None:
        await self.wait_for(lambda: not self.adapter._refusal_tasks)

    async def start_long_turn(self) -> None:
        await self.deliver(raw_event(1, "long running work"))
        await asyncio.wait_for(self.started.wait(), 2)

    def state(self, message_id: str) -> str:
        return self.inbox[message_id][1]

    async def wait_for(self, predicate, timeout: float = 5) -> None:
        async def poll():
            while not predicate():
                await asyncio.sleep(0.01)

        await asyncio.wait_for(poll(), timeout)

    async def wait_all_settled(self) -> None:
        await self.wait_for(
            lambda: not any(state == "leased" for _raw, state in self.inbox.values())
        )

    def settlements(self, message_id: str) -> list[str]:
        return [
            action
            for action, payload in self.calls
            if action in ("ack", "release") and payload.get("message_id") == message_id
        ]

    def refusals(self) -> list[dict[str, Any]]:
        return [send for send in self.sent if "isn't available in Finite" in send["text"]]

    def refusal_sends(self) -> int:
        return sum(
            1
            for action, payload in self.calls
            if action == "send" and "isn't available in Finite" in payload["text"]
        )

    async def close(self) -> None:
        await self.adapter._cancel_admission_tasks()
        await self.adapter.cancel_background_tasks()
        self.runner.close_all_session_db_handles()
        self.runner.session_store.close_all_db_handles()
        self.runner._shutdown_executor()


class AdapterTestCase(unittest.TestCase):
    def run_scenario(self, scenario, **harness_kwargs):
        with (
            tempfile.TemporaryDirectory(prefix="finite-slash-") as home,
            patch.dict(os.environ, {"HERMES_HOME": home}),
        ):
            Path(home, "config.yaml").write_text("{}\n", encoding="utf-8")

            async def main():
                harness = PolicyHarness(home, **harness_kwargs)
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
        self.assertEqual(refusal["attachments"], [])
        self.assertTrue(refusal["text"].startswith(f"/{command} isn't available"))


class AdapterRefusalTests(AdapterTestCase):
    def test_idle_restricted_commands_are_refused_before_hermes(self):
        cases = list(REFUSED_TEXT.items())

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
            ("/again", "restart"),
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

        self.run_scenario(scenario, quick_commands=QUICK_COMMANDS)

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
            "/loop stop",
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

    def test_busy_queue_pause_off_and_loop_stop_reach_hermes(self):
        async def scenario(h: PolicyHarness):
            await h.start_long_turn()
            await h.deliver(raw_event(2, "/queue then summarize it"))
            await h.deliver(raw_event(3, "/pause resume"))
            await h.deliver(raw_event(4, "/loop stop"))
            self.assertEqual(h.refusals(), [])
            self.assertEqual(
                h.admitted,
                ["long running work", "/queue then summarize it", "/pause resume", "/loop stop"],
            )

        self.run_scenario(scenario)


class AdapterQuickCommandTests(AdapterTestCase):
    def test_deep_alias_chain_preserves_the_gateway_unknown_command_reply(self):
        aliases = {
            "a": {"type": "alias", "target": "/b"},
            "b": {"type": "alias", "target": "/c"},
            "c": {"type": "alias", "target": "/restart"},
        }

        async def scenario(h: PolicyHarness):
            await h.deliver(raw_event(1, "/a"))
            await h.wait_for(lambda: h.state("msg-1") == "acked")
            self.assertEqual(h.sentinel_hits, [])
            self.assertEqual(h.refusals(), [])
            self.assertTrue(any("Unknown command `/c`" in sent["text"] for sent in h.sent))

        self.run_scenario(scenario, quick_commands=aliases, real_runner=True)

    def test_idle_aliases_to_restricted_commands_never_reach_their_handlers(self):
        cases = [
            ("/again", "restart"),
            ("/diag local", "debug"),
            ("/repeat", "loop"),
            ("/double", "restart"),
            ("/triple local", "debug"),
        ]

        async def scenario(h: PolicyHarness):
            for seq, (text, command) in enumerate(cases, start=1):
                await h.deliver(raw_event(seq, text))
                with self.subTest(text=text):
                    self.assert_refused(h, seq, command)
            self.assertEqual(h.admitted, [])
            self.assertEqual(h.sentinel_hits, [])

        self.run_scenario(scenario, quick_commands=QUICK_COMMANDS, real_runner=True)

    def test_alias_to_an_allowed_loop_verb_still_runs_through_hermes(self):
        async def scenario(h: PolicyHarness):
            await h.deliver(raw_event(1, "/stoploop"))
            await h.wait_for(lambda: h.sentinel_hits)
            self.assertEqual(h.sentinel_hits, [("loop", "/loop stop")])
            self.assertEqual(h.refusals(), [])

        self.run_scenario(scenario, quick_commands=QUICK_COMMANDS, real_runner=True)


class AdapterSettlementTests(AdapterTestCase):
    def test_retryable_send_failure_releases_after_backoff_then_redelivery_refuses(self):
        async def scenario(h: PolicyHarness):
            h.send_behaviors = ["retryable"]
            await h.deliver(raw_event(1, "/update"))
            self.assertEqual(h.state("msg-1"), "pending")
            self.assertEqual(h.settlements("msg-1"), ["release"])
            self.assertEqual(h.refusals(), [])

            await h.deliver(raw_event(1, "/update"))
            self.assertEqual(h.settlements("msg-1"), ["release", "ack"])
            self.assertEqual(h.state("msg-1"), "acked")
            self.assertEqual(len(h.refusals()), 1)
            self.assertEqual(h.admitted, [])
            self.assertEqual(h.adapter._refusal_retry_delays, {})

        self.run_scenario(scenario)

    def test_retry_backoff_doubles_and_is_bounded(self):
        async def scenario(h: PolicyHarness):
            key = h.module._adapter_event_key(ROOM_ID, 1, "msg-1")
            delays = []
            for _ in range(4):
                h.send_behaviors = ["retryable"]
                await h.deliver(raw_event(1, "/update"))
                delays.append(h.adapter._refusal_retry_delays[key])
                self.assertEqual(h.state("msg-1"), "pending")
            self.assertEqual(delays, [0.02, 0.04, 0.04, 0.04])
            self.assertEqual(h.settlements("msg-1"), ["release"] * 4)

        self.run_scenario(scenario)

    def test_send_exception_is_retried_like_a_retryable_failure(self):
        async def scenario(h: PolicyHarness):
            h.send_behaviors = ["raise"]
            await h.deliver(raw_event(1, "/update"))
            self.assertEqual(h.state("msg-1"), "pending")
            self.assertEqual(h.settlements("msg-1"), ["release"])
            self.assertEqual(h.admitted, [])

        self.run_scenario(scenario)

    def test_non_retryable_send_failure_is_never_acked(self):
        async def scenario(h: PolicyHarness):
            h.send_behaviors = ["permanent"]
            with self.assertLogs(h.module.logger, logging.WARNING) as logs:
                await h.deliver(raw_event(1, "/update"))
            self.assertEqual(h.settlements("msg-1"), ["release"])
            self.assertEqual(h.state("msg-1"), "pending")
            self.assertEqual(h.refusals(), [])
            self.assertTrue(any("rejected route" in line for line in logs.output))

            await h.deliver(raw_event(1, "/update"))
            self.assertEqual(h.settlements("msg-1"), ["release", "ack"])
            self.assertEqual(len(h.refusals()), 1)
            self.assertEqual(h.admitted, [])

        self.run_scenario(scenario)

    def test_failed_ack_after_a_sent_refusal_redelivers_at_least_once(self):
        async def scenario(h: PolicyHarness):
            h.ack_failures = 1
            await h.deliver(raw_event(1, "/update"))
            self.assertEqual(h.state("msg-1"), "leased")
            self.assertEqual(len(h.refusals()), 1)
            self.assertEqual(h.adapter._refusal_owners, {})

            # Lease expiry redelivers the entry: the refusal is sent again.
            await h.deliver(raw_event(1, "/update"))
            self.assertEqual(h.state("msg-1"), "acked")
            self.assertEqual(len(h.refusals()), 2)
            self.assertEqual(h.settlements("msg-1"), ["ack", "ack"])
            self.assertEqual(h.admitted, [])

        self.run_scenario(scenario)

    def test_redelivery_while_the_reply_is_sending_joins_its_owner(self):
        async def scenario(h: PolicyHarness):
            h.send_behaviors = ["block"]
            await h.deliver(raw_event(1, "/update"), settle=False)
            await asyncio.wait_for(h.send_started.wait(), 1)
            await h.deliver(raw_event(1, "/update"), settle=False)
            await h.deliver(raw_event(1, "/UPDATE"), settle=False)
            self.assertEqual(h.refusal_sends(), 1, "one entry never has two replies in flight")

            h.send_gate.set()
            await h.settle_refusals()
            self.assertEqual(len(h.refusals()), 1)
            self.assertEqual(h.settlements("msg-1"), ["ack"])
            self.assertEqual(h.state("msg-1"), "acked")

        self.run_scenario(scenario)

    def test_redelivery_during_the_hand_back_starts_a_new_attempt(self):
        async def scenario(h: PolicyHarness):
            release_started = asyncio.Event()
            release_blocked = asyncio.Event()
            original_release = h.adapter._release_finitechat_event

            async def stalled_release(*args):
                release_started.set()
                await release_blocked.wait()
                await original_release(*args)

            h.adapter._release_finitechat_event = stalled_release
            h.send_behaviors = ["retryable"]
            await h.deliver(raw_event(1, "/update"), settle=False)
            await asyncio.wait_for(release_started.wait(), 1)
            # The hand-back is in flight; a redelivery must not join it.
            await h.deliver(raw_event(1, "/update"), settle=False)
            await h.wait_for(lambda: h.settlements("msg-1") == ["ack"])
            self.assertEqual(len(h.refusals()), 1)

            release_blocked.set()
            await h.settle_refusals()
            self.assertEqual(h.settlements("msg-1"), ["ack", "release"])
            self.assertEqual(h.state("msg-1"), "acked", "a late release never undoes the ack")
            self.assertEqual(h.adapter._refusal_owners, {})

        self.run_scenario(scenario)

    def test_shutdown_releases_a_refusal_waiting_to_retry(self):
        async def scenario(h: PolicyHarness):
            h.module.REFUSAL_RETRY_SECS = 60
            h.send_behaviors = ["retryable"]
            await h.deliver(raw_event(1, "/update"), settle=False)
            self.assertEqual(h.settlements("msg-1"), [])
            await h.adapter._cancel_admission_tasks()
            self.assertEqual(h.settlements("msg-1"), ["release"])
            self.assertEqual(h.state("msg-1"), "pending")
            self.assertEqual(h.adapter._refusal_tasks, {})

        self.run_scenario(scenario)

    def test_shutdown_joins_a_refusal_release_already_in_progress(self):
        async def scenario(h: PolicyHarness):
            release_started = asyncio.Event()
            release_blocked = asyncio.Event()
            original_release = h.adapter._release_finitechat_event
            calls = 0

            async def stalled_release(*args):
                nonlocal calls
                calls += 1
                if calls == 1:
                    release_started.set()
                    await release_blocked.wait()
                await original_release(*args)

            h.adapter._release_finitechat_event = stalled_release
            h.send_behaviors = ["retryable"]
            await h.deliver(raw_event(1, "/update"), settle=False)
            (release_task,) = h.adapter._refusal_tasks
            await asyncio.wait_for(release_started.wait(), timeout=1)
            await h.adapter._cancel_admission_tasks()
            self.assertTrue(release_task.done(), "shutdown must join in-progress release tasks")
            self.assertEqual(h.state("msg-1"), "pending")
            self.assertEqual(h.settlements("msg-1"), ["release"])
            self.assertEqual(h.adapter._refusal_tasks, {})

        self.run_scenario(scenario)

    def test_shutdown_before_the_send_starts_releases_the_entry(self):
        async def scenario(h: PolicyHarness):
            raw = raw_event(1, "/update")
            h.inbox[raw["message_id"]] = (raw, "leased")
            await h.adapter._handle_finitechat_event(raw)
            await h.adapter._cancel_admission_tasks()
            self.assertEqual(h.refusal_sends(), 0)
            self.assertEqual(h.settlements("msg-1"), ["release"])
            self.assertEqual(h.state("msg-1"), "pending")

        self.run_scenario(scenario)

    def test_disconnect_keeps_the_lease_of_a_send_still_running_in_its_worker(self):
        async def scenario(h: PolicyHarness):
            h.send_behaviors = ["block"]
            await h.deliver(raw_event(1, "/update"), settle=False)
            await asyncio.wait_for(h.send_started.wait(), 1)
            (task,) = h.adapter._refusal_tasks
            with self.assertLogs(h.module.logger, logging.INFO) as logs:
                await h.adapter.disconnect()
            self.assertTrue(task.done(), "disconnect joins the refusal task")
            self.assertEqual(h.adapter._refusal_tasks, {})
            self.assertEqual(h.adapter._refusal_owners, {})
            self.assertEqual(h.settlements("msg-1"), [])
            self.assertEqual(h.state("msg-1"), "leased")
            self.assertTrue(any("keeping its lease" in line for line in logs.output))

            # The worker still completes the send after disconnect. Nothing
            # handed the entry back while it ran, and nothing acks it now: the
            # lease expiry redelivers it, and the refusal can be seen twice.
            h.send_gate.set()
            await h.wait_for(lambda: h.refusals())
            for _ in range(20):
                await asyncio.sleep(0)
            self.assertEqual(h.settlements("msg-1"), [])
            self.assertEqual(h.state("msg-1"), "leased")

        self.run_scenario(scenario)

    def test_refusal_sends_are_bounded_and_ordinary_work_keeps_moving(self):
        async def scenario(h: PolicyHarness):
            h.send_behaviors = ["block"] * 3
            for seq in (1, 2, 3):
                await h.deliver(raw_event(seq, "/update"), settle=False)
            await h.wait_for(lambda: h.refusal_sends() == 2)
            for _ in range(20):
                await asyncio.sleep(0)
            self.assertEqual(h.refusal_sends(), h.module.REFUSAL_MAX_ACTIVE_SENDS)

            await h.deliver(raw_event(4, "hello there"), settle=False)
            await h.wait_for(lambda: h.state("msg-4") == "acked")
            await h.deliver(raw_event(5, "/status"), settle=False)
            await h.wait_for(lambda: h.state("msg-5") == "acked")
            self.assertEqual(h.handled, ["hello there", "/status"])
            self.assertEqual(h.refusal_sends(), 2)

            await h.adapter.disconnect()
            # Active sends may still finish in their workers and keep their
            # leases; the one still waiting for a slot was never sent.
            self.assertEqual(
                [h.state(f"msg-{seq}") for seq in (1, 2, 3)], ["leased"] * 2 + ["pending"]
            )
            self.assertEqual(
                [h.settlements(f"msg-{seq}") for seq in (1, 2, 3)], [[], [], ["release"]]
            )
            h.send_gate.set()
            await h.wait_for(lambda: len(h.refusals()) == 2)
            self.assertEqual(h.refusal_sends(), 2)

        self.run_scenario(scenario)

    def test_refusal_never_carries_a_pending_brain_approval(self):
        async def scenario(h: PolicyHarness):
            broker = h.module._BrainApprovalFilings()
            h.adapter._brain_approval_filings = broker
            broker.after_tool_call(
                tool_name="terminal",
                result=json.dumps(
                    {"output": "finite-brain-approval-filed brain=brain-1 request=approval-1"}
                ),
            )
            await h.deliver(raw_event(1, "/update"))
            (refusal,) = h.refusals()
            self.assertNotIn("approve", refusal["metadata"])
            self.assertEqual([f["requestId"] for f in broker.take_pending()], ["approval-1"])

            # The filing still rides the next ordinary final reply, on another chat.
            raw = raw_event(2, "hello there")
            raw["segment_id"] = "segment-2"
            raw["source"]["thread_id"] = "segment-2"
            await h.deliver(raw)
            await h.wait_for(lambda: h.state("msg-2") == "acked")
            (reply,) = [sent for sent in h.sent if sent["text"] == "done"]
            self.assertEqual(reply["thread_id"], "segment-2")
            self.assertEqual(
                [r["requestId"] for r in reply["metadata"]["approve"]["requests"]], ["approval-1"]
            )
            self.assertEqual(broker.take_pending(), [])

        self.run_scenario(scenario)


class AdapterRouteTests(AdapterTestCase):
    def test_refusal_without_a_segment_stays_on_the_topic_route(self):
        async def scenario(h: PolicyHarness):
            raw = raw_event(1, "/help")
            raw["conversation_id"] = "topic-9"
            del raw["segment_id"]
            raw["source"]["thread_id"] = "topic-9"
            await h.deliver(raw)
            (refusal,) = h.refusals()
            self.assertEqual(refusal["conversation_id"], "topic-9")
            self.assertIsNone(refusal["segment_id"])
            self.assertIsNone(refusal["thread_id"])
            self.assertEqual(refusal["reply_to_message_id"], "msg-1")
            self.assertEqual(h.settlements("msg-1"), ["ack"])

        self.run_scenario(scenario)

    def test_refusal_without_a_route_uses_the_source_thread(self):
        async def scenario(h: PolicyHarness):
            raw = raw_event(1, "/help")
            del raw["conversation_id"]
            del raw["segment_id"]
            raw["source"]["thread_id"] = "legacy-thread"
            await h.deliver(raw)
            (refusal,) = h.refusals()
            self.assertIsNone(refusal["conversation_id"])
            self.assertIsNone(refusal["segment_id"])
            self.assertEqual(refusal["thread_id"], "legacy-thread")
            self.assertEqual(h.settlements("msg-1"), ["ack"])

        self.run_scenario(scenario)


FAKE_FINITECHAT_CLI = """
import json
import pathlib
import sys

state = pathlib.Path(sys.argv[1])
action = sys.argv[-2]
request = json.loads(sys.stdin.read() or "{}")
with (state / "calls.jsonl").open("a", encoding="utf-8") as log:
    log.write(json.dumps({"action": action, "request": request}) + "\\n")
if action == "poll":
    print((state / "poll.json").read_text(encoding="utf-8"))
elif action == "send":
    print(json.dumps({"message_id": "reply-1"}))
else:
    print("{}")
"""


class NonStreamTransportTests(AdapterTestCase):
    """The poll loop and the ``finitechat`` CLI, without a resident service."""

    def test_poll_refuses_through_the_cli_and_admits_the_ordinary_message(self):
        async def scenario(h: PolicyHarness):
            state = Path(h.adapter.home) / "fake-cli"
            state.mkdir()
            (state / "cli.py").write_text(FAKE_FINITECHAT_CLI, encoding="utf-8")
            (state / "poll.json").write_text(
                json.dumps({"events": [raw_event(1, "/update"), raw_photo(2, "hello there")]}),
                encoding="utf-8",
            )
            del h.adapter._finitechat_json
            h.adapter._finitechat_cmd = [sys.executable, str(state / "cli.py"), str(state)]
            self.assertFalse(h.adapter.inbound_stream)
            self.assertEqual(h.adapter.service_url, "")

            def calls() -> list[dict[str, Any]]:
                path = state / "calls.jsonl"
                if not path.exists():
                    return []
                return [json.loads(line) for line in path.read_text().splitlines()]

            def acked() -> set[str]:
                return {c["request"]["message_id"] for c in calls() if c["action"] == "ack"}

            self.assertTrue(await h.adapter._poll_once())
            await h.wait_for(lambda: acked() == {"msg-1", "msg-2"}, timeout=20)
            await h.settle_refusals()

            self.assertEqual(h.handled, ["hello there"])
            sends = [c["request"] for c in calls() if c["action"] == "send"]
            (refusal,) = [send for send in sends if send["reply_to_message_id"] == "msg-1"]
            self.assertTrue(refusal["text"].startswith("/update isn't available"))
            self.assertEqual(
                (refusal["room_id"], refusal["conversation_id"], refusal["segment_id"]),
                (ROOM_ID, "home", "segment-1"),
            )
            self.assertEqual((refusal["kind"], refusal["status"]), ("message", "complete"))
            settlements = [
                (c["action"], c["request"]["message_id"])
                for c in calls()
                if c["action"] in ("ack", "release")
            ]
            self.assertEqual(sorted(settlements), [("ack", "msg-1"), ("ack", "msg-2")])
            order = [(c["action"], c["request"].get("message_id")) for c in calls()]
            refusal_index = next(
                i
                for i, c in enumerate(calls())
                if c["action"] == "send" and c["request"]["reply_to_message_id"] == "msg-1"
            )
            ack_index = order.index(("ack", "msg-1"))
            self.assertLess(refusal_index, ack_index, "ack only after the reply was sent")

        self.run_scenario(scenario)


class RestoredLoopTests(unittest.TestCase):
    """A loop persisted before the upgrade re-fires through ``handle_message``."""

    def test_restored_loop_ticks_are_refused_with_one_reply(self):
        with (
            tempfile.TemporaryDirectory(prefix="finite-slash-loop-") as home,
            patch.dict(os.environ, {"HERMES_HOME": home}),
        ):
            Path(home, "config.yaml").write_text("{}\n", encoding="utf-8")
            module = load_module("finitechat_pinned_slash_adapter_under_test", ADAPTER_PATH)
            platform = module._finite_platform()
            source = SessionSource(
                platform=platform,
                chat_id=ROOM_ID,
                chat_type="dm",
                user_id="alice",
                thread_id="segment-1",
            )
            session_id = self.seed_loop(home, source)
            asyncio.run(self.tick_twice(home, session_id))

    @staticmethod
    def seed_loop(home: str, source: SessionSource) -> str:
        runner = GatewayRunner(GatewayConfig(sessions_dir=Path(home) / "sessions"))
        try:
            session_id = runner.session_store.get_or_create_session(source).session_id
            LoopManager(session_id=session_id).set(
                "/debug",
                interval_seconds=30,
                route={
                    "platform": source.platform.value,
                    "chat_id": source.chat_id,
                    "chat_type": source.chat_type,
                    "thread_id": source.thread_id or "",
                    "user_id": source.user_id or "",
                },
            )
        finally:
            runner.close_all_session_db_handles()
            runner.session_store.close_all_db_handles()
            runner._shutdown_executor()
        return session_id

    async def tick_twice(self, home: str, session_id: str) -> None:
        real_sleep = asyncio.sleep

        async def fast_sleep(delay, *args, **kwargs):
            return await real_sleep(min(delay, 0.01), *args, **kwargs)

        h = PolicyHarness(home, real_runner=True)
        h.runner._running = True
        with (
            patch("asyncio.sleep", fast_sleep),
            self.assertLogs(h.module.logger, logging.INFO) as logs,
        ):
            watcher = asyncio.create_task(h.runner._loop_wakeup_watcher(interval=0.01))
            try:

                def ticks() -> int:
                    state = load_loop(session_id)
                    return state.ticks_fired if state else 0

                await h.wait_for(lambda: ticks() >= 1 and h.sent)
                state = load_loop(session_id)
                assert state is not None
                state.next_due_at = 0
                save_loop(session_id, state)
                await h.wait_for(lambda: ticks() >= 2)
            finally:
                h.runner._running = False
                watcher.cancel()
                await asyncio.gather(watcher, return_exceptions=True)
                await h.close()

        self.assertEqual(h.sentinel_hits, [])
        (reply,) = h.sent
        self.assertTrue(reply["text"].startswith("A scheduled /debug was skipped."), reply)
        self.assertIn("/loop stop", reply["text"])
        self.assertEqual(reply["thread_id"], "segment-1")
        refused = [line for line in logs.output if "refused scheduled /debug" in line]
        self.assertGreaterEqual(len(refused), 2)
        restored = load_loop(session_id)
        self.assertIsNotNone(restored, "refusing a tick never purges the owner's loop")


if __name__ == "__main__":
    unittest.main()

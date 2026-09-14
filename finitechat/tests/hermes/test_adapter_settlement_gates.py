"""Adapter settlement mappings for the explicit Hermes admission contract.

These small mapping tests complement test_pinned_hermes_delivery_contract.py,
which exercises real pinned-Hermes admission, active controls, deferred input,
cancellation, terminal response delivery, and historical replay behavior.
Rust lease persistence/restart semantics remain covered by hermes_flow.rs and
hermes_settlement_gates.rs: this adapter adds no durable queue or ack journal.
"""

from __future__ import annotations

import asyncio
import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[2]
HARNESS_PATH = REPO_ROOT / "tests" / "hermes" / "test_finite_platform_adapter.py"
GATEWAY_MODULE_NAMES = (
    "gateway",
    "gateway.config",
    "gateway.platforms",
    "gateway.platforms.base",
    "gateway.session_context",
)

# Reuse the canonical adapter test harness (fake gateway classes, the
# per-test agent home, the recorded `_finitechat_json` stubs) by path-loading
# it exactly like `tests/container/test_adapter_regression_report.py`
# path-loads its script under test. Executing the module only defines its
# fixtures; all stateful work happens inside its TestCase methods.
_HARNESS_SPEC = importlib.util.spec_from_file_location(
    "finite_platform_adapter_test_harness", HARNESS_PATH
)
if _HARNESS_SPEC is None or _HARNESS_SPEC.loader is None:
    raise RuntimeError(f"failed to load harness from {HARNESS_PATH}")
harness: Any = importlib.util.module_from_spec(_HARNESS_SPEC)
sys.modules["finite_platform_adapter_test_harness"] = harness
_HARNESS_SPEC.loader.exec_module(harness)


class AdapterSettlementGateTests(unittest.TestCase):
    """Adapter-side settle mappings against the Rust sidecar's inbox."""

    def setUp(self):
        self.original_gateway_modules = {
            name: sys.modules.get(name) for name in GATEWAY_MODULE_NAMES
        }
        self.module = harness.load_adapter_module()
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
        extra = {
            "home": self.state_home,
            "finitechat_bin": "/bin/echo",
            "room_id": "room-agent-1",
        }
        home_channel = harness.HomeChannel(
            platform=harness.Platform.FINITECHAT,
            chat_id="room-agent-1",
            name="Finite Chat",
        )
        return self.module.FiniteChatAdapter(
            harness.PlatformConfig(extra=extra, home_channel=home_channel)
        )

    def record_json(self, calls: list[tuple[str, dict[str, Any], int]]):
        async def fake_json(action, payload, *, timeout):
            calls.append((action, payload, timeout))
            return self.module._FiniteChatResult(True, {}, None, False)

        return fake_json

    @staticmethod
    def text_event(seq: int, message_id: str, text: str) -> dict[str, Any]:
        return {
            "room_id": "room-agent-1",
            "seq": seq,
            "message_id": message_id,
            "conversation_id": "topic-build",
            "segment_id": "chat-build-1",
            "text": text,
            "message_type": "text",
            "source": {
                "platform": "finitechat",
                "chat_id": "room-agent-1",
                "chat_type": "dm",
                "user_id": "alice",
            },
        }

    def test_cancelled_turn_releases_the_sidecar_lease_for_redelivery(self):
        adapter = self.adapter()
        calls: list[tuple[str, dict[str, Any], int]] = []
        adapter._finitechat_json = self.record_json(calls)
        handled: list[Any] = []

        async def handle(event):
            handled.append(event)
            # The cancelled turn ran far enough to fire its completion hook;
            # the hook must release the lease so the sidecar redelivers whole.
            await adapter.on_processing_complete(event, harness.ProcessingOutcome.CANCELLED)

        adapter.handle_message = handle

        asyncio.run(adapter._handle_finitechat_event(self.text_event(21, "msg-21", "cancel me")))

        self.assertEqual([event.text for event in handled], ["cancel me"])
        self.assertEqual([call[0] for call in calls], ["activity", "release"])
        release_payload = calls[-1][1]
        self.assertEqual(release_payload["room_id"], "room-agent-1")
        self.assertEqual(release_payload["seq"], 21)
        self.assertEqual(release_payload["message_id"], "msg-21")
        self.assertEqual([], [call for call in calls if call[0] == "ack"])

    def test_failed_turn_acks_completed_turn_so_redelivery_replays_nothing(self):
        adapter = self.adapter()
        calls: list[tuple[str, dict[str, Any], int]] = []
        adapter._finitechat_json = self.record_json(calls)
        handled: list[Any] = []

        async def handle(event):
            handled.append(event)
            # A failed turn still ran to completion and answered the user, so
            # its settle outcome is an ack — never a redelivering release.
            await adapter.on_processing_complete(event, harness.ProcessingOutcome.FAILURE)

        adapter.handle_message = handle

        asyncio.run(adapter._handle_finitechat_event(self.text_event(21, "msg-21", "boom")))

        self.assertEqual([event.text for event in handled], ["boom"])
        self.assertEqual([call[0] for call in calls], ["activity", "ack"])
        ack_payload = calls[-1][1]
        self.assertEqual(ack_payload["room_id"], "room-agent-1")
        self.assertEqual(ack_payload["seq"], 21)
        self.assertEqual(ack_payload["message_id"], "msg-21")
        self.assertEqual([], [call for call in calls if call[0] == "release"])

    def test_precompletion_failure_releases_the_event_whole_without_acking(self):
        adapter = self.adapter()
        calls: list[tuple[str, dict[str, Any], int]] = []
        adapter._finitechat_json = self.record_json(calls)

        async def handle(_event):
            # Fails before any completion hook fires: the entry was never
            # consumed, so it must go back to the inbox unacked.
            raise RuntimeError("handler exploded mid-turn")

        adapter.handle_message = handle

        with self.assertRaises(RuntimeError):
            asyncio.run(adapter._handle_finitechat_event(self.text_event(21, "msg-21", "boom")))

        releases = [call for call in calls if call[0] == "release"]
        acks = [call for call in calls if call[0] == "ack"]
        self.assertEqual(len(releases), 1, "the unconsumed event is released once")
        self.assertEqual(releases[0][1]["message_id"], "msg-21")
        self.assertEqual(releases[0][1]["seq"], 21)
        self.assertEqual([], acks, "no ack may precede a terminal outcome")

    def test_in_flight_turn_owns_the_event_until_the_completion_hook_settles_once(self):
        adapter = self.adapter()
        calls: list[tuple[str, dict[str, Any], int]] = []
        adapter._finitechat_json = self.record_json(calls)
        handled: list[Any] = []

        async def handle(event):
            # A still-running turn: dispatched, no completion hook yet. The
            # in-flight marker records that the turn owns the event; nothing
            # may settle while it runs (dispatch alone never acked, and the
            # sidecar lease means a redelivery cannot even arrive mid-turn —
            # the deleted gate's "no second dispatch" is now structural).
            handled.append(event)

        adapter.handle_message = handle
        raw_event = self.text_event(21, "msg-21", "still running")

        asyncio.run(adapter._handle_finitechat_event(raw_event))

        self.assertEqual([], [call for call in calls if call[0] in ("ack", "release")])

        # The completion hook fires exactly once per turn, settles the lease
        # exactly once, without consulting adapter state, so the inline-admission path
        # can never double-ack behind it.
        asyncio.run(adapter.on_processing_complete(handled[0], harness.ProcessingOutcome.SUCCESS))

        acks = [call for call in calls if call[0] == "ack"]
        self.assertEqual(len(acks), 1, "the completion hook settles the event exactly once")
        self.assertEqual([], [call for call in calls if call[0] == "release"])
        self.assertEqual(acks[0][1]["message_id"], "msg-21")


if __name__ == "__main__":
    unittest.main()

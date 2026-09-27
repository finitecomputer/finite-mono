"""A user /stop is final for Finite Chat delivery, against the pinned gateway.

Canary on 2026-09-27 (runtime 2026-09-27.1): /stop acknowledged, then the
stopped message ran again and a follow-up sent before the stop ran too. Two
adapter gaps caused it, both invisible to the fake-gateway harness:

- Hermes reports a user-cancelled turn with the same CANCELLED outcome as
  shutdown, and the adapter released every cancelled lease, so the sidecar
  redelivered the stopped message as a fresh run under a new generation;
- the Finite admission head (and the later events cycling through the
  durable inbox behind it) live outside the Hermes queue that /stop clears.

These tests drive the real pinned ``BasePlatformAdapter`` dispatch and
``GatewayRunner._busy_stop_command``. Only inference, reply transport, and the
sidecar are simulated; the sidecar model is the documented lease contract
(ack settles, release returns the entry to pending for the next tick).
"""

import asyncio
import importlib.util
import os
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any
from unittest.mock import patch

from gateway.config import GatewayConfig, PlatformConfig
from gateway.platforms.base import SendResult
from gateway.run import GatewayRunner

REPO_ROOT = Path(__file__).resolve().parents[2]
ADAPTER_PATH = REPO_ROOT / "integrations" / "hermes" / "finitechat" / "adapter.py"
ROOM_ID = "room-agent-1"


def load_adapter_module():
    module_name = "finitechat_pinned_stop_adapter_under_test"
    sys.modules.pop(module_name, None)
    spec = importlib.util.spec_from_file_location(module_name, ADAPTER_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load adapter from {ADAPTER_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


def raw_event(seq: int, text: str) -> dict[str, Any]:
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
            "chat_type": "group",
            "user_id": "alice",
            "thread_id": "segment-1",
            "is_bot": False,
        },
        "attachments": [],
        "internal": False,
    }


def raw_photo(seq: int) -> dict[str, Any]:
    raw = raw_event(seq, "what is in this picture?")
    raw["attachments"] = [{"path": f"/synthetic/photo-{seq}.png", "mime_type": "image/png"}]
    return raw


class StopHarness:
    """Real pinned gateway + adapter; simulated model, transport, and sidecar."""

    def __init__(self, home: str):
        self.module = load_adapter_module()
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
        self.settled: list[tuple[str, str]] = []
        self.runs: list[str] = []
        self.replies: list[str] = []
        self.started = asyncio.Event()
        self.adapter._finitechat_json = self._sidecar
        self.adapter.send = self._send
        self.adapter.set_message_handler(self._handle)

    async def _sidecar(self, action, payload, *, timeout):
        del timeout
        if action in ("ack", "release"):
            message_id = payload["message_id"]
            self.settled.append((action, message_id))
            raw, state = self.inbox[message_id]
            if state == "leased":
                self.inbox[message_id] = (raw, "acked" if action == "ack" else "pending")
        return self.module._FiniteChatResult(True, {}, None, False)

    async def _send(self, chat_id, content, **_kwargs):
        del chat_id
        self.replies.append(content)
        return SendResult(success=True, message_id="reply")

    async def _handle(self, event):
        session_key = self.runner._session_key_for_source(event.source)
        if (event.text or "").startswith("/stop"):
            return await self.runner._busy_stop_command(event, session_key, event.source)
        self.runs.append(event.message_id)
        self.runner._begin_session_run_generation(session_key)

        class Agent:
            def hard_interrupt(self, message=None):
                del message

        self.runner._session_state(session_key).turn.agent = Agent()
        if event.message_id == "msg-1":
            self.started.set()
            await asyncio.Event().wait()
        return "done"

    async def deliver(self, raw: dict[str, Any]) -> None:
        self.inbox[raw["message_id"]] = (raw, "leased")
        await self.adapter._handle_finitechat_event(raw)
        await self.settle_loop()

    async def tick(self) -> None:
        """One inbound stream tick: lease and deliver every pending entry once."""
        pending = [raw for raw, state in self.inbox.values() if state == "pending"]
        for raw in pending:
            await self.deliver(raw)

    @staticmethod
    async def settle_loop() -> None:
        for _ in range(20):
            await asyncio.sleep(0)

    def state(self, message_id: str) -> str:
        return self.inbox[message_id][1]

    async def wait_settled(self, message_id: str) -> None:
        async def settled():
            while self.state(message_id) == "leased":
                await asyncio.sleep(0.01)

        await asyncio.wait_for(settled(), 5)

    async def close(self) -> None:
        await self.adapter._cancel_admission_tasks()
        await self.adapter.cancel_background_tasks()
        self.runner.close_all_session_db_handles()
        self.runner.session_store.close_all_db_handles()
        self.runner._shutdown_executor()


class PinnedHermesStopSettlementTests(unittest.TestCase):
    def run_scenario(self, scenario):
        with (
            tempfile.TemporaryDirectory(prefix="finite-stop-") as home,
            patch.dict(os.environ, {"HERMES_HOME": home}),
        ):

            async def main():
                harness = StopHarness(home)
                try:
                    await scenario(harness)
                finally:
                    await harness.close()

            asyncio.run(main())

    def test_stop_acks_the_cancelled_turn_instead_of_redelivering_it(self):
        async def scenario(h: StopHarness):
            await h.deliver(raw_event(1, "long running work"))
            await asyncio.wait_for(h.started.wait(), 2)
            await h.deliver(raw_event(2, "/stop"))
            await h.tick()

            self.assertEqual(h.runs, ["msg-1"], "the stopped message must not run again")
            self.assertEqual(h.state("msg-1"), "acked")
            self.assertNotIn(("release", "msg-1"), h.settled)
            self.assertEqual(h.state("msg-2"), "acked")

        self.run_scenario(scenario)

    def test_stop_discards_messages_sent_before_it_and_admits_later_ones(self):
        async def scenario(h: StopHarness):
            await h.deliver(raw_event(1, "long running work"))
            await asyncio.wait_for(h.started.wait(), 2)
            # msg-2 becomes the held admission head; msg-3 is released back to
            # the durable inbox behind it.
            await h.deliver(raw_event(2, "queued follow-up"))
            await h.deliver(raw_event(3, "second follow-up"))
            self.assertEqual(h.state("msg-3"), "pending")

            await h.deliver(raw_event(4, "/stop"))
            await h.tick()
            await h.deliver(raw_event(5, "after the stop"))
            await h.wait_settled("msg-5")

            self.assertEqual(h.runs, ["msg-1", "msg-5"])
            for message_id in ("msg-1", "msg-2", "msg-3", "msg-4", "msg-5"):
                self.assertEqual(h.state(message_id), "acked", message_id)
            self.assertEqual(h.adapter._deferred_admissions, {})

        self.run_scenario(scenario)

    def test_stop_acks_a_photo_queued_in_hermes_behind_the_running_turn(self):
        async def scenario(h: StopHarness):
            await h.deliver(raw_event(1, "long running work"))
            await asyncio.wait_for(h.started.wait(), 2)
            # Non-text bypasses the Finite admission head and waits in the
            # Hermes pending slot, which the gateway stop handler discards.
            await h.deliver(raw_photo(2))
            self.assertEqual(h.state("msg-2"), "leased")

            await h.deliver(raw_event(3, "/stop"))

            # Settled durably by the stop itself, not by the in-memory
            # boundary on a later redelivery, so a gateway restart inside the
            # lease window cannot resurrect it.
            self.assertEqual(h.state("msg-2"), "acked")
            self.assertNotIn(("release", "msg-2"), h.settled)
            await h.tick()
            self.assertEqual(h.runs, ["msg-1"])

        self.run_scenario(scenario)

    def test_photo_sent_after_stop_runs_normally(self):
        async def scenario(h: StopHarness):
            await h.deliver(raw_event(1, "long running work"))
            await asyncio.wait_for(h.started.wait(), 2)
            await h.deliver(raw_event(2, "/stop"))
            await h.deliver(raw_photo(3))
            await h.wait_settled("msg-3")

            self.assertEqual(h.runs, ["msg-1", "msg-3"])
            self.assertEqual(h.state("msg-3"), "acked")

        self.run_scenario(scenario)

    def test_shutdown_cancellation_still_releases_for_redelivery(self):
        async def scenario(h: StopHarness):
            await h.deliver(raw_event(1, "long running work"))
            await asyncio.wait_for(h.started.wait(), 2)
            await h.adapter.cancel_background_tasks()

            self.assertEqual(h.state("msg-1"), "pending", "shutdown must keep the turn durable")
            self.assertNotIn(("ack", "msg-1"), h.settled)

        self.run_scenario(scenario)


if __name__ == "__main__":
    unittest.main()

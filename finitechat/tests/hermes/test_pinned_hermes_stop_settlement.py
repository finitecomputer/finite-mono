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
from gateway.run import _INTERRUPT_REASON_GATEWAY_SHUTDOWN, GatewayRunner

REPO_ROOT = Path(__file__).resolve().parents[2]
ADAPTER_PATH = REPO_ROOT / "integrations" / "hermes" / "finitechat" / "adapter.py"
ROOM_ID = "room-agent-1"


def load_adapter_module() -> Any:
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
    raw["attachments"] = [
        {
            "kind": "image",
            "name": f"photo-{seq}.png",
            "path": f"/synthetic/photo-{seq}.png",
            "mime_type": "image/png",
        }
    ]
    return raw


class StopHarness:
    """Real pinned gateway + adapter; simulated model, transport, and sidecar."""

    def __init__(
        self,
        home: str,
        *,
        inbox: dict[str, tuple[dict[str, Any], str]] | None = None,
        stall: bool = True,
    ):
        self.home = home
        self.module = load_adapter_module()
        config = PlatformConfig(enabled=True, extra={"home": home, "room_id": ROOM_ID})
        config.typing_indicator = False
        self.adapter = self.module.FiniteChatAdapter(config)
        self.adapter._home_channel_hydrated = True
        self.runner = GatewayRunner(GatewayConfig(sessions_dir=Path(home) / "sessions"))
        self.runner._session_db = None
        self.runner._persist_active_agents = lambda: None
        self.runner.adapters[self.adapter.platform] = self.adapter
        # The pinned _create_adapter injects this into every plugin adapter.
        self.adapter.gateway_runner = self.runner
        self.module._finite_private_control_request = lambda *_args: None

        self.inbox: dict[str, tuple[dict[str, Any], str]] = {} if inbox is None else inbox
        self.stall = stall
        self.settled: list[tuple[str, str]] = []
        self.runs: list[str] = []
        self.refused: list[str] = []
        self.replies: list[str] = []
        self.started = asyncio.Event()
        # Holds the stalled msg-1 turn open until the test finishes it or the
        # gateway's shutdown interrupt reaches its agent.
        self.turn_gate = asyncio.Event()
        self.interrupted_by_shutdown = False
        # The sidecar wakes the inbound stream on release and re-leases the
        # entry to it until the adapter's stream is closed (disconnect).
        self.redeliver_on_release = False
        self.stream_open = True
        self.redeliveries: list[str] = []
        self._stream_tasks: set[asyncio.Task] = set()
        self.adapter._finitechat_json = self._sidecar
        self.adapter.send = self._send
        self.adapter.set_message_handler(self._handle)
        disconnect = self.adapter.disconnect

        async def close_stream_then_disconnect():
            self.stream_open = False
            await disconnect()

        self.adapter.disconnect = close_stream_then_disconnect

    async def _sidecar(self, action, payload, *, timeout):
        del timeout
        if action in ("ack", "release"):
            message_id = payload["message_id"]
            self.settled.append((action, message_id))
            raw, state = self.inbox[message_id]
            if state == "leased":
                self.inbox[message_id] = (raw, "acked" if action == "ack" else "pending")
            if action == "release" and self.redeliver_on_release and self.stream_open:
                self.redeliveries.append(message_id)
                self.inbox[message_id] = (raw, "leased")
                task = asyncio.create_task(self.adapter._handle_finitechat_event(raw))
                self._stream_tasks.add(task)
                task.add_done_callback(self._stream_tasks.discard)
        return self.module._FiniteChatResult(True, {}, None, False)

    async def _send(self, chat_id, content, **_kwargs):
        del chat_id
        self.replies.append(content)
        return SendResult(success=True, message_id="reply")

    async def _handle(self, event):
        session_key = self.runner._session_key_for_source(event.source)
        if (event.text or "").startswith("/stop"):
            return await self.runner._busy_stop_command(event, session_key, event.source)
        if self.runner._draining:
            # The pinned GatewayRunner._handle_message refuses a new turn
            # while it drains for stop or restart; the base adapter reports
            # the delivered refusal as a successful turn.
            self.refused.append(event.message_id)
            return (
                f"⏳ Gateway is {self.runner._status_action_gerund()} "
                "and is not accepting new work right now."
            )
        self.runs.append(event.message_id)
        generation = self.runner._begin_session_run_generation(session_key)
        harness = self

        class Agent:
            def hard_interrupt(self, message=None):
                if message == _INTERRUPT_REASON_GATEWAY_SHUTDOWN:
                    harness.interrupted_by_shutdown = True
                    harness.turn_gate.set()

        self.runner._session_state(session_key).turn.agent = Agent()
        try:
            if self.stall and event.message_id == "msg-1":
                self.started.set()
                await self.turn_gate.wait()
                # Shutdown interrupts the run cooperatively. An interrupted
                # run that already called the model returns its empty
                # response, which the pinned base reports as SUCCESS.
                return "" if self.interrupted_by_shutdown else "done"
        finally:
            self.runner._release_running_agent_state(session_key, run_generation=generation)
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
        if self._stream_tasks:
            await asyncio.gather(*self._stream_tasks, return_exceptions=True)
        await self.adapter._cancel_admission_tasks()
        await self.adapter.cancel_background_tasks()
        self.runner.close_all_session_db_handles()
        self.runner.session_store.close_all_db_handles()
        self.runner._shutdown_executor()


class PinnedHermesStopSettlementTests(unittest.TestCase):
    def run_scenario(self, scenario):
        with (
            tempfile.TemporaryDirectory(prefix="finite-stop-") as home,
            # Finite ships no drain override, so stop() interrupts running
            # turns at once (pinned default 0s); a developer config must not
            # change that here.
            patch.dict(os.environ, {"HERMES_HOME": home, "HERMES_RESTART_DRAIN_TIMEOUT": "0"}),
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
            # Both queued leases stay held until their turn or the stop.
            await h.deliver(raw_event(2, "queued follow-up"))
            await h.deliver(raw_event(3, "second follow-up"))
            self.assertEqual(h.state("msg-3"), "leased")

            await h.deliver(raw_event(4, "/stop"))
            await h.tick()
            await h.deliver(raw_event(5, "after the stop"))
            await h.wait_settled("msg-5")

            self.assertEqual(h.runs, ["msg-1", "msg-5"])
            for message_id in ("msg-1", "msg-2", "msg-3", "msg-4", "msg-5"):
                self.assertEqual(h.state(message_id), "acked", message_id)
            self.assertEqual(h.adapter._deferred_admissions, {})

        self.run_scenario(scenario)

    def test_stop_acks_a_photo_waiting_behind_the_running_turn(self):
        async def scenario(h: StopHarness):
            await h.deliver(raw_event(1, "long running work"))
            await asyncio.wait_for(h.started.wait(), 2)
            # Media waits at the same durable admission boundary as text.
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

    def test_graceful_stop_releases_the_interrupted_turn_and_the_queued_follow_up(self):
        """Canonical Linux smoke 37424007408 acked both entries on SIGTERM.

        The pinned ``stop()`` drains with a 0s budget and interrupts the turn
        cooperatively, so its completion hook reports SUCCESS. The next queued
        turn is then refused with a reply, which also reports SUCCESS. Neither
        turn ran, so both must stay durable and run once after restart.
        """

        async def scenario(h: StopHarness):
            h.redeliver_on_release = True
            await h.deliver(raw_event(1, "long running work"))
            await asyncio.wait_for(h.started.wait(), 2)
            await h.deliver(raw_event(2, "queued follow-up"))
            self.assertEqual(h.state("msg-2"), "leased")

            # Skip the pinned out-of-loop watchdog, which hard-exits on overrun.
            with patch.dict(os.environ, {"PYTEST_CURRENT_TEST": "finite-graceful-stop"}):
                await asyncio.wait_for(h.runner.stop(), 30)

            self.assertTrue(h.interrupted_by_shutdown)
            self.assertEqual(
                {"active": h.state("msg-1"), "queued": h.state("msg-2")},
                {"active": "pending", "queued": "pending"},
            )
            self.assertEqual(h.runs, ["msg-1"])
            self.assertEqual(h.refused, [], "a draining gateway must not be handed a turn")
            self.assertNotIn(("ack", "msg-1"), h.settled)
            self.assertNotIn(("ack", "msg-2"), h.settled)
            # Released while the stream was still open, then held again and
            # released for good once the stream closed.
            self.assertIn("msg-1", h.redeliveries)

            restarted = StopHarness(h.home, inbox=h.inbox, stall=False)
            try:
                await restarted.tick()
                await restarted.wait_settled("msg-1")
                await restarted.wait_settled("msg-2")
            finally:
                await restarted.close()
            self.assertEqual(restarted.runs, ["msg-1", "msg-2"])
            self.assertEqual(h.state("msg-1"), "acked")
            self.assertEqual(h.state("msg-2"), "acked")

        self.run_scenario(scenario)

    def test_turn_cancelled_during_stop_is_held_when_redelivered(self):
        """A turn that ignores the interrupt is cancelled at adapter teardown.

        The pinned teardown cancels background turns before ``disconnect()``,
        so the released lease comes back on the still-open stream to an idle
        session. Handing it to the stopping gateway would ack it unrun.
        """

        async def scenario(h: StopHarness):
            await h.deliver(raw_event(1, "long running work"))
            await asyncio.wait_for(h.started.wait(), 2)
            h.runner._running = False
            h.runner._draining = True

            await h.adapter.cancel_background_tasks()
            self.assertEqual(h.state("msg-1"), "pending")
            # The open stream re-leases it once the session is idle.
            await h.tick()
            self.assertEqual(h.state("msg-1"), "leased")
            self.assertEqual(h.refused, [])

            await h.adapter.disconnect()
            self.assertEqual(h.state("msg-1"), "pending")
            self.assertEqual(h.runs, ["msg-1"])
            self.assertEqual(h.refused, [])

        self.run_scenario(scenario)

    def test_restart_drain_acks_a_finished_turn_and_holds_new_messages(self):
        """``request_restart`` keeps adapters up while running turns finish.

        A turn that finishes in that window ran, so it is acked. A message
        that arrives is not handed to the draining gateway, which would only
        refuse it; its lease is held and released when the adapter stops.
        """

        async def scenario(h: StopHarness):
            h.runner._running = True
            await h.deliver(raw_event(1, "long running work"))
            await asyncio.wait_for(h.started.wait(), 2)
            h.runner._draining = True
            await h.deliver(raw_event(2, "sent during the restart drain"))
            h.turn_gate.set()
            await h.wait_settled("msg-1")
            await h.settle_loop()

            self.assertEqual(h.state("msg-1"), "acked")
            self.assertEqual(h.state("msg-2"), "leased")
            self.assertEqual(h.refused, [])

            await h.adapter.disconnect()
            self.assertEqual(h.state("msg-2"), "pending")
            self.assertEqual(h.runs, ["msg-1"])

        self.run_scenario(scenario)


if __name__ == "__main__":
    unittest.main()

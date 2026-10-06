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

import ast
import asyncio
import contextlib
import importlib.util
import inspect
import os
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path
from typing import Any
from unittest.mock import patch

from gateway.config import GatewayConfig, PlatformConfig
from gateway.platforms.base import MessageEvent, SendResult
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


def raw_event(
    seq: int,
    text: str,
    *,
    segment: str = "segment-1",
    chat_type: str = "group",
) -> dict[str, Any]:
    return {
        "room_id": ROOM_ID,
        "seq": seq,
        "message_id": f"msg-{seq}",
        "conversation_id": "home",
        "segment_id": segment,
        "text": text,
        "message_type": "text",
        "source": {
            "platform": "finitechat",
            "chat_id": ROOM_ID,
            "chat_type": chat_type,
            "user_id": "alice",
            "thread_id": segment,
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
        # The pinned _handle_message_with_agent binds the run to the adapter's
        # active-session guard before the agent starts.
        self.runner._bind_adapter_run_generation(self.adapter, session_key, generation)
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


class GatewayHarness(StopHarness):
    """The real pinned ``GatewayRunner._handle_message`` behind the adapter.

    Its startup-restore gate, ``resume_pending`` marking, auto-resume
    scheduler, gate drain and session bookkeeping all run for real; only the
    agent run itself (``_run_agent``) is simulated. ``timeline`` interleaves
    model handoffs with sidecar settlements across both gateway processes.
    """

    def __init__(self, home: str, *, timeline: list[tuple[str, str]], **kwargs: Any):
        super().__init__(home, **kwargs)
        self.timeline = timeline
        # Message ids the adapter handed to the gateway's handler.
        self.handed: list[str | None] = []
        gateway_handler = self.runner._handle_message

        async def handle_message(event):
            self.handed.append(event.message_id)
            return await gateway_handler(event)

        self.adapter.set_message_handler(handle_message)
        self.runner._run_agent = self._run_agent
        # The /new banner resolves the model and can probe a provider.
        self.runner._format_session_info = lambda: ""
        # start() sets this before adapters connect; only stop() clears it.
        self.runner._running = True
        # Model turns held open by message text, as active work that keeps a
        # restart drain open until the test finishes them.
        self.held_turns: dict[str, asyncio.Event] = {}
        # Model turns, by message text, whose agent cannot take a /steer.
        self.steerless: set[str] = set()
        # How long a reply send takes, as a real transport would.
        self.reply_delay = 0.0
        # The next admission await at which a restart drain begins:
        # "home-channel-show" or "activity" (the adapter's RPCs before
        # handoff), or "dispatch" (the base background turn, before the
        # gateway's handler runs).
        self.drain_window: str | None = None
        processing_start = self.adapter.on_processing_start

        async def on_processing_start(event):
            if self.drain_window == "dispatch":
                self.begin_restart_drain()
            await processing_start(event)

        self.adapter.on_processing_start = on_processing_start

    async def _sidecar(self, action, payload, *, timeout):
        if action in ("ack", "release"):
            self.timeline.append((action, payload["message_id"]))
        if action == self.drain_window and payload.get("action", "set") == "set":
            self.begin_restart_drain()
        return await super()._sidecar(action, payload, timeout=timeout)

    async def _send(self, chat_id, content, **kwargs):
        await asyncio.sleep(self.reply_delay)
        return await super()._send(chat_id, content, **kwargs)

    def begin_restart_drain(self) -> None:
        """What an in-band ``/restart`` does: refuse new work at once, keep
        adapters up while running turns finish, then ``stop()``."""
        self.drain_window = None
        self.runner.request_restart()

    async def finish_restart(self) -> None:
        for _ in range(500):
            if getattr(self.runner, "_restart_task", None) is not None:
                break
            await asyncio.sleep(0.01)
        restart = getattr(self.runner, "_restart_task", None)
        assert restart is not None, "the restart drain never began"
        await asyncio.wait_for(restart, 30)

    def models(self) -> list[str]:
        return [message for kind, message in self.timeline if kind == "model"]

    async def hold_turn_open(self, raw: dict[str, Any]) -> asyncio.Event:
        """Start ``raw``'s turn and keep it running until the returned gate is set."""
        gate = self.held_turns[raw["text"]] = asyncio.Event()
        await self.deliver(raw)
        for _ in range(500):
            if raw["text"] in self.models():
                return gate
            await asyncio.sleep(0.01)
        raise AssertionError(f"{raw['message_id']} never reached the model")

    async def _run_agent(
        self,
        message,
        context_prompt,
        history,
        source,
        session_id,
        session_key: str | None = None,
        *_args: Any,
        **_kwargs: Any,
    ):
        del context_prompt, history, source, session_id
        self.timeline.append(("model", message))
        self.runs.append(message)
        harness = self
        gate = self.held_turns.get(message)
        if self.stall and message == "long running work":
            gate = self.turn_gate

        class Agent:
            def interrupt(self, *_args, **_kwargs):
                harness.interrupted_by_shutdown = True
                harness.turn_gate.set()
                if gate is not None:
                    gate.set()

            def hard_interrupt(self, *_args, **_kwargs):
                self.interrupt()

        class SteerableAgent(Agent):
            def steer(self, text):
                harness.timeline.append(("steer", text))
                return True

        assert session_key is not None, "the gateway must bind the model turn to a session"
        agent = Agent() if message in self.steerless else SteerableAgent()
        self.runner._session_state(session_key).turn.agent = agent
        if gate is not None:
            if gate is self.turn_gate:
                self.started.set()
            await gate.wait()
            if self.interrupted_by_shutdown:
                return {"final_response": "", "interrupted": True, "messages": [], "api_calls": 1}
        return {"final_response": "done", "messages": [], "api_calls": 1}

    def resume_pending(self) -> bool:
        return any(entry.resume_pending for entry in self.runner.session_store._entries.values())

    async def stop_gracefully(self) -> None:
        # Skip the pinned out-of-loop watchdog, which hard-exits on overrun.
        with patch.dict(os.environ, {"PYTEST_CURRENT_TEST": "finite-graceful-stop"}):
            await asyncio.wait_for(self.runner.stop(), 30)

    async def boot_with_gate_closed(self) -> None:
        """What ``start()`` does before adapters connect, then one stream tick."""
        self.runner._startup_restore_in_progress = True
        self.runner._startup_restore_queue = []
        self.runner._startup_restore_tasks = []
        await self.tick()

    async def open_gate(self) -> int:
        scheduled = self.runner._schedule_resume_pending_sessions()
        await asyncio.wait_for(self.runner._finish_startup_restore(), 10)
        return scheduled


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


class PinnedHermesRestartRecoveryTests(unittest.TestCase):
    """Canonical Linux smoke 37428809877: the restart after a graceful stop.

    Both released entries were acked about 2s before any model call, because
    the pinned startup-restore gate queued them in memory and returned. The
    interrupted turn then ran twice: once as Hermes's own auto-resume turn
    for the session ``stop()`` marked, once as the inbox redelivery replayed
    from the gate.
    """

    def run_scenario(self, scenario):
        with (
            tempfile.TemporaryDirectory(prefix="finite-restart-") as home,
            patch.dict(
                os.environ,
                {
                    "HERMES_HOME": home,
                    "HERMES_RESTART_DRAIN_TIMEOUT": "0",
                    # The real handler authorizes senders; Finite's admission
                    # policy is enforced upstream of this boundary.
                    "GATEWAY_ALLOW_ALL_USERS": "true",
                },
            ),
        ):
            asyncio.run(scenario(home))

    @staticmethod
    def pending_inbox() -> dict[str, tuple[dict[str, Any], str]]:
        return {
            "msg-1": (raw_event(1, "long running work"), "pending"),
            "msg-2": (raw_event(2, "queued follow-up"), "pending"),
        }

    def test_graceful_restart_runs_each_released_turn_once_after_the_gate(self):
        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            stopped = GatewayHarness(home, timeline=timeline)
            try:
                await stopped.deliver(raw_event(1, "long running work"))
                await asyncio.wait_for(stopped.started.wait(), 2)
                await stopped.deliver(raw_event(2, "queued follow-up"))
                await stopped.stop_gracefully()
                self.assertTrue(stopped.interrupted_by_shutdown)
                self.assertEqual(
                    {"active": stopped.state("msg-1"), "queued": stopped.state("msg-2")},
                    {"active": "pending", "queued": "pending"},
                )
                self.assertTrue(stopped.resume_pending(), "stop() marks the session to resume")
            finally:
                await stopped.close()

            timeline.clear()
            restarted = GatewayHarness(home, timeline=timeline, inbox=stopped.inbox, stall=False)
            try:
                await restarted.boot_with_gate_closed()
                # Held, not acked: a stop or crash before the gate opens
                # leaves both entries in the durable inbox.
                self.assertEqual(
                    {"active": restarted.state("msg-1"), "queued": restarted.state("msg-2")},
                    {"active": "leased", "queued": "leased"},
                )
                self.assertEqual(timeline, [])

                self.assertEqual(await restarted.open_gate(), 1, "Hermes scheduled its resume")
                await restarted.wait_settled("msg-1")
                await restarted.wait_settled("msg-2")
                self.assertEqual(
                    timeline,
                    [
                        ("model", "long running work"),
                        ("ack", "msg-1"),
                        ("model", "queued follow-up"),
                        ("ack", "msg-2"),
                    ],
                )
                self.assertFalse(restarted.resume_pending(), "the redelivered turn resumed it")
            finally:
                await restarted.close()

        self.run_scenario(scenario)

    def test_stop_before_the_startup_gate_opens_releases_held_turns(self):
        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline, inbox=self.pending_inbox(), stall=False)
            try:
                await h.boot_with_gate_closed()
                await h.adapter.disconnect()
                self.assertEqual(h.state("msg-1"), "pending")
                self.assertEqual(h.state("msg-2"), "pending")
                self.assertEqual(timeline, [("release", "msg-1"), ("release", "msg-2")])
            finally:
                await h.close()

        self.run_scenario(scenario)

    def test_user_stop_during_the_startup_gate_stays_final(self):
        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline, inbox=self.pending_inbox(), stall=False)
            try:
                await h.boot_with_gate_closed()
                await h.deliver(raw_event(3, "/stop"))
                self.assertEqual(h.state("msg-1"), "acked")
                self.assertEqual(h.state("msg-2"), "acked")
                # The gate queues commands too; the stop is held until it opens.
                self.assertEqual(h.state("msg-3"), "leased")

                await h.open_gate()
                await h.wait_settled("msg-3")
                await h.tick()
                self.assertEqual(h.state("msg-3"), "acked")
                self.assertEqual(h.runs, [], "nothing sent before the stop runs")
            finally:
                await h.close()

        self.run_scenario(scenario)

    def test_only_the_hermes_resume_event_is_declined(self):
        async def scenario(home: str):
            h = GatewayHarness(home, timeline=[], stall=False)
            try:
                source = h.adapter.build_source(
                    chat_id=ROOM_ID, chat_type="group", user_id="alice", thread_id="segment-1"
                )
                await h.adapter.handle_message(MessageEvent(text="", source=source, internal=True))
                self.assertEqual(h.adapter._session_tasks, {}, "no resume turn starts")
                await h.settle_loop()
                self.assertEqual(h.runs, [])

                notice = MessageEvent(text="background job finished", source=source, internal=True)
                await h.adapter.handle_message(notice)
                for _ in range(200):
                    if h.runs:
                        break
                    await asyncio.sleep(0.01)
                self.assertEqual(h.runs, ["background job finished"])
                injected = MessageEvent(
                    text="", source=source, internal=True, metadata={"hermes_plugin_id": "p"}
                )
                self.assertFalse(h.module._is_hermes_resume_event(injected))
            finally:
                await h.close()

        self.run_scenario(scenario)

    def test_message_behind_a_reserved_resume_slot_is_acked_after_it_runs(self):
        """A reconnect schedules auto-resume with the startup gate already open.

        The scheduler reserves the session's slot before its resume task runs.
        A message admitted in that window is queued by the gateway behind the
        reservation and reported as a successful turn; it must not be acked
        until the queued turn runs it.
        """

        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            stopped = GatewayHarness(home, timeline=timeline)
            try:
                await stopped.deliver(raw_event(1, "long running work"))
                await asyncio.wait_for(stopped.started.wait(), 2)
                await stopped.stop_gracefully()
                self.assertEqual(stopped.state("msg-1"), "pending")
                self.assertTrue(stopped.resume_pending())
            finally:
                await stopped.close()

            timeline.clear()
            reconnected = GatewayHarness(home, timeline=timeline, inbox=stopped.inbox, stall=False)
            try:
                self.assertEqual(reconnected.runner._schedule_resume_pending_sessions(), 1)
                await reconnected.tick()
                await reconnected.wait_settled("msg-1")
                await reconnected.settle_loop()
                self.assertEqual(timeline, [("model", "long running work"), ("ack", "msg-1")])
            finally:
                await reconnected.close()

        self.run_scenario(scenario)


class DrainScenario(unittest.TestCase):
    def run_scenario(self, scenario):
        with (
            tempfile.TemporaryDirectory(prefix="finite-drain-") as home,
            patch.dict(
                os.environ,
                {
                    "HERMES_HOME": home,
                    "HERMES_RESTART_DRAIN_TIMEOUT": "0",
                    "GATEWAY_ALLOW_ALL_USERS": "true",
                    # The restart task calls stop() itself; skip the pinned
                    # out-of-loop watchdog, which hard-exits on overrun.
                    "PYTEST_CURRENT_TEST": "finite-restart-drain",
                },
            ),
        ):
            os.environ.pop("FINITECHAT_HOME_CHANNEL", None)
            asyncio.run(scenario(home))

    @staticmethod
    async def run_after_restart(
        home: str,
        inbox: dict[str, tuple[dict[str, Any], str]],
        *message_ids: str,
    ) -> list[tuple[str, str]]:
        timeline: list[tuple[str, str]] = []
        restarted = GatewayHarness(home, timeline=timeline, inbox=inbox, stall=False)
        try:
            await restarted.boot_with_gate_closed()
            await restarted.open_gate()
            for message_id in message_ids:
                await restarted.wait_settled(message_id)
            await restarted.settle_loop()
        finally:
            await restarted.close()
        return timeline

    def assert_ran_once_then_acked(
        self,
        timeline: list[tuple[str, str]],
        message_id: str,
        task: str = "synthetic task",
    ) -> None:
        """``task`` reached the model once, and its entry was acked once after."""
        runs = [i for i, (kind, text) in enumerate(timeline) if kind == "model" and task in text]
        self.assertEqual(len(runs), 1, timeline)
        self.assertEqual(timeline.count(("ack", message_id)), 1, timeline)
        self.assertNotIn(("release", message_id), timeline)
        self.assertLess(runs[0], timeline.index(("ack", message_id)), timeline)


class PinnedHermesRestartDrainAdmissionTests(DrainScenario):
    """Review 5426200391: a restart drain that wins the admission race.

    ``request_restart`` refuses new work at once and keeps ``_running`` set
    while running turns finish (up to the pinned 1800s after-turn cap). The
    gateway refuses a message handed to it after that point with a reply,
    which the base adapter reports as a successful turn, so the entry was
    acked with no model call. Path-like text starting with ``/`` skipped the
    adapter's drain hold the same way.
    """

    def test_drain_holds_ordinary_work_and_model_launching_commands(self):
        async def scenario(home: str):
            h = GatewayHarness(home, timeline=[], stall=False)
            try:
                # (pinned command, held while the gateway drains)
                cases = {
                    ("/usr/bin/python3 crashes", "group"): (None, True),
                    ("/etc/hosts looks wrong", "group"): (None, True),
                    ("/not-a-command please", "group"): (None, True),
                    ("plain text", "group"): (None, True),
                    # Answered by the gateway itself, without a model turn.
                    ("/status", "group"): ("status", False),
                    ("/restart", "group"): ("restart", False),
                    ("/stop", "group"): ("stop", False),
                    ("/new", "group"): ("new", False),
                    ("/reset", "group"): ("new", False),
                    ("/approve", "group"): ("approve", False),
                    ("/title renamed", "group"): ("title", False),
                    ("/bg synthetic task", "group"): ("bg", False),
                    ("/btw synthetic task", "group"): ("btw", False),
                    # The pinned base adapter rewrites this DM phrase to
                    # /restart; replaying it after the restart would restart
                    # again.
                    ("restart the gateway", "dm"): ("restart", False),
                    ("restart the gateway", "group"): (None, True),
                    # Hermes turns these into a model turn for this message,
                    # which a draining gateway refuses.
                    ("/queue synthetic task", "group"): ("queue", True),
                    ("/q synthetic task", "group"): ("queue", True),
                    ("/steer synthetic task", "group"): ("steer", True),
                    ("/plan synthetic task", "group"): ("plan", True),
                    ("/learn synthetic task", "group"): ("learn", True),
                    ("/init", "group"): ("init", True),
                    ("/blueprint synthetic", "group"): ("blueprint", True),
                    ("/moa synthetic task", "group"): ("moa", True),
                    ("/retry", "group"): ("retry", True),
                    ("/goal synthetic task", "group"): ("goal", True),
                }
                h.runner._draining = True
                for (text, chat_type), expected in cases.items():
                    raw = raw_event(1, text, chat_type=chat_type)
                    source = h.adapter.build_source(
                        chat_id=ROOM_ID, chat_type=chat_type, user_id="alice"
                    )
                    event = MessageEvent(text=text, source=source, raw_message=raw)
                    session_key = h.adapter._event_session_key(event)
                    with self.subTest(text=text, chat_type=chat_type):
                        self.assertEqual(
                            (
                                h.module._gateway_command(event),
                                h.adapter._must_wait(event, session_key, ""),
                            ),
                            expected,
                        )
                        self.assertEqual(event.text, text, "classification must not rewrite")
            finally:
                h.runner._draining = False
                await h.close()

        self.run_scenario(scenario)

    def test_drain_beginning_during_an_immediate_handoff_keeps_the_message(self):
        for window in ("home-channel-show", "activity", "dispatch"):
            with self.subTest(window=window):

                async def scenario(home: str, window: str = window):
                    timeline: list[tuple[str, str]] = []
                    h = GatewayHarness(home, timeline=timeline, stall=False)
                    h.redeliver_on_release = True
                    h.adapter._home_channel_hydrated = False
                    try:
                        # Another chat's running turn keeps the drain open.
                        other = await h.hold_turn_open(raw_event(1, "other work", segment="s2"))
                        h.drain_window = window
                        await h.deliver(raw_event(2, "hello"))
                        self.assertNotIn(("ack", "msg-2"), timeline)

                        # Held before handoff; or, once handed over, refused
                        # by the gateway and released at settlement.
                        self.assertEqual("msg-2" in h.handed, window == "dispatch")

                        other.set()
                        await h.finish_restart()
                        self.assertEqual(h.models(), ["other work"])
                        self.assertEqual(h.state("msg-1"), "acked")
                        self.assertNotIn(("ack", "msg-2"), timeline)
                        self.assertEqual(h.state("msg-2"), "pending")
                    finally:
                        await h.close()

                    after = await self.run_after_restart(home, h.inbox, "msg-2")
                    self.assertEqual(after, [("model", "hello"), ("ack", "msg-2")])

                self.run_scenario(scenario)

    def test_drain_beginning_during_a_deferred_handoff_keeps_the_message(self):
        for window in ("home-channel-show", "activity", "dispatch"):
            with self.subTest(window=window):

                async def scenario(home: str, window: str = window):
                    timeline: list[tuple[str, str]] = []
                    h = GatewayHarness(home, timeline=timeline)
                    h.redeliver_on_release = True
                    h.adapter._home_channel_hydrated = False
                    try:
                        other = await h.hold_turn_open(raw_event(1, "other work", segment="s2"))
                        await h.deliver(raw_event(2, "long running work"))
                        await asyncio.wait_for(h.started.wait(), 2)
                        await h.deliver(raw_event(3, "queued follow-up"))
                        self.assertEqual(h.state("msg-3"), "leased")
                        # The drain begins inside msg-3's handoff, once msg-2
                        # has finished and the session is idle.
                        h.drain_window = window
                        h.turn_gate.set()
                        await h.wait_settled("msg-2")
                        for _ in range(500):
                            if h.drain_window is None:
                                break
                            await asyncio.sleep(0.01)
                        await h.settle_loop()
                        self.assertEqual("msg-3" in h.handed, window == "dispatch")

                        other.set()
                        await h.finish_restart()
                        self.assertEqual(h.models(), ["other work", "long running work"])
                        self.assertEqual(h.state("msg-1"), "acked")
                        self.assertEqual(h.state("msg-2"), "acked")
                        self.assertNotIn(("ack", "msg-3"), timeline)
                        self.assertEqual(h.state("msg-3"), "pending")
                    finally:
                        await h.close()

                    after = await self.run_after_restart(home, h.inbox, "msg-3")
                    self.assertEqual(after, [("model", "queued follow-up"), ("ack", "msg-3")])

                self.run_scenario(scenario)

    def test_turn_finishing_during_the_drain_is_acked_and_slash_text_is_held(self):
        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline)
            h.redeliver_on_release = True
            try:
                await h.deliver(raw_event(1, "long running work"))
                await asyncio.wait_for(h.started.wait(), 2)
                h.begin_restart_drain()
                # Path-like text is ordinary work, in an idle session and
                # behind the running turn alike.
                await h.deliver(raw_event(2, "/usr/bin/python3 crashes", segment="segment-2"))
                await h.deliver(raw_event(3, "/etc/hosts looks wrong"))
                # Gateway commands still answer while the gateway drains.
                await h.deliver(raw_event(4, "/status", segment="segment-4"))
                await h.deliver(raw_event(5, "/restart", segment="segment-5"))
                await h.deliver(raw_event(6, "restart the gateway", segment="dm", chat_type="dm"))
                for message_id in ("msg-4", "msg-5", "msg-6"):
                    with contextlib.suppress(TimeoutError):
                        await h.wait_settled(message_id)
                self.assertEqual(h.state("msg-2"), "leased")
                self.assertEqual(h.state("msg-3"), "leased")
                for message_id in ("msg-4", "msg-5", "msg-6"):
                    self.assertEqual(h.state(message_id), "acked", message_id)
                self.assertEqual(h.handed, ["msg-1", "msg-4", "msg-5", "msg-6"])

                h.turn_gate.set()
                await h.finish_restart()
                # Finished during the drain: it ran, so it is acked.
                self.assertEqual(h.models(), ["long running work"])
                self.assertEqual(h.state("msg-1"), "acked")
                self.assertLess(
                    timeline.index(("model", "long running work")),
                    timeline.index(("ack", "msg-1")),
                )
                self.assertNotIn(("release", "msg-1"), timeline)
                for message_id in ("msg-2", "msg-3"):
                    self.assertNotIn(("ack", message_id), timeline)
                    self.assertEqual(h.state(message_id), "pending", message_id)
            finally:
                await h.close()

            after = await self.run_after_restart(home, h.inbox, "msg-2", "msg-3")
            self.assertEqual(
                sorted(after),
                sorted(
                    [
                        ("model", "/usr/bin/python3 crashes"),
                        ("ack", "msg-2"),
                        ("model", "/etc/hosts looks wrong"),
                        ("ack", "msg-3"),
                    ]
                ),
            )
            for message_id, text in (
                ("msg-2", "/usr/bin/python3 crashes"),
                ("msg-3", "/etc/hosts looks wrong"),
            ):
                self.assertLess(after.index(("model", text)), after.index(("ack", message_id)))

        self.run_scenario(scenario)

    def test_user_stop_during_the_drain_stays_final(self):
        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline)
            h.redeliver_on_release = True
            try:
                await h.deliver(raw_event(1, "long running work"))
                await asyncio.wait_for(h.started.wait(), 2)
                h.begin_restart_drain()
                await h.deliver(raw_event(2, "queued follow-up"))
                await h.deliver(raw_event(3, "/stop"))
                await h.finish_restart()
                for message_id in ("msg-1", "msg-2", "msg-3"):
                    self.assertEqual(h.state(message_id), "acked", message_id)
                self.assertEqual(h.models(), ["long running work"])
            finally:
                await h.close()

            after = await self.run_after_restart(home, h.inbox)
            self.assertEqual(after, [])

        self.run_scenario(scenario)


def pinned_turn_commands() -> set[str]:
    """Commands the pinned idle dispatch turns into this message's model turn.

    ``GatewayRunner._handle_message`` rewrites ``event.text`` and falls
    through to the agent for some commands; for others it hands off to a
    handler that re-enters the gateway or queues the turn itself.
    """
    tree = ast.parse(textwrap.dedent(inspect.getsource(GatewayRunner._handle_message)))
    commands: set[str] = set()
    handlers: dict[str, set[str]] = {}
    for node in ast.walk(tree):
        test = node.test if isinstance(node, ast.If) else None
        if not (
            isinstance(test, ast.Compare)
            and isinstance(test.left, ast.Name)
            and test.left.id == "canonical"
            and isinstance(test.ops[0], ast.Eq)
            and isinstance(test.comparators[0], ast.Constant)
        ):
            continue
        name = str(test.comparators[0].value)
        for sub in ast.walk(node):
            if isinstance(sub, ast.Assign) and any(
                isinstance(target, ast.Attribute)
                and target.attr == "text"
                and isinstance(target.value, ast.Name)
                and target.value.id == "event"
                for target in sub.targets
            ):
                commands.add(name)
            if (
                isinstance(sub, ast.Call)
                and isinstance(sub.func, ast.Attribute)
                and isinstance(sub.func.value, ast.Name)
                and sub.func.value.id == "self"
                and sub.func.attr.startswith("_handle_")
            ):
                handlers.setdefault(name, set()).add(sub.func.attr)
    plain = GatewayRunner._gateway_plain_command_handlers(GatewayRunner.__new__(GatewayRunner))
    for name, handler in plain.items():
        handlers.setdefault(name, set()).add(handler.__name__)
    for name, handler_names in handlers.items():
        for handler_name in handler_names:
            source = inspect.getsource(getattr(GatewayRunner, handler_name))
            if "self._handle_message(" in source or "_enqueue_fifo(" in source:
                commands.add(name)
    return commands


class PinnedHermesModelCommandTests(DrainScenario):
    """Review r4193904833 and round 06: commands that become a model turn.

    In an idle session the pinned gateway rewrites /queue, /steer, /plan and
    /learn (and /init, /blueprint and /moa) into this message's agent turn;
    /retry and /goal start one. A draining gateway refuses that turn after the
    rewrite, and the entry was acked as a finished command. In a busy session
    Hermes keeps /queue, and a /steer the running agent cannot take, as a new
    in-memory event, so the entry was acked before its model call and a stop
    or drain dropped the work.
    """

    MODEL_COMMANDS = (
        "/queue synthetic task",
        "/q synthetic task",
        "/steer synthetic task",
        "/plan synthetic task",
        "/learn synthetic task",
    )

    def test_drain_hold_covers_every_pinned_command_that_starts_a_turn(self):
        # Pin-bump tripwire: a new command that becomes a model turn must
        # wait out a drain too.
        self.assertEqual(pinned_turn_commands(), load_adapter_module()._HERMES_TURN_COMMANDS)

    def test_model_commands_run_once_without_a_drain(self):
        for text in self.MODEL_COMMANDS:
            with self.subTest(text=text):

                async def scenario(home: str, text: str = text):
                    timeline: list[tuple[str, str]] = []
                    h = GatewayHarness(home, timeline=timeline, stall=False)
                    try:
                        await h.deliver(raw_event(1, text))
                        await h.wait_settled("msg-1")
                        await h.settle_loop()
                        self.assert_ran_once_then_acked(timeline, "msg-1")
                    finally:
                        await h.close()

                self.run_scenario(scenario)

    def test_model_commands_wait_out_a_restart_drain(self):
        for text in self.MODEL_COMMANDS:
            with self.subTest(text=text):

                async def scenario(home: str, text: str = text):
                    timeline: list[tuple[str, str]] = []
                    h = GatewayHarness(home, timeline=timeline, stall=False)
                    h.redeliver_on_release = True
                    try:
                        # Another chat's running turn keeps the drain open.
                        other = await h.hold_turn_open(raw_event(1, "other work", segment="s2"))
                        h.begin_restart_drain()
                        await h.deliver(raw_event(2, text))
                        self.assertNotIn("msg-2", h.handed)
                        self.assertEqual(h.state("msg-2"), "leased")

                        other.set()
                        await h.finish_restart()
                        self.assertEqual(h.models(), ["other work"])
                        self.assertNotIn(("ack", "msg-2"), timeline)
                        self.assertEqual(h.state("msg-2"), "pending")
                    finally:
                        await h.close()

                    after = await self.run_after_restart(home, h.inbox, "msg-2")
                    self.assert_ran_once_then_acked(after, "msg-2")

                self.run_scenario(scenario)

    def test_drain_beginning_during_a_command_handoff_keeps_it(self):
        for window in ("home-channel-show", "activity", "dispatch"):
            for text in ("/queue synthetic task", "/plan synthetic task"):
                with self.subTest(window=window, text=text):

                    async def scenario(home: str, window: str = window, text: str = text):
                        timeline: list[tuple[str, str]] = []
                        h = GatewayHarness(home, timeline=timeline, stall=False)
                        h.redeliver_on_release = True
                        h.adapter._home_channel_hydrated = False
                        try:
                            other = await h.hold_turn_open(raw_event(1, "other work", segment="s2"))
                            h.drain_window = window
                            await h.deliver(raw_event(2, text))
                            self.assertNotIn(("ack", "msg-2"), timeline)
                            # Refused once at most: the redelivery is held.
                            self.assertEqual(h.handed.count("msg-2"), int(window == "dispatch"))

                            other.set()
                            await h.finish_restart()
                            self.assertEqual(h.models(), ["other work"])
                            self.assertNotIn(("ack", "msg-2"), timeline)
                            self.assertEqual(h.state("msg-2"), "pending")
                        finally:
                            await h.close()

                        after = await self.run_after_restart(home, h.inbox, "msg-2")
                        self.assert_ran_once_then_acked(after, "msg-2")

                    self.run_scenario(scenario)

    def test_drain_beginning_during_a_deferred_queue_handoff_keeps_it(self):
        for window in ("home-channel-show", "activity", "dispatch"):
            with self.subTest(window=window):

                async def scenario(home: str, window: str = window):
                    timeline: list[tuple[str, str]] = []
                    h = GatewayHarness(home, timeline=timeline)
                    h.redeliver_on_release = True
                    h.adapter._home_channel_hydrated = False
                    try:
                        other = await h.hold_turn_open(raw_event(1, "other work", segment="s2"))
                        await h.deliver(raw_event(2, "long running work"))
                        await asyncio.wait_for(h.started.wait(), 2)
                        await h.deliver(raw_event(3, "/queue synthetic task"))
                        self.assertNotIn("msg-3", h.handed)
                        self.assertEqual(h.state("msg-3"), "leased")
                        # The drain begins inside msg-3's handoff, once msg-2
                        # has finished and the session is idle.
                        h.drain_window = window
                        h.turn_gate.set()
                        await h.wait_settled("msg-2")
                        for _ in range(500):
                            if h.drain_window is None:
                                break
                            await asyncio.sleep(0.01)
                        await h.settle_loop()
                        self.assertEqual(h.handed.count("msg-3"), int(window == "dispatch"))

                        other.set()
                        await h.finish_restart()
                        self.assertEqual(h.models(), ["other work", "long running work"])
                        self.assertNotIn(("ack", "msg-3"), timeline)
                        self.assertEqual(h.state("msg-3"), "pending")
                    finally:
                        await h.close()

                    after = await self.run_after_restart(home, h.inbox, "msg-3")
                    self.assert_ran_once_then_acked(after, "msg-3")

                self.run_scenario(scenario)

    def test_busy_queue_waits_for_the_running_turn(self):
        for text in ("/queue synthetic task", "/q synthetic task"):
            with self.subTest(text=text):

                async def scenario(home: str, text: str = text):
                    timeline: list[tuple[str, str]] = []
                    h = GatewayHarness(home, timeline=timeline)
                    try:
                        await h.deliver(raw_event(1, "long running work"))
                        await asyncio.wait_for(h.started.wait(), 2)
                        await h.deliver(raw_event(2, text))
                        # Never handed to Hermes's in-memory queue while busy.
                        self.assertEqual(h.handed, ["msg-1"])
                        self.assertEqual(h.adapter._pending_messages, {})
                        self.assertEqual(h.state("msg-2"), "leased")

                        h.turn_gate.set()
                        await h.wait_settled("msg-1")
                        await h.wait_settled("msg-2")
                        await h.settle_loop()
                        self.assertEqual(h.models(), ["long running work", "synthetic task"])
                        self.assert_ran_once_then_acked(timeline, "msg-2")
                        self.assertLess(
                            timeline.index(("ack", "msg-1")),
                            timeline.index(("model", "synthetic task")),
                        )
                    finally:
                        await h.close()

                self.run_scenario(scenario)

    def test_busy_queue_survives_a_graceful_stop(self):
        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline)
            try:
                await h.deliver(raw_event(1, "long running work"))
                await asyncio.wait_for(h.started.wait(), 2)
                await h.deliver(raw_event(2, "/queue synthetic task"))
                await h.stop_gracefully()
                self.assertEqual(h.state("msg-1"), "pending")
                self.assertEqual(h.state("msg-2"), "pending")
                self.assertNotIn(("ack", "msg-2"), timeline)
            finally:
                await h.close()

            after = await self.run_after_restart(home, h.inbox, "msg-1", "msg-2")
            self.assert_ran_once_then_acked(after, "msg-1", task="long running work")
            self.assert_ran_once_then_acked(after, "msg-2")

        self.run_scenario(scenario)

    def test_busy_queue_survives_a_restart_drain(self):
        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline)
            h.redeliver_on_release = True
            try:
                await h.deliver(raw_event(1, "long running work"))
                await asyncio.wait_for(h.started.wait(), 2)
                h.begin_restart_drain()
                await h.deliver(raw_event(2, "/queue synthetic task"))
                self.assertEqual(h.handed, ["msg-1"])
                h.turn_gate.set()
                await h.finish_restart()
                self.assertEqual(h.models(), ["long running work"])
                self.assertEqual(h.state("msg-1"), "acked")
                self.assertEqual(h.state("msg-2"), "pending")
            finally:
                await h.close()

            after = await self.run_after_restart(home, h.inbox, "msg-2")
            self.assert_ran_once_then_acked(after, "msg-2")

        self.run_scenario(scenario)

    def test_busy_steer_settles_with_the_turn_it_steered(self):
        for draining in (False, True):
            with self.subTest(draining=draining):

                async def scenario(home: str, draining: bool = draining):
                    timeline: list[tuple[str, str]] = []
                    h = GatewayHarness(home, timeline=timeline)
                    h.redeliver_on_release = True
                    try:
                        await h.deliver(raw_event(1, "long running work"))
                        await asyncio.wait_for(h.started.wait(), 2)
                        if draining:
                            # A restart drain lets the running turn finish, so
                            # a steer still reaches it.
                            h.begin_restart_drain()
                        await h.deliver(raw_event(2, "/steer change course"))
                        self.assertIn(("steer", "change course"), timeline)
                        # Accepted into the running turn, so it settles with it.
                        self.assertEqual(h.state("msg-2"), "leased")

                        h.turn_gate.set()
                        if draining:
                            await h.finish_restart()
                        await h.wait_settled("msg-1")
                        await h.wait_settled("msg-2")
                        await h.settle_loop()
                        self.assertEqual(h.models(), ["long running work"])
                        self.assertEqual(h.state("msg-1"), "acked")
                        self.assertEqual(h.state("msg-2"), "acked")
                        self.assertNotIn(("release", "msg-2"), timeline)
                        self.assertLess(
                            timeline.index(("steer", "change course")),
                            timeline.index(("ack", "msg-2")),
                        )
                    finally:
                        await h.close()

                self.run_scenario(scenario)

    def test_queue_behind_a_reserved_resume_slot_is_acked_after_it_runs(self):
        """Hermes copies a /queue behind a reserved slot into its pending slot.

        The copy carries the entry's inbox record, so the turn that runs the
        copy settles the entry, not the /queue turn Hermes answered at once.
        """

        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            stopped = GatewayHarness(home, timeline=timeline)
            try:
                await stopped.deliver(raw_event(1, "long running work"))
                await asyncio.wait_for(stopped.started.wait(), 2)
                await stopped.stop_gracefully()
                self.assertTrue(stopped.resume_pending())
            finally:
                await stopped.close()

            timeline.clear()
            reconnected = GatewayHarness(home, timeline=timeline, inbox=stopped.inbox, stall=False)
            try:
                self.assertEqual(reconnected.runner._schedule_resume_pending_sessions(), 1)
                await reconnected.deliver(raw_event(2, "/queue synthetic task"))
                await reconnected.wait_settled("msg-2")
                await reconnected.settle_loop()
                self.assertTrue(
                    any("Queued for the next turn" in reply for reply in reconnected.replies),
                    "the gateway took the busy /queue path",
                )
                self.assert_ran_once_then_acked(timeline, "msg-2")
            finally:
                await reconnected.close()

        self.run_scenario(scenario)

    def test_a_refused_command_missing_from_the_set_waits_after_one_refusal(self):
        """A pin bump could add a command that becomes a model turn.

        Until the set names it, a drain refuses it once after the rewrite; the
        entry is released, and its redelivery waits for the restart instead
        of being handed over and refused again.
        """

        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline, stall=False)
            h.redeliver_on_release = True
            h.module._HERMES_TURN_COMMANDS = frozenset()
            try:
                other = await h.hold_turn_open(raw_event(1, "other work", segment="s2"))
                h.begin_restart_drain()
                await h.deliver(raw_event(2, "/plan synthetic task"))
                for _ in range(500):
                    if h.redeliveries:
                        break
                    await asyncio.sleep(0.01)
                await h.settle_loop()
                self.assertEqual(h.handed.count("msg-2"), 1)
                self.assertEqual(h.redeliveries, ["msg-2"])
                self.assertNotIn(("ack", "msg-2"), timeline)

                other.set()
                await h.finish_restart()
                self.assertEqual(h.handed.count("msg-2"), 1)
                self.assertEqual(h.state("msg-2"), "pending")
            finally:
                await h.close()

            after = await self.run_after_restart(home, h.inbox, "msg-2")
            self.assert_ran_once_then_acked(after, "msg-2")

        self.run_scenario(scenario)

    def test_busy_steer_follows_its_turn_through_a_graceful_stop(self):
        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline)
            try:
                await h.deliver(raw_event(1, "long running work"))
                await asyncio.wait_for(h.started.wait(), 2)
                await h.deliver(raw_event(2, "/steer change course"))
                self.assertIn(("steer", "change course"), timeline)
                await h.stop_gracefully()
                # The interrupted turn reruns after restart; so does its steer.
                self.assertEqual(h.state("msg-1"), "pending")
                self.assertEqual(h.state("msg-2"), "pending")
            finally:
                await h.close()

            after = await self.run_after_restart(home, h.inbox, "msg-1", "msg-2")
            self.assert_ran_once_then_acked(after, "msg-1", task="long running work")
            self.assert_ran_once_then_acked(after, "msg-2", task="change course")

        self.run_scenario(scenario)

    def test_steer_the_running_agent_cannot_take_waits_for_its_turn(self):
        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline)
            h.steerless.add("long running work")
            try:
                await h.deliver(raw_event(1, "long running work"))
                await asyncio.wait_for(h.started.wait(), 2)
                await h.deliver(raw_event(2, "/steer synthetic task"))
                self.assertEqual(h.handed, ["msg-1"])
                self.assertEqual(h.adapter._pending_messages, {})
                self.assertEqual(h.state("msg-2"), "leased")

                h.turn_gate.set()
                await h.wait_settled("msg-1")
                await h.wait_settled("msg-2")
                await h.settle_loop()
                self.assertNotIn("steer", [kind for kind, _text in timeline])
                self.assert_ran_once_then_acked(timeline, "msg-2")
            finally:
                await h.close()

        self.run_scenario(scenario)

    def test_busy_reject_policy_commands_keep_the_pinned_reply(self):
        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline)
            try:
                await h.deliver(raw_event(1, "long running work"))
                await asyncio.wait_for(h.started.wait(), 2)
                await h.deliver(raw_event(2, "/plan synthetic task"))
                # Hermes answers it at once ("can't run mid-turn"), unchanged.
                self.assertEqual(h.handed, ["msg-1", "msg-2"])
                self.assertEqual(h.state("msg-2"), "acked")
                self.assertTrue(any("/plan" in reply for reply in h.replies), h.replies)
                h.turn_gate.set()
                await h.wait_settled("msg-1")
                self.assertEqual(h.models(), ["long running work"])
            finally:
                await h.close()

        self.run_scenario(scenario)

    def test_user_stop_or_new_during_a_restart_drain_is_final(self):
        for command in ("/stop", "/new"):
            with self.subTest(command=command):

                async def scenario(home: str, command: str = command):
                    timeline: list[tuple[str, str]] = []
                    h = GatewayHarness(home, timeline=timeline)
                    # The stopped turn unwinds while the command's reply is
                    # still being sent, before the base adapter marks the
                    # turn's task cancelled. Nothing is redelivered in this
                    # process, so only the settlement itself is final.
                    h.reply_delay = 0.05
                    try:
                        await h.deliver(raw_event(1, "long running work"))
                        await asyncio.wait_for(h.started.wait(), 2)
                        h.begin_restart_drain()
                        await h.deliver(raw_event(2, command))
                        await h.finish_restart()
                        self.assertNotIn(("release", "msg-1"), timeline)
                        self.assertEqual(h.state("msg-1"), "acked")
                        self.assertEqual(h.state("msg-2"), "acked")
                        self.assertEqual(h.models(), ["long running work"])
                    finally:
                        await h.close()

                    after = await self.run_after_restart(home, h.inbox, "msg-1", "msg-2")
                    self.assertEqual(after, [])

                self.run_scenario(scenario)


if __name__ == "__main__":
    unittest.main()

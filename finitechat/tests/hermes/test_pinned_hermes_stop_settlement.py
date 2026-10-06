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
import socket
import sqlite3
import sys
import tempfile
import textwrap
import threading
import unittest
from collections.abc import Awaitable, Callable, Iterator
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


_LOOPBACK_HOSTS = frozenset({None, "localhost", "127.0.0.1", "::1"})
USAGE_NOTICE = "synthetic usage notice"


@contextlib.contextmanager
def offline_gateway(home: str) -> Iterator[None]:
    """Keep the pinned gateway off the network and out of the real Hermes home.

    Provider resolution reads ambient credentials and walks a fallback chain,
    the gateway constructor downloads the tirith scanner, /restart launches a
    detached ``hermes gateway restart``, and the /goal judge asks an
    auxiliary model after each goal turn (here it finds the goal done).
    ``gateway.run`` fixes its home at import, before a test points
    ``HERMES_HOME`` at a scratch dir.
    """
    real_getaddrinfo = socket.getaddrinfo

    def loopback_only(host: Any, *args: Any, **kwargs: Any) -> Any:
        if host not in _LOOPBACK_HOSTS:
            raise socket.gaierror(socket.EAI_NONAME, "network is disabled in these tests")
        return real_getaddrinfo(host, *args, **kwargs)

    def no_provider() -> dict[str, Any]:
        raise RuntimeError("model providers are disabled in these tests")

    async def no_detached_restart(_runner: Any) -> None:
        return None

    def goal_done(*_args: Any, **_kwargs: Any) -> tuple[str, str, bool, None, bool]:
        return "done", "synthetic judge", False, None, False

    with (
        patch("socket.getaddrinfo", loopback_only),
        patch("gateway.run._hermes_home", Path(home)),
        patch("gateway.run._resolve_runtime_agent_kwargs", no_provider),
        patch("tools.tirith_security.ensure_installed", lambda **_kwargs: None),
        patch("hermes_cli.goals.judge_goal", goal_done),
        patch.object(GatewayRunner, "_launch_detached_restart_command", no_detached_restart),
    ):
        yield


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
            # As the sidecar's cmd_ack and cmd_release: an ack removes the
            # entry whatever its lease, and a release returns only a leased
            # entry to pending.
            if action == "ack":
                self.inbox[message_id] = (raw, "acked")
            elif state == "leased":
                self.inbox[message_id] = (raw, "pending")
            if (
                action == "release"
                and self.state(message_id) == "pending"
                and self.redeliver_on_release
                and self.stream_open
            ):
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

    async def wait_turns_finished(self) -> None:
        """Wait for every background turn to end, past its ack and its completion tail.

        An entry is acked inside its turn's completion hook. The base adapter
        keeps the chat's session guard until the turn's task ends, after the
        hook's usage call and cleanup.
        """

        async def finished():
            while tasks := [task for task in self.adapter._background_tasks if not task.done()]:
                await asyncio.gather(*tasks, return_exceptions=True)

        await asyncio.wait_for(finished(), 5)

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
        # Usage notices held in a finished turn's tail (``hold_turn_tails``).
        self.notices_held = 0
        # Awaited before each reply is sent, once a test sets it.
        self.before_reply: Callable[[str], Awaitable[None]] | None = None
        # Awaited before an ack or release reaches the sidecar, once a test
        # sets it; False makes the sidecar refuse the call. A cancelled call
        # never reaches it.
        self.before_settle: Callable[[str, str], Awaitable[bool]] | None = None
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
        # An in-band restart calls stop() 50ms after the last agent run ends.
        # The gateway's own work after that run can outlast it; the turn is
        # then released and runs once more after the restart, a documented
        # limit. A test that asserts what a turn finishing during the drain
        # settles to sets this, so the restart's stop() waits for it.
        self.restart_after_finished_turns_settle = False
        stop = self.runner.stop

        async def stop_after_finished_turns_settle(*args: Any, **kwargs: Any) -> None:
            if self.restart_after_finished_turns_settle and kwargs.get("restart"):
                await self.wait_turns_finished()
            await stop(*args, **kwargs)

        self.runner.stop = stop_after_finished_turns_settle

    async def _sidecar(self, action, payload, *, timeout):
        if action in ("ack", "release"):
            if self.before_settle is not None and not await self.before_settle(
                action, payload["message_id"]
            ):
                return self.module._FiniteChatResult(False, {}, "synthetic sidecar error", True)
            self.timeline.append((action, payload["message_id"]))
        if action == self.drain_window and payload.get("action", "set") == "set":
            self.begin_restart_drain()
        return await super()._sidecar(action, payload, timeout=timeout)

    async def _send(self, chat_id, content, **kwargs):
        if self.before_reply is not None:
            await self.before_reply(content)
        await asyncio.sleep(self.reply_delay)
        return await super()._send(chat_id, content, **kwargs)

    def hold_turn_tails(self) -> asyncio.Event:
        """Hold each turn that finishes from now on in its tail until the returned gate is set.

        After the completion hook acks a successful turn, it asks Finite
        Private for a usage notice and sends any it gets; the turn is held in
        that send. Its chat stays busy to the adapter while the gateway is
        idle.
        """
        gate = asyncio.Event()
        self.module._finite_private_control_request = lambda *_args: {
            "notice": {"message": USAGE_NOTICE}
        }
        before_reply = self.before_reply

        async def hold_notice(content: str) -> None:
            if content == USAGE_NOTICE:
                self.notices_held += 1
                await gate.wait()
            elif before_reply is not None:
                await before_reply(content)

        self.before_reply = hold_notice
        return gate

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
            offline_gateway(home),
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
            offline_gateway(home),
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
                resume = MessageEvent(text="", source=source, internal=True)
                # The chat's latest turn was an inbox turn, which the inbox
                # redelivers itself.
                h.adapter._record_turn_owner(
                    h.adapter._event_session_key(resume), (ROOM_ID, 1, "msg-1")
                )
                await h.adapter.handle_message(resume)
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
            offline_gateway(home),
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

    async def assert_retried_once_after_restart(
        self, home: str, inbox: dict[str, tuple[dict[str, Any], str]]
    ) -> None:
        timeline: list[tuple[str, str]] = []
        restarted = GatewayHarness(home, timeline=timeline, inbox=inbox, stall=False)
        try:
            await restarted.boot_with_gate_closed()
            await restarted.open_gate()
            await restarted.wait_settled("msg-4")
            await restarted.settle_loop()
            # The last question, once, then the ack; nothing older is retried.
            self.assertEqual(timeline, [("model", "third question"), ("ack", "msg-4")])
            self.assertEqual(chat_transcript(restarted), SEEDED_EXCHANGES)
        finally:
            await restarted.close()

    async def boot_after_restart(
        self, home: str, inbox: dict[str, tuple[dict[str, Any], str]]
    ) -> GatewayHarness:
        restarted = GatewayHarness(home, timeline=[], inbox=inbox, stall=False)
        await restarted.boot_with_gate_closed()
        await restarted.open_gate()
        for _ in range(50):
            await restarted.settle_loop()
        return restarted

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
                    # A registry command the gateway hands the model as text.
                    ("/curator status", "group"): (None, True),
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
                    h.restart_after_finished_turns_settle = True
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
                    h.restart_after_finished_turns_settle = True
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
            h.restart_after_finished_turns_settle = True
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


def pinned_model_text_commands() -> set[str]:
    """Gateway commands the pinned dispatch has no branch for before the drain check.

    ``GatewayRunner._handle_message`` answers every other registry command
    in its own branch, before it refuses new work while draining; these fall
    through to the model as ordinary text.
    """
    from hermes_cli.commands import COMMAND_REGISTRY, is_gateway_known_command

    source = textwrap.dedent(inspect.getsource(GatewayRunner._handle_message))
    tree = ast.parse(source)
    drain_checks = [
        node.lineno
        for node in ast.walk(tree)
        if isinstance(node, ast.If)
        and isinstance(node.test, ast.Attribute)
        and node.test.attr == "_draining"
        and "not accepting new work" in (ast.get_source_segment(source, node) or "")
    ]
    assert len(drain_checks) == 1, drain_checks
    dispatched: set[str] = set()
    for node in ast.walk(tree):
        if not (
            isinstance(node, ast.Compare)
            and isinstance(node.left, ast.Name)
            and node.left.id == "canonical"
            and node.lineno < drain_checks[0]
        ):
            continue
        for comparator in node.comparators:
            elements = (
                comparator.elts if isinstance(comparator, ast.Tuple | ast.Set) else [comparator]
            )
            dispatched.update(
                str(e.value)
                for e in elements
                if isinstance(e, ast.Constant) and isinstance(e.value, str)
            )
    dispatched.update(
        GatewayRunner._gateway_plain_command_handlers(GatewayRunner.__new__(GatewayRunner))
    )
    known = {command.name for command in COMMAND_REGISTRY if is_gateway_known_command(command.name)}
    return known - dispatched


class PinnedHermesModelCommandTests(DrainScenario):
    """Review r4193904833: commands that become a model turn.

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
        # Every other command is answered before the drain check, so the
        # adapter acks it once; these few reach the model as text instead.
        self.assertEqual(
            pinned_model_text_commands(), load_adapter_module()._HERMES_MODEL_TEXT_COMMANDS
        )

    def test_goal_control_forms_match_the_pinned_handler(self):
        """A /goal the adapter keeps live in a turn's tail queues no goal work.

        Each form runs through the pinned idle handler against a paused goal,
        so /goal resume has work to queue. Near-misses set a new goal.
        """
        args = (
            *("", "status", "STATUS", " show ", "pause", "Pause", "clear", "stop", "done"),
            *("wait", "wait 1", "wait 1 build", "unwait", "gate", "gate list", "gate add true"),
            *("resume", "draft a plan", "synthetic task", "status please", "stopper"),
            *("waiting for rain", "wait\t1", "gateway work", "unwait now"),
        )

        async def scenario(home: str):
            from hermes_cli.goals import GoalManager

            h = GatewayHarness(home, timeline=[], stall=False)
            queued: list[str] = []
            enqueue = patch.object(
                h.runner, "_enqueue_fifo", lambda _key, event, _adapter: queued.append(event.text)
            )
            try:
                session_id = chat_session_id(h)
                for arg in args:
                    with self.subTest(arg=arg), enqueue:
                        goal = GoalManager(session_id=session_id)
                        goal.set("synthetic task")
                        goal.pause()
                        queued.clear()
                        event = MessageEvent(text=f"/goal {arg}", source=chat_source(h))
                        await h.runner._handle_goal_command(event)
                        self.assertEqual(h.module._goal_control(event), not queued, queued)
            finally:
                await h.close()

        with patch("hermes_cli.goals.draft_contract", lambda _objective: None):
            self.run_scenario(scenario)

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

    def test_goal_controls_wait_out_a_restart_drain(self):
        """A /goal control waits out a drain with every /goal, then runs once after it.

        Only a running turn's tail keeps a control live; a drain holds it.
        """
        for text, status in (("/goal pause", "paused"), ("/goal status", "active")):
            with self.subTest(text=text):

                async def scenario(home: str, text: str = text, status: str = status):
                    from hermes_cli.goals import GoalManager, load_goal

                    timeline: list[tuple[str, str]] = []
                    h = GatewayHarness(home, timeline=timeline, stall=False)
                    h.redeliver_on_release = True
                    try:
                        sid = chat_session_id(h)
                        GoalManager(session_id=sid).set("synthetic task")
                        # Another chat's running turn keeps the drain open.
                        other = await h.hold_turn_open(raw_event(1, "other work", segment="s2"))
                        h.begin_restart_drain()
                        await h.deliver(raw_event(2, text))
                        self.assertNotIn("msg-2", h.handed)
                        self.assertEqual(h.state("msg-2"), "leased")

                        other.set()
                        await h.finish_restart()
                        self.assertNotIn(("ack", "msg-2"), timeline)
                        self.assertEqual(h.state("msg-2"), "pending")
                        self.assertEqual(getattr(load_goal(sid), "status", None), "active")
                    finally:
                        await h.close()

                    after = await self.run_after_restart(home, h.inbox, "msg-2")
                    self.assertEqual(after.count(("ack", "msg-2")), 1, after)
                    self.assertEqual(getattr(load_goal(sid), "status", None), status)

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
            h.restart_after_finished_turns_settle = True
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
                    h.restart_after_finished_turns_settle = True
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


SEEDED_EXCHANGES = [
    ("user", "first question"),
    ("assistant", "done"),
    ("user", "second question"),
    ("assistant", "done"),
    ("user", "third question"),
    ("assistant", "done"),
]


def chat_source(h: StopHarness) -> Any:
    return h.adapter.build_source(
        chat_id=ROOM_ID, chat_type="group", user_id="alice", thread_id="segment-1"
    )


def chat_session_id(h: StopHarness) -> str:
    return h.runner.session_store.get_or_create_session(chat_source(h)).session_id


def chat_transcript(h: StopHarness) -> list[tuple[str, str]]:
    """The model transcript of the chat ``raw_event`` writes to, as (role, text)."""
    rows = h.runner.session_store.load_transcript(chat_session_id(h))
    return [
        (row["role"], str(row.get("content")))
        for row in rows
        if row.get("role") in ("user", "assistant")
    ]


def durable_chat_transcript(db_path: Path, session_id: str) -> list[tuple[str, str]]:
    """``chat_transcript`` read from state.db itself, through a fresh read-only connection.

    A stopped gateway has closed its session store, so a read through it
    races the close.
    """
    with contextlib.closing(sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)) as db:
        rows = db.execute(
            "SELECT role, content FROM messages"
            " WHERE session_id = ? AND active = 1 AND role IN ('user', 'assistant')"
            " ORDER BY id",
            (session_id,),
        ).fetchall()
    return [(role, str(content)) for role, content in rows]


async def seed_exchanges(h: GatewayHarness) -> None:
    """Three finished exchanges in an idle chat, then a clear timeline."""
    for seq, text in enumerate(("first question", "second question", "third question"), 1):
        await h.deliver(raw_event(seq, text))
        await h.wait_settled(f"msg-{seq}")
        await h.wait_turns_finished()
    assert chat_transcript(h) == SEEDED_EXCHANGES, chat_transcript(h)
    h.timeline.clear()


async def eventually(predicate: Callable[[], bool], timeout: float = 5.0) -> None:
    async def poll() -> None:
        while not predicate():
            await asyncio.sleep(0.01)

    await asyncio.wait_for(poll(), timeout)


class PinnedHermesRetryRewindTests(DrainScenario):
    """Review 5427227655: /retry when a drain or stop begins after it rewinds.

    The pinned ``_handle_retry_command`` runs before the drain check. It
    rewinds the transcript, then re-sends the last message as a new event,
    which a draining gateway refuses. The original /retry text is unchanged,
    so the entry was acked with the last message gone from the transcript.
    A graceful stop at the same point released it instead; after restart it
    rewound again and retried the message before.
    """

    @staticmethod
    def begin_after(h: GatewayHarness, method: str, begin: Callable[[], Awaitable[None]]) -> None:
        """Run ``begin`` once, as the gateway's next ``method`` store call returns."""
        facade = h.runner.async_session_store
        fired: list[str] = []

        async def call(*args: Any, **kwargs: Any) -> Any:
            result = await type(facade).__getattr__(facade, method)(*args, **kwargs)
            if not fired:
                fired.append(method)
                await begin()
            return result

        setattr(facade, method, call)

    def test_retry_runs_the_last_message_once_without_a_drain(self):
        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline, stall=False)
            try:
                await seed_exchanges(h)
                await h.deliver(raw_event(4, "/retry"))
                await h.wait_settled("msg-4")
                await h.settle_loop()
                self.assertEqual(timeline, [("model", "third question"), ("ack", "msg-4")])
                self.assertEqual(chat_transcript(h), SEEDED_EXCHANGES)
            finally:
                await h.close()

        self.run_scenario(scenario)

    def test_retry_keeps_the_last_message_when_a_restart_drain_begins(self):
        for window in ("dispatch", "load_transcript", "rewrite_transcript"):
            with self.subTest(window=window):

                async def scenario(home: str, window: str = window):
                    timeline: list[tuple[str, str]] = []
                    h = GatewayHarness(home, timeline=timeline, stall=False)
                    h.restart_after_finished_turns_settle = True
                    h.redeliver_on_release = True
                    try:
                        await seed_exchanges(h)
                        # Another chat's running turn keeps the drain open.
                        other = await h.hold_turn_open(raw_event(9, "other work", segment="s2"))

                        async def begin_drain() -> None:
                            h.begin_restart_drain()

                        if window == "dispatch":
                            h.drain_window = window
                        else:
                            self.begin_after(h, window, begin_drain)
                        await h.deliver(raw_event(4, "/retry"))
                        await eventually(
                            lambda: bool({("ack", "msg-4"), ("release", "msg-4")} & set(timeline))
                        )
                        await h.settle_loop()
                        self.assertNotIn(("ack", "msg-4"), timeline)
                        self.assertEqual(chat_transcript(h), SEEDED_EXCHANGES)

                        other.set()
                        await h.finish_restart()
                        self.assertEqual(h.models(), ["other work"])
                        self.assertEqual(h.state("msg-4"), "pending")
                    finally:
                        await h.close()

                    await self.assert_retried_once_after_restart(home, h.inbox)

                self.run_scenario(scenario)

    def test_retry_keeps_the_last_message_when_a_graceful_stop_begins(self):
        for window in ("dispatch", "load_transcript", "rewrite_transcript"):
            with self.subTest(window=window):

                async def scenario(home: str, window: str = window):
                    timeline: list[tuple[str, str]] = []
                    h = GatewayHarness(home, timeline=timeline, stall=False)
                    stops: list[asyncio.Task] = []
                    try:
                        await seed_exchanges(h)
                        db_path = Path(home) / "state.db"
                        session_id = chat_session_id(h)

                        async def begin_stop() -> None:
                            stops.append(asyncio.create_task(h.stop_gracefully()))
                            await eventually(lambda: h.runner._draining)

                        if window == "dispatch":
                            processing_start = h.adapter.on_processing_start

                            async def stop_at_dispatch(event):
                                if event.message_id == "msg-4" and not stops:
                                    await begin_stop()
                                await processing_start(event)

                            h.adapter.on_processing_start = stop_at_dispatch
                        else:
                            self.begin_after(h, window, begin_stop)
                        await h.deliver(raw_event(4, "/retry"))
                        await eventually(lambda: bool(stops))
                        await stops[0]
                        await h.settle_loop()
                        self.assertNotIn(("ack", "msg-4"), timeline)
                        self.assertEqual(h.state("msg-4"), "pending")
                        self.assertEqual(h.models(), [])
                        self.assertEqual(
                            durable_chat_transcript(db_path, session_id), SEEDED_EXCHANGES
                        )
                    finally:
                        await h.close()

                    await self.assert_retried_once_after_restart(home, h.inbox)

                self.run_scenario(scenario)

    def test_retry_interrupted_by_a_graceful_stop_runs_once_more(self):
        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline, stall=False)
            try:
                await seed_exchanges(h)
                gate = h.held_turns["third question"] = asyncio.Event()
                await h.deliver(raw_event(4, "/retry"))
                await eventually(lambda: ("model", "third question") in timeline)
                await h.stop_gracefully()
                self.assertTrue(gate.is_set(), "the stop interrupted the retried turn")
                self.assertEqual(h.state("msg-4"), "pending")
            finally:
                await h.close()

            await self.assert_retried_once_after_restart(home, h.inbox)

        self.run_scenario(scenario)

    def test_rewind_still_on_its_way_when_a_stop_cancels_the_retry_never_lands(self):
        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline, stall=False)
            proceed = threading.Event()
            writing: list[str] = []
            written: list[Any] = []
            try:
                await seed_exchanges(h)

                async def hold_the_rewrite() -> None:
                    # The /retry rewrite starts in a worker thread and waits
                    # there until the stop has cancelled and settled the turn.
                    store = h.runner.session_store
                    rewrite = store.rewrite_transcript

                    def held(*args: Any, **kwargs: Any) -> Any:
                        writing.append("rewrite")
                        proceed.wait(10)
                        written.append(rewrite(*args, **kwargs))
                        return written[-1]

                    store.rewrite_transcript = held

                self.begin_after(h, "load_transcript", hold_the_rewrite)
                await h.deliver(raw_event(4, "/retry"))
                await eventually(lambda: bool(writing))
                await h.stop_gracefully()
                self.assertEqual(h.state("msg-4"), "pending")
                proceed.set()
                await eventually(lambda: bool(written))
                self.assertEqual(written, [False], "the late rewind is refused")
                self.assertEqual(chat_transcript(h), SEEDED_EXCHANGES)
            finally:
                proceed.set()
                await h.close()

            await self.assert_retried_once_after_restart(home, h.inbox)

        self.run_scenario(scenario)

    def test_retry_never_overwrites_a_later_transcript_write(self):
        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline, stall=False)
            h.redeliver_on_release = True
            try:
                await seed_exchanges(h)
                other = await h.hold_turn_open(raw_event(9, "other work", segment="s2"))

                async def write_then_drain() -> None:
                    h.runner.session_store.append_to_transcript(
                        chat_session_id(h), {"role": "user", "content": "written elsewhere"}
                    )
                    h.begin_restart_drain()

                self.begin_after(h, "rewrite_transcript", write_then_drain)
                await h.deliver(raw_event(4, "/retry"))
                await h.wait_settled("msg-4")
                await h.settle_loop()
                # The rewind cannot be undone without losing the later write,
                # so the entry is acked rather than rewound again on redelivery.
                expected = [*SEEDED_EXCHANGES[:4], ("user", "written elsewhere")]
                self.assertEqual(h.state("msg-4"), "acked")
                self.assertEqual(chat_transcript(h), expected)
                other.set()
                await h.finish_restart()
            finally:
                await h.close()

            timeline.clear()
            restarted = GatewayHarness(home, timeline=timeline, inbox=h.inbox, stall=False)
            try:
                await restarted.boot_with_gate_closed()
                await restarted.open_gate()
                await restarted.settle_loop()
                self.assertNotIn("msg-4", restarted.handed)
                self.assertEqual(chat_transcript(restarted), expected)
            finally:
                await restarted.close()

        self.run_scenario(scenario)


class PinnedHermesCommandFinalityTests(DrainScenario):
    """A command Hermes answers itself is settled once, even across a stop.

    The release-on-stop rule this release added for interrupted model work
    also released commands that had already taken effect, and a stop that
    cancelled a command's turn while its reply was sending released it too
    (as in 458a). The redelivery after restart ran the command again: a
    second /undo, /yolo turning the approval bypass back on, or /restart
    restarting the gateway again.
    """

    @staticmethod
    def stop_before_reply(h: GatewayHarness) -> list[asyncio.Task]:
        """Begin a graceful stop as the next reply is sent, and send it once stopping."""
        stops: list[asyncio.Task] = []

        async def before_reply(_content: str) -> None:
            if not stops:
                stops.append(asyncio.create_task(h.stop_gracefully()))
                await eventually(h.adapter._gateway_stopping)

        h.before_reply = before_reply
        return stops

    def test_restart_whose_reply_a_stop_cuts_off_does_not_restart_again(self):
        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline, stall=False)
            # The restart stops the gateway while its reply is still sending.
            h.reply_delay = 0.2
            try:
                await h.deliver(raw_event(1, "/restart"))
                await h.finish_restart()
                await h.settle_loop()
                self.assertTrue(h.runner._restart_requested)
                self.assertEqual(h.replies, [], "the stop cancelled the reply send")
                self.assertEqual(h.state("msg-1"), "acked")
                self.assertNotIn(("release", "msg-1"), timeline)
            finally:
                await h.close()

            restarted = await self.boot_after_restart(home, h.inbox)
            try:
                self.assertEqual(restarted.handed, [])
                self.assertFalse(restarted.runner._restart_requested)
            finally:
                await restarted.close()

        self.run_scenario(scenario)

    def test_yolo_finished_during_a_stop_is_not_replayed(self):
        from tools import approval

        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline, stall=False)
            session_key = h.runner._session_key_for_source(chat_source(h))
            stops = self.stop_before_reply(h)
            try:
                await h.deliver(raw_event(1, "/yolo"))
                await eventually(lambda: bool(stops))
                await stops[0]
                self.assertTrue(approval.is_session_yolo_enabled(session_key))
                self.assertEqual(h.state("msg-1"), "acked")
                self.assertNotIn(("release", "msg-1"), timeline)
            finally:
                await h.close()

            # A restarted process starts with no session in yolo mode.
            with patch.object(approval, "_session_yolo", set()):
                restarted = await self.boot_after_restart(home, h.inbox)
                try:
                    self.assertEqual(restarted.handed, [])
                    self.assertFalse(approval.is_session_yolo_enabled(session_key))
                finally:
                    await restarted.close()

        with patch.object(approval, "_session_yolo", set()):
            self.run_scenario(scenario)

    def test_undo_finished_during_a_stop_is_not_replayed(self):
        async def scenario(home: str):
            # Synthetic config: Finite keeps the pinned default, which asks
            # for confirmation before /undo rewinds anything.
            Path(home, "config.yaml").write_text(
                "approvals:\n  destructive_slash_confirm: false\n", encoding="utf-8"
            )
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline, stall=False)
            try:
                await seed_exchanges(h)
                stops = self.stop_before_reply(h)
                await h.deliver(raw_event(4, "/undo"))
                await eventually(lambda: bool(stops))
                await stops[0]
                self.assertEqual(chat_transcript(h), SEEDED_EXCHANGES[:4])
                self.assertEqual(h.state("msg-4"), "acked")
                self.assertNotIn(("release", "msg-4"), timeline)
            finally:
                await h.close()

            restarted = await self.boot_after_restart(home, h.inbox)
            try:
                self.assertEqual(restarted.handed, [])
                self.assertEqual(chat_transcript(restarted), SEEDED_EXCHANGES[:4])
            finally:
                await restarted.close()

        self.run_scenario(scenario)


class PinnedHermesTurnBoundaryTests(DrainScenario):
    """Where a turn's work begins and ends, as its settlement sees it.

    Linux CI job 112252876639 and review 5427930434. The pinned base adapter
    runs the completion hook a second time, as CANCELLED, when a shutdown
    cancels the turn's task after the hook began: in the usage-notice tail
    after the ack, or while a settlement is still on its way. That second run
    released an acked turn, released the steer the turn had taken, and acked
    a /retry the first run had just released, which removes the entry.
    Settling a model-launching command by its unchanged
    text acked one a stop cancelled before Hermes ran it. A command sent while
    the previous turn finished after its ack ran inline, outside any
    background turn, where a drain after a /retry rewind lost the last
    message. A /retry whose re-sent message Hermes queued behind a reserved
    slot kept its rewind when a stop dropped that queue.
    """

    # Each command that can start model work, with the text its model run carries.
    MODEL_COMMANDS = (
        ("/queue synthetic task", "synthetic task"),
        ("/steer synthetic task", "synthetic task"),
        ("/plan synthetic task", "synthetic task"),
        ("/learn synthetic task", "synthetic task"),
        ("/init", "[/init]"),
        ("/blueprint morning-brief", "Morning briefing"),
        ("/moa synthetic task", "synthetic task"),
        ("/goal synthetic task", "synthetic task"),
    )

    @staticmethod
    def stop_here(h: GatewayHarness, stops: list[asyncio.Task]) -> Callable[[], Awaitable[None]]:
        """A step that begins a graceful stop once and waits there until the stop cancels it."""

        async def step() -> None:
            if not stops:
                stops.append(asyncio.create_task(h.stop_gracefully()))
                await eventually(h.adapter._gateway_stopping)
            await asyncio.Event().wait()

        return step

    async def run_command_after_restart(
        self, home: str, inbox: dict[str, tuple[dict[str, Any], str]], task: str
    ) -> list[tuple[str, str]]:
        """Boot over ``inbox`` and run until ``task`` reached the model and the turns ended."""
        timeline: list[tuple[str, str]] = []
        restarted = GatewayHarness(home, timeline=timeline, inbox=inbox, stall=False)
        try:
            await restarted.boot_with_gate_closed()
            await restarted.open_gate()
            await eventually(
                lambda: any(task in text for kind, text in timeline if kind == "model")
            )
            await restarted.wait_turns_finished()
        finally:
            await restarted.close()
        return timeline

    def assert_command_ran_once(
        self, timeline: list[tuple[str, str]], command: str, task: str
    ) -> None:
        runs = [text for kind, text in timeline if kind == "model" and task in text]
        self.assertEqual(len(runs), 1, timeline)
        self.assertEqual(timeline.count(("ack", "msg-1")), 1, timeline)
        self.assertNotIn(("release", "msg-1"), timeline)
        # /goal's kickoff turn settles the /goal entry.
        self.assert_ran_once_then_acked(timeline, "msg-1", task=task)

    def test_a_stop_after_the_ack_leaves_the_finished_turn_acked(self):
        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline, stall=False)
            tail = h.hold_turn_tails()
            try:
                await h.deliver(raw_event(1, "synthetic task"))
                await h.wait_settled("msg-1")
                await eventually(lambda: h.notices_held == 1)
                # The stop cancels the turn's task in its usage-notice send.
                await h.stop_gracefully()
                self.assertEqual(timeline, [("model", "synthetic task"), ("ack", "msg-1")])
                self.assertEqual(h.state("msg-1"), "acked")
            finally:
                tail.set()
                await h.close()

            self.assertEqual(await self.run_after_restart(home, h.inbox), [])

        self.run_scenario(scenario)

    def test_a_stop_after_an_unconfirmed_ack_settles_the_turn_and_its_steer_once(self):
        # "on its way": the stop cancels the turn while its ack is still on
        # its way to the sidecar. "refused": the sidecar refused the ack, and
        # the stop cancels the turn in its tail.
        for case in ("on its way", "refused"):
            with self.subTest(case=case):

                async def scenario(home: str, case: str = case):
                    timeline: list[tuple[str, str]] = []
                    h = GatewayHarness(home, timeline=timeline)
                    sent = asyncio.Event()
                    first_ack: list[str] = []

                    async def first_ack_unconfirmed(action: str, message_id: str) -> bool:
                        if action != "ack" or first_ack:
                            return True
                        first_ack.append(message_id)
                        if case == "refused":
                            return False
                        await sent.wait()
                        return True

                    tail = h.hold_turn_tails() if case == "refused" else asyncio.Event()
                    try:
                        await h.deliver(raw_event(1, "long running work"))
                        await asyncio.wait_for(h.started.wait(), 2)
                        await h.deliver(raw_event(2, "/steer change course"))
                        self.assertIn(("steer", "change course"), timeline)
                        h.before_settle = first_ack_unconfirmed
                        h.turn_gate.set()
                        await eventually(lambda: first_ack == ["msg-1"])
                        if case == "refused":
                            await eventually(lambda: h.notices_held == 1)
                            self.assertEqual(h.state("msg-1"), "leased")
                            await h.stop_gracefully()
                        else:
                            stop = asyncio.create_task(h.stop_gracefully())
                            await eventually(
                                lambda: bool(h.adapter._expected_cancelled_tasks) or stop.done()
                            )
                            sent.set()
                            await stop
                        self.assertEqual(h.state("msg-1"), "acked")
                        self.assertEqual(h.state("msg-2"), "acked")
                        self.assertNotIn(("release", "msg-1"), timeline)
                        self.assertNotIn(("release", "msg-2"), timeline)
                    finally:
                        sent.set()
                        tail.set()
                        await h.close()

                    self.assertEqual(await self.run_after_restart(home, h.inbox), [])

                self.run_scenario(scenario)

    def test_a_released_retry_stays_released_when_the_stop_cancels_its_turn(self):
        # "tail": the stop cancels the turn after it released the /retry, in
        # the usage call. "restore": the stop cancels it while the rewind is
        # still being undone in a worker thread.
        for boundary in ("tail", "restore"):
            with self.subTest(boundary=boundary):

                async def scenario(home: str, boundary: str = boundary):
                    timeline: list[tuple[str, str]] = []
                    h = GatewayHarness(home, timeline=timeline, stall=False)
                    stops: list[asyncio.Task] = []
                    restoring = threading.Event()
                    proceed = threading.Event()
                    try:
                        await seed_exchanges(h)
                        if boundary == "tail":
                            tail = h.hold_turn_tails()
                            proceed.set()
                        else:
                            tail = asyncio.Event()
                            tail.set()
                            restore = h.module._restore_retry_rewind

                            def held_restore(rewind: Any) -> str:
                                restoring.set()
                                proceed.wait(5)
                                return restore(rewind)

                            h.module._restore_retry_rewind = held_restore

                        # The stop reaches the adapter's teardown, which
                        # cancels the turn, only once the turn is there.
                        cancel = h.adapter.cancel_background_tasks

                        async def cancel_at_the_boundary() -> None:
                            if boundary == "tail":
                                await eventually(lambda: h.notices_held == 1)
                            else:
                                await eventually(restoring.is_set)
                            await cancel()

                        h.adapter.cancel_background_tasks = cancel_at_the_boundary

                        async def begin_stop() -> None:
                            stops.append(asyncio.create_task(h.stop_gracefully()))
                            await eventually(lambda: h.runner._draining)

                        PinnedHermesRetryRewindTests.begin_after(
                            h, "rewrite_transcript", begin_stop
                        )
                        await h.deliver(raw_event(4, "/retry"))
                        await eventually(lambda: bool(stops))
                        if boundary == "restore":
                            await eventually(
                                lambda: bool(h.adapter._expected_cancelled_tasks) or stops[0].done()
                            )
                            proceed.set()
                        await stops[0]
                        await h.settle_loop()
                        self.assertNotIn(("ack", "msg-4"), timeline)
                        self.assertEqual(timeline.count(("release", "msg-4")), 1, timeline)
                        self.assertEqual(h.state("msg-4"), "pending")
                        self.assertEqual(chat_transcript(h), SEEDED_EXCHANGES)
                    finally:
                        proceed.set()
                        tail.set()
                        await h.close()

                    await self.assert_retried_once_after_restart(home, h.inbox)

                self.run_scenario(scenario)

    def test_turn_that_finished_before_the_stop_is_acked_when_its_reply_lands_after(self):
        # A turn that finished during a restart drain, or just before a
        # graceful stop, was released and ran again when its completion hook
        # came after stop() began.
        for how in ("restart", "graceful"):
            with self.subTest(how=how):

                async def scenario(home: str, how: str = how):
                    timeline: list[tuple[str, str]] = []
                    h = GatewayHarness(home, timeline=timeline, stall=False)
                    stops: list[asyncio.Task] = []

                    async def reply_once_stopping(_content: str) -> None:
                        if how == "graceful" and not stops:
                            stops.append(asyncio.create_task(h.stop_gracefully()))
                        await eventually(h.adapter._gateway_stopping)

                    # Teardown cancels unsettled turns; this turn settles first.
                    cancel = h.adapter.cancel_background_tasks

                    async def cancel_once_settled() -> None:
                        await h.wait_settled("msg-1")
                        await cancel()

                    h.adapter.cancel_background_tasks = cancel_once_settled
                    try:
                        if how == "restart":
                            gate = await h.hold_turn_open(raw_event(1, "synthetic task"))
                            h.begin_restart_drain()
                            h.before_reply = reply_once_stopping
                            gate.set()
                            await h.finish_restart()
                        else:
                            h.before_reply = reply_once_stopping
                            await h.deliver(raw_event(1, "synthetic task"))
                            await eventually(lambda: bool(stops))
                            await stops[0]
                        self.assertEqual(timeline, [("model", "synthetic task"), ("ack", "msg-1")])
                        self.assertEqual(h.state("msg-1"), "acked")
                    finally:
                        await h.close()

                    self.assertEqual(await self.run_after_restart(home, h.inbox), [])

                self.run_scenario(scenario)

    def test_model_command_a_stop_cancels_before_the_gateway_runs_it_runs_after_restart(self):
        for command, task in self.MODEL_COMMANDS:
            with self.subTest(command=command):

                async def scenario(home: str, command: str = command, task: str = task):
                    timeline: list[tuple[str, str]] = []
                    h = GatewayHarness(home, timeline=timeline, stall=False)
                    stops: list[asyncio.Task] = []
                    step = self.stop_here(h, stops)
                    processing_start = h.adapter.on_processing_start

                    async def stop_at_processing_start(event):
                        if event.message_id == "msg-1":
                            await step()
                        await processing_start(event)

                    h.adapter.on_processing_start = stop_at_processing_start
                    try:
                        await h.deliver(raw_event(1, command))
                        await eventually(lambda: bool(stops))
                        await stops[0]
                        self.assertEqual(h.handed, [])
                        self.assertEqual(timeline, [("release", "msg-1")])
                        self.assertEqual(h.state("msg-1"), "pending")
                    finally:
                        await h.close()

                    after = await self.run_command_after_restart(home, h.inbox, task)
                    self.assert_command_ran_once(after, command, task)

                self.run_scenario(scenario)

    def test_model_command_a_stop_cancels_during_its_acknowledgment_runs_after_restart(self):
        for command in (
            "/plan synthetic task",
            "/learn synthetic task",
            "/init",
            "/blueprint morning-brief",
        ):
            task = dict(self.MODEL_COMMANDS)[command]
            with self.subTest(command=command):

                async def scenario(home: str, command: str = command, task: str = task):
                    timeline: list[tuple[str, str]] = []
                    h = GatewayHarness(home, timeline=timeline, stall=False)
                    stops: list[asyncio.Task] = []
                    step = self.stop_here(h, stops)

                    async def stop_at_the_acknowledgment(_content: str) -> None:
                        await step()

                    h.before_reply = stop_at_the_acknowledgment
                    try:
                        await h.deliver(raw_event(1, command))
                        await eventually(lambda: bool(stops))
                        await stops[0]
                        # The handler had begun, but nothing was rewritten or run.
                        self.assertEqual(h.handed, ["msg-1"])
                        self.assertEqual(h.models(), [])
                        self.assertEqual(timeline, [("release", "msg-1")])
                        self.assertEqual(h.state("msg-1"), "pending")
                    finally:
                        await h.close()

                    after = await self.run_command_after_restart(home, h.inbox, task)
                    self.assert_command_ran_once(after, command, task)

                self.run_scenario(scenario)

    def test_control_a_stop_cancels_before_the_gateway_runs_it_runs_once_after_restart(self):
        from tools import approval

        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline, stall=False)
            session_key = h.runner._session_key_for_source(chat_source(h))
            stops: list[asyncio.Task] = []
            step = self.stop_here(h, stops)
            processing_start = h.adapter.on_processing_start

            async def stop_at_processing_start(event):
                await step()
                await processing_start(event)

            h.adapter.on_processing_start = stop_at_processing_start
            try:
                await h.deliver(raw_event(1, "/yolo"))
                await eventually(lambda: bool(stops))
                await stops[0]
                self.assertFalse(approval.is_session_yolo_enabled(session_key))
                self.assertEqual(timeline, [("release", "msg-1")])
            finally:
                await h.close()

            timeline.clear()
            restarted = GatewayHarness(home, timeline=timeline, inbox=h.inbox, stall=False)
            try:
                await restarted.boot_with_gate_closed()
                await restarted.open_gate()
                await restarted.wait_settled("msg-1")
                await restarted.wait_turns_finished()
                self.assertEqual(restarted.handed, ["msg-1"])
                self.assertEqual(timeline, [("ack", "msg-1")])
                self.assertTrue(approval.is_session_yolo_enabled(session_key))
            finally:
                await restarted.close()

        with patch.object(approval, "_session_yolo", set()):
            self.run_scenario(scenario)

    def test_goal_whose_kickoff_a_stop_drops_kicks_off_after_restart(self):
        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline, stall=False)
            stops = PinnedHermesCommandFinalityTests.stop_before_reply(h)
            try:
                await h.deliver(raw_event(1, "/goal synthetic task"))
                await eventually(lambda: bool(stops))
                await stops[0]
                # The handler saved the goal and queued its kickoff, which the
                # stop dropped: a saved goal alone is not its kickoff, so the
                # entry goes back with it.
                self.assertEqual(h.models(), [])
                self.assertEqual(timeline, [("release", "msg-1")])
                self.assertEqual(h.state("msg-1"), "pending")
            finally:
                await h.close()

            after = await self.run_command_after_restart(home, h.inbox, "synthetic task")
            self.assert_command_ran_once(after, "/goal synthetic task", "synthetic task")

        self.run_scenario(scenario)

    def test_retry_sent_while_the_last_turn_finishes_keeps_its_message(self):
        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline, stall=False)
            h.restart_after_finished_turns_settle = True
            h.redeliver_on_release = True
            fourth = [*SEEDED_EXCHANGES, ("user", "fourth question"), ("assistant", "done")]
            try:
                await seed_exchanges(h)
                other = await h.hold_turn_open(raw_event(9, "other work", segment="s2"))
                tail = h.hold_turn_tails()
                await h.deliver(raw_event(4, "fourth question"))
                await h.wait_settled("msg-4")
                await eventually(lambda: h.notices_held == 1)
                session_key = h.runner._session_key_for_source(chat_source(h))
                # Busy to the adapter, idle to the gateway.
                self.assertTrue(h.adapter._session_is_active(session_key))
                self.assertFalse(h.runner._is_session_running(session_key))

                async def begin_drain() -> None:
                    h.begin_restart_drain()

                PinnedHermesRetryRewindTests.begin_after(h, "rewrite_transcript", begin_drain)
                delivery = asyncio.create_task(h.deliver(raw_event(5, "/retry")))
                await h.settle_loop()
                self.assertNotIn("msg-5", h.handed)
                self.assertEqual(h.state("msg-5"), "leased")

                # Its own turn rewinds, the drain begins, the rewind is undone.
                tail.set()
                await delivery
                await eventually(
                    lambda: bool({("ack", "msg-5"), ("release", "msg-5")} & set(timeline))
                )
                await h.settle_loop()
                self.assertNotIn(("ack", "msg-5"), timeline)
                self.assertEqual(chat_transcript(h), fourth)

                other.set()
                await h.finish_restart()
                self.assertEqual(h.state("msg-5"), "pending")
            finally:
                await h.close()

            after: list[tuple[str, str]] = []
            restarted = GatewayHarness(home, timeline=after, inbox=h.inbox, stall=False)
            try:
                await restarted.boot_with_gate_closed()
                await restarted.open_gate()
                await restarted.wait_settled("msg-5")
                await restarted.wait_turns_finished()
                self.assertEqual(after, [("model", "fourth question"), ("ack", "msg-5")])
                self.assertEqual(chat_transcript(restarted), fourth)
            finally:
                await restarted.close()

        self.run_scenario(scenario)

    def test_model_command_sent_while_the_last_turn_finishes_runs_as_its_own_turn(self):
        from agent.plan_prompt import build_plan_prompt

        cases = {
            "/plan synthetic task": build_plan_prompt("synthetic task"),
            "/moa synthetic task": "synthetic task",
        }
        for command, prompt in cases.items():
            with self.subTest(command=command):

                async def scenario(home: str, command: str = command, prompt: str = prompt):
                    timeline: list[tuple[str, str]] = []
                    h = GatewayHarness(home, timeline=timeline, stall=False)
                    tail = h.hold_turn_tails()
                    gate = h.held_turns[prompt] = asyncio.Event()
                    delivery: asyncio.Task | None = None
                    try:
                        await h.deliver(raw_event(1, "first question"))
                        await h.wait_settled("msg-1")
                        await eventually(lambda: h.notices_held == 1)
                        # A control still answers at once.
                        await h.deliver(raw_event(2, "/status"))
                        self.assertEqual(h.state("msg-2"), "acked")
                        delivery = asyncio.create_task(h.deliver(raw_event(3, command)))
                        await h.settle_loop()
                        self.assertNotIn("msg-3", h.handed)
                        self.assertEqual(h.state("msg-3"), "leased")

                        tail.set()
                        await delivery
                        await eventually(lambda: ("model", prompt) in timeline)
                        # A stop interrupts the command's own turn, which settles it.
                        await h.stop_gracefully()
                        self.assertTrue(gate.is_set())
                        self.assertNotIn(("ack", "msg-3"), timeline)
                        self.assertEqual(h.state("msg-3"), "pending")
                    finally:
                        tail.set()
                        gate.set()
                        if delivery is not None:
                            await asyncio.gather(delivery, return_exceptions=True)
                        await h.close()

                    after = await self.run_after_restart(home, h.inbox, "msg-3")
                    self.assert_ran_once_then_acked(after, "msg-3", task="synthetic task")

                self.run_scenario(scenario)

    def test_goal_control_sent_between_goal_turns_answers_at_once(self):
        """A /goal pause sent between goal turns stops the loop instead of waiting it out.

        The judge keeps the goal going, so the next continuation is queued
        while the kickoff turn finishes after its ack. A control form answers
        there and stops the loop; a goal or other work still waits its turn.
        """
        cases = {
            "/goal pause": "paused",
            "/goal clear": "cleared",
            "/goal status": "live",
            "/goal resume": "held",
            "/goal waiting for rain": "held",
            "/retry": "held",
            "follow-up question": "held",
        }
        for text, expect in cases.items():
            with self.subTest(text=text):

                async def scenario(home: str, text: str = text, expect: str = expect):
                    from hermes_cli.goals import GoalManager

                    timeline: list[tuple[str, str]] = []
                    h = GatewayHarness(home, timeline=timeline, stall=False)
                    verdicts = ["continue"] * 3
                    tail = h.hold_turn_tails()
                    delivery: asyncio.Task | None = None
                    try:
                        with patch(
                            "hermes_cli.goals.judge_goal",
                            lambda *_a, **_k: (
                                verdicts.pop(0) if verdicts else "done",
                                "synthetic judge",
                                False,
                                None,
                                False,
                            ),
                        ):
                            await h.deliver(raw_event(1, "/goal synthetic task"))
                            # The command's own tail, then the kickoff turn's,
                            # which settles the /goal entry.
                            await eventually(lambda: h.notices_held == 1)
                            self.assertEqual(h.state("msg-1"), "leased")
                            tail.set()
                            await eventually(lambda: ("model", "synthetic task") in timeline)
                            tail = h.hold_turn_tails()
                            await eventually(lambda: h.notices_held == 2)
                            self.assertEqual(h.state("msg-1"), "acked")
                            self.assertEqual(len(h.adapter._pending_messages), 1)
                            runs = len(h.models())

                            delivery = asyncio.create_task(h.deliver(raw_event(2, text)))
                            if expect == "held":
                                await eventually(lambda: bool(h.adapter._deferred_admissions))
                                self.assertNotIn("msg-2", h.handed)
                                self.assertEqual(h.state("msg-2"), "leased")
                            else:
                                # Answered while the kickoff turn is still in its tail.
                                await eventually(lambda: "msg-2" in h.inbox)
                                await h.wait_settled("msg-2")
                                self.assertEqual(h.state("msg-2"), "acked")
                                self.assertEqual(h.handed.count("msg-2"), 1)
                                self.assertFalse(tail.is_set())
                            tail.set()
                            await delivery
                            await h.wait_settled("msg-2")
                            await eventually(
                                lambda: not verdicts or expect in ("paused", "cleared")
                            )
                            await h.wait_turns_finished()
                            await h.settle_loop()

                        goal = GoalManager(session_id=chat_session_id(h))
                        if expect in ("paused", "cleared"):
                            self.assertEqual(len(h.models()), runs, timeline)
                            if expect == "paused":
                                self.assertIsNotNone(goal.state)
                                self.assertEqual(getattr(goal.state, "status", None), "paused")
                            else:
                                self.assertFalse(goal.has_goal())
                        else:
                            # The loop went on to the judge's verdict.
                            self.assertEqual(verdicts, [])
                    finally:
                        tail.set()
                        if delivery is not None:
                            await asyncio.gather(delivery, return_exceptions=True)
                        await h.close()

                self.run_scenario(scenario)

    def test_retry_queued_behind_a_reserved_slot_is_undone_when_a_stop_drops_it(self):
        from gateway.run import _AGENT_PENDING_SENTINEL

        async def scenario(home: str):
            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline, stall=False)
            session_key = h.runner._session_key_for_source(chat_source(h))
            queued: list[str] = []
            background = h.adapter._process_message_background

            async def queued_turn_never_starts(event: MessageEvent, key: str) -> None:
                raw = event.raw_message if isinstance(event.raw_message, dict) else {}
                if raw.get("message_id") == "msg-4" and event.text != "/retry":
                    # The stop cancels it before it runs, with no completion hook.
                    queued.append(event.text)
                    await asyncio.Event().wait()
                await background(event, key)

            try:
                await seed_exchanges(h)
                h.adapter._process_message_background = queued_turn_never_starts

                async def reserve_the_slot() -> None:
                    # As the pinned cold path claims it for a competing turn,
                    # right after the rewind: the re-sent message is queued.
                    h.runner._session_state(session_key).turn.agent = _AGENT_PENDING_SENTINEL

                PinnedHermesRetryRewindTests.begin_after(h, "rewrite_transcript", reserve_the_slot)
                await h.deliver(raw_event(4, "/retry"))
                await eventually(lambda: bool(queued))
                h.runner._release_running_agent_state(session_key)
                self.assertEqual(queued, ["third question"])
                self.assertEqual(len(h.adapter._retry_rewinds), 1)
                self.assertEqual(h.state("msg-4"), "leased")
                self.assertEqual(chat_transcript(h), SEEDED_EXCHANGES[:4])

                await h.stop_gracefully()
                self.assertEqual(h.adapter._retry_rewinds, {})
                self.assertEqual(timeline, [("release", "msg-4")])
                self.assertEqual(h.state("msg-4"), "pending")
                self.assertEqual(chat_transcript(h), SEEDED_EXCHANGES)
            finally:
                await h.close()

            await self.assert_retried_once_after_restart(home, h.inbox)

        self.run_scenario(scenario)


class HeldJudge:
    """The /goal judge, giving ``verdicts`` in turn; the first call waits for ``release``.

    A "wait" verdict parks the goal on this process.
    """

    def __init__(self, verdicts: list[str], *, hold: bool = True):
        self.loop = asyncio.get_running_loop()
        self.verdicts = verdicts
        self.calls = 0
        self.entered = asyncio.Event()
        self.release = threading.Event()
        if not hold:
            self.release.set()

    def __call__(self, *_args: Any, **_kwargs: Any) -> tuple[str, str, bool, Any, bool]:
        self.calls += 1
        if self.calls == 1:
            self.loop.call_soon_threadsafe(self.entered.set)
            if not self.release.wait(10):
                raise RuntimeError("the test never released the judge")
        verdict = self.verdicts.pop(0) if self.verdicts else "done"
        directive = {"pid": os.getpid()} if verdict == "wait" else None
        return verdict, "synthetic judge", False, directive, False


@contextlib.contextmanager
def real_agent_runs(h: GatewayHarness, judge: HeldJudge) -> Iterator[None]:
    """Run turns through the real pinned ``_run_agent``; only the model call is synthetic."""
    from gateway import run as gateway_run

    loop = asyncio.get_running_loop()
    h.runner._run_agent = GatewayRunner._run_agent.__get__(h.runner)

    def run_sync(turn: Any) -> dict[str, Any]:
        ctx = turn._ctx

        class Agent:
            def interrupt(self, *_args: Any, **_kwargs: Any) -> None:
                pass

            def hard_interrupt(self, *_args: Any, **_kwargs: Any) -> None:
                pass

            def steer(self, _text: str) -> bool:
                return True

        ctx.agent_holder[0] = Agent()
        loop.call_soon_threadsafe(h.timeline.append, ("model", ctx.message))
        result = {"final_response": "done", "messages": [], "completed": True, "api_calls": 1}
        ctx.result_holder[0] = result
        return result

    with (
        patch.object(gateway_run.TurnRunner, "run_sync", run_sync),
        patch.object(
            gateway_run, "_load_gateway_config", return_value={"display": {"tool_progress": "off"}}
        ),
        patch("agent.model_metadata.fetch_model_metadata", lambda *_a, **_k: {}),
        patch("hermes_cli.goals.judge_goal", judge),
        patch.dict(os.environ, {"HERMES_AGENT_TIMEOUT": "0"}),
    ):
        yield


def on_another_thread(action: Callable[[], Any]) -> None:
    """Run ``action`` to its end on another thread, as the event loop runs a /goal control."""
    worker = threading.Thread(target=action)
    worker.start()
    worker.join(5)


class PinnedHermesGoalJudgeTests(DrainScenario):
    """A /goal control or /subgoal sent while the goal judge runs.

    The pinned runner releases the chat's agent slot before it awaits the
    judge, so the adapter answers a /goal control there at once. The judge's
    GoalManager had loaded the goal before; its verdict wrote that state back,
    reviving a paused or cleared goal and queueing another turn after the
    control's reply. The Finite patch hermes-goal-judge-supersede.patch keeps
    a decision only while the stored goal is still the one judged, judges
    the turn again when a gate or subgoal change leaves the goal running, and
    stops the loop with a notice when the goal store fails. These run the
    real ``_run_agent``, post-turn judge and goal state; only the model call,
    the verdict and transport are synthetic.
    """

    CONTINUING = "↻ Continuing toward goal"
    STORE_FAILED = "⚠ Goal loop stopped"
    # Controls that stop the goal loop wherever they land.
    STOPS = ("/goal pause", "/goal clear", "/goal stop", "/goal wait")

    async def control_goal_loop(
        self, home: str, text: str, when: str, *, parked: bool = False
    ) -> None:
        """Send ``text`` at ``when`` to a goal loop the judge keeps going for one turn.

        "judging": while the first judge call runs. "committed": after the
        judge's state is saved, before the gateway acts on its decision.
        "interleaved": the control loads the goal, the judge saves its state,
        then the control saves its own. "queued": after the continuation is
        queued, before that turn runs. With ``parked``, the judge parks the
        goal instead.
        """
        from hermes_cli import goals

        timeline: list[tuple[str, str]] = []
        h = GatewayHarness(home, timeline=timeline, stall=False)
        stops = parked or text.startswith(self.STOPS)
        # A gate or subgoal change while the judge runs: the turn is judged again.
        rejudged = when == "judging" and not stops and text != "/goal status"
        judge = HeldJudge(
            ["wait" if parked else "continue"] * (2 if rejudged else 1),
            hold=when in ("judging", "interleaved"),
        )
        tail = h.hold_turn_tails() if when == "queued" else asyncio.Event()
        committed, resume = asyncio.Event(), asyncio.Event()
        if when == "committed":
            executor = h.runner._run_in_executor_with_context

            async def hold_after_the_judge(func: Any, *args: Any) -> Any:
                result = await executor(func, *args)
                decision = isinstance(result, dict) and "should_continue" in result
                if decision and not committed.is_set():
                    committed.set()
                    await resume.wait()
                return result

            h.runner._run_in_executor_with_context = hold_after_the_judge
        loop_thread = threading.current_thread()
        interleave: dict[str, Any] = {"armed": False, "fired": False, "saved": []}
        judge_saved = threading.Event()
        load, save, commit = goals.load_goal, goals.save_goal, goals._commit_goal_if_unchanged

        def load_then_let_the_judge_save(session_id: str) -> Any:
            state = load(session_id)
            if interleave["armed"] and threading.current_thread() is loop_thread:
                interleave["armed"], interleave["fired"] = False, True
                judge.release.set()
                judge_saved.wait(5)
            return state

        def save_recording_the_control(session_id: str, state: Any) -> None:
            if interleave["fired"] and threading.current_thread() is loop_thread:
                interleave["saved"].append(state.turns_used)
            save(session_id, state)

        def commit_then_signal(*args: Any) -> bool | None:
            try:
                return commit(*args)
            finally:
                judge_saved.set()

        delivery: asyncio.Task | None = None
        try:
            with contextlib.ExitStack() as stack:
                stack.enter_context(real_agent_runs(h, judge))
                if when == "interleaved":
                    stack.enter_context(
                        patch.object(goals, "load_goal", load_then_let_the_judge_save)
                    )
                    stack.enter_context(
                        patch.object(goals, "save_goal", save_recording_the_control)
                    )
                    stack.enter_context(
                        patch.object(goals, "_commit_goal_if_unchanged", commit_then_signal)
                    )
                await h.deliver(raw_event(1, "/goal synthetic task"))
                sid = chat_session_id(h)
                if when in ("judging", "interleaved"):
                    await asyncio.wait_for(judge.entered.wait(), 5)
                    interleave["armed"] = when == "interleaved"
                elif when == "committed":
                    await asyncio.wait_for(committed.wait(), 5)
                    self.assertEqual(getattr(goals.load_goal(sid), "turns_used", None), 1)
                else:
                    await eventually(lambda: h.notices_held == 1)
                    tail.set()
                    tail = h.hold_turn_tails()
                    await eventually(lambda: h.notices_held == 2)
                    self.assertEqual(len(h.adapter._pending_messages), 1)
                self.assertEqual(h.models(), ["synthetic task"])

                delivery = asyncio.create_task(h.deliver(raw_event(2, text)))
                await eventually(lambda: "msg-2" in h.inbox)
                await h.wait_settled("msg-2")
                # Answered at once, before the judge's decision is acted on.
                self.assertEqual(h.state("msg-2"), "acked")
                if when == "interleaved":
                    # The control saved the goal it loaded before the judge's
                    # save, which had counted the turn.
                    self.assertEqual(interleave["saved"][:1], [0])
                answered = len(h.replies)
                judge.release.set()
                resume.set()
                tail.set()
                await delivery
                await h.wait_settled("msg-1")
                await h.wait_turns_finished()
                for _ in range(20):
                    await h.settle_loop()
                await eventually(lambda: not h.adapter._pending_messages)
                await h.wait_turns_finished()

            goal = goals.load_goal(sid)
            self.assertIsNotNone(goal)
            assert goal is not None
            if not stops:
                # The loop goes on to the judge's verdict, as for a control
                # sent once the continuation is queued.
                self.assertEqual(len(h.models()), 2, timeline)
                self.assertIn(self.CONTINUING, " ".join(h.replies))
                self.assertEqual(judge.calls, 3 if rejudged else 2)
                self.assertEqual(goal.status, "done")
                if text.startswith("/goal gate"):
                    self.assertEqual(len(goal.gates), 1)
                if text.startswith("/subgoal"):
                    self.assertEqual(goal.subgoals, ["extra criterion"])
                if rejudged and text.startswith("/subgoal"):
                    # Judged again on the stored goal, its continuation asks
                    # for the new subgoal.
                    self.assertIn("extra criterion", h.models()[1])
                return
            self.assertEqual(h.models(), ["synthetic task"], timeline)
            self.assertEqual(judge.calls, 1)
            later = " ".join(h.replies[answered:])
            self.assertNotIn(self.CONTINUING, later)
            # Nor a judge notice the control made stale, such as "parked".
            self.assertNotIn("Goal parked —", later)
            self.assertNotIn("Goal parked (judge)", later)
            if when != "queued":
                self.assertNotIn(self.CONTINUING, " ".join(h.replies))
            if text == "/goal unwait":
                self.assertEqual((goal.status, goal.waiting_on_pid), ("active", None))
            elif text == "/goal pause":
                self.assertEqual(goal.status, "paused")
            elif text.startswith("/goal wait"):
                self.assertEqual((goal.status, goal.waiting_on_pid), ("active", os.getpid()))
            else:
                self.assertEqual(goal.status, "cleared")
                self.assertFalse(goals.GoalManager(session_id=sid).has_goal())
        finally:
            judge.release.set()
            resume.set()
            tail.set()
            judge_saved.set()
            if delivery is not None:
                await asyncio.gather(delivery, return_exceptions=True)
            await h.close()

    def test_goal_control_stays_in_force_when_an_earlier_judge_returns(self):
        wait = f"/goal wait {os.getpid()}"
        cases = [
            *(("judging", text) for text in ("/goal pause", "/goal clear", "/goal stop", wait)),
            ("committed", "/goal pause"),
            ("committed", "/goal clear"),
            ("committed", wait),
            ("interleaved", "/goal pause"),
            ("queued", "/goal pause"),
            ("queued", "/goal clear"),
        ]
        for when, text in cases:
            with self.subTest(when=when, text=text):
                self.run_scenario(
                    lambda home, when=when, text=text: self.control_goal_loop(home, text, when)
                )

    def test_unwait_after_the_judge_parks_the_goal_drops_its_parked_notice(self):
        self.run_scenario(
            lambda home: self.control_goal_loop(home, "/goal unwait", "committed", parked=True)
        )

    def test_goal_change_that_leaves_the_goal_running_keeps_the_loop_going(self):
        """A gate or subgoal change while the judge runs no longer stalls the loop.

        The decision made before the change is not written over it. While
        the judge runs, the turn is judged again on the stored goal. After
        the judge's save, the continuation goes on with its notice, as when
        the change lands once the continuation is queued.
        """
        cases = [
            *(("judging", text) for text in ("/goal status", "/goal gate add true")),
            ("judging", "/subgoal extra criterion"),
            ("committed", "/goal gate add true"),
            ("committed", "/subgoal extra criterion"),
            ("interleaved", "/goal gate add true"),
        ]
        for when, text in cases:
            with self.subTest(when=when, text=text):
                self.run_scenario(
                    lambda home, when=when, text=text: self.control_goal_loop(home, text, when)
                )

    def test_new_goal_is_not_judged_on_the_previous_goals_turn(self):
        """A goal set while the judge runs, or after it saves, gets no verdict from that turn."""

        async def scenario(home: str):
            from hermes_cli.goals import GoalManager, load_goal

            sid = "finite-judged-goal"

            def set_new_goal() -> None:
                # Another thread, as the event loop is to the gateway's judge.
                worker = threading.Thread(
                    target=lambda: GoalManager(session_id=sid).set("new synthetic goal")
                )
                worker.start()
                worker.join(5)

            judged: list[str] = []
            race = [True]

            def judge(goal: str, *_args: Any, **_kwargs: Any) -> tuple[str, str, bool, None, bool]:
                judged.append(goal)
                if race[0]:
                    set_new_goal()
                return "continue", "synthetic judge", False, None, False

            with patch("hermes_cli.goals.judge_goal", judge):
                GoalManager(session_id=sid).set("synthetic task")
                decision = GoalManager(session_id=sid).evaluate_after_turn("done")
                self.assertEqual((decision["verdict"], judged), ("superseded", ["synthetic task"]))
                self.assertEqual(getattr(load_goal(sid), "turns_used", None), 0)

                race[0] = False
                GoalManager(session_id=sid).set("synthetic task")
                mgr = GoalManager(session_id=sid)
                decision = mgr.evaluate_after_turn("done")
                self.assertTrue(decision["should_continue"])
                set_new_goal()
                self.assertIsNone(mgr.standing_decision(decision))

        self.run_scenario(scenario)

    def test_judge_holds_its_goal_writes_only_while_it_runs(self):
        """A later goal write on the judge's thread reaches the store, after a verdict or an error.

        The judge's own writes are held while it runs. A hold left in place
        would silently drop the thread's next write for that session, such as
        a compression's goal migration.
        """
        for outcome in ("verdict", "error"):
            with self.subTest(outcome=outcome):

                async def scenario(home: str, outcome: str = outcome):
                    from hermes_cli.goals import GoalManager, load_goal

                    def judge(*_args: Any, **_kwargs: Any) -> tuple[str, str, bool, None, bool]:
                        if outcome == "error":
                            raise RuntimeError("synthetic judge failure")
                        return "continue", "synthetic judge", False, None, False

                    sid = "finite-judged-goal"
                    GoalManager(session_id=sid).set("synthetic task")
                    with patch("hermes_cli.goals.judge_goal", judge):
                        try:
                            GoalManager(session_id=sid).evaluate_after_turn("done")
                        except RuntimeError:
                            self.assertEqual(outcome, "error")
                    GoalManager(session_id=sid).pause()
                    self.assertEqual(getattr(load_goal(sid), "status", None), "paused")

                self.run_scenario(scenario)

    def test_change_while_the_turn_is_judged_again_is_not_written_over(self):
        """A subgoal sent while the judge runs has the turn judged once more, and only once.

        A pause, clear or wait sent during that second judgement stays in
        force, with no further turn. So does a second subgoal; the goal then
        waits for the next message, and the turn is not judged a third time.
        """
        pid = os.getpid()
        controls: dict[str, Callable[[Any], Any]] = {
            "pause": lambda mgr: mgr.pause(),
            "clear": lambda mgr: mgr.clear(),
            "wait": lambda mgr: mgr.wait_on(pid),
            "subgoal": lambda mgr: mgr.add_subgoal("another criterion"),
        }
        stored = {
            "pause": ("paused", None, ["extra criterion"]),
            "clear": ("cleared", None, ["extra criterion"]),
            "wait": ("active", pid, ["extra criterion"]),
            "subgoal": ("active", None, ["extra criterion", "another criterion"]),
        }
        for name, control in controls.items():
            with self.subTest(control=name):

                async def scenario(home: str, name: str = name, control: Any = control) -> None:
                    from hermes_cli import goals

                    sid = "finite-judged-goal"
                    goals.GoalManager(session_id=sid).set("synthetic task")
                    changes = [lambda mgr: mgr.add_subgoal("extra criterion"), control]
                    judged: list[list[str]] = []

                    def judge(
                        _goal: str, _response: str, *, subgoals: Any = None, **_kwargs: Any
                    ) -> tuple[str, str, bool, None, bool]:
                        judged.append(list(subgoals or []))
                        if len(judged) <= len(changes):
                            change = changes[len(judged) - 1]
                            on_another_thread(lambda: change(goals.GoalManager(session_id=sid)))
                        return "continue", "synthetic judge", False, None, False

                    with patch("hermes_cli.goals.judge_goal", judge):
                        mgr = goals.GoalManager(session_id=sid)
                        decision = mgr.evaluate_after_turn("done")
                        acted = mgr.standing_decision(decision) or {}
                    self.assertEqual(judged, [[], ["extra criterion"]])
                    self.assertEqual(decision["verdict"], "superseded")
                    self.assertFalse(acted.get("should_continue"))
                    self.assertFalse(acted.get("message"))
                    goal = goals.load_goal(sid)
                    assert goal is not None
                    self.assertEqual(goal.turns_used, 0)
                    self.assertEqual(
                        (goal.status, goal.waiting_on_pid, goal.subgoals), stored[name]
                    )

                self.run_scenario(scenario)

    def test_gate_added_while_the_judge_runs_is_checked_before_the_goal_completes(self):
        """A failing gate added while the judge says "done" keeps the goal going on its failure."""

        async def scenario(home: str) -> None:
            from hermes_cli import goals

            sid = "finite-judged-goal"
            goals.GoalManager(session_id=sid).set("synthetic task")
            judged: list[str] = []

            def judge(*_args: Any, **_kwargs: Any) -> tuple[str, str, bool, None, bool]:
                judged.append("done")
                if len(judged) == 1:
                    on_another_thread(lambda: goals.GoalManager(session_id=sid).add_gate("false"))
                return "done", "synthetic judge", False, None, False

            with (
                patch("hermes_cli.goals.judge_goal", judge),
                # Outside a git checkout every gate runs; keep git out of it.
                patch("hermes_cli.goals.workspace_fingerprint", lambda *_args: ""),
            ):
                mgr = goals.GoalManager(session_id=sid)
                decision = mgr.evaluate_after_turn("done")
                self.assertIs(mgr.standing_decision(decision), decision)
            # Judged again, the new gate ran first and failed; the judge was not asked again.
            self.assertEqual(judged, ["done"])
            self.assertEqual(
                (decision["verdict"], decision["should_continue"]), ("gate_failed", True)
            )
            goal = goals.load_goal(sid)
            assert goal is not None
            self.assertEqual((goal.status, goal.turns_used), ("active", 1))
            self.assertEqual([(gate.command, gate.attempts) for gate in goal.gates], [("false", 1)])

        self.run_scenario(scenario)

    def test_goal_write_during_the_judges_commit_waits_for_it(self):
        """A pause saved while the judge commits lands after the commit, so it is not written over.

        The judge reads the stored goal back and writes its own state under
        the lock every goal write takes. A pause saved between that read and
        that write would otherwise be lost under the judge's state.
        """

        class WatchedLock:
            """The goal write lock, noting when a writer has to wait for it."""

            def __init__(self) -> None:
                self.lock = threading.Lock()
                self.waited = threading.Event()

            def __enter__(self) -> None:
                if not self.lock.acquire(blocking=False):
                    self.waited.set()
                    self.lock.acquire()

            def __exit__(self, *_exc: Any) -> None:
                self.lock.release()

        async def scenario(home: str) -> None:
            from hermes_cli import goals

            sid = "finite-judged-goal"
            goals.GoalManager(session_id=sid).set("synthetic task")
            lock, load = WatchedLock(), goals.load_goal
            judging, judged = threading.current_thread(), threading.Event()
            pause = threading.Thread(target=lambda: goals.GoalManager(session_id=sid).pause())

            def judge(*_args: Any, **_kwargs: Any) -> tuple[str, str, bool, None, bool]:
                judged.set()
                return "continue", "synthetic judge", False, None, False

            def read_back_while_a_pause_saves(session_id: str) -> Any:
                state = load(session_id)
                if judged.is_set() and threading.current_thread() is judging and not pause.ident:
                    pause.start()
                    # The pause saves at once unless the commit's lock holds it back.
                    for _ in range(500):
                        if not pause.is_alive() or lock.waited.is_set():
                            break
                        pause.join(0.01)
                return state

            with (
                patch("hermes_cli.goals.judge_goal", judge),
                patch.object(goals, "_GOAL_WRITE_LOCK", lock),
                patch.object(goals, "load_goal", read_back_while_a_pause_saves),
            ):
                mgr = goals.GoalManager(session_id=sid)
                decision = mgr.evaluate_after_turn("done")
                pause.join(5)
                self.assertTrue(lock.waited.is_set())
                self.assertIsNone(mgr.standing_decision(decision))
            self.assertEqual(getattr(goals.load_goal(sid), "status", None), "paused")

        self.run_scenario(scenario)

    def test_subgoal_sent_after_the_goals_wait_ran_out_is_judged(self):
        """A subgoal sent while the judge runs, once the goal's wait is over, keeps the loop going.

        The judge clears a wait that is over, but the stored goal still has
        it until the judge's own write. That wait does not park the changed
        goal: the turn is judged again on it.
        """

        async def scenario(home: str) -> None:
            from hermes_cli import goals

            sid = "finite-judged-goal"
            goals.GoalManager(session_id=sid).set("synthetic task")
            goals.GoalManager(session_id=sid).wait_for_seconds(60)
            parked = goals.load_goal(sid)
            assert parked is not None
            parked.waiting_until = 1.0
            goals.save_goal(sid, parked)
            judged: list[list[str]] = []

            def judge(
                _goal: str, _response: str, *, subgoals: Any = None, **_kwargs: Any
            ) -> tuple[str, str, bool, None, bool]:
                judged.append(list(subgoals or []))
                if len(judged) == 1:
                    on_another_thread(
                        lambda: goals.GoalManager(session_id=sid).add_subgoal("extra criterion")
                    )
                return "continue", "synthetic judge", False, None, False

            with patch("hermes_cli.goals.judge_goal", judge):
                mgr = goals.GoalManager(session_id=sid)
                decision = mgr.evaluate_after_turn("done")
                self.assertIs(mgr.standing_decision(decision), decision)
            self.assertEqual(judged, [[], ["extra criterion"]])
            self.assertEqual((decision["verdict"], decision["should_continue"]), ("continue", True))
            goal = goals.load_goal(sid)
            assert goal is not None
            self.assertEqual(
                (goal.turns_used, goal.subgoals, goal.waiting_until), (1, ["extra criterion"], 0.0)
            )

        self.run_scenario(scenario)

    def test_goal_store_failing_while_the_judge_runs_stops_the_loop_with_a_notice(self):
        """A goal store that fails while the judge runs stops the loop and says so; nothing is written.

        A failed read could hide a control saved meanwhile, and a failed
        write saved nothing. With no session DB at all there is nothing to
        write over and the judge's decision stands, as before the patch; the
        gateway then cannot read the goal back and stops the loop the same way.
        """
        for failure in ("read", "write", "unavailable"):
            with self.subTest(failure=failure):

                async def scenario(home: str, failure: str = failure) -> None:
                    from hermes_cli import goals

                    sid = "finite-judged-goal"
                    goals.GoalManager(session_id=sid).set("synthetic task")
                    db = goals._get_session_db()
                    error = sqlite3.OperationalError("synthetic disk I/O error")
                    breaks = {
                        "read": lambda: patch.object(db, "get_meta", side_effect=error),
                        "write": lambda: patch.object(db, "set_meta", side_effect=error),
                        "unavailable": lambda: patch.object(goals, "_get_session_db", lambda: None),
                    }
                    with contextlib.ExitStack() as stack:

                        def judge(*_args: Any, **_kwargs: Any) -> tuple[str, str, bool, None, bool]:
                            stack.enter_context(breaks[failure]())
                            return "continue", "synthetic judge", False, None, False

                        stack.enter_context(patch("hermes_cli.goals.judge_goal", judge))
                        mgr = goals.GoalManager(session_id=sid)
                        decision = mgr.evaluate_after_turn("done")
                        acted = mgr.standing_decision(decision)
                        assert acted is not None
                        self.assertEqual(
                            decision["verdict"],
                            "continue" if failure == "unavailable" else "store_failed",
                        )
                        self.assertEqual(
                            (acted["verdict"], acted["should_continue"]), ("store_failed", False)
                        )
                        self.assertIn(self.STORE_FAILED, acted["message"])
                        self.assertTrue(mgr.still_reports(acted))
                    self.assertEqual(getattr(goals.load_goal(sid), "turns_used", None), 0)

                self.run_scenario(scenario)

    def test_notice_of_a_checked_decision_is_sent_when_the_goal_cannot_be_read(self):
        """A goal read that fails as the judge's notice goes out does not drop the notice.

        The decision was saved and checked, and an unreadable store is no
        sign of a control. Dropping a "Goal achieved" notice would end the
        loop without a word.
        """

        async def scenario(home: str) -> None:
            from hermes_cli import goals

            sid = "finite-judged-goal"
            goals.GoalManager(session_id=sid).set("synthetic task")

            def judge(*_args: Any, **_kwargs: Any) -> tuple[str, str, bool, None, bool]:
                return "done", "synthetic judge", False, None, False

            with patch("hermes_cli.goals.judge_goal", judge):
                mgr = goals.GoalManager(session_id=sid)
                decision = mgr.evaluate_after_turn("done")
            self.assertIs(mgr.standing_decision(decision), decision)
            error = sqlite3.OperationalError("synthetic disk I/O error")
            with patch.object(goals._get_session_db(), "get_meta", side_effect=error):
                self.assertTrue(mgr.still_reports(decision))

        self.run_scenario(scenario)

    def test_control_saved_before_a_failed_goal_read_stays_in_force(self):
        """A control is not undone by a goal read that fails right after it is saved.

        The control lands while the judge runs, then the judge's read-back of
        the goal fails, or the read after it that would judge the turn again.
        Or the control lands after the judge's save, then the gateway's check
        fails to read the goal. Nothing is written over the control and no
        further turn is queued; the loop stops with a notice.
        """
        pid = os.getpid()
        controls: dict[str, Callable[[Any], Any]] = {
            "pause": lambda mgr: mgr.pause(),
            "clear": lambda mgr: mgr.clear(),
            "wait": lambda mgr: mgr.wait_on(pid),
            "gate": lambda mgr: mgr.add_gate("true"),
            "subgoal": lambda mgr: mgr.add_subgoal("extra criterion"),
        }
        for failing in ("commit", "reload", "check"):
            for name, control in controls.items():
                with self.subTest(failing=failing, control=name):

                    async def scenario(
                        home: str, failing: str = failing, control: Any = control
                    ) -> None:
                        from hermes_cli import goals

                        sid = "finite-judged-goal"
                        goals.GoalManager(session_id=sid).set("synthetic task")
                        db = goals._get_session_db()
                        assert db is not None
                        get_meta = db.get_meta
                        # Once armed: how many goal reads succeed before one fails.
                        armed: list[int] = []
                        saved: list[str | None] = []

                        def read_failing_once(key: str) -> Any:
                            if armed and key.startswith("goal:"):
                                if armed[0] == 0:
                                    armed.clear()
                                    raise sqlite3.OperationalError("synthetic disk I/O error")
                                armed[0] -= 1
                            return get_meta(key)

                        def send_control() -> None:
                            on_another_thread(lambda: control(goals.GoalManager(session_id=sid)))
                            saved.append(goals._goal_json(goals.load_goal(sid)))
                            armed.append(1 if failing == "reload" else 0)

                        def judge(*_args: Any, **_kwargs: Any) -> tuple[str, str, bool, None, bool]:
                            if failing != "check":
                                send_control()
                            return "continue", "synthetic judge", False, None, False

                        with (
                            patch("hermes_cli.goals.judge_goal", judge),
                            patch.object(db, "get_meta", read_failing_once),
                        ):
                            mgr = goals.GoalManager(session_id=sid)
                            decision = mgr.evaluate_after_turn("done")
                            if failing == "check":
                                send_control()
                            acted = mgr.standing_decision(decision)
                        self.assertEqual(armed, [])
                        assert acted is not None
                        self.assertEqual(
                            (acted["verdict"], acted["should_continue"]), ("store_failed", False)
                        )
                        self.assertEqual(goals._goal_json(goals.load_goal(sid)), saved[0])

                    self.run_scenario(scenario)

    def test_goal_store_failing_after_the_turn_stops_the_loop_with_a_notice(self):
        """Through the gateway: a goal read that fails after a goal turn stops the loop, saying so.

        The judge's read-back fails, or the gateway's check after the judge's
        save does. A /goal pause acked just before stays, and no further turn
        runs; without one, the goal stays as last saved.
        """
        for failing in ("commit", "check"):
            for text in ("/goal pause", None):
                with self.subTest(failing=failing, control=text):
                    self.run_scenario(
                        lambda home, failing=failing, text=text: self.fail_goal_read(
                            home, failing, text
                        )
                    )

    async def fail_goal_read(self, home: str, failing: str, text: str | None) -> None:
        from hermes_cli import goals

        h = GatewayHarness(home, timeline=[], stall=False)
        judge = HeldJudge(["continue"], hold=failing == "commit")
        loop_thread = threading.current_thread()
        fail = threading.Event()
        committed, resume = asyncio.Event(), asyncio.Event()
        if failing == "check":
            executor = h.runner._run_in_executor_with_context

            async def hold_after_the_judge(func: Any, *args: Any) -> Any:
                result = await executor(func, *args)
                if isinstance(result, dict) and "should_continue" in result:
                    committed.set()
                    await resume.wait()
                return result

            h.runner._run_in_executor_with_context = hold_after_the_judge
        delivery: asyncio.Task | None = None
        try:
            with real_agent_runs(h, judge):
                await h.deliver(raw_event(1, "/goal synthetic task"))
                sid = chat_session_id(h)
                db = goals._get_session_db()
                assert db is not None
                get_meta = db.get_meta

                def read_failing_once(key: str) -> Any:
                    # The judge reads on the executor, the gateway's check on the loop.
                    on_loop = threading.current_thread() is loop_thread
                    if (
                        fail.is_set()
                        and key.startswith("goal:")
                        and on_loop == (failing == "check")
                    ):
                        fail.clear()
                        raise sqlite3.OperationalError("synthetic disk I/O error")
                    return get_meta(key)

                with patch.object(db, "get_meta", read_failing_once):
                    await asyncio.wait_for(
                        (judge.entered if failing == "commit" else committed).wait(), 5
                    )
                    if text is not None:
                        delivery = asyncio.create_task(h.deliver(raw_event(2, text)))
                        await eventually(lambda: "msg-2" in h.inbox)
                        await h.wait_settled("msg-2")
                        self.assertEqual(h.state("msg-2"), "acked")
                    answered = len(h.replies)
                    fail.set()
                    judge.release.set()
                    resume.set()
                    if delivery is not None:
                        await delivery
                    await h.wait_settled("msg-1")
                    await h.wait_turns_finished()
                    for _ in range(20):
                        await h.settle_loop()
                    await h.wait_turns_finished()
                self.assertFalse(fail.is_set())

            self.assertEqual(h.models(), ["synthetic task"])
            self.assertEqual(judge.calls, 1)
            self.assertNotIn(self.CONTINUING, " ".join(h.replies))
            self.assertIn(self.STORE_FAILED, " ".join(h.replies[answered:]))
            goal = goals.load_goal(sid)
            self.assertEqual(
                (getattr(goal, "status", None), getattr(goal, "turns_used", None)),
                ("paused" if text else "active", 0 if failing == "commit" else 1),
            )
        finally:
            judge.release.set()
            resume.set()
            if delivery is not None:
                await asyncio.gather(delivery, return_exceptions=True)
            await h.close()

    def test_another_chats_goal_control_leaves_the_judged_goal_going(self):
        async def scenario(home: str):
            from hermes_cli.goals import GoalManager, load_goal

            timeline: list[tuple[str, str]] = []
            h = GatewayHarness(home, timeline=timeline, stall=False)
            judge = HeldJudge(["continue"])
            other = raw_event(2, "/goal clear", segment="segment-2")
            delivery: asyncio.Task | None = None
            try:
                with real_agent_runs(h, judge):
                    other_source = h.adapter.build_source(
                        chat_id=ROOM_ID, chat_type="group", user_id="alice", thread_id="segment-2"
                    )
                    other_sid = h.runner.session_store.get_or_create_session(
                        other_source
                    ).session_id
                    GoalManager(session_id=other_sid).set("another task")
                    await h.deliver(raw_event(1, "/goal synthetic task"))
                    sid = chat_session_id(h)
                    self.assertNotEqual(sid, other_sid)
                    await asyncio.wait_for(judge.entered.wait(), 5)

                    delivery = asyncio.create_task(h.deliver(other))
                    await eventually(lambda: "msg-2" in h.inbox)
                    await h.wait_settled("msg-2")
                    self.assertEqual(getattr(load_goal(other_sid), "status", None), "cleared")
                    judge.release.set()
                    await delivery
                    await h.wait_settled("msg-1")
                    await eventually(lambda: judge.calls == 2)
                    await h.wait_turns_finished()
                    await h.settle_loop()

                self.assertEqual(len(h.models()), 2, timeline)
                self.assertEqual(getattr(load_goal(sid), "status", None), "done")
                self.assertEqual(getattr(load_goal(other_sid), "status", None), "cleared")
                self.assertIn("✓ Goal achieved: synthetic judge", h.replies)
            finally:
                judge.release.set()
                if delivery is not None:
                    await asyncio.gather(delivery, return_exceptions=True)
                await h.close()

        self.run_scenario(scenario)


if __name__ == "__main__":
    unittest.main()

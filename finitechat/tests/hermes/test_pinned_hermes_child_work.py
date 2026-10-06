"""Accepted /bg and /btw work keeps its inbox entry until it delivers, against the pinned gateway.

Review 5429204449 on #1069 and the round 09 reviews: the pinned handlers reply
as soon as they start a child task, which sends the result to the chat
itself. The command's entry was acked with that reply, so a graceful stop or
an in-band restart cancelled the child with no result, no notice and nothing
to replay, and the rollout's idle gate read the chat as idle while the child
ran. The entry now stays leased until the child has delivered its result:
a stop releases it, the command runs again after the restart, and a lease
expiry redelivery joins the running child instead of starting another.

These tests drive the real pinned handlers and the adapter's own ``send``.
Only the /bg agent, the /btw side question, the model, transport and sidecar
are synthetic. Each restart boots a new gateway over the same home and inbox.
"""

import asyncio
import contextlib
import importlib.util
import json
import shutil
import tempfile
import threading
import time
from collections.abc import Callable, Iterator
from pathlib import Path
from typing import Any
from unittest.mock import patch

from gateway.run import GatewayRunner

from tests.hermes.test_pinned_hermes_stop_settlement import (
    ROOM_ID,
    DrainScenario,
    GatewayHarness,
    eventually,
    raw_event,
)

IDLE_GATE = Path(__file__).resolve().parents[3] / "scripts" / "finite_status_runtime_idle.py"
BG_RESULT = "BG-RESULT"
BTW_ANSWER = "BTW-ANSWER"
COMMANDS = {
    "bg": "/bg synthetic task",
    "btw": "/btw what did we decide?",
}


def is_result(command: str, text: str) -> bool:
    """``text`` is the child's result, as the pinned handler words it.

    The pinned /btw handler words it through its message catalog, which
    renders the key itself where the runtime ships no catalog.
    """
    if command == "bg":
        return BG_RESULT in text
    return BTW_ANSWER in text or text.startswith("gateway.btw.answer")


class SyntheticChildren:
    """The /bg agent and the /btw side question, held in the gateway's worker threads."""

    def __init__(self) -> None:
        # Set: children answer at once. Clear: they wait for it, or for
        # their own gate in ``gates``, by prompt.
        self.gate = threading.Event()
        self.gates: dict[str, threading.Event] = {}
        self.started: dict[str, list[str]] = {"bg": [], "btw": []}
        # What the /bg agent answers.
        self.bg_response = BG_RESULT
        self._lock = threading.Lock()

    def runs(self, command: str) -> int:
        with self._lock:
            return len(self.started[command])

    def _start(self, command: str, text: str) -> None:
        with self._lock:
            self.started[command].append(text)
        self.gates.get(text, self.gate).wait(10)

    @contextlib.contextmanager
    def patched(self) -> Iterator[None]:
        children = self

        class Agent:
            def __init__(self, **_kwargs: Any) -> None:
                pass

            def run_conversation(self, user_message: str = "", **_kwargs: Any) -> dict[str, Any]:
                children._start("bg", user_message)
                return {"final_response": children.bg_response, "messages": []}

        def side_question(question: str, _history: Any, **_kwargs: Any) -> str:
            children._start("btw", question)
            return BTW_ANSWER

        with (
            patch("run_agent.AIAgent", Agent),
            patch("agent.side_question.answer_side_question", side_question),
        ):
            yield


class ChildHarness(GatewayHarness):
    """GatewayHarness whose replies go through the adapter's real ``send``.

    A child delivers its result with ``adapter.send``, which is where the
    adapter learns whether that delivery reached the sidecar.
    """

    def __init__(self, home: str, *, timeline: list[tuple[str, str]], **kwargs: Any):
        super().__init__(home, timeline=timeline, stall=False, **kwargs)
        del self.adapter.send
        # Decides a send's fate by its text: None delivers it, otherwise the
        # sidecar refuses it, retryably (True) or for good (False).
        self.refuse: Callable[[str], bool | None] | None = None
        # The route each delivered reply carried: (room, conversation, segment, thread).
        self.routes: list[tuple[str, ...]] = []
        runtime = {
            "api_key": "synthetic",
            "provider": "synthetic",
            "base_url": "http://127.0.0.1:9",
            "api_mode": "chat_completions",
        }
        self.runner._resolve_session_agent_runtime = lambda **_kw: (
            "synthetic/model",
            dict(runtime),
        )
        self.runner._resolve_turn_agent_config = lambda user_message, model, runtime_kwargs: {
            "model": model,
            "runtime": dict(runtime_kwargs),
            "request_overrides": None,
        }
        self.runner._cleanup_agent_resources = lambda *_a, **_k: None

    async def _sidecar(self, action, payload, *, timeout):
        if action != "send":
            return await super()._sidecar(action, payload, timeout=timeout)
        text = str(payload.get("text") or "")
        if self.before_reply is not None:
            await self.before_reply(text)
        await asyncio.sleep(self.reply_delay)
        refused = self.refuse(text) if self.refuse is not None else None
        if refused is not None:
            self.timeline.append(("refused", text))
            return self.module._FiniteChatResult(False, {}, "synthetic send failure", refused)
        self.replies.append(text)
        self.timeline.append(("reply", text))
        self.routes.append(
            tuple(
                str(payload.get(field) or "")
                for field in ("room_id", "conversation_id", "segment_id", "thread_id")
            )
        )
        return self.module._FiniteChatResult(
            True, {"message_id": f"reply-{len(self.replies)}"}, None, False
        )

    def results(self, command: str) -> list[str]:
        return [text for text in self.replies if is_result(command, text)]

    async def seed(self) -> None:
        """One finished exchange, so /btw has history, then a clear timeline."""
        await self.deliver(raw_event(1, "first question"))
        await self.wait_settled("msg-1")
        await self.wait_turns_finished()
        self.timeline.clear()
        self.replies.clear()
        self.routes.clear()


def idle_gate_report(h: ChildHarness, home: str) -> dict[str, Any]:
    """What ``finite-status --runtime-idle`` reads from this runtime right now.

    Hermes's own ``_persist_active_agents`` writes ``gateway_state.json``,
    and the inbox file mirrors the harness sidecar's leases.
    """
    from gateway.status import write_runtime_status

    spec = importlib.util.spec_from_file_location("finite_status_runtime_idle", IDLE_GATE)
    assert spec is not None and spec.loader is not None, IDLE_GATE
    idle = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(idle)
    write_runtime_status(gateway_state="running", active_agents=0)
    GatewayRunner._persist_active_agents(h.runner)
    now_ms = int(time.time() * 1000)
    events = []
    for message_id, (_raw, state) in h.inbox.items():
        if state == "pending":
            events.append({"key": message_id, "created_at_ms": now_ms})
        elif state == "leased":
            lease = {"state": "leased", "lease_id": "synthetic", "leased_at_ms": now_ms}
            events.append({"key": message_id, "created_at_ms": now_ms, "lease": lease})
    with tempfile.TemporaryDirectory(prefix="finite-idle-root-") as root:
        agent = Path(root) / "agent"
        (agent / "hermes-home").mkdir(parents=True)
        shutil.copyfile(
            Path(home) / "gateway_state.json", agent / "hermes-home" / "gateway_state.json"
        )
        (agent / "hermes-inbox.json").write_text(
            json.dumps({"events": events, "acked": [], "cursors": {}}), encoding="utf-8"
        )
        return idle.observe(Path(root), now_ms)


class ChildWorkScenario(DrainScenario):
    def run_scenario(self, scenario):
        self.children = SyntheticChildren()
        with (
            self.children.patched(),
            patch("agent.model_metadata.fetch_model_metadata", lambda *_a, **_k: {}),
        ):
            try:
                super().run_scenario(scenario)
            finally:
                self.children.gate.set()

    async def launch(self, h: ChildHarness, command: str) -> None:
        """Send the command as msg-2 and wait until its child is running past its reply."""
        runs = self.children.runs(command)
        await h.deliver(raw_event(2, COMMANDS[command]))
        await eventually(lambda: self.children.runs(command) > runs)
        await h.wait_turns_finished()
        await h.settle_loop()

    async def stop(self, h: ChildHarness, how: str) -> None:
        if how == "stop":
            await h.stop_gracefully()
        else:
            h.begin_restart_drain()
            await h.finish_restart()

    async def restart_and_deliver(
        self, home: str, inbox: dict[str, tuple[dict[str, Any], str]], command: str
    ) -> ChildHarness:
        """Boot a new gateway over ``inbox``, with children answering at once, until msg-2 settles."""
        self.children.gate.set()
        restarted = ChildHarness(home, timeline=[], inbox=inbox)
        await restarted.boot_with_gate_closed()
        await restarted.open_gate()
        await restarted.wait_settled("msg-2")
        await restarted.wait_turns_finished()
        await restarted.settle_loop()
        return restarted

    def assert_handed_back(self, h: ChildHarness, command: str) -> None:
        """The stop released msg-2 whole: the original command, on its chat and topic."""
        self.assertEqual(h.state("msg-2"), "pending", h.timeline)
        self.assertEqual(h.timeline.count(("release", "msg-2")), 1, h.timeline)
        self.assertNotIn(("ack", "msg-2"), h.timeline)
        self.assertEqual(h.results(command), [])
        raw, _state = h.inbox["msg-2"]
        self.assertEqual(raw["text"], COMMANDS[command])
        self.assertEqual((raw["room_id"], raw["segment_id"]), (ROOM_ID, "segment-1"))

    def assert_delivered_once(self, h: ChildHarness, command: str) -> None:
        """The child's result reached msg-2's topic once, and only then was msg-2 acked."""
        results = h.results(command)
        self.assertEqual(len(results), 1, h.timeline)
        self.assertEqual(h.timeline.count(("ack", "msg-2")), 1, h.timeline)
        self.assertNotIn(("release", "msg-2"), h.timeline)
        self.assertLess(h.timeline.index(("reply", results[0])), h.timeline.index(("ack", "msg-2")))
        route = h.routes[h.replies.index(results[0])]
        self.assertEqual((route[0], route[3] or route[2]), (ROOM_ID, "segment-1"))


class PinnedHermesChildWorkTests(ChildWorkScenario):
    def test_the_child_result_settles_the_command_entry(self):
        for command in COMMANDS:
            with self.subTest(command=command):

                async def scenario(home: str, command: str = command):
                    h = ChildHarness(home, timeline=[])
                    try:
                        await h.seed()
                        await self.launch(h, command)
                        # The launch reply only.
                        self.assertEqual(len(h.replies), 1, h.replies)
                        self.assertFalse(is_result(command, h.replies[0]))
                        # The command's turn is over and its child still runs.
                        self.assertEqual(h.state("msg-2"), "leased", h.timeline)
                        self.assertNotIn(("ack", "msg-2"), h.timeline)
                        self.children.gate.set()
                        await h.wait_settled("msg-2")
                        await h.settle_loop()
                        self.assertEqual(h.state("msg-2"), "acked")
                        self.assert_delivered_once(h, command)
                        self.assertEqual(self.children.runs(command), 1)
                    finally:
                        await h.close()

                self.run_scenario(scenario)

    def test_rollout_idle_gate_reads_a_running_child_as_busy(self):
        for command in COMMANDS:
            with self.subTest(command=command):

                async def scenario(home: str, command: str = command):
                    h = ChildHarness(home, timeline=[])
                    try:
                        await h.seed()
                        self.assertEqual(idle_gate_report(h, home)["verdict"], "idle")
                        await self.launch(h, command)
                        # Hermes itself counts no agent for the child.
                        self.assertEqual(h.runner._active_work_count(), 0)
                        report = idle_gate_report(h, home)
                        self.assertEqual(report["verdict"], "busy", report)
                        self.assertEqual(report["reasons"], ["inbox_leased"], report)
                        self.children.gate.set()
                        await h.wait_settled("msg-2")
                        await h.settle_loop()
                        self.assertEqual(idle_gate_report(h, home)["verdict"], "idle")
                    finally:
                        await h.close()

                self.run_scenario(scenario)

    def test_a_stop_while_the_child_runs_hands_the_command_back(self):
        for command in COMMANDS:
            for how in ("stop", "restart"):
                with self.subTest(command=command, how=how):

                    async def scenario(home: str, command: str = command, how: str = how):
                        h = ChildHarness(home, timeline=[])
                        try:
                            await h.seed()
                            await self.launch(h, command)
                            await self.stop(h, how)
                            self.assert_handed_back(h, command)
                        finally:
                            await h.close()

                        restarted = await self.restart_and_deliver(home, h.inbox, command)
                        try:
                            self.assertEqual(restarted.handed, ["msg-2"])
                            self.assert_delivered_once(restarted, command)
                            self.assertEqual(self.children.runs(command), 2)
                        finally:
                            await restarted.close()

                    self.run_scenario(scenario)

    def test_a_stop_during_the_launch_reply_hands_the_command_back(self):
        for command in COMMANDS:
            for how in ("stop", "restart"):
                with self.subTest(command=command, how=how):

                    async def scenario(home: str, command: str = command, how: str = how):
                        h = ChildHarness(home, timeline=[])
                        stops: list[asyncio.Task] = []

                        async def stop_at_the_launch_reply(text: str) -> None:
                            if not is_result(command, text) and not stops:
                                if how == "stop":
                                    stops.append(asyncio.create_task(h.stop_gracefully()))
                                else:
                                    h.begin_restart_drain()
                                    stops.append(asyncio.create_task(h.finish_restart()))
                                await asyncio.Event().wait()

                        try:
                            await h.seed()
                            h.before_reply = stop_at_the_launch_reply
                            await h.deliver(raw_event(2, COMMANDS[command]))
                            await eventually(lambda: bool(stops))
                            await stops[0]
                            self.assertEqual(h.handed, ["msg-1", "msg-2"])
                            self.assert_handed_back(h, command)
                        finally:
                            await h.close()

                        restarted = await self.restart_and_deliver(home, h.inbox, command)
                        try:
                            self.assert_delivered_once(restarted, command)
                        finally:
                            await restarted.close()

                    self.run_scenario(scenario)

    def test_btw_stopped_before_it_starts_its_child_runs_after_restart(self):
        async def scenario(home: str):
            h = ChildHarness(home, timeline=[])
            stops: list[asyncio.Task] = []
            store = h.runner.async_session_store
            load_transcript = store.load_transcript
            armed = False

            async def stop_in_the_handler(*args: Any, **kwargs: Any) -> Any:
                if armed and not stops:
                    stops.append(asyncio.create_task(h.stop_gracefully()))
                    await asyncio.Event().wait()
                return await load_transcript(*args, **kwargs)

            try:
                with patch.object(store, "load_transcript", stop_in_the_handler):
                    await h.seed()
                    armed = True
                    # The handler reads the transcript before it starts the child.
                    await h.deliver(raw_event(2, COMMANDS["btw"]))
                    await eventually(lambda: bool(stops))
                    await stops[0]
                self.assertEqual(self.children.runs("btw"), 0)
                self.assert_handed_back(h, "btw")
            finally:
                await h.close()

            restarted = await self.restart_and_deliver(home, h.inbox, "btw")
            try:
                self.assert_delivered_once(restarted, "btw")
                self.assertEqual(self.children.runs("btw"), 1)
            finally:
                await restarted.close()

        self.run_scenario(scenario)

    def test_a_result_already_sending_at_a_stop_is_delivered_once(self):
        for command in COMMANDS:
            with self.subTest(command=command):

                async def scenario(home: str, command: str = command):
                    h = ChildHarness(home, timeline=[])
                    stops: list[asyncio.Task] = []

                    async def stop_while_sending(text: str) -> None:
                        if is_result(command, text) and not stops:
                            stops.append(asyncio.create_task(h.stop_gracefully()))
                            await eventually(h.adapter._gateway_stopping)
                            # The send lands while the adapter disconnects.
                            await asyncio.sleep(0.2)

                    try:
                        await h.seed()
                        await self.launch(h, command)
                        h.before_reply = stop_while_sending
                        self.children.gate.set()
                        await eventually(lambda: bool(stops))
                        await stops[0]
                        self.assertEqual(h.state("msg-2"), "acked")
                        self.assert_delivered_once(h, command)
                    finally:
                        await h.close()

                    restarted = await self.boot_after_restart(home, h.inbox)
                    try:
                        self.assertEqual(restarted.handed, [])
                        self.assertEqual(self.children.runs(command), 1)
                    finally:
                        await restarted.close()

                self.run_scenario(scenario)

    def test_a_result_send_cut_off_by_a_stop_runs_the_command_again(self):
        async def scenario(home: str):
            h = ChildHarness(home, timeline=[])
            stops: list[asyncio.Task] = []

            async def stop_and_hang(text: str) -> None:
                if is_result("bg", text):
                    if not stops:
                        stops.append(asyncio.create_task(h.stop_gracefully()))
                    await asyncio.Event().wait()

            try:
                await h.seed()
                await self.launch(h, "bg")
                h.before_reply = stop_and_hang
                self.children.gate.set()
                await eventually(lambda: bool(stops))
                await stops[0]
                self.assert_handed_back(h, "bg")
            finally:
                await h.close()

            restarted = await self.restart_and_deliver(home, h.inbox, "bg")
            try:
                self.assert_delivered_once(restarted, "bg")
                self.assertEqual(self.children.runs("bg"), 2)
            finally:
                await restarted.close()

        self.run_scenario(scenario)

    def test_a_delivered_result_is_not_run_again_when_a_stop_cuts_off_its_attachment(self):
        async def scenario(home: str):
            h = ChildHarness(home, timeline=[])
            stops: list[asyncio.Task] = []
            report = Path(home) / "report.txt"
            report.write_text("synthetic report", encoding="utf-8")
            self.children.bg_response = f"{BG_RESULT}\nMEDIA:{report}"

            async def stop_at_the_attachment(text: str) -> None:
                # The attachment follows the text, with no caption.
                if not text and h.results("bg"):
                    if not stops:
                        stops.append(asyncio.create_task(h.stop_gracefully()))
                    await asyncio.Event().wait()

            try:
                await h.seed()
                await self.launch(h, "bg")
                h.before_reply = stop_at_the_attachment
                self.children.gate.set()
                await eventually(lambda: bool(stops))
                await stops[0]
                # Running it again would repeat the finished agent run and its text.
                self.assertEqual(h.state("msg-2"), "acked")
                self.assert_delivered_once(h, "bg")
            finally:
                await h.close()

            restarted = await self.boot_after_restart(home, h.inbox)
            try:
                self.assertEqual(restarted.handed, [])
                self.assertEqual(self.children.runs("bg"), 1)
            finally:
                await restarted.close()

        self.run_scenario(scenario)

    def test_a_lease_expiry_redelivery_joins_the_running_child(self):
        for command in COMMANDS:
            with self.subTest(command=command):

                async def scenario(home: str, command: str = command):
                    h = ChildHarness(home, timeline=[])
                    try:
                        await h.seed()
                        await self.launch(h, command)
                        # The sidecar re-leases the expired entry to the stream.
                        await h.deliver(h.inbox["msg-2"][0])
                        await h.wait_turns_finished()
                        await h.settle_loop()
                        self.assertEqual(h.handed, ["msg-1", "msg-2"])
                        self.assertEqual(h.state("msg-2"), "leased")
                        self.children.gate.set()
                        await h.wait_settled("msg-2")
                        await h.settle_loop()
                        self.assert_delivered_once(h, command)
                        self.assertEqual(self.children.runs(command), 1)
                    finally:
                        await h.close()

                self.run_scenario(scenario)

    def test_an_undelivered_result_keeps_the_entry_for_redelivery(self):
        for retryable in (True, False):
            with self.subTest(retryable=retryable):

                async def scenario(home: str, retryable: bool = retryable):
                    h = ChildHarness(home, timeline=[])
                    try:
                        await h.seed()
                        await self.launch(h, "bg")
                        h.refuse = lambda text: retryable if is_result("bg", text) else None
                        self.children.gate.set()
                        await eventually(lambda: any(kind == "refused" for kind, _ in h.timeline))
                        await h.settle_loop()
                        if not retryable:
                            # A send the sidecar refuses for good never succeeds.
                            await h.wait_settled("msg-2")
                            self.assertEqual(h.state("msg-2"), "acked")
                            return
                        self.assertEqual(h.state("msg-2"), "leased")
                        self.assertNotIn(("ack", "msg-2"), h.timeline)
                        await h.stop_gracefully()
                        self.assertEqual(h.state("msg-2"), "pending")
                    finally:
                        await h.close()

                    restarted = await self.restart_and_deliver(home, h.inbox, "bg")
                    try:
                        self.assert_delivered_once(restarted, "bg")
                    finally:
                        await restarted.close()

                self.run_scenario(scenario)

    def test_each_bg_owns_only_the_child_it_started(self):
        async def scenario(home: str):
            h = ChildHarness(home, timeline=[])
            first, second = threading.Event(), threading.Event()
            self.children.gates.update({"first task": first, "second task": second})
            try:
                await h.seed()
                await h.deliver(raw_event(2, "/bg first task"))
                await eventually(lambda: self.children.runs("bg") == 1)
                await h.wait_turns_finished()
                await h.deliver(raw_event(3, "/bg second task"))
                await eventually(lambda: self.children.runs("bg") == 2)
                await h.wait_turns_finished()
                second.set()
                await h.wait_settled("msg-3")
                await h.settle_loop()
                self.assertEqual(h.state("msg-3"), "acked")
                self.assertEqual(h.state("msg-2"), "leased")
                self.assertEqual(len(h.results("bg")), 1)
                first.set()
                await h.wait_settled("msg-2")
                self.assertEqual(len(h.results("bg")), 2)
            finally:
                first.set()
                second.set()
                await h.close()

        self.run_scenario(scenario)

    def test_a_busy_chat_bg_is_settled_by_its_child(self):
        async def scenario(home: str):
            h = ChildHarness(home, timeline=[])
            try:
                await h.seed()
                busy = await h.hold_turn_open(raw_event(3, "long work"))
                # Hermes dispatches it inline beside the running turn.
                await h.deliver(raw_event(2, COMMANDS["bg"]))
                await eventually(lambda: self.children.runs("bg") == 1)
                await h.settle_loop()
                self.assertEqual(h.state("msg-2"), "leased")
                busy.set()
                await h.wait_settled("msg-3")
                await h.wait_turns_finished()
                self.assertEqual(h.state("msg-2"), "leased")
                self.children.gate.set()
                await h.wait_settled("msg-2")
                await h.settle_loop()
                self.assert_delivered_once(h, "bg")
            finally:
                await h.close()

        self.run_scenario(scenario)

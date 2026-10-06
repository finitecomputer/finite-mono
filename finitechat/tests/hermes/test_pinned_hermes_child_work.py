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
import hashlib
import json
import shutil
import threading
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
        # Model turns whose message this matches wait for ``model_gate``.
        self.hold_model: Callable[[str], bool] | None = None
        self.model_gate = asyncio.Event()
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

    async def _run_agent(self, message, *args: Any, **kwargs: Any):
        if self.hold_model is not None and self.hold_model(message):
            self.held_turns.setdefault(message, self.model_gate)
        return await super()._run_agent(message, *args, **kwargs)

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
                        # The command's turn is over and its child still runs. The
                        # leased entry is all that shows it: Hermes counts no agent.
                        self.assertEqual(h.state("msg-2"), "leased", h.timeline)
                        self.assertNotIn(("ack", "msg-2"), h.timeline)
                        self.assertEqual(h.runner._active_work_count(), 0)
                        self.children.gate.set()
                        await h.wait_settled("msg-2")
                        await h.settle_loop()
                        self.assertEqual(h.state("msg-2"), "acked")
                        self.assert_delivered_once(h, command)
                        self.assertEqual(self.children.runs(command), 1)
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


GOALS = {"set": "/goal synthetic task", "resume": "/goal resume"}
CONTINUATION = "[Continuing toward your standing goal]"


def is_kickoff(kind: str, message: str) -> bool:
    """``message`` is the model turn the /goal command queued."""
    if kind == "set":
        return message == "synthetic task"
    return message.startswith(CONTINUATION)


class GoalScenario(ChildWorkScenario):
    async def prepare(self, h: ChildHarness, kind: str) -> None:
        """An idle chat; for a resume, one whose goal the judge found done."""
        if kind == "set":
            await h.seed()
            return
        await h.deliver(raw_event(1, GOALS["set"]))
        await h.wait_settled("msg-1")
        await h.wait_turns_finished()
        await h.settle_loop()
        h.timeline.clear()
        h.replies.clear()

    async def launch_goal(self, h: ChildHarness, kind: str) -> None:
        """Send the command as msg-2 and wait until its kickoff turn is held running."""
        h.hold_model = lambda message: is_kickoff(kind, message)
        await h.deliver(raw_event(2, GOALS[kind]))
        await eventually(lambda: any(is_kickoff(kind, m) for m in h.models()))

    @staticmethod
    def goal_replies(h: ChildHarness) -> list[str]:
        """The /goal handler's own replies (the kickoff carries msg-2's id too)."""
        return [text for text in h.replies if text.startswith("gateway.goal.")]

    def assert_kicked_off_once(self, h: ChildHarness, kind: str) -> None:
        """The kickoff ran once, and msg-2 was acked once, after it."""
        runs = [
            i for i, (k, text) in enumerate(h.timeline) if k == "model" and is_kickoff(kind, text)
        ]
        self.assertEqual(len(runs), 1, h.timeline)
        self.assertEqual(h.timeline.count(("ack", "msg-2")), 1, h.timeline)
        self.assertNotIn(("release", "msg-2"), h.timeline)
        self.assertLess(runs[0], h.timeline.index(("ack", "msg-2")), h.timeline)


class PinnedHermesGoalLaunchTests(GoalScenario):
    """A /goal set or resume keeps its entry until the turn it queued has run.

    The pinned handler saves the goal (or resumes it) and queues that turn,
    which kicks the goal loop off. A saved goal alone is not its kickoff: a
    stop that drops or interrupts the kickoff hands the command back, so after
    the restart the goal is set or resumed again and kicks off once.
    """

    def test_the_kickoff_turn_settles_the_goal_entry(self):
        for kind in GOALS:
            with self.subTest(kind=kind):

                async def scenario(home: str, kind: str = kind):
                    h = ChildHarness(home, timeline=[])
                    try:
                        await self.prepare(h, kind)
                        await self.launch_goal(h, kind)
                        # The command answered; its kickoff is still running.
                        self.assertEqual(len(self.goal_replies(h)), 1, h.replies)
                        self.assertEqual(h.state("msg-2"), "leased", h.timeline)
                        h.model_gate.set()
                        await h.wait_settled("msg-2")
                        await h.wait_turns_finished()
                        self.assert_kicked_off_once(h, kind)
                    finally:
                        await h.close()

                self.run_scenario(scenario)

    def test_a_stop_hands_the_goal_command_back(self):
        for kind in GOALS:
            for boundary in ("reply", "kickoff", "kickoff reply"):
                for how in ("stop", "restart"):
                    with self.subTest(kind=kind, boundary=boundary, how=how):
                        self.run_scenario(
                            lambda home, kind=kind, boundary=boundary, how=how: (
                                self.stop_at_goal_boundary(home, kind, boundary, how)
                            )
                        )

    async def stop_at_goal_boundary(self, home: str, kind: str, boundary: str, how: str) -> None:
        """Stop where the command replies, while its kickoff runs, or as the kickoff replies.

        An in-band restart waits for a running kickoff turn to finish, as
        Hermes's restart drain waits for every running agent.
        """
        h = ChildHarness(home, timeline=[])
        # The restart's stop() waits for the kickoff turn's own settlement.
        h.restart_after_finished_turns_settle = boundary == "kickoff"
        stops: list[asyncio.Task] = []

        async def begin_stop() -> None:
            if not stops:
                if how == "stop":
                    stops.append(asyncio.create_task(h.stop_gracefully()))
                else:
                    h.begin_restart_drain()
                    stops.append(asyncio.create_task(h.finish_restart()))

        async def stop_at_reply(text: str) -> None:
            if (boundary == "reply" and text.startswith("gateway.goal.")) or (
                boundary == "kickoff reply" and text == "done"
            ):
                await begin_stop()
                await asyncio.Event().wait()

        try:
            await self.prepare(h, kind)
            h.before_reply = stop_at_reply
            if boundary == "kickoff":
                await self.launch_goal(h, kind)
                await begin_stop()
                if how == "restart":
                    h.model_gate.set()
                    await stops[0]
                    self.assertEqual(h.state("msg-2"), "acked", h.timeline)
                    self.assert_kicked_off_once(h, kind)
                    return
            else:
                await h.deliver(raw_event(2, GOALS[kind]))
            await eventually(lambda: bool(stops))
            await stops[0]
            self.assertEqual(h.state("msg-2"), "pending", h.timeline)
            self.assertEqual(h.timeline.count(("release", "msg-2")), 1, h.timeline)
            self.assertNotIn(("ack", "msg-2"), h.timeline)
            self.assertEqual(h.inbox["msg-2"][0]["text"], GOALS[kind])
        finally:
            await h.close()

        restarted = await self.restart_and_deliver(home, h.inbox, "goal")
        try:
            # Hermes's own auto-resume of the interrupted kickoff is declined:
            # the inbox runs the command again instead.
            self.assertEqual(len(self.goal_replies(restarted)), 1, restarted.replies)
            self.assert_kicked_off_once(restarted, kind)
            self.assertEqual(len(restarted.models()), 1, restarted.timeline)
        finally:
            await restarted.close()

    def test_ending_the_goal_makes_an_earlier_goal_command_final(self):
        for control, status in (("/goal pause", "paused"), ("/goal clear", "cleared")):
            for when in ("queued", "running"):
                with self.subTest(control=control, when=when):

                    async def scenario(
                        home: str, control: str = control, status: str | None = status, when=when
                    ):
                        from hermes_cli.goals import GoalManager

                        from tests.hermes.test_pinned_hermes_stop_settlement import (
                            chat_session_id,
                        )

                        h = ChildHarness(home, timeline=[])
                        tail: asyncio.Event | None = None
                        try:
                            await h.seed()
                            if when == "queued":
                                tail = h.hold_turn_tails()
                                # The command's own tail holds its kickoff queued.
                                await h.deliver(raw_event(2, GOALS["set"]))
                                await eventually(lambda: h.notices_held == 1)
                            else:
                                await self.launch_goal(h, "set")
                            await h.deliver(raw_event(3, control))
                            await h.wait_settled("msg-3")
                            # Ending the goal settles the goal command for good.
                            if when == "queued":
                                await h.wait_settled("msg-2")
                                self.assertEqual(h.state("msg-2"), "acked")
                            await h.stop_gracefully()
                            self.assertEqual(h.state("msg-2"), "acked", h.timeline)
                            self.assertNotIn(("release", "msg-2"), h.timeline)
                        finally:
                            if tail is not None:
                                tail.set()
                            h.model_gate.set()
                            await h.close()

                        restarted = await self.boot_after_restart(home, h.inbox)
                        try:
                            self.assertEqual(restarted.handed, [])
                            goal = GoalManager(session_id=chat_session_id(restarted))
                            self.assertEqual(getattr(goal.state, "status", None), status)
                        finally:
                            await restarted.close()

                    self.run_scenario(scenario)

    def test_a_lease_expiry_redelivery_joins_the_goal_kickoff(self):
        for when in ("queued", "running"):
            with self.subTest(when=when):

                async def scenario(home: str, when: str = when):
                    h = ChildHarness(home, timeline=[])
                    tail: asyncio.Event | None = None
                    try:
                        await h.seed()
                        if when == "queued":
                            tail = h.hold_turn_tails()
                            await h.deliver(raw_event(2, GOALS["set"]))
                            await eventually(lambda: h.notices_held == 1)
                        else:
                            await self.launch_goal(h, "set")
                        # The sidecar re-leases the expired entry to the stream.
                        await h.deliver(h.inbox["msg-2"][0])
                        await h.settle_loop()
                        self.assertEqual(len(self.goal_replies(h)), 1, h.replies)
                        self.assertEqual(h.state("msg-2"), "leased")
                        if tail is not None:
                            tail.set()
                        h.model_gate.set()
                        await h.wait_settled("msg-2")
                        await h.wait_turns_finished()
                        self.assert_kicked_off_once(h, "set")
                    finally:
                        if tail is not None:
                            tail.set()
                        h.model_gate.set()
                        await h.close()

                self.run_scenario(scenario)


class RealRunHarness(ChildHarness):
    """ChildHarness that runs the pinned ``GatewayRunner._run_agent`` itself.

    Only ``TurnRunner.run_sync``, the agent's model loop, is synthetic
    (``synthetic_model``). The real run takes the chat's queued follow-up
    with ``adapter.get_pending_message`` and runs it inside the same turn.
    """

    def __init__(self, home: str, *, timeline: list[tuple[str, str]], **kwargs: Any):
        super().__init__(home, timeline=timeline, **kwargs)
        self.runner._run_agent = GatewayRunner._run_agent.__get__(self.runner)
        # Model loops wait for the gate this returns for their message, if any.
        self.hold_thread: Callable[[str], threading.Event | None] = lambda _message: None

    @contextlib.contextmanager
    def synthetic_model(self) -> Iterator[None]:
        from gateway import run as gateway_run

        loop = asyncio.get_running_loop()
        harness = self

        def run_sync(turn: Any) -> dict[str, Any]:
            ctx = turn._ctx
            interrupted = threading.Event()
            gate = harness.hold_thread(ctx.message)

            class Agent:
                def interrupt(self, *_args: Any, **_kwargs: Any) -> None:
                    interrupted.set()
                    if gate is not None:
                        gate.set()

                def hard_interrupt(self, *_args: Any, **_kwargs: Any) -> None:
                    self.interrupt()

            ctx.agent_holder[0] = Agent()
            loop.call_soon_threadsafe(harness.timeline.append, ("model", ctx.message))
            if gate is not None:
                gate.wait(10)
            if interrupted.is_set():
                result = {"final_response": "", "interrupted": True, "messages": [], "api_calls": 1}
            else:
                result = {
                    "final_response": "done",
                    "messages": [],
                    "completed": True,
                    "api_calls": 1,
                }
            ctx.result_holder[0] = result
            return result

        with (
            patch.object(gateway_run.TurnRunner, "run_sync", run_sync),
            patch.object(
                gateway_run,
                "_load_gateway_config",
                return_value={"display": {"tool_progress": "off"}},
            ),
        ):
            yield


class PinnedHermesGoalFollowUpTests(GoalScenario):
    def test_a_busy_goal_resume_settles_with_the_turn_that_runs_it(self):
        for ending in ("finish", "stop", "restart drain"):
            with self.subTest(ending=ending):

                async def scenario(home: str, ending: str = ending):
                    h = RealRunHarness(home, timeline=[])
                    h.restart_after_finished_turns_settle = True
                    busy, continuation = threading.Event(), threading.Event()
                    with h.synthetic_model():
                        try:
                            await self.prepare(h, "resume")
                            h.hold_thread = lambda message: (
                                busy
                                if message == "long work"
                                else continuation
                                if ending == "stop" and is_kickoff("resume", message)
                                else None
                            )
                            await h.deliver(raw_event(3, "long work"))
                            await eventually(lambda: ("model", "long work") in h.timeline)
                            # Hermes takes it inline beside the running turn and
                            # queues the continuation behind that turn.
                            await h.deliver(raw_event(2, GOALS["resume"]))
                            await eventually(lambda: len(self.goal_replies(h)) == 1)
                            await h.settle_loop()
                            self.assertEqual(h.state("msg-2"), "leased")
                            if ending == "restart drain":
                                # The drain waits for the running turn, which
                                # then discards the continuation it would run.
                                h.begin_restart_drain()
                                restart = asyncio.create_task(h.finish_restart())
                                busy.set()
                                await restart
                                self.assertEqual(h.state("msg-3"), "acked", h.timeline)
                                self.assertEqual(h.state("msg-2"), "pending", h.timeline)
                                self.assertFalse(
                                    any(is_kickoff("resume", m) for m in h.models()), h.timeline
                                )
                            elif ending == "stop":
                                busy.set()
                                await eventually(
                                    lambda: any(is_kickoff("resume", m) for m in h.models())
                                )
                                await h.stop_gracefully()
                                for message_id in ("msg-2", "msg-3"):
                                    self.assertEqual(h.state(message_id), "pending", h.timeline)
                            else:
                                busy.set()
                                await h.wait_settled("msg-3")
                                await h.wait_turns_finished()
                                # The running turn took the continuation and ran
                                # it inside itself, then settled both entries.
                                self.assert_kicked_off_once(h, "resume")
                                self.assertEqual(h.state("msg-3"), "acked")
                                return
                        finally:
                            busy.set()
                            continuation.set()
                            await h.close()

                    restarted = await self.restart_and_deliver(home, h.inbox, "goal")
                    try:
                        await restarted.wait_turns_finished()
                        self.assert_kicked_off_once(restarted, "resume")
                        self.assertEqual(restarted.state("msg-3"), "acked")
                    finally:
                        await restarted.close()

                self.run_scenario(scenario)

    def test_a_redelivery_joins_the_goal_command_riding_another_turn(self):
        """Round 13 F6: a lease-expiry redelivery must not run /goal resume a second time."""

        async def scenario(home: str):
            h = RealRunHarness(home, timeline=[])
            busy, continuation = threading.Event(), threading.Event()
            with h.synthetic_model():
                try:
                    await self.prepare(h, "resume")
                    h.hold_thread = lambda message: (
                        busy
                        if message == "long work"
                        else continuation
                        if is_kickoff("resume", message)
                        else None
                    )
                    await h.deliver(raw_event(3, "long work"))
                    await eventually(lambda: ("model", "long work") in h.timeline)
                    await h.deliver(raw_event(2, GOALS["resume"]))
                    await eventually(lambda: len(self.goal_replies(h)) == 1)
                    busy.set()
                    await eventually(lambda: any(is_kickoff("resume", m) for m in h.models()))
                    await h.deliver(h.inbox["msg-2"][0])
                    await h.settle_loop()
                    self.assertEqual(len(self.goal_replies(h)), 1, h.replies)
                    continuation.set()
                    await h.wait_settled("msg-3")
                    await h.wait_turns_finished()
                    self.assert_kicked_off_once(h, "resume")
                finally:
                    busy.set()
                    continuation.set()
                    await h.close()

        self.run_scenario(scenario)

    def test_continuations_still_in_hermes_queue_are_settled_once(self):
        """Two busy /goal resumes: the second continuation waits in Hermes's overflow queue.

        A graceful stop never ran either, so both entries go back. A user /new
        drops both, so both are final.
        """
        for ending in ("stop", "new"):
            with self.subTest(ending=ending):

                async def scenario(home: str, ending: str = ending):
                    h = RealRunHarness(home, timeline=[])
                    busy = threading.Event()
                    with h.synthetic_model():
                        try:
                            await self.prepare(h, "resume")
                            h.hold_thread = lambda message: busy if message == "long work" else None
                            await h.deliver(raw_event(4, "long work"))
                            await eventually(lambda: ("model", "long work") in h.timeline)
                            for seq in (2, 3):
                                await h.deliver(raw_event(seq, GOALS["resume"]))
                                await eventually(
                                    lambda seq=seq: len(self.goal_replies(h)) == seq - 1
                                )
                            await h.settle_loop()
                            self.assertEqual((h.state("msg-2"), h.state("msg-3")), ("leased",) * 2)
                            if ending == "new":
                                await h.deliver(raw_event(5, "/new"))
                                await h.wait_settled("msg-5")
                            await h.stop_gracefully()
                            expected = "pending" if ending == "stop" else "acked"
                            for message_id in ("msg-2", "msg-3"):
                                self.assertEqual(h.state(message_id), expected, h.timeline)
                            self.assertFalse(
                                any(is_kickoff("resume", m) for m in h.models()), h.timeline
                            )
                        finally:
                            busy.set()
                            await h.close()

                self.run_scenario(scenario)


class PinnedHermesPluginRediscoveryTests(GoalScenario):
    """Hermes plugin rediscovery can import this plugin's module again in-process.

    The running adapter keeps its first module's globals, while code from the
    new import gets fresh copies (FIN-117 lost requester context that way).
    Launch ownership lives on the adapter instance, so neither the running
    adapter nor one built from the new import loses it.
    """

    def test_launch_ownership_survives_a_plugin_module_reimport(self):
        async def scenario(home: str):
            from tests.hermes.test_pinned_hermes_stop_settlement import load_adapter_module

            h = ChildHarness(home, timeline=[])
            try:
                # The first module's /goal installs its gateway queue hook.
                await self.prepare(h, "resume")
                await self.launch(h, "bg")
                hooks: list[str] = []

                class PluginContext:
                    def register_hook(self, name: str, _hook: Any) -> None:
                        hooks.append(name)

                    def register_platform(self, **_kwargs: Any) -> None:
                        hooks.append("platform")

                fresh = load_adapter_module()
                fresh._finite_private_control_request = lambda *_args: None
                fresh.register(PluginContext())
                self.assertIn("platform", hooks)
                # The running adapter still settles its child's entry.
                self.children.gate.set()
                await h.wait_settled("msg-2")
                await h.settle_loop()
                self.assert_delivered_once(h, "bg")

                # An adapter built from the new import, on the same gateway.
                config = h.adapter.config
                adapter = fresh.FiniteChatAdapter(config)
                adapter._home_channel_hydrated = True
                adapter.gateway_runner = h.runner
                adapter._finitechat_json = h._sidecar

                async def handle_message(event: Any) -> Any:
                    h.handed.append(event.message_id)
                    return await h.runner._handle_message(event)

                adapter.set_message_handler(handle_message)
                h.runner.adapters[adapter.platform] = adapter
                h.adapter, h.module = adapter, fresh
                h.timeline.clear()
                await h.deliver(raw_event(3, GOALS["resume"]))
                await h.wait_settled("msg-3")
                await h.wait_turns_finished()
                runs = [i for i, (k, text) in enumerate(h.timeline) if k == "model"]
                self.assertEqual(len(runs), 1, h.timeline)
                self.assertTrue(is_kickoff("resume", h.timeline[runs[0]][1]), h.timeline)
                # The kickoff ran before the /goal entry was acked.
                self.assertLess(runs[0], h.timeline.index(("ack", "msg-3")), h.timeline)
            finally:
                await h.close()

        self.run_scenario(scenario)


class PinnedHermesAutoResumeTests(GoalScenario):
    """Hermes's own auto-resume runs only for interrupted turns the inbox does not own.

    The adapter declined every resume (6439252c), which dropped interrupted
    goal continuations that the 458a baseline resumed: they have no inbox
    entry to redeliver. Each turn start now records durably whether the inbox
    owns that chat's latest turn, and only such a chat's resume is declined.
    """

    @staticmethod
    def owner_marker(h: ChildHarness) -> Path:
        from tests.hermes.test_pinned_hermes_stop_settlement import chat_source

        session_key = h.runner._session_key_for_source(chat_source(h))
        digest = hashlib.sha256(session_key.encode("utf-8")).hexdigest()
        return Path(h.home) / "hermes-turn-owners" / f"{digest}.json"

    async def restart(
        self, home: str, inbox: dict[str, tuple[dict[str, Any], str]]
    ) -> ChildHarness:
        restarted = ChildHarness(home, timeline=[], inbox=inbox)
        await restarted.boot_with_gate_closed()
        self.assertEqual(await restarted.open_gate(), 1, "Hermes schedules its resume")
        return restarted

    def test_an_interrupted_goal_continuation_resumes_after_restart(self):
        async def scenario(home: str):
            h = ChildHarness(home, timeline=[])
            verdicts = ["continue"]

            def judge(*_args: Any, **_kwargs: Any) -> tuple[str, str, bool, None, bool]:
                return (
                    verdicts.pop(0) if verdicts else "done",
                    "synthetic judge",
                    False,
                    None,
                    False,
                )

            try:
                with patch("hermes_cli.goals.judge_goal", judge):
                    await h.seed()
                    h.hold_model = lambda message: message.startswith(CONTINUATION)
                    await h.deliver(raw_event(2, GOALS["set"]))
                    await eventually(lambda: any(m.startswith(CONTINUATION) for m in h.models()))
                    # The kickoff settled the /goal entry; the continuation has none.
                    self.assertEqual(h.state("msg-2"), "acked")
                    self.assertFalse(self.owner_marker(h).exists())
                    await h.stop_gracefully()
                    self.assertTrue(h.interrupted_by_shutdown)
                    self.assertTrue(h.resume_pending())
            finally:
                h.model_gate.set()
                await h.close()

            restarted = await self.restart(home, h.inbox)
            try:
                await eventually(lambda: bool(restarted.models()))
                await restarted.wait_turns_finished()
                # Hermes resumed the continuation itself, once, as on 458a; no
                # inbox entry ran.
                self.assertEqual(len(restarted.models()), 1, restarted.timeline)
                self.assertEqual([m for m in restarted.handed if m is not None], [])
                self.assertFalse(restarted.resume_pending())
            finally:
                await restarted.close()

        self.run_scenario(scenario)

    def test_an_interrupted_inbox_turn_runs_once_after_restart(self):
        """By the inbox redelivery here; after an upgrade from 458a, by Hermes's resume."""
        for runtime in ("this", "458a"):
            with self.subTest(runtime=runtime):

                async def scenario(home: str, runtime: str = runtime):
                    h = ChildHarness(home, timeline=[])
                    try:
                        await h.seed()
                        h.hold_model = lambda message: message == "long work"
                        await h.deliver(raw_event(2, "long work"))
                        await eventually(lambda: ("model", "long work") in h.timeline)
                        marker = json.loads(self.owner_marker(h).read_text(encoding="utf-8"))
                        self.assertEqual(
                            (marker["room_id"], marker["seq"], marker["message_id"]),
                            (ROOM_ID, 2, "msg-2"),
                        )
                        await h.stop_gracefully()
                        self.assertEqual(h.state("msg-2"), "pending")
                        self.assertTrue(h.resume_pending())
                        # Still there after the release, and survives the restart.
                        self.assertTrue(self.owner_marker(h).exists())
                        if runtime == "458a":
                            # The 458a runtime keeps no owner records, and its
                            # stop acks the turn it interrupted.
                            shutil.rmtree(self.owner_marker(h).parent)
                            raw, _state = h.inbox["msg-2"]
                            h.inbox["msg-2"] = (raw, "acked")
                    finally:
                        h.model_gate.set()
                        await h.close()

                    restarted = await self.restart(home, h.inbox)
                    try:
                        await eventually(lambda: bool(restarted.models()))
                        await restarted.wait_turns_finished()
                        await restarted.settle_loop()
                        self.assertEqual(restarted.state("msg-2"), "acked")
                        if runtime == "this":
                            # Hermes's resume was declined; the redelivery ran it.
                            self.assertEqual(restarted.handed, ["msg-2"])
                            self.assertEqual(restarted.models(), ["long work"])
                        else:
                            # Hermes resumed it, once, as 458a's own restart would.
                            self.assertEqual([m for m in restarted.handed if m is not None], [])
                            self.assertEqual(len(restarted.models()), 1, restarted.timeline)
                    finally:
                        await restarted.close()

                self.run_scenario(scenario)


class PinnedHermesAdapterReplacementTests(GoalScenario):
    """A gateway that replaces a failed adapter disconnects the old one first.

    The pinned fatal-error path calls only ``disconnect()`` on the old
    instance (no ``cancel_background_tasks()`` first) before its reconnect
    watcher builds a new one. The old instance hands back the work it owned
    then, so the new one starts with nothing to coalesce against.
    """

    def test_disconnecting_a_replaced_adapter_hands_back_its_work(self):
        async def scenario(home: str):
            h = ChildHarness(home, timeline=[])
            tail: asyncio.Event | None = None
            try:
                await self.prepare(h, "resume")
                await self.launch(h, "bg")
                tail = h.hold_turn_tails()
                await h.deliver(raw_event(3, GOALS["resume"]))
                await eventually(lambda: h.notices_held == 1)
                self.assertEqual((h.state("msg-2"), h.state("msg-3")), ("leased", "leased"))
                await h.adapter.disconnect()
                for message_id in ("msg-2", "msg-3"):
                    self.assertEqual(h.state(message_id), "pending", h.timeline)
                self.assertEqual(h.results("bg"), [])
                self.assertFalse(any(is_kickoff("resume", m) for m in h.models()))
            finally:
                if tail is not None:
                    tail.set()
                self.children.gate.set()
                await h.close()

        self.run_scenario(scenario)


class PinnedHermesStoppedInboxTurnTests(GoalScenario):
    """Round 13 F1: a turn that starts after a stop released an inbox turn must not disown it.

    The stop interrupts the inbox turn and releases its entry; the base adapter
    then drains whatever was queued behind it, which the draining gateway
    refuses. Hermes must not resume the chat as well, or the turn runs twice.
    """

    async def restart_counting_models(
        self, home: str, inbox: dict[str, tuple[dict[str, Any], str]], message_id: str
    ) -> ChildHarness:
        restarted = ChildHarness(home, timeline=[], inbox=inbox)
        await restarted.boot_with_gate_closed()
        await restarted.open_gate()
        await restarted.wait_settled(message_id)
        await restarted.wait_turns_finished()
        for _ in range(20):
            await restarted.settle_loop()
        await restarted.wait_turns_finished()
        return restarted

    def test_a_notice_queued_behind_a_stopped_inbox_turn_does_not_disown_it(self):
        async def scenario(home: str):
            from gateway.platforms.base import MessageEvent

            from tests.hermes.test_pinned_hermes_stop_settlement import chat_source

            h = ChildHarness(home, timeline=[])
            try:
                await h.seed()
                h.hold_model = lambda message: message == "long work"
                await h.deliver(raw_event(2, "long work"))
                await eventually(lambda: ("model", "long work") in h.timeline)
                notice = MessageEvent(
                    text="background job finished", source=chat_source(h), internal=True
                )
                await h.adapter.handle_message(notice)
                await h.settle_loop()
                await h.stop_gracefully()
                self.assertEqual(h.state("msg-2"), "pending")
                self.assertTrue(h.resume_pending())
            finally:
                h.model_gate.set()
                await h.close()

            restarted = await self.restart_counting_models(home, h.inbox, "msg-2")
            try:
                self.assertEqual(restarted.models(), ["long work"], restarted.timeline)
                self.assertEqual(restarted.state("msg-2"), "acked")
            finally:
                await restarted.close()

        self.run_scenario(scenario)

    def test_a_stop_during_the_kickoffs_goal_judge_runs_the_goal_once(self):
        async def scenario(home: str):
            h = ChildHarness(home, timeline=[])
            entered, release = threading.Event(), threading.Event()

            def judge(*_args: Any, **_kwargs: Any) -> tuple[str, str, bool, None, bool]:
                entered.set()
                release.wait(10)
                return "continue", "synthetic judge", False, None, False

            try:
                await h.seed()
                with patch("hermes_cli.goals.judge_goal", judge):
                    await h.deliver(raw_event(2, GOALS["set"]))
                    await eventually(entered.is_set)
                    stop = asyncio.create_task(h.stop_gracefully())
                    await eventually(h.adapter._gateway_stopping)
                    release.set()
                    await stop
                self.assertEqual(h.state("msg-2"), "pending", h.timeline)
            finally:
                release.set()
                await h.close()

            restarted = await self.restart_counting_models(home, h.inbox, "msg-2")
            try:
                self.assertEqual(restarted.models(), ["synthetic task"], restarted.timeline)
                self.assertEqual(restarted.state("msg-2"), "acked")
            finally:
                await restarted.close()

        self.run_scenario(scenario)

    def test_a_goal_command_riding_an_unowned_turn_runs_once_across_a_stop(self):
        """Round 13 F1(c) and Opus S1: the turn the /goal event rides has no inbox entry."""
        for running in ("goal continuation", "internal notice"):
            for ending in ("finish", "stop"):
                with self.subTest(running=running, ending=ending):
                    self.run_scenario(
                        lambda home, running=running, ending=ending: self.ride_unowned_turn(
                            home, running, ending
                        )
                    )

    async def ride_unowned_turn(self, home: str, running: str, ending: str) -> None:
        from gateway.platforms.base import MessageEvent, MessageType

        from tests.hermes.test_pinned_hermes_stop_settlement import chat_source

        h = RealRunHarness(home, timeline=[])
        h.restart_after_finished_turns_settle = True
        first, rider = threading.Event(), threading.Event()
        verdicts = ["continue"] if running == "goal continuation" else []
        seen: list[str] = []

        def judge(*_args: Any, **_kwargs: Any) -> tuple[str, str, bool, None, bool]:
            return (verdicts.pop(0) if verdicts else "done", "synthetic judge", False, None, False)

        def hold(message: str) -> threading.Event | None:
            # msg-3's own continuation runs; the judge's next one (no inbox
            # entry) is held; the /goal resume riding it is held too.
            if message == "internal work":
                return first
            if not message.startswith(CONTINUATION):
                return None
            seen.append(message)
            if running == "goal continuation" and len(seen) == 1:
                return None
            return first if len(seen) <= 2 and not first.is_set() else rider

        with h.synthetic_model():
            try:
                await self.prepare(h, "resume")
                h.timeline.clear()
                h.hold_thread = hold
                if running == "goal continuation":
                    with patch("hermes_cli.goals.judge_goal", judge):
                        await h.deliver(raw_event(3, GOALS["resume"]))
                        await h.wait_settled("msg-3")
                        await eventually(lambda: len(self.continuations(h)) == 2)
                else:
                    internal = MessageEvent(
                        text="internal work",
                        message_type=MessageType.TEXT,
                        source=chat_source(h),
                        internal=True,
                    )
                    await h.adapter.handle_message(internal)
                    await eventually(lambda: ("model", "internal work") in h.timeline)
                runs = len(self.continuations(h))
                # The user's /goal resume lands inline beside that unowned turn.
                await h.deliver(raw_event(5, GOALS["resume"]))
                await eventually(
                    lambda: (
                        h.replies.count("gateway.goal.resumed")
                        >= 1 + (running == "goal continuation")
                    )
                )
                await h.settle_loop()
                self.assertEqual(h.state("msg-5"), "leased")
                first.set()
                await eventually(lambda: len(self.continuations(h)) == runs + 1)
                if ending == "finish":
                    rider.set()
                    await h.wait_settled("msg-5")
                    await h.wait_turns_finished()
                    self.assertEqual(h.state("msg-5"), "acked", h.timeline)
                    self.assertNotIn(("release", "msg-5"), h.timeline)
                    self.assertEqual(len(self.continuations(h)), runs + 1, h.timeline)
                    return
                # Production's running agent is marked for resume; the harness's
                # synthetic agent stays Hermes's pending sentinel, which stop() skips.
                session_key = h.runner._session_key_for_source(chat_source(h))
                await h.runner.async_session_store.mark_resume_pending(
                    session_key, "shutdown_timeout"
                )
                await h.stop_gracefully()
                self.assertEqual(h.state("msg-5"), "pending", h.timeline)
            finally:
                first.set()
                rider.set()
                await h.close()

        restarted = RealRunHarness(home, timeline=[], inbox=h.inbox)
        with restarted.synthetic_model():
            try:
                await restarted.boot_with_gate_closed()
                await restarted.open_gate()
                await restarted.wait_settled("msg-5")
                await restarted.wait_turns_finished()
                for _ in range(20):
                    await restarted.settle_loop()
                await restarted.wait_turns_finished()
                # The redelivered /goal resume ran its continuation; Hermes did
                # not also resume the stopped turn.
                self.assertEqual(
                    [m.startswith(CONTINUATION) for m in restarted.models()],
                    [True],
                    restarted.timeline,
                )
            finally:
                await restarted.close()

    @staticmethod
    def continuations(h: ChildHarness) -> list[str]:
        return [m for m in h.models() if m.startswith(CONTINUATION)]

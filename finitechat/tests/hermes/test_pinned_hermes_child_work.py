"""Accepted child work against the sealed pinned handlers and a real SQLite journal.

Inference and transport are synthetic. Restart uses a new GatewayRunner over
retained state; the journal crash case exits a real subprocess without cleanup.
"""

import asyncio
import json
import os
import subprocess
import sys
import threading
from pathlib import Path
from unittest.mock import patch

from gateway.finite_child_work import capture_adapter, journal, schedule_recovery
from test_pinned_hermes_stop_settlement import (
    DrainScenario,
    GatewayHarness,
    PinnedHermesCommandFinalityTests,
    chat_source,
    eventually,
    raw_event,
    seed_exchanges,
)


class ChildWorkTests(DrainScenario):
    async def shutdown(self, harness):
        for task in list(harness.runner._background_tasks):
            task.cancel()
        await asyncio.gather(*harness.runner._background_tasks, return_exceptions=True)
        await harness.close()

    async def boot(self, home, inbox):
        h = GatewayHarness(home, timeline=[], inbox=inbox, stall=False)
        await h.boot_with_gate_closed()
        await h.open_gate()
        return h

    def test_background_stop_retains_interrupted_outcome_without_reexecution(self):
        async def scenario(home):
            h = GatewayHarness(home, timeline=[], stall=False)
            ran = []
            started = asyncio.Event()

            async def background(*args, **kwargs):
                ran.append(True)
                started.set()
                await asyncio.Event().wait()

            h.runner._run_background_task = background
            try:
                await h.deliver(raw_event(1, "/bg synthetic task"))
                await h.wait_settled("msg-1")
                await asyncio.wait_for(started.wait(), 2)
                self.assertGreater(h.runner._active_work_count(), 0)
                await h.stop_gracefully()
            finally:
                await self.shutdown(h)
            restarted = await self.boot(home, h.inbox)
            try:
                await eventually(lambda: journal(restarted.runner).count() == 0)
                self.assertTrue(any("was interrupted" in reply for reply in restarted.replies))
                self.assertEqual(ran, [True])
                self.assertEqual(restarted.runs, [])
            finally:
                await self.shutdown(restarted)
            again = await self.boot(home, h.inbox)
            try:
                await asyncio.sleep(0.05)
                self.assertFalse(any("was interrupted" in reply for reply in again.replies))
                self.assertEqual(again.runs, [])
            finally:
                await self.shutdown(again)

        self.run_scenario(scenario)

    def test_completed_background_outbox_survives_cancelled_delivery(self):
        async def scenario(home):
            h = GatewayHarness(home, timeline=[], stall=False)
            sent = asyncio.Event()
            ran = []
            send = h.adapter.send

            async def background(prompt, source, *_args, **_kwargs):
                ran.append(prompt)
                adapter = capture_adapter(h.adapter)
                assert adapter is not None
                await adapter.send(
                    source.chat_id, "durable answer", metadata={"thread_id": source.thread_id}
                )

            async def blocked_send(chat_id, content, **kwargs):
                if content == "durable answer":
                    sent.set()
                    await asyncio.Event().wait()
                return await send(chat_id, content, **kwargs)

            h.runner._run_background_task = background
            h.adapter.send = blocked_send
            try:
                await h.deliver(raw_event(1, "/bg synthetic task"))
                await h.wait_settled("msg-1")
                await asyncio.wait_for(sent.wait(), 2)
                self.assertEqual(journal(h.runner).pending()[0]["state"], "outcome")
                await h.stop_gracefully()
            finally:
                await self.shutdown(h)
            restarted = await self.boot(home, h.inbox)
            try:
                await eventually(lambda: journal(restarted.runner).count() == 0)
                self.assertEqual(restarted.replies.count("durable answer"), 1)
                self.assertEqual(ran, ["synthetic task"])
                # Even an unconfirmed inbox ack must not launch the child again.
                raw, _ = h.inbox["msg-1"]
                await restarted.deliver(raw)
                await restarted.wait_turns_finished()
                self.assertEqual(ran, ["synthetic task"])
                self.assertEqual(restarted.runs, [])
            finally:
                await self.shutdown(restarted)

        self.run_scenario(scenario)

    def test_btw_acceptance_is_durable_while_auxiliary_thread_runs(self):
        async def scenario(home):
            h = GatewayHarness(home, timeline=[], stall=False)
            started, release = threading.Event(), threading.Event()
            calls = []

            def answer(*_args, **_kwargs):
                calls.append(True)
                started.set()
                release.wait(10)
                return "side answer"

            try:
                await seed_exchanges(h)
                h.runner._resolve_session_agent_runtime = lambda **_kwargs: (
                    "synthetic",
                    {"api_key": "synthetic"},
                )
                with patch("agent.side_question.answer_side_question", answer):
                    await h.deliver(raw_event(4, "/btw synthetic question"))
                    await h.wait_settled("msg-4")
                    await eventually(started.is_set)
                    self.assertGreater(h.runner._active_work_count(), 0)
                    await h.stop_gracefully()
            finally:
                release.set()
                await self.shutdown(h)
            restarted = await self.boot(home, h.inbox)
            try:
                await eventually(lambda: journal(restarted.runner).count() == 0)
                self.assertTrue(
                    any(
                        "side question work was interrupted" in reply for reply in restarted.replies
                    )
                )
                self.assertEqual(calls, [True])
            finally:
                await self.shutdown(restarted)

        self.run_scenario(scenario)

    def test_saved_goal_kickoff_resumes_once_without_replaying_inbox(self):
        for command in ("/goal synthetic task", "/goal resume"):
            with self.subTest(command=command):

                async def scenario(home, command=command):
                    h = GatewayHarness(home, timeline=[], stall=False)
                    if command.endswith("resume"):
                        from hermes_cli.goals import GoalManager

                        entry = await h.runner.async_session_store.get_or_create_session(
                            chat_source(h)
                        )
                        manager = GoalManager(session_id=entry.session_id)
                        manager.set("synthetic task")
                        manager.pause()
                    stops = PinnedHermesCommandFinalityTests.stop_before_reply(h)
                    try:
                        await h.deliver(raw_event(1, command))
                        await eventually(lambda: bool(stops))
                        await stops[0]
                        self.assertEqual(h.state("msg-1"), "acked")
                        self.assertTrue(
                            any(row["state"] == "queued" for row in journal(h.runner).pending())
                        )
                    finally:
                        await self.shutdown(h)
                    restarted = await self.boot(home, h.inbox)
                    try:
                        await eventually(lambda: len(restarted.runs) > 0)
                        await restarted.wait_turns_finished()
                        self.assertEqual(len(restarted.runs), 1)
                        self.assertIn("synthetic task", restarted.runs[0])
                        self.assertEqual(restarted.handed, [None])
                    finally:
                        await self.shutdown(restarted)

                self.run_scenario(scenario)

    def test_process_death_after_acceptance_reports_interruption(self):
        async def scenario(home):
            h = GatewayHarness(home, timeline=[], stall=False)
            source = chat_source(h).to_dict()
            code = (
                "import os,sys,json; from gateway.finite_child_work import Journal; "
                'j=Journal(sys.argv[1]); j.accept("crash", "background", '
                '{"source":json.loads(sys.argv[2]),"metadata":{"thread_id":"segment-1"}}); os._exit(23)'
            )
            proc = subprocess.run(
                [sys.executable, "-c", code, home, json.dumps(source)], env=os.environ.copy()
            )
            self.assertEqual(proc.returncode, 23)
            try:
                schedule_recovery(h.runner)
                await eventually(lambda: journal(h.runner).count() == 0)
                self.assertEqual(h.runs, [])
                self.assertTrue(any("was interrupted" in reply for reply in h.replies))
            finally:
                await self.shutdown(h)

        self.run_scenario(scenario)

    def test_goal_control_after_saved_kickoff_remains_final_on_restart(self):
        for control in ("pause", "clear"):
            with self.subTest(control=control):

                async def scenario(home, control=control):
                    from hermes_cli.goals import GoalManager

                    h = GatewayHarness(home, timeline=[], stall=False)
                    stops = PinnedHermesCommandFinalityTests.stop_before_reply(h)
                    try:
                        await h.deliver(raw_event(1, "/goal synthetic task"))
                        await eventually(lambda: bool(stops))
                        await stops[0]
                        row = journal(h.runner).pending()[0]
                        manager = GoalManager(session_id=json.loads(row["payload"])["session_id"])
                        getattr(manager, control)()
                    finally:
                        await self.shutdown(h)
                    restarted = await self.boot(home, h.inbox)
                    try:
                        await eventually(lambda: journal(restarted.runner).count() == 0)
                        self.assertEqual(restarted.runs, [])
                    finally:
                        await self.shutdown(restarted)

                self.run_scenario(scenario)

    def test_goal_final_text_is_retained_before_delivery(self):
        async def scenario(home):
            h = GatewayHarness(home, timeline=[], stall=False)
            sending = asyncio.Event()
            send = h.adapter.send

            async def blocked(chat_id, content, **kwargs):
                if content == "done":
                    sending.set()
                    await asyncio.Event().wait()
                return await send(chat_id, content, **kwargs)

            h.adapter.send = blocked
            try:
                await h.deliver(raw_event(1, "/goal synthetic task"))
                await asyncio.wait_for(sending.wait(), 2)
                self.assertTrue(
                    any(row["state"] == "outcome" for row in journal(h.runner).pending())
                )
                await h.stop_gracefully()
            finally:
                await self.shutdown(h)
            restarted = await self.boot(home, h.inbox)
            try:
                await eventually(lambda: journal(restarted.runner).count() == 0)
                self.assertIn("done", restarted.replies)
                self.assertEqual(restarted.runs, [])
            finally:
                await self.shutdown(restarted)

        self.run_scenario(scenario)

    def test_background_attachment_survives_worker_temporary_files(self):
        async def scenario(home):
            h = GatewayHarness(home, timeline=[], stall=False)
            sending = asyncio.Event()
            original = Path(home, "temporary-result.txt")
            original.write_text("durable bytes")

            async def background(prompt, source, *_args, **_kwargs):
                adapter = capture_adapter(h.adapter)
                assert adapter is not None
                await adapter.send_document(
                    chat_id=source.chat_id,
                    file_path=str(original),
                    metadata={"thread_id": source.thread_id},
                )

            async def blocked(**_kwargs):
                sending.set()
                await asyncio.Event().wait()

            h.runner._run_background_task = background
            h.adapter.send_document = blocked
            try:
                await h.deliver(raw_event(1, "/bg attachment"))
                await asyncio.wait_for(sending.wait(), 2)
                original.unlink()
                row = journal(h.runner).pending()[0]
                retained = json.loads(row["deliveries"])[0]["kwargs"]["file_path"]
                self.assertEqual(Path(retained).read_text(), "durable bytes")
                await h.stop_gracefully()
            finally:
                await self.shutdown(h)
            restarted = GatewayHarness(home, timeline=[], inbox=h.inbox, stall=False)
            seen = []

            async def delivered(**kwargs):
                from gateway.platforms.base import SendResult

                seen.append(Path(kwargs["file_path"]).read_text())
                return SendResult(success=True)

            restarted.adapter.send_document = delivered
            try:
                await restarted.boot_with_gate_closed()
                await restarted.open_gate()
                await eventually(lambda: journal(restarted.runner).count() == 0)
                self.assertEqual(seen, ["durable bytes"])
            finally:
                await self.shutdown(restarted)

        self.run_scenario(scenario)

    def test_foreground_inbox_recovery_does_not_duplicate_goal_work(self):
        async def scenario(home):
            h = GatewayHarness(home, timeline=[], stall=False)
            stops = PinnedHermesCommandFinalityTests.stop_before_reply(h)
            try:
                await h.deliver(raw_event(1, "/goal synthetic task"))
                await eventually(lambda: bool(stops))
                await stops[0]
                h.inbox["msg-2"] = (raw_event(2, "foreground follow-up"), "pending")
            finally:
                await self.shutdown(h)
            restarted = await self.boot(home, h.inbox)
            try:
                await restarted.wait_settled("msg-2")
                await restarted.wait_turns_finished()
                await eventually(lambda: journal(restarted.runner).count() == 0)
                self.assertEqual(restarted.runs, ["foreground follow-up"])
                self.assertEqual(restarted.handed, ["msg-2"])
            finally:
                await self.shutdown(restarted)

        self.run_scenario(scenario)

    def test_started_goal_is_interrupted_and_not_automatically_reexecuted(self):
        async def scenario(home):
            h = GatewayHarness(home, timeline=[], stall=False)
            h.held_turns["synthetic task"] = asyncio.Event()
            try:
                await h.deliver(raw_event(1, "/goal synthetic task"))
                await eventually(lambda: "synthetic task" in h.runs)
                await h.stop_gracefully()
                self.assertTrue(
                    any(row["state"] == "outcome" for row in journal(h.runner).pending())
                )
            finally:
                await self.shutdown(h)
            restarted = await self.boot(home, h.inbox)
            try:
                await eventually(lambda: journal(restarted.runner).count() == 0)
                self.assertEqual(restarted.runs, [])
                self.assertTrue(
                    any("goal work was interrupted" in reply for reply in restarted.replies)
                )
            finally:
                await self.shutdown(restarted)

        self.run_scenario(scenario)

"""Exercise the packaged gateway's stop/queued-turn boundary without inference."""

import asyncio
import os
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch


class StopGenerationTests(unittest.IsolatedAsyncioTestCase):
    @classmethod
    def setUpClass(cls):
        cls.scratch = tempfile.TemporaryDirectory(prefix="hermes-stop-")
        cls.environment = patch.dict(
            os.environ, {"HERMES_HOME": cls.scratch.name, "HERMES_AGENT_TIMEOUT": "0"}
        )
        cls.environment.start()
        from gateway import run
        from gateway.config import GatewayConfig, Platform, PlatformConfig
        from gateway.platforms.base import BasePlatformAdapter, MessageEvent, SendResult
        from gateway.session import SessionSource

        cls.gateway = run
        cls.MessageEvent = MessageEvent
        cls.source = SessionSource(
            platform=Platform.LOCAL,
            chat_id="synthetic-room",
            chat_type="group",
            thread_id="synthetic-thread",
        )
        cls.GatewayConfig = GatewayConfig

        class Adapter(BasePlatformAdapter):
            def __init__(self):
                super().__init__(PlatformConfig(enabled=True), Platform.LOCAL)
                self.sent = []

            async def connect(self, **kwargs):
                return True

            async def disconnect(self):
                pass

            async def send(self, chat_id, content, **kwargs):
                self.sent.append(content)
                return SendResult(success=True, message_id="synthetic-reply")

            async def get_chat_info(self, chat_id):
                return {"name": "synthetic", "type": "group"}

        cls.Adapter = Adapter

    @classmethod
    def tearDownClass(cls):
        cls.environment.stop()
        cls.scratch.cleanup()

    def setUp(self):
        self.config_patch = patch.object(
            self.gateway, "_load_gateway_config", return_value={"display": {"tool_progress": "off"}}
        )
        self.config_patch.start()
        self.addCleanup(self.config_patch.stop)
        self.runner = self.gateway.GatewayRunner(
            self.GatewayConfig(sessions_dir=Path(self.scratch.name) / self.id())
        )
        self.runner._session_db = None
        self.runner._persist_active_agents = lambda: None
        self.addCleanup(self.runner.close_all_session_db_handles)
        self.addCleanup(self.runner.session_store.close_all_db_handles)
        self.addCleanup(self.runner._shutdown_executor)
        self.adapter = self.Adapter()
        self.runner.adapters[self.source.platform] = self.adapter
        self.key = self.runner._session_key_for_source(self.source)
        self.generation = self.runner._begin_session_run_generation(self.key)
        self.calls = []

    async def turn(self, message="original", generation=None):
        return await asyncio.wait_for(
            self.runner._run_agent_inner(
                message,
                "",
                [],
                self.source,
                "synthetic-session",
                session_key=self.key,
                run_generation=self.generation if generation is None else generation,
            ),
            timeout=15,
        )

    def result(self, turn, **extra):
        ctx = turn._ctx
        self.calls.append((ctx.message, ctx._run_still_current()))
        result = {
            "final_response": "done",
            "messages": [],
            "completed": True,
            "api_calls": 0,
            **extra,
        }
        ctx.result_holder[0] = result
        return result

    async def test_stop_during_worker_completion_does_not_start_queued_turn(self):
        """Stop yields during typing cleanup while the interrupted worker returns."""
        interrupted = threading.Event()
        started = asyncio.Event()
        loop = asyncio.get_running_loop()

        class Agent:
            def hard_interrupt(self, message=None):
                interrupted.set()

        agent = Agent()

        def run_sync(turn):
            if turn._ctx.message == "original":
                turn._ctx.agent_holder[0] = agent
                loop.call_soon_threadsafe(started.set)
                if not interrupted.wait(2):
                    raise AssertionError("stop did not reach active worker")
                return self.result(turn, interrupted=True, interrupt_message="Stop requested")
            return self.result(turn)

        self.adapter._pending_messages[self.key] = self.MessageEvent(
            text="queued follow-up", source=self.source
        )
        with patch.object(self.gateway.TurnRunner, "run_sync", run_sync):
            task = asyncio.create_task(self.turn())
            await asyncio.wait_for(started.wait(), 2)
            # The production tracker publishes the agent asynchronously. Keep
            # this test focused on completion/stop ordering, after publication.
            self.runner._session_state(self.key).turn.agent = agent

            async def stop_typing(*args, **kwargs):
                await asyncio.wait_for(asyncio.shield(task), 2)

            with patch.object(self.adapter, "stop_typing", stop_typing):
                reply = await self.runner._busy_stop_command(
                    self.MessageEvent(text="/stop", source=self.source), self.key, self.source
                )
            await task
            self.assertTrue(reply)
            self.assertEqual(self.calls, [("original", False)])
            self.assertNotIn(self.key, self.adapter._pending_messages)
            # A genuinely new message gets a fresh generation and still runs.
            next_generation = self.runner._begin_session_run_generation(self.key)
            await self.turn("new message", next_generation)
            self.assertEqual(self.calls[-1], ("new message", True))

    async def test_already_stopped_generation_cannot_enter_worker(self):
        self.runner._invalidate_session_run_generation(self.key, reason="stop_command")
        with patch.object(self.gateway.TurnRunner, "run_sync", lambda turn: self.result(turn)):
            result = await self.turn()
        self.assertEqual(self.calls, [])
        self.assertTrue(result["interrupted"])

    async def test_stop_does_not_leave_a_followup_holding_the_durable_lease(self):
        """Real adapter cancellation, gateway queue drain, and SQLite lease.

        Only inference is replaced by a worker with event-controlled timing.
        A competing DB handle must acquire the existing session after stop.
        """
        from hermes_state import SessionDB

        database = Path(self.scratch.name) / "lease-proof.db"
        owner = SessionDB(database)
        contender = SessionDB(database)
        self.addCleanup(owner.close)
        self.addCleanup(contender.close)
        owner.create_session("synthetic-session", "local")
        owner.append_message("synthetic-session", "user", "existing question")
        owner.append_message("synthetic-session", "assistant", "existing answer")
        transcript = owner.get_messages("synthetic-session")
        before = owner.get_session("synthetic-session")
        started = asyncio.Event()
        followup_started = asyncio.Event()
        interrupted = threading.Event()
        finish_followup = threading.Event()
        loop = asyncio.get_running_loop()

        class Agent:
            def hard_interrupt(self, message=None):
                interrupted.set()

        agent = Agent()

        def run_sync(turn):
            holder = "original-worker" if turn._ctx.message == "original" else "followup-worker"
            if not owner.try_acquire_session_turn_lease("synthetic-session", holder):
                raise AssertionError("synthetic worker could not acquire its session")
            try:
                if holder == "original-worker":
                    turn._ctx.agent_holder[0] = agent
                    loop.call_soon_threadsafe(started.set)
                    if not interrupted.wait(2):
                        raise AssertionError("stop did not interrupt worker")
                    return self.result(turn, interrupted=True, pending_steer="follow-up")
                loop.call_soon_threadsafe(followup_started.set)
                if not finish_followup.wait(5):
                    raise AssertionError("test did not release follow-up worker")
                return self.result(turn)
            finally:
                owner.release_session_turn_lease("synthetic-session", holder)

        async def handle_stop(event):
            return await self.runner._busy_stop_command(event, self.key, self.source)

        with patch.object(self.gateway.TurnRunner, "run_sync", run_sync):
            task = asyncio.create_task(self.turn())
            waiter = asyncio.create_task(followup_started.wait())
            try:
                await asyncio.wait_for(started.wait(), 2)
                self.runner._session_state(self.key).turn.agent = agent
                self.adapter._active_sessions[self.key] = asyncio.Event()
                self.adapter._session_tasks[self.key] = task
                self.adapter.set_message_handler(handle_stop)

                async def stop_typing(*args, **kwargs):
                    done, _ = await asyncio.wait(
                        {task, waiter}, timeout=2, return_when=asyncio.FIRST_COMPLETED
                    )
                    if not done:
                        raise AssertionError("neither original nor follow-up reached the barrier")

                with patch.object(self.adapter, "stop_typing", stop_typing):
                    await self.adapter._dispatch_active_session_command(
                        self.MessageEvent(text="/stop", source=self.source), self.key, "stop"
                    )
                acquired = contender.try_acquire_session_turn_lease(
                    "synthetic-session", "next-real-turn"
                )
                if acquired:
                    contender.release_session_turn_lease("synthetic-session", "next-real-turn")
                self.assertTrue(acquired, "untracked follow-up still owns the lease after /stop")
                self.assertFalse(followup_started.is_set())
                self.assertEqual(contender.get_session("synthetic-session"), before)
                self.assertEqual(contender.get_messages("synthetic-session"), transcript)
            finally:
                interrupted.set()
                finish_followup.set()
                waiter.cancel()
                await asyncio.gather(task, waiter, return_exceptions=True)
                # A cancelled asyncio task does not join its underlying worker.
                executor = self.runner._executor
                self.runner._shutdown_executor()
                if executor is not None:
                    await asyncio.to_thread(executor.shutdown, wait=True)

    async def test_stale_completion_preserves_successor_queue(self):
        replacement = self.MessageEvent(text="new owner's message", source=self.source)

        def run_sync(turn):
            result = self.result(turn, interrupted=True)
            if len(self.calls) == 1:
                self.runner._begin_session_run_generation(self.key)
                self.adapter._pending_messages[self.key] = replacement
            return result

        with patch.object(self.gateway.TurnRunner, "run_sync", run_sync):
            await self.turn()
        self.assertEqual(len(self.calls), 1)
        self.assertIs(self.adapter._pending_messages[self.key], replacement)

    async def test_stop_during_followup_preparation_cannot_start_it(self):
        def run_sync(turn):
            return self.result(turn, **({"pending_steer": "follow-up"} if not self.calls else {}))

        async def send_typing(*args, **kwargs):
            self.runner._invalidate_session_run_generation(self.key, reason="stop_command")

        with (
            patch.object(self.gateway.TurnRunner, "run_sync", run_sync),
            patch.object(self.adapter, "send_typing", send_typing),
        ):
            await self.turn()
        self.assertEqual(self.calls, [("original", True)])

    async def test_stop_during_stream_finalization_preserves_successor_queue(self):
        replacement = self.MessageEvent(text="new owner's message", source=self.source)

        class Stream:
            done = True
            suppress_whole_file = False

            def finish(self):
                pass

            async def wait_complete(stream, **kwargs):
                self.runner._begin_session_run_generation(self.key)
                self.adapter._pending_messages[self.key] = replacement

        def run_sync(turn):
            turn._ctx.streaming_tts_consumer_holder[0] = Stream()
            return self.result(turn)

        with patch.object(self.gateway.TurnRunner, "run_sync", run_sync):
            await self.turn()
        self.assertEqual(self.calls, [("original", True)])
        self.assertIs(self.adapter._pending_messages[self.key], replacement)

    async def test_current_generation_still_drains_leftover_steer(self):
        def run_sync(turn):
            return self.result(turn, **({"pending_steer": "follow-up"} if not self.calls else {}))

        with patch.object(self.gateway.TurnRunner, "run_sync", run_sync):
            await self.turn()
        self.assertEqual(self.calls, [("original", True), ("follow-up", True)])


if __name__ == "__main__":
    unittest.main()

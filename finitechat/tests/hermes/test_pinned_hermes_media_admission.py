"""Busy-session media retains its lease until its own turn completes.

Use the real pinned adapter dispatch and runner, including its recursive
pending-slot drain. Replace inference and media preprocessing, not queueing
or completion. The sidecar fixture models ack/release without network access.
"""

import asyncio
import os
import tempfile
import threading
import unittest
from unittest.mock import patch

from gateway import run as gateway

from tests.hermes.test_pinned_hermes_stop_settlement import StopHarness, raw_event, raw_photo


class MediaHarness(StopHarness):
    def __init__(self, home):
        super().__init__(home)
        self.loop = asyncio.get_running_loop()
        self.gates = {"msg-1": threading.Event()}
        self.entered = {}
        self.received = {}
        self.tasks = {}
        self.failures = set()
        self.runner._prepare_profile_scoped_inbound_message_text = self._prepare_media
        self.adapter.set_busy_session_handler(self.runner._handle_active_session_busy_message)

    async def _prepare_media(self, *, event, **_kwargs):
        self.received[event.message_id] = event
        return event.text

    async def _handle(self, event):
        key = self.runner._session_key_for_source(event.source)
        if event.text == "/stop":
            return await self.runner._busy_stop_command(event, key, event.source)
        self.received[event.message_id] = event
        self.tasks[event.message_id] = asyncio.current_task()
        generation = self.runner._begin_session_run_generation(key)
        result = await self.runner._run_agent_inner(
            event.text,
            "",
            [],
            event.source,
            f"synthetic-{key}",
            session_key=key,
            run_generation=generation,
            event_message_id=event.message_id,
        )
        return result.get("final_response", "")

    def run_sync(self, turn):
        ctx = turn._ctx
        message_id = ctx.event_message_id
        gate = self.gates.get(message_id)

        class Agent:
            def hard_interrupt(self, message=None):
                del message
                if gate is not None:
                    gate.set()

        agent = Agent()
        ctx.agent_holder[0] = agent

        def entered():
            self.runner._session_state(ctx.session_key).turn.agent = agent
            self.runs.append(message_id)
            self.entered.setdefault(message_id, asyncio.Event()).set()

        self.loop.call_soon_threadsafe(entered)
        if gate is not None and not gate.wait(10):
            raise AssertionError(f"test did not release {message_id}")
        if message_id in self.failures:
            raise RuntimeError("synthetic inference failure")
        result = {"final_response": "done", "messages": [], "completed": True, "api_calls": 0}
        ctx.result_holder[0] = result
        return result

    async def wait_running(self, message_id):
        await asyncio.wait_for(self.entered.setdefault(message_id, asyncio.Event()).wait(), 5)

    async def close(self):
        for gate in self.gates.values():
            gate.set()
        executor = self.runner._executor
        await super().close()
        if executor is not None:
            executor.shutdown(wait=True)


class PinnedHermesMediaAdmissionTests(unittest.TestCase):
    def run_scenario(self, scenario):
        with (
            tempfile.TemporaryDirectory(prefix="finite-media-") as home,
            patch.dict(os.environ, {"HERMES_HOME": home, "HERMES_AGENT_TIMEOUT": "0"}),
            patch.object(
                gateway, "_load_gateway_config", return_value={"display": {"tool_progress": "off"}}
            ),
        ):

            async def main():
                h = MediaHarness(home)
                try:
                    with patch.object(
                        gateway.TurnRunner, "run_sync", lambda turn: h.run_sync(turn)
                    ):
                        await scenario(h)
                finally:
                    await h.close()

            asyncio.run(main())

    def test_busy_photo_is_acked_after_it_runs_and_does_not_replay(self):
        async def scenario(h):
            h.gates["msg-2"] = threading.Event()
            await h.deliver(raw_event(1, "long running work"))
            await h.wait_running("msg-1")
            await h.deliver(raw_photo(2))
            self.assertEqual(h.state("msg-2"), "leased")
            h.gates["msg-1"].set()
            await h.wait_running("msg-2")
            self.assertEqual(h.state("msg-2"), "leased", "dispatch must not ack the photo")
            h.gates["msg-2"].set()
            await h.wait_settled("msg-1")
            await h.wait_settled("msg-2")
            self.assertEqual(h.state("msg-2"), "acked")
            self.assertEqual(h.received["msg-2"].media_urls, ["/synthetic/photo-2.png"])
            await h.tick()
            self.assertEqual(h.runs, ["msg-1", "msg-2"])
            self.assertEqual(h.settled.count(("ack", "msg-2")), 1)

        self.run_scenario(scenario)

    def test_each_media_message_keeps_its_own_lease_and_order(self):
        async def scenario(h):
            await h.deliver(raw_event(1, "long running work"))
            await h.wait_running("msg-1")
            events = [raw_photo(2), raw_photo(3)]
            for seq, (kind, mime) in enumerate(
                (("video", "video/mp4"), ("audio", "audio/ogg"), ("file", "application/pdf")), 4
            ):
                event = raw_photo(seq)
                event["attachments"][0]["kind"] = kind
                event["attachments"][0]["mime_type"] = mime
                events.append(event)
            events.append(raw_event(7, "text after the media"))
            for event in events:
                h.gates[event["message_id"]] = threading.Event()
                await h.deliver(event)
                self.assertNotEqual(h.state(event["message_id"]), "acked")
            self.assertEqual(h.runs, ["msg-1"])
            h.gates["msg-1"].set()
            for event in events:
                if h.state(event["message_id"]) == "pending":
                    await h.tick()
                await h.wait_running(event["message_id"])
                self.assertEqual(h.state(event["message_id"]), "leased")
                h.gates[event["message_id"]].set()
                await h.wait_settled(event["message_id"])
                await asyncio.wait_for(asyncio.shield(h.tasks[event["message_id"]]), 5)
            self.assertEqual(h.runs, [f"msg-{seq}" for seq in range(1, 8)])
            self.assertEqual(len(set(h.tasks.values())), 7, "each message owns a separate turn")
            for event in events:
                self.assertEqual(h.state(event["message_id"]), "acked")
                expected_media = [attachment["path"] for attachment in event["attachments"]]
                self.assertEqual(h.received[event["message_id"]].media_urls, expected_media)

        self.run_scenario(scenario)

    def test_stream_batch_cannot_overtake_an_earlier_released_photo(self):
        async def scenario(h):
            await h.deliver(raw_event(1, "long running work"))
            await h.wait_running("msg-1")
            for seq in (2, 3, 4):
                await h.deliver(raw_photo(seq))

            # The real sidecar leases a batch before streaming its records.
            # Pause delivery between two records while the running turn and
            # held head finish. A released record must not lose its place to
            # the later record already travelling in the same stream batch.
            batch = [raw for raw, state in h.inbox.values() if state == "pending"]
            for raw in batch:
                h.inbox[raw["message_id"]] = (raw, "leased")
            if batch:
                await h.adapter._handle_finitechat_event(batch[0])
            h.gates["msg-1"].set()
            await h.wait_settled("msg-1")
            await h.wait_settled("msg-2")
            await asyncio.wait_for(asyncio.shield(h.tasks["msg-2"]), 5)
            for raw in batch[1:]:
                await h.adapter._handle_finitechat_event(raw)
                await h.settle_loop()
            await h.tick()
            for seq in (3, 4):
                await h.wait_running(f"msg-{seq}")
                await h.wait_settled(f"msg-{seq}")
            self.assertEqual(h.runs, ["msg-1", "msg-2", "msg-3", "msg-4"])

        self.run_scenario(scenario)

    def test_shutdown_before_media_admission_keeps_it_for_restart(self):
        async def scenario(h):
            await h.deliver(raw_event(1, "long running work"))
            await h.wait_running("msg-1")
            await h.deliver(raw_photo(2))
            # The production disconnect cancels admission tasks before turns.
            await h.adapter._cancel_admission_tasks()
            await h.adapter.cancel_background_tasks()
            self.assertEqual(h.state("msg-1"), "pending")
            self.assertEqual(h.state("msg-2"), "pending")
            self.assertNotIn(("ack", "msg-2"), h.settled)
            self.assertEqual(h.runs, ["msg-1"])

            # A fresh adapter accepts the released photo using the unchanged
            # sidecar contract, without waiting for lease expiry or migration.
            with tempfile.TemporaryDirectory(prefix="finite-media-restarted-") as home:
                restarted = MediaHarness(home)
                try:
                    with patch.object(
                        gateway.TurnRunner, "run_sync", lambda turn: restarted.run_sync(turn)
                    ):
                        await restarted.deliver(h.inbox["msg-2"][0])
                        await restarted.wait_settled("msg-2")
                    self.assertEqual(restarted.state("msg-2"), "acked")
                    self.assertEqual(restarted.runs, ["msg-2"])
                finally:
                    await restarted.close()

        self.run_scenario(scenario)

    def test_queued_and_running_lease_redelivery_does_not_duplicate_turns(self):
        async def scenario(h):
            await h.deliver(raw_event(1, "long running work"))
            await h.wait_running("msg-1")
            for seq in (2, 3, 4):
                await h.deliver(raw_photo(seq))
            # Simulate the unchanged sidecar renewing leases after its TTL,
            # including an event already running and several still queued.
            await h.deliver(raw_event(1, "long running work"))
            for seq in (4, 3, 2):
                await h.deliver(raw_photo(seq))
            self.assertEqual(h.settled, [], "a busy queue must not spin release/redelivery")
            h.gates["msg-1"].set()
            for seq in (1, 2, 3, 4):
                await h.wait_settled(f"msg-{seq}")
            self.assertEqual(h.runs, ["msg-1", "msg-2", "msg-3", "msg-4"])
            self.assertEqual(h.settled, [("ack", f"msg-{seq}") for seq in (1, 2, 3, 4)])

        self.run_scenario(scenario)

    def test_graceful_shutdown_releases_the_entire_queue_in_order(self):
        async def scenario(h):
            await h.deliver(raw_event(1, "long running work"))
            await h.wait_running("msg-1")
            for seq in (2, 3, 4):
                await h.deliver(raw_photo(seq))
            await h.adapter._cancel_admission_tasks()
            await h.adapter.cancel_background_tasks()
            self.assertEqual(h.runs, ["msg-1"])
            for seq in (1, 2, 3, 4):
                self.assertEqual(h.state(f"msg-{seq}"), "pending")
            self.assertFalse(any(action == "ack" for action, _ in h.settled))
            self.assertEqual(h.adapter._deferred_admissions, {})

        self.run_scenario(scenario)

    def test_failed_head_handoff_retries_before_later_photos(self):
        async def scenario(h):
            await h.deliver(raw_event(1, "long running work"))
            await h.wait_running("msg-1")
            for seq in (2, 3, 4):
                await h.deliver(raw_photo(seq))
            original = h.adapter.handle_message
            attempts = []

            async def transient_failure(event):
                attempts.append(event.message_id)
                if len(attempts) == 1:
                    raise RuntimeError("synthetic handoff failure")
                return await original(event)

            h.adapter.handle_message = transient_failure
            h.gates["msg-1"].set()
            for seq in (1, 2, 3, 4):
                await h.wait_settled(f"msg-{seq}")
                await h.wait_running(f"msg-{seq}")
            self.assertEqual(h.runs, ["msg-1", "msg-2", "msg-3", "msg-4"])
            self.assertEqual(attempts, ["msg-2", "msg-2", "msg-3", "msg-4"])

        self.run_scenario(scenario)

    def test_stop_between_turns_discards_queued_handoff(self):
        async def scenario(h):
            await h.deliver(raw_event(1, "long running work"))
            await h.wait_running("msg-1")
            for seq in (2, 3, 4):
                await h.deliver(raw_photo(seq))
            entered = asyncio.Event()
            release = asyncio.Event()
            original = h.adapter._set_processing_activity

            async def delayed_activity(*args):
                if not entered.is_set():
                    entered.set()
                    await release.wait()
                return await original(*args)

            h.adapter._set_processing_activity = delayed_activity
            h.gates["msg-1"].set()
            await asyncio.wait_for(entered.wait(), 5)
            await h.deliver(raw_event(5, "/stop"))
            await h.wait_settled("msg-5")
            release.set()
            await h.settle_loop()
            for seq in (2, 3, 4):
                self.assertEqual(h.state(f"msg-{seq}"), "acked")
            self.assertEqual(h.runs, ["msg-1"])
            await h.deliver(raw_event(6, "after stop"))
            await h.wait_settled("msg-6")
            self.assertEqual(h.runs, ["msg-1", "msg-6"])

        self.run_scenario(scenario)

    def test_persistent_handoff_failure_does_not_spin_and_stop_still_works(self):
        async def scenario(h):
            await h.deliver(raw_event(1, "long running work"))
            await h.wait_running("msg-1")
            await h.deliver(raw_photo(2))
            original = h.adapter.handle_message
            attempted = asyncio.Event()
            attempts = []

            async def failure(event):
                if event.message_id == "msg-2":
                    attempts.append(event.message_id)
                    attempted.set()
                    raise RuntimeError("persistent synthetic handoff failure")
                return await original(event)

            h.adapter.handle_message = failure
            h.gates["msg-1"].set()
            await asyncio.wait_for(attempted.wait(), 5)
            await asyncio.sleep(0.2)
            self.assertEqual(attempts, ["msg-2"])
            await h.deliver(raw_event(3, "/stop"))
            self.assertNotIn("msg-2", h.runs)
            self.assertEqual(h.adapter._deferred_admissions, {})

        self.run_scenario(scenario)

    def test_cancelled_handoff_clears_activity_and_inflight_marker(self):
        async def scenario(h):
            await h.deliver(raw_event(1, "long running work"))
            await h.wait_running("msg-1")
            await h.deliver(raw_photo(2))
            entered = asyncio.Event()
            original = h.adapter.handle_message
            cleared = []

            async def stalled_handoff(event):
                if event.message_id == "msg-2":
                    entered.set()
                    await asyncio.Event().wait()
                return await original(event)

            async def clear_activity(*args):
                cleared.append(args)

            h.adapter.handle_message = stalled_handoff
            h.adapter._clear_processing_activity = clear_activity
            h.gates["msg-1"].set()
            await asyncio.wait_for(entered.wait(), 5)
            await h.deliver(raw_event(3, "/stop"))
            await h.wait_settled("msg-3")
            self.assertEqual(h.state("msg-2"), "acked")
            self.assertEqual(h.adapter._inflight_admissions, set())
            self.assertTrue(cleared)
            self.assertEqual(h.runs, ["msg-1"])

        self.run_scenario(scenario)

    def test_stale_inflight_marker_does_not_suppress_recovery(self):
        async def scenario(h):
            event = raw_photo(2)
            key = h.module._adapter_event_key(event["room_id"], 2, "msg-2")
            # Model an old pending event whose completion hook was lost. Once
            # the session has no owner, its expired lease must be admissible.
            h.adapter._inflight_admissions.add(key)
            await h.deliver(event)
            await h.wait_settled("msg-2")
            self.assertEqual(h.runs, ["msg-2"])
            self.assertEqual(h.state("msg-2"), "acked")
            self.assertEqual(h.adapter._inflight_admissions, set())

        self.run_scenario(scenario)

    def test_busy_media_does_not_block_or_settle_another_session(self):
        async def scenario(h):
            await h.deliver(raw_event(1, "long running work"))
            await h.wait_running("msg-1")
            await h.deliver(raw_photo(2))
            other = raw_photo(3)
            other["source"]["user_id"] = "bob"
            other["segment_id"] = "segment-bob"
            other["source"]["thread_id"] = "segment-bob"
            await h.deliver(other)
            await h.wait_running("msg-3")
            await h.wait_settled("msg-3")
            self.assertEqual(h.runs, ["msg-1", "msg-3"])
            self.assertEqual(h.state("msg-2"), "leased")
            self.assertEqual(h.state("msg-3"), "acked")
            await h.deliver(raw_event(4, "/stop"))
            self.assertEqual(h.state("msg-2"), "acked")
            self.assertEqual(h.settled.count(("ack", "msg-3")), 1)

        self.run_scenario(scenario)

    def test_shutdown_during_media_turn_releases_it(self):
        async def scenario(h):
            h.gates["msg-2"] = threading.Event()
            await h.deliver(raw_event(1, "long running work"))
            await h.wait_running("msg-1")
            await h.deliver(raw_photo(2))
            h.gates["msg-1"].set()
            await h.wait_running("msg-2")
            await h.adapter.cancel_background_tasks()
            self.assertEqual(h.state("msg-1"), "acked")
            self.assertEqual(h.state("msg-2"), "pending")

        self.run_scenario(scenario)

    def test_stop_during_media_turn_is_final(self):
        async def scenario(h):
            h.gates["msg-2"] = threading.Event()
            await h.deliver(raw_event(1, "long running work"))
            await h.wait_running("msg-1")
            await h.deliver(raw_photo(2))
            h.gates["msg-1"].set()
            await h.wait_running("msg-2")
            await h.deliver(raw_event(3, "/stop"))
            await h.tick()
            self.assertEqual(h.runs, ["msg-1", "msg-2"])
            for message_id in ("msg-1", "msg-2", "msg-3"):
                self.assertEqual(h.state(message_id), "acked")

        self.run_scenario(scenario)

    def test_media_failure_is_settled_without_retrying_inference(self):
        async def scenario(h):
            h.failures.add("msg-2")
            await h.deliver(raw_event(1, "long running work"))
            await h.wait_running("msg-1")
            await h.deliver(raw_photo(2))
            h.gates["msg-1"].set()
            await h.wait_settled("msg-1")
            await h.wait_settled("msg-2")
            self.assertEqual(h.state("msg-2"), "acked")
            await h.tick()
            self.assertEqual(h.runs, ["msg-1", "msg-2"])

        self.run_scenario(scenario)

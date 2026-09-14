"""Exercise the packaged Hermes owner, not an adapter imitation of its queues."""

import asyncio
import importlib.util
import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import AsyncMock, patch

from gateway.config import PlatformConfig
from gateway.platforms.base import MessageDisposition
from gateway.run import GatewayRunner

ADAPTER_PATH = Path(__file__).resolve().parents[2] / "integrations/hermes/finitechat/adapter.py"
spec = importlib.util.spec_from_file_location("finite_delivery_contract_adapter", ADAPTER_PATH)
assert spec and spec.loader
adapter_module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = adapter_module
spec.loader.exec_module(adapter_module)


class PinnedHermesDeliveryContractTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.home = tempfile.TemporaryDirectory()
        self.addCleanup(self.home.cleanup)
        self.env = patch.dict(
            os.environ, {"HERMES_HOME": self.home.name, "HERMES_HUMAN_DELAY_MODE": "off"}
        )
        self.env.start()
        self.addCleanup(self.env.stop)
        self.adapter = adapter_module.FiniteChatAdapter(
            PlatformConfig(
                extra={
                    "home": self.home.name,
                    "finitechat_bin": "/bin/false",
                    "room_id": "room",
                }
            )
        )
        self.adapter._home_channel_hydrated = True
        self.calls = []

        async def service(action, payload, *, timeout):
            self.calls.append((action, payload))
            return adapter_module._FiniteChatResult(True, {"message_id": "reply"}, None, False)

        self.adapter._finitechat_json = service
        usage = patch.object(adapter_module, "_finite_private_control_request", return_value=None)
        usage.start()
        self.addCleanup(usage.stop)

    async def asyncTearDown(self):
        await self.adapter._cancel_admission_tasks()
        await self.adapter.cancel_background_tasks()

    def raw(self, seq, text, chat="chat"):
        return {
            "room_id": "room",
            "seq": seq,
            "message_id": f"message-{seq}",
            "text": text,
            "message_type": "text",
            "conversation_id": "topic",
            "segment_id": chat,
            "source": {"user_id": "aa" * 32, "chat_id": "room", "chat_type": "dm"},
        }

    def settled(self, action):
        return [payload["message_id"] for kind, payload in self.calls if kind == action]

    async def start_blocked(self):
        started, finish = asyncio.Event(), asyncio.Event()
        seen = []

        async def handler(event):
            seen.append(event.text)
            if event.text == "first":
                started.set()
                await finish.wait()
            return "reply to " + event.text

        self.adapter.set_message_handler(handler)
        await self.adapter._handle_finitechat_event(self.raw(1, "first"))
        await asyncio.wait_for(started.wait(), 2)
        return finish, seen

    async def idle(self):
        # Observation only: tests may inspect Hermes's actual task ownership.
        for _ in range(100):
            tasks = list(self.adapter._background_tasks) + list(
                self.adapter._admission_tasks.values()
            )
            if not tasks:
                return
            await asyncio.wait(tasks, timeout=0.02)
        self.fail("Hermes failed to become idle")

    async def test_ordered_inputs_stay_unacked_while_controls_bypass(self):
        finish, seen = await self.start_blocked()
        await self.adapter._handle_finitechat_event(self.raw(2, "second"))
        await self.adapter._handle_finitechat_event(self.raw(3, "third"))
        self.assertEqual(self.settled("ack"), [])
        self.assertEqual(self.settled("release"), ["message-3"])
        self.assertEqual(self.adapter._pending_messages, {})
        await self.adapter._handle_finitechat_event(self.raw(4, "/status"))
        self.assertEqual(self.settled("ack"), ["message-4"])
        self.assertEqual(seen, ["first", "/status"])
        finish.set()
        await self.idle()
        self.assertEqual(seen, ["first", "/status", "second"])
        await self.adapter._handle_finitechat_event(self.raw(3, "third"))
        await self.idle()
        self.assertEqual(seen, ["first", "/status", "second", "third"])
        self.assertEqual(self.settled("ack"), ["message-4", "message-1", "message-2", "message-3"])

    async def test_stop_finishes_command_before_deferred_head_starts(self):
        _, seen = await self.start_blocked()
        await self.adapter._handle_finitechat_event(self.raw(2, "second"))
        await self.adapter._handle_finitechat_event(self.raw(3, "/stop"))
        await self.idle()
        self.assertEqual(seen, ["first", "/stop", "second"])
        self.assertEqual(self.settled("release"), ["message-1"])
        self.assertEqual(self.settled("ack"), ["message-3", "message-2"])

    async def test_wait_cancellation_does_not_cancel_active_turn_or_ack_head(self):
        finish, seen = await self.start_blocked()
        await self.adapter._handle_finitechat_event(self.raw(2, "second"))
        await self.adapter._cancel_admission_tasks()
        self.assertEqual(self.settled("ack"), [])
        self.assertEqual(self.settled("release"), [])
        finish.set()
        await self.idle()
        # Simulate Rust redelivery after the abandoned head's lease expires.
        await self.adapter._handle_finitechat_event(self.raw(2, "second"))
        await self.idle()
        self.assertEqual(seen, ["first", "second"])
        self.assertEqual(self.settled("ack"), ["message-1", "message-2"])

    async def test_actual_cancelled_background_turn_releases_without_ack(self):
        await self.start_blocked()
        await self.adapter.cancel_background_tasks()
        self.assertEqual(self.settled("ack"), [])
        self.assertEqual(self.settled("release"), ["message-1"])

    async def test_handler_failure_is_terminal_but_admission_failure_is_not(self):
        async def fail(_event):
            raise ValueError("model failure")

        self.adapter.set_message_handler(fail)
        await self.adapter._handle_finitechat_event(self.raw(1, "fail"))
        await self.idle()
        self.assertEqual(self.settled("ack"), ["message-1"])
        self.adapter.admit_message = AsyncMock(side_effect=RuntimeError("not ready"))
        with self.assertRaises(RuntimeError):
            await self.adapter._handle_finitechat_event(self.raw(2, "unaccepted"))
        self.assertEqual(self.settled("release"), ["message-2"])

    async def test_inline_handler_exception_releases_control(self):
        finish, _ = await self.start_blocked()
        original = self.adapter._message_handler

        async def handler(event):
            if event.text == "/status":
                raise RuntimeError("inline failure")
            return await original(event)

        self.adapter.set_message_handler(handler)
        with self.assertRaises(RuntimeError):
            await self.adapter._handle_finitechat_event(self.raw(2, "/status"))
        self.assertEqual(self.settled("release"), ["message-2"])
        self.assertEqual(self.settled("ack"), [])
        finish.set()
        await self.idle()

    async def test_runner_authorization_and_plain_approval_remain_canonical(self):
        finish, _ = await self.start_blocked()
        runner = object.__new__(GatewayRunner)
        runner._is_user_authorized = lambda source, **_kwargs: True
        runner._effective_busy_input_mode = lambda source: "queue"
        runner._draining = False
        runner._handle_approve_command = AsyncMock(return_value=None)
        runner._handle_deny_command = AsyncMock(return_value=None)
        runner._adapter_for_source = lambda source: self.adapter
        self.adapter.set_busy_session_handler(runner._handle_active_session_busy_message)
        with patch("tools.approval.has_blocking_approval", return_value=True):
            await self.adapter._handle_finitechat_event(self.raw(2, "yes"))
        runner._handle_approve_command.assert_awaited_once()
        self.assertEqual(self.settled("ack"), ["message-2"])
        runner._is_user_authorized = lambda source, **_kwargs: False
        await self.adapter._handle_finitechat_event(self.raw(3, "unauthorized"))
        self.assertEqual(self.settled("ack"), ["message-2", "message-3"])
        self.assertEqual(self.adapter._pending_messages, {})
        finish.set()
        await self.idle()

    async def test_deferred_head_cannot_become_a_later_clarification_answer(self):
        finish, seen = await self.start_blocked()
        await self.adapter._handle_finitechat_event(self.raw(2, "ordinary follow-up"))
        # Even an explicit re-admission while a later prompt exists stays deferred.
        event = next(iter(self.adapter._deferred_admissions.values()))[0]
        with patch("tools.clarify_gateway.get_pending_for_session", return_value=object()):
            receipt = await self.adapter.admit_message(event)
        self.assertIs(receipt.disposition, MessageDisposition.DEFERRED)
        self.assertEqual(seen, ["first"])
        finish.set()
        await self.idle()
        self.assertEqual(seen, ["first", "ordinary follow-up"])

    async def test_service_response_lost_never_resends_or_falls_back(self):
        # Run the actual service request + actual Hermes retry wrapper. The
        # stand-in server records acceptance, then loses its response.
        self.adapter.service_url = "http://127.0.0.1:12345"
        del self.adapter._finitechat_json
        accepted = []

        def lost(request, timeout):
            accepted.append(json.loads(request.data))
            raise adapter_module.urllib.error.URLError("connection reset after acceptance")

        with patch.object(adapter_module.urllib.request, "urlopen", side_effect=lost):
            result = await self.adapter._send_with_retry(
                "room", "the generated answer", base_delay=0
            )
        self.assertFalse(result.success)
        self.assertEqual(result.delivery_disposition, "unknown")
        self.assertEqual(len(accepted), 1)
        self.assertEqual(accepted[0]["text"], "the generated answer")

    async def test_invalid_success_receipt_is_unknown_without_wrapper_resend(self):
        self.adapter.service_url = "http://127.0.0.1:12345"
        del self.adapter._finitechat_json
        for data in ({}, [], {"message_id": None}, {"message_id": 12}, {"message_id": " "}):
            with self.subTest(receipt=data):

                class Response:
                    def __init__(self, body):
                        self.body = body

                    def __enter__(self):
                        return self

                    def __exit__(self, *_args):
                        pass

                    def read(self):
                        return self.body

                with patch.object(
                    adapter_module.urllib.request,
                    "urlopen",
                    return_value=Response(json.dumps(data).encode()),
                ) as send:
                    result = await self.adapter._send_with_retry("room", "answer", base_delay=0)
                self.assertEqual(send.call_count, 1)
                self.assertFalse(result.success)
                self.assertEqual(result.delivery_disposition, "unknown")

    async def test_typed_refusal_not_overridden_by_retry_flag_or_error_text(self):
        self.adapter.service_url = "http://127.0.0.1:12345"
        del self.adapter._finitechat_json
        for retryable in (False, True):
            body = json.dumps(
                {
                    "error": "connection reset, timeout",
                    "error_kind": "hermes",
                    "retryable": retryable,
                }
            ).encode()
            refusal = adapter_module.urllib.error.HTTPError(
                "http://local", 409, "refused", {}, io.BytesIO(body)
            )
            with patch.object(
                adapter_module.urllib.request, "urlopen", side_effect=refusal
            ) as send:
                result = await self.adapter._send_with_retry("room", "answer", base_delay=0)
            self.assertEqual(send.call_count, 1)
            self.assertFalse(result.success)
            self.assertEqual(result.retryable, retryable)
            self.assertEqual(result.error_kind, "hermes")

    async def test_background_final_response_never_registers_an_outbound_obligation(self):
        async def handler(_event):
            return "Generated answer"

        self.adapter.set_message_handler(handler)
        with (
            patch("gateway.delivery_ledger.ledger_enabled", return_value=True),
            patch("gateway.delivery_ledger.record_obligation") as record,
        ):
            await self.adapter._handle_finitechat_event(self.raw(1, "question"))
            await self.idle()
        record.assert_not_called()
        self.assertEqual(
            [payload["text"] for action, payload in self.calls if action == "send"],
            ["Generated answer"],
        )
        self.assertEqual(self.settled("ack"), ["message-1"])

    async def test_background_media_unknown_does_not_send_a_failure_notice(self):
        media = Path(self.home.name) / "result.pdf"
        media.write_bytes(b"%PDF-1.4 synthetic test document")
        original_service = self.adapter._finitechat_json

        async def service(action, payload, *, timeout):
            if action == "send":
                self.calls.append((action, payload))
                return adapter_module._FiniteChatResult(
                    False, {}, "response lost", False, outcome_unknown=True
                )
            return await original_service(action, payload, timeout=timeout)

        async def handler(_event):
            return f"MEDIA:{media}"

        self.adapter._finitechat_json = service
        self.adapter.set_message_handler(handler)
        self.adapter._notify_media_delivery_failure = AsyncMock()
        await self.adapter._handle_finitechat_event(self.raw(1, "make a PDF"))
        await self.idle()
        sends = [payload for action, payload in self.calls if action == "send"]
        self.assertEqual(len(sends), 1)
        self.assertEqual(sends[0]["attachments"][0]["path"], str(media.resolve()))
        self.adapter._notify_media_delivery_failure.assert_not_awaited()
        self.assertEqual(self.settled("ack"), ["message-1"])

    async def test_busy_session_does_not_block_a_different_chat(self):
        finish, seen = await self.start_blocked()
        await self.adapter._handle_finitechat_event(self.raw(2, "other chat", chat="other"))
        for task in list(self.adapter._background_tasks):
            if task is not next(iter(self.adapter._session_tasks.values())):
                await asyncio.wait_for(task, 2)
        self.assertIn("other chat", seen)
        self.assertEqual(self.settled("ack"), ["message-2"])
        finish.set()
        await self.idle()

    async def test_historical_outbound_obligation_never_replayed(self):
        runner = object.__new__(GatewayRunner)
        runner.adapters = {self.adapter.platform: self.adapter}
        self.adapter.send = AsyncMock()
        count = await runner._redeliver_claimed_obligations(
            [
                {
                    "platform": self.adapter.platform.value,
                    "obligation_id": "old-attempt",
                    "content": "previously generated response",
                    "chat_id": "room",
                    "attempts": 1,
                }
            ]
        )
        self.assertEqual(count, 0)
        self.adapter.send.assert_not_awaited()

    async def test_unsupported_runtime_fails_before_recovery_or_inbound(self):
        with patch.object(adapter_module.BasePlatformAdapter, "DELIVERY_CONTRACT_VERSION", 0):
            self.assertFalse(await self.adapter.connect())
        self.assertEqual(self.calls, [])


if __name__ == "__main__":
    unittest.main()

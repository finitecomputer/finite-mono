"""Inference route notices at the Finite Chat adapter boundary (DESIGN §7.3).

The observer records, per Finite conversation and turn, which routes the main
agent tried (`pre_api_request`), which failed and why (`api_request_error`),
and which answered (`post_api_request`). `on_processing_complete` turns that
record into at most one notice that states only what was observed. The
adapter also swallows pinned Hermes's own route-switch status lines, whose
wording is rendered here from the pinned Hermes source (T-F11).

No test starts a Hermes gateway. Hermes modules are imported under a scratch
HERMES_HOME because importing them creates files there.
"""

from __future__ import annotations

import ast
import asyncio
import contextvars
import importlib
import importlib.util
import json
import os
import sys
import tempfile
import threading
import types
import unittest
from pathlib import Path
from typing import Any, cast
from unittest.mock import patch

REPO_ROOT = Path(__file__).resolve().parents[2]
MONOREPO_ROOT = REPO_ROOT.parent
HARNESS_PATH = REPO_ROOT / "tests" / "hermes" / "test_finite_platform_adapter.py"
GATEWAY_MODULE_NAMES = (
    "gateway",
    "gateway.config",
    "gateway.platforms",
    "gateway.platforms.base",
    "gateway.session_context",
)

# Reuse the canonical adapter test harness (fake gateway classes and the
# recorded `_finitechat_json` stub), path-loaded like the settlement gates.
_HARNESS_SPEC = importlib.util.spec_from_file_location(
    "finite_platform_adapter_test_harness", HARNESS_PATH
)
if _HARNESS_SPEC is None or _HARNESS_SPEC.loader is None:
    raise RuntimeError(f"failed to load harness from {HARNESS_PATH}")
harness: Any = importlib.util.module_from_spec(_HARNESS_SPEC)
sys.modules["finite_platform_adapter_test_harness"] = harness
_HARNESS_SPEC.loader.exec_module(harness)

FP_URL = "https://finite-private.example.invalid/v1"
OPENROUTER_URL = "https://openrouter.ai/api/v1"
CODEX_URL = "https://chatgpt.com/backend-api/codex"
USER_CUSTOM_URL = "https://llm.example.invalid/v1"
ROOM = "room-agent-1"
MAIN_SESSION = "20260927_120000_abc123"
REPLY_METADATA = {"conversation_id": "topic-1", "segment_id": "chat-a", "notify": True}

_SCRATCH_HERMES_HOME: tempfile.TemporaryDirectory | None = None
_HERMES_HOME_PATCH: Any = None
hermes: dict[str, Any] = {}


def setUpModule() -> None:
    global _SCRATCH_HERMES_HOME, _HERMES_HOME_PATCH
    _SCRATCH_HERMES_HOME = tempfile.TemporaryDirectory(prefix="finitechat-notice-hermes-")
    _HERMES_HOME_PATCH = patch.dict(os.environ, {"HERMES_HOME": _SCRATCH_HERMES_HOME.name})
    _HERMES_HOME_PATCH.start()
    for name in (
        "agent.chat_completion_helpers",
        "agent.agent_runtime_helpers",
        "agent.delegation_context",
        "agent.error_classifier",
    ):
        hermes[name] = importlib.import_module(name)


def tearDownModule() -> None:
    if _HERMES_HOME_PATCH is not None:
        _HERMES_HOME_PATCH.stop()
    if _SCRATCH_HERMES_HOME is not None:
        _SCRATCH_HERMES_HOME.cleanup()


# The harness stub keeps session values in one module-level dict; the
# observer reads them per hook call, so a ContextVar-backed stub gives each
# simulated agent thread its own binding, as the real gateway does.
_SESSION_VALUES: contextvars.ContextVar[dict[str, str] | None] = contextvars.ContextVar(
    "test_session_values", default=None
)


def _get_session_env(name: str, default: str = "") -> str:
    return (_SESSION_VALUES.get() or {}).get(name, default)


class _NoticeTestCase(unittest.TestCase):
    def setUp(self):
        self.original_gateway_modules = {
            name: sys.modules.get(name) for name in GATEWAY_MODULE_NAMES
        }
        self.module = harness.load_adapter_module()
        cast(Any, sys.modules["gateway.session_context"]).get_session_env = _get_session_env
        state_home = tempfile.TemporaryDirectory()
        self.addCleanup(state_home.cleanup)
        self.state_home = state_home.name
        fp_endpoint = self.module._endpoint_identity(FP_URL)
        self._patch(self.module, "_configured_finite_private_endpoint", lambda: fp_endpoint)
        # The Finite Private usage notice is covered elsewhere; keep it inert.
        self._patch(self.module, "_finite_private_control_request", lambda path, method: None)
        self.ctx = harness.MockPluginContext()
        with (
            tempfile.TemporaryDirectory() as finite_home,
            patch.dict(os.environ, {"FINITE_HOME": finite_home}),
        ):
            self.module.register(self.ctx)
        self.hooks = self.ctx.registered_hooks

    def tearDown(self):
        for name, module in self.original_gateway_modules.items():
            if module is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = module

    def _patch(self, target, name, value):
        patcher = patch.object(target, name, value)
        patcher.start()
        self.addCleanup(patcher.stop)

    def adapter(self):
        extra = {"home": self.state_home, "finitechat_bin": "/bin/echo", "room_id": ROOM}
        adapter = self.module.FiniteChatAdapter(harness.PlatformConfig(extra=extra))
        self.calls: list[tuple[str, dict[str, Any], int]] = []

        async def fake_json(action, payload, *, timeout):
            self.calls.append((action, payload, timeout))
            return self.module._FiniteChatResult(True, {"message_id": "m-1"}, None, False)

        adapter._finitechat_json = fake_json
        return adapter

    # -- simulated agent turns -------------------------------------------------

    @staticmethod
    def binding(thread: str = "chat-a", *, platform: str = "local", chat: str = ROOM):
        return {
            "HERMES_SESSION_PLATFORM": platform,
            "HERMES_SESSION_CHAT_ID": chat,
            "HERMES_SESSION_THREAD_ID": thread,
            # The gateway binds an empty id on every turn; only a fresh agent's
            # construction rebinds it, so cached-agent turns see "".
            "HERMES_SESSION_ID": "",
        }

    def fire(
        self,
        hook: str,
        provider: str,
        base_url: str,
        *,
        turn: str = "turn-1",
        thread: str = "chat-a",
        session_id: str = MAIN_SESSION,
        task_id: str | None = None,
        reason: str | None = None,
        binding: dict[str, str] | None = None,
        delegated: bool = False,
    ) -> Any:
        kwargs: dict[str, Any] = {
            "task_id": session_id if task_id is None else task_id,
            "turn_id": turn,
            "api_request_id": f"{turn}:req",
            "session_id": session_id,
            "platform": "finitechat",
            "model": "some-model",
            "provider": provider,
            "base_url": base_url,
            "api_mode": "chat_completions",
            "telemetry_schema_version": 1,
        }
        if hook == "api_request_error":
            kwargs.update(
                reason=reason,
                status_code=402,
                error={"type": "APIStatusError", "message": "provider said no"},
            )

        def run():
            _SESSION_VALUES.set(self.binding(thread) if binding is None else binding)
            if delegated:
                hermes["agent.delegation_context"]._DELEGATED_CHILD_CONTEXT.set(True)
            return self.hooks[hook](**kwargs)

        return contextvars.copy_context().run(run)

    def fallback_turn(
        self, primary=("openrouter", OPENROUTER_URL), reason: str | None = "billing", **kw
    ):
        """Primary errors, Finite Private answers."""
        self.fire("pre_api_request", *primary, **kw)
        self.fire("api_request_error", *primary, reason=reason, **kw)
        self.fire("pre_api_request", "custom", FP_URL, **kw)
        self.fire("post_api_request", "custom", FP_URL, **kw)

    def event(self, thread: str = "chat-a", *, conversation: str = "topic-1", internal=False):
        return harness.MessageEvent(
            text="hello",
            source=types.SimpleNamespace(chat_id=ROOM, thread_id=thread),
            raw_message={"room_id": ROOM, "conversation_id": conversation, "segment_id": thread},
            internal=internal,
        )

    def complete(self, adapter, event, outcome: str = "success") -> list[dict[str, Any]]:
        before = len(self.calls)
        asyncio.run(adapter.on_processing_complete(event, types.SimpleNamespace(value=outcome)))
        return [payload for action, payload, _ in self.calls[before:] if action == "send"]

    def notice_text(self, sends: list[dict[str, Any]]) -> str:
        self.assertEqual(len(sends), 1, sends)
        return sends[0]["text"]


class DecisionTableTests(_NoticeTestCase):
    """T-F1: the §7.3 decision table."""

    def test_finite_private_answering_after_a_primary_error_sends_one_notice(self):
        adapter = self.adapter()
        self.fallback_turn()
        sends = self.complete(adapter, self.event())

        self.assertEqual(len(sends), 1)
        payload = sends[0]
        self.assertEqual(
            payload["text"],
            "Finite Private answered this response because OpenRouter is out of credits or quota.",
        )
        self.assertEqual(payload["kind"], "message")
        self.assertEqual(payload["status"], "complete")
        self.assertEqual(payload["conversation_id"], "topic-1")
        self.assertEqual(payload["segment_id"], "chat-a")
        self.assertEqual(
            payload["metadata"],
            {
                "finite_notice": {
                    "v": 1,
                    "type": "inference_fallback",
                    "attempted": "openrouter",
                    "served_by": "finite_private",
                    "reason": "billing",
                }
            },
        )
        self.assertNotIn("notify", payload["metadata"])

    def test_codex_primary_is_labelled_chatgpt(self):
        adapter = self.adapter()
        self.fallback_turn(primary=("openai-codex", CODEX_URL), reason="rate_limit")
        sends = self.complete(adapter, self.event())
        self.assertEqual(
            self.notice_text(sends),
            "Finite Private answered this response because ChatGPT is rate-limited.",
        )
        self.assertEqual(sends[0]["metadata"]["finite_notice"]["attempted"], "openai_codex")

    def test_every_reason_maps_to_its_phrase(self):
        expected = {
            "rate_limit": "is rate-limited",
            "upstream_rate_limit": "is rate-limited",
            "billing": "is out of credits or quota",
            "auth": "rejected its sign-in",
            "auth_permanent": "rejected its sign-in",
            "overloaded": "is having problems",
            "server_error": "is having problems",
            "timeout": "is having problems",
            "model_not_found": "doesn't have that model",
            "content_policy_blocked": "declined this request",
            "provider_policy_blocked": "declined this request",
            "format_error": "rejected the request",
        }
        failover_values = {
            reason.value for reason in hermes["agent.error_classifier"].FailoverReason
        }
        self.assertLessEqual(set(expected), failover_values)
        for index, (reason, phrase) in enumerate(expected.items()):
            with self.subTest(reason=reason):
                adapter = self.adapter()
                self.fallback_turn(reason=reason, turn=f"turn-{index}")
                self.assertEqual(
                    self.notice_text(self.complete(adapter, self.event())),
                    f"Finite Private answered this response because OpenRouter {phrase}.",
                )

    def test_unmapped_or_missing_reason_states_only_that_an_error_was_returned(self):
        for index, reason in enumerate((None, "unknown", "invalid_response", "context_overflow")):
            with self.subTest(reason=reason):
                adapter = self.adapter()
                self.fallback_turn(reason=reason, turn=f"turn-{index}")
                sends = self.complete(adapter, self.event())
                self.assertEqual(
                    self.notice_text(sends),
                    "Finite Private answered this response after OpenRouter returned an error.",
                )
                self.assertEqual(sends[0]["metadata"]["finite_notice"]["reason"], reason)

    def test_unknown_provider_is_your_selected_model(self):
        adapter = self.adapter()
        self.fallback_turn(primary=("anthropic", "https://api.anthropic.com"), reason="overloaded")
        sends = self.complete(adapter, self.event())
        self.assertEqual(
            self.notice_text(sends),
            "Finite Private answered this response because your selected model is having problems.",
        )
        self.assertEqual(sends[0]["metadata"]["finite_notice"]["attempted"], "other")

    def test_a_user_custom_backup_is_never_called_finite_private(self):
        adapter = self.adapter()
        self.fire("pre_api_request", "openrouter", OPENROUTER_URL)
        self.fire("api_request_error", "openrouter", OPENROUTER_URL, reason="timeout")
        # Same provider name as the bare Finite Private primary, other endpoint.
        self.fire("pre_api_request", "custom", USER_CUSTOM_URL)
        self.fire("post_api_request", "custom", USER_CUSTOM_URL)
        sends = self.complete(adapter, self.event())

        self.assertEqual(
            self.notice_text(sends),
            "Your backup model answered this response because OpenRouter is having problems.",
        )
        self.assertEqual(
            sends[0]["metadata"]["finite_notice"],
            {
                "v": 1,
                "type": "inference_backup",
                "attempted": "openrouter",
                "served_by": "backup",
                "reason": "timeout",
            },
        )

    def test_finite_private_is_identified_by_provider_and_configured_endpoint(self):
        cases = {
            "finite-private at another path": ("finite-private", FP_URL + "/other", False),
            "custom at a case-changed path": ("custom", FP_URL.replace("/v1", "/V1"), False),
            "an unrelated provider at the FP endpoint": ("openrouter", FP_URL, False),
            "host case and trailing slash": (
                "custom:finite-private",
                "HTTPS://Finite-Private.Example.INVALID/v1/",
                True,
            ),
        }
        for index, (name, (provider, url, is_fp)) in enumerate(cases.items()):
            with self.subTest(name):
                adapter = self.adapter()
                turn = f"turn-{index}"
                self.fire("pre_api_request", "openrouter", OPENROUTER_URL, turn=turn)
                self.fire(
                    "api_request_error", "openrouter", OPENROUTER_URL, reason="billing", turn=turn
                )
                self.fire("pre_api_request", provider, url, turn=turn)
                self.fire("post_api_request", provider, url, turn=turn)
                text = self.notice_text(self.complete(adapter, self.event()))
                self.assertEqual(text.startswith("Finite Private answered"), is_fp, text)

    def test_without_a_configured_finite_private_endpoint_nothing_is_called_finite_private(self):
        self._patch(self.module, "_configured_finite_private_endpoint", lambda: None)
        adapter = self.adapter()
        self.fallback_turn()
        text = self.notice_text(self.complete(adapter, self.event()))
        self.assertTrue(text.startswith("Your backup model answered"), text)

    def test_both_routes_failing_names_both(self):
        adapter = self.adapter()
        self.fire("pre_api_request", "openrouter", OPENROUTER_URL)
        self.fire("api_request_error", "openrouter", OPENROUTER_URL, reason="billing")
        self.fire("pre_api_request", "finite-private", FP_URL)
        self.fire("api_request_error", "finite-private", FP_URL, reason="server_error")
        sends = self.complete(adapter, self.event(), "failure")

        self.assertEqual(
            self.notice_text(sends), "OpenRouter and Finite Private couldn't answer this message."
        )
        self.assertEqual(
            sends[0]["metadata"]["finite_notice"],
            {
                "v": 1,
                "type": "inference_fallback_failed",
                "attempted": "openrouter",
                "served_by": None,
                "reason": "billing",
            },
        )

    def test_failure_label_is_capitalized_for_an_unknown_provider(self):
        adapter = self.adapter()
        self.fire("pre_api_request", "anthropic", "https://api.anthropic.com")
        self.fire("api_request_error", "anthropic", "https://api.anthropic.com", reason="auth")
        self.fire("pre_api_request", "custom", FP_URL)
        sends = self.complete(adapter, self.event(), "failure")
        self.assertEqual(
            self.notice_text(sends),
            "Your selected model and Finite Private couldn't answer this message.",
        )

    def test_failure_without_a_finite_private_attempt_sends_nothing(self):
        adapter = self.adapter()
        self.fire("pre_api_request", "openrouter", OPENROUTER_URL)
        self.fire("api_request_error", "openrouter", OPENROUTER_URL, reason="billing")
        self.assertEqual(self.complete(adapter, self.event(), "failure"), [])

    def test_notice_never_rides_a_brain_approval_card(self):
        adapter = self.adapter()
        filings = self.module._BRAIN_APPROVAL_FILINGS
        filings.after_tool_call(
            tool_name="terminal",
            result="finite-brain-approval-filed brain=team request=req-1\n",
        )
        self.addCleanup(filings.mark_reported, {"req-1"})
        self.fallback_turn()
        sends = self.complete(adapter, self.event())
        self.assertNotIn("approve", sends[0]["metadata"])
        self.assertEqual([filing["requestId"] for filing in filings.take_pending()], ["req-1"])


class UpstreamStatusTests(_NoticeTestCase):
    """T-F2: `send_or_update_status` drops only the upstream route-switch lines."""

    FALLBACK_LINE = (
        "⚠️ Model fallback: openai/gpt-4o-mini via openrouter unavailable "
        "(billing or quota exhausted); using glm-5-3-flash via finite-private."
    )
    RESTORE_LINE = (
        "✅ Primary model restored: openai/gpt-4o-mini via openrouter; "
        "fallback glm-5-3-flash via finite-private is no longer active."
    )

    def test_lifecycle_route_switch_lines_are_not_sent(self):
        adapter = self.adapter()
        for line in (self.FALLBACK_LINE, self.RESTORE_LINE, f"  {self.FALLBACK_LINE}\n"):
            with self.subTest(line=line):
                result = asyncio.run(
                    adapter.send_or_update_status(
                        ROOM, "lifecycle", line, metadata={"thread_id": "chat-a"}
                    )
                )
                self.assertTrue(result.success)
        self.assertEqual(self.calls, [])

    def test_everything_else_is_sent_exactly_as_send_would(self):
        cases = [
            ("lifecycle", "⚠️ Rate limited, retrying in 5s"),
            ("lifecycle", f"{self.FALLBACK_LINE} Extra."),
            ("lifecycle", f"Note: {self.RESTORE_LINE}"),
            ("retry", self.FALLBACK_LINE),
            ("warning", self.RESTORE_LINE),
        ]
        for status_key, content in cases:
            with self.subTest(status_key=status_key, content=content):
                metadata = {"conversation_id": "topic-1", "segment_id": "chat-a", "extra": 1}
                adapter = self.adapter()
                asyncio.run(adapter.send(ROOM, content, metadata=dict(metadata)))
                expected = self.calls
                adapter = self.adapter()
                result = asyncio.run(
                    adapter.send_or_update_status(
                        ROOM, status_key, content, metadata=dict(metadata)
                    )
                )
                self.assertTrue(result.success)
                self.assertEqual(self.calls, expected)


class NoNoticeTests(_NoticeTestCase):
    """T-F3: normal Finite Private serving and a primary that answers are silent."""

    def test_finite_private_serving_alone_sends_nothing(self):
        routes = {
            "FP default (bare custom)": ("custom", FP_URL),
            "conversation override to FP": ("finite-private", FP_URL),
            "channel override to FP": ("custom:finite-private", FP_URL),
            "--once to FP": ("finite-private", FP_URL + "/"),
        }
        for index, (name, route) in enumerate(routes.items()):
            with self.subTest(name):
                adapter = self.adapter()
                turn = f"turn-{index}"
                self.fire("pre_api_request", *route, turn=turn)
                self.fire("post_api_request", *route, turn=turn)
                self.fire("pre_api_request", *route, turn=turn)
                self.fire("post_api_request", *route, turn=turn)
                self.assertEqual(self.complete(adapter, self.event()), [])

    def test_finite_private_error_then_finite_private_success_sends_nothing(self):
        adapter = self.adapter()
        self.fire("pre_api_request", "custom", FP_URL)
        self.fire("api_request_error", "custom", FP_URL, reason="timeout")
        self.fire("pre_api_request", "finite-private", FP_URL)
        self.fire("post_api_request", "finite-private", FP_URL)
        self.assertEqual(self.complete(adapter, self.event()), [])

    def test_primary_answering_sends_nothing(self):
        adapter = self.adapter()
        self.fire("pre_api_request", "openrouter", OPENROUTER_URL)
        self.fire("post_api_request", "openrouter", OPENROUTER_URL)
        self.assertEqual(self.complete(adapter, self.event()), [])

    def test_primary_retry_that_answers_sends_nothing(self):
        adapter = self.adapter()
        self.fire("pre_api_request", "openrouter", OPENROUTER_URL)
        self.fire("api_request_error", "openrouter", OPENROUTER_URL, reason="timeout")
        self.fire("pre_api_request", "openrouter", OPENROUTER_URL)
        self.fire("post_api_request", "openrouter", OPENROUTER_URL)
        self.assertEqual(self.complete(adapter, self.event()), [])

    def test_a_turn_with_no_observed_request_sends_nothing(self):
        # V18: a fallback Hermes picks while resolving credentials makes no
        # request on the primary, so the observer sees only Finite Private.
        adapter = self.adapter()
        self.fire("pre_api_request", "custom", FP_URL)
        self.fire("post_api_request", "custom", FP_URL)
        self.assertEqual(self.complete(adapter, self.event()), [])
        self.assertEqual(self.complete(adapter, self.event("chat-never-seen")), [])


class AttributionTests(_NoticeTestCase):
    """T-F4 to T-F7: records are per conversation and per turn."""

    def test_concurrent_conversations_in_one_room_are_not_cross_attributed(self):
        adapter = self.adapter()
        self.fire("pre_api_request", "openrouter", OPENROUTER_URL, thread="chat-a", turn="a-1")
        self.fire("pre_api_request", "openrouter", OPENROUTER_URL, thread="chat-b", turn="b-1")
        self.fire(
            "api_request_error",
            "openrouter",
            OPENROUTER_URL,
            thread="chat-a",
            turn="a-1",
            reason="billing",
        )
        self.fire("post_api_request", "openrouter", OPENROUTER_URL, thread="chat-b", turn="b-1")
        self.fire("pre_api_request", "custom", FP_URL, thread="chat-a", turn="a-1")
        self.fire("post_api_request", "custom", FP_URL, thread="chat-a", turn="a-1")

        self.assertEqual(self.complete(adapter, self.event("chat-b", conversation="topic-b")), [])
        sends = self.complete(adapter, self.event("chat-a", conversation="topic-a"))
        self.assertEqual(len(sends), 1)
        self.assertEqual(sends[0]["segment_id"], "chat-a")
        self.assertEqual(sends[0]["conversation_id"], "topic-a")

    def test_threaded_turns_record_only_their_own_conversation(self):
        adapter = self.adapter()
        barrier = threading.Barrier(2)
        errors: list[BaseException] = []

        def run_turn(thread: str, failing: bool):
            try:
                barrier.wait()
                for _ in range(50):
                    if failing:
                        self.fallback_turn(thread=thread, turn=f"{thread}-turn")
                    else:
                        self.fire(
                            "pre_api_request",
                            "openrouter",
                            OPENROUTER_URL,
                            thread=thread,
                            turn=f"{thread}-turn",
                        )
                        self.fire(
                            "post_api_request",
                            "openrouter",
                            OPENROUTER_URL,
                            thread=thread,
                            turn=f"{thread}-turn",
                        )
            except BaseException as exc:
                errors.append(exc)

        workers = [
            threading.Thread(target=run_turn, args=("chat-a", True)),
            threading.Thread(target=run_turn, args=("chat-b", False)),
        ]
        for worker in workers:
            worker.start()
        for worker in workers:
            worker.join()
        self.assertEqual(errors, [])
        self.assertEqual(self.complete(adapter, self.event("chat-b")), [])
        self.assertEqual(len(self.complete(adapter, self.event("chat-a"))), 1)

    def test_a_new_turn_id_resets_the_conversation_record(self):
        adapter = self.adapter()
        # A queued earlier turn's failure must not describe the next turn.
        self.fire("pre_api_request", "openrouter", OPENROUTER_URL, turn="turn-1")
        self.fire(
            "api_request_error", "openrouter", OPENROUTER_URL, turn="turn-1", reason="billing"
        )
        self.fire("pre_api_request", "custom", FP_URL, turn="turn-2")
        self.fire("post_api_request", "custom", FP_URL, turn="turn-2")
        self.assertEqual(self.complete(adapter, self.event()), [])

    def test_delegated_and_review_requests_are_ignored(self):
        adapter = self.adapter()
        # Main turn: the primary errors, then Finite Private answers.
        self.fallback_turn()
        # A delegated child runs under Hermes's delegated-child context with its
        # own session and task ids; its primary answers.
        for delegated, session_id, task_id in (
            (True, "child-session", "child-task"),
            (True, MAIN_SESSION, MAIN_SESSION),
            (False, "child-session", "child-task"),
            # The background review fork shares the parent's session id.
            (False, MAIN_SESSION, "0b9d3f7e-review-task"),
        ):
            for hook in ("pre_api_request", "api_request_error", "post_api_request"):
                self.fire(
                    hook,
                    "user-custom",
                    USER_CUSTOM_URL,
                    session_id=session_id,
                    task_id=task_id,
                    turn="child-turn",
                    delegated=delegated,
                    reason="auth",
                )
        self.assertEqual(
            self.notice_text(self.complete(adapter, self.event())),
            "Finite Private answered this response because OpenRouter is out of credits or quota.",
        )

    def test_delegated_requests_alone_produce_nothing(self):
        adapter = self.adapter()
        self.fallback_turn(session_id="child-session", task_id="child-task", delegated=True)
        self.fallback_turn(task_id="review-task")
        self.assertEqual(self.complete(adapter, self.event()), [])

    def test_requests_outside_a_finite_conversation_are_ignored(self):
        adapter = self.adapter()
        self.fallback_turn(binding=self.binding(platform="telegram"))
        self.fallback_turn(binding=self.binding(chat=""))
        self.fallback_turn(binding={})
        self.assertEqual(self.complete(adapter, self.event()), [])

    def test_internal_event_produces_nothing_and_clears_the_record(self):
        adapter = self.adapter()
        self.fallback_turn()
        self.assertEqual(self.complete(adapter, self.event(internal=True)), [])
        self.assertEqual(self.complete(adapter, self.event()), [])

    def test_cancellation_clears_the_record(self):
        adapter = self.adapter()
        self.fallback_turn()
        self.assertEqual(self.complete(adapter, self.event(), "cancelled"), [])
        self.assertEqual(self.complete(adapter, self.event()), [])

    def test_the_record_is_consumed_once(self):
        adapter = self.adapter()
        self.fallback_turn()
        self.assertEqual(len(self.complete(adapter, self.event())), 1)
        self.assertEqual(self.complete(adapter, self.event()), [])

    def test_a_reason_comes_only_from_the_same_turn(self):
        adapter = self.adapter()
        self.fire("pre_api_request", "openrouter", OPENROUTER_URL, turn="turn-1")
        self.fire(
            "api_request_error", "openrouter", OPENROUTER_URL, turn="turn-1", reason="billing"
        )
        self.fallback_turn(turn="turn-2", reason=None)
        self.assertEqual(
            self.notice_text(self.complete(adapter, self.event())),
            "Finite Private answered this response after OpenRouter returned an error.",
        )


class ObserverNeverDisturbsDeliveryTests(_NoticeTestCase):
    """A failing observer leaves every message delivered unchanged."""

    def reply_payload(self) -> dict[str, Any]:
        adapter = self.adapter()
        asyncio.run(adapter.send(ROOM, "the answer", metadata=dict(REPLY_METADATA)))
        return self.calls[-1][1]

    def test_each_hook_swallows_its_own_failure(self):
        baseline = self.reply_payload()

        def explode(kwargs):
            raise RuntimeError("sk-or-v1-secret-must-not-appear")

        self._patch(self.module, "_inference_turn_key", explode)
        for hook in ("pre_api_request", "api_request_error", "post_api_request"):
            with self.subTest(hook=hook):
                self.assertIsNone(self.fire(hook, "openrouter", OPENROUTER_URL, reason="billing"))
        self.assertEqual(self.reply_payload(), baseline)

    def test_a_failing_decision_still_settles_the_turn_and_sends_nothing_extra(self):
        adapter = self.adapter()
        self.fallback_turn()

        def explode(*args):
            raise RuntimeError("sk-or-v1-secret-must-not-appear")

        self._patch(self.module._INFERENCE_ROUTE_OBSERVER, "take_notice", explode)
        event = self.event()
        event.raw_message.update({"seq": 7, "message_id": "msg-7"})
        with self.assertLogs("finite_platform_adapter_under_test", "WARNING") as logs:
            self.assertEqual(self.complete(adapter, event), [])
        self.assertNotIn("secret", "\n".join(logs.output))
        self.assertIn("RuntimeError", "\n".join(logs.output))
        self.assertEqual([action for action, _, _ in self.calls], ["ack"])

    def test_a_failing_notice_send_does_not_raise(self):
        adapter = self.adapter()

        async def failing_json(action, payload, *, timeout):
            self.calls.append((action, payload, timeout))
            if action == "send":
                return self.module._FiniteChatResult(False, {}, "sidecar down", True)
            return self.module._FiniteChatResult(True, {}, None, False)

        adapter._finitechat_json = failing_json
        self.fallback_turn()
        self.assertEqual(len(self.complete(adapter, self.event())), 1)

    def test_a_failing_status_filter_still_sends_the_status_unchanged(self):
        class Exploding:
            def fullmatch(self, text):
                raise RuntimeError("boom")

        adapter = self.adapter()
        asyncio.run(
            adapter.send(ROOM, UpstreamStatusTests.FALLBACK_LINE, metadata=dict(REPLY_METADATA))
        )
        expected = self.calls
        self._patch(self.module, "FALLBACK_RE", Exploding())
        adapter = self.adapter()
        asyncio.run(
            adapter.send_or_update_status(
                ROOM, "lifecycle", UpstreamStatusTests.FALLBACK_LINE, metadata=dict(REPLY_METADATA)
            )
        )
        self.assertEqual(self.calls, expected)

    def test_a_recorded_fallback_never_changes_the_reply_itself(self):
        baseline = self.reply_payload()
        self.fallback_turn()
        self.assertEqual(self.reply_payload(), baseline)


def _hermes_pin() -> str:
    lock = json.loads((MONOREPO_ROOT / "flake.lock").read_text(encoding="utf-8"))
    return str(lock["nodes"]["hermes-agent"]["locked"]["rev"])


class PinnedUpstreamWordingTests(unittest.TestCase):
    """T-F11: the upstream route-switch lines, rendered from pinned Hermes."""

    def setUp(self):
        self.original_gateway_modules = {
            name: sys.modules.get(name) for name in GATEWAY_MODULE_NAMES
        }
        self.module = harness.load_adapter_module()

    def tearDown(self):
        for name, module in self.original_gateway_modules.items():
            if module is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = module

    def leak_message(self, what: str) -> str:
        return (
            f"Hermes pin {_hermes_pin()}: {what}. The adapter's FALLBACK_RE/RESTORE_RE no longer "
            "match the upstream route-switch line, so it would now leak into chat beside the "
            "Finite notice. Update the patterns in finitechat/integrations/hermes/finitechat/"
            "adapter.py to the new wording."
        )

    def render_upstream(self, module: Any, prefix: str, names: dict[str, Any]) -> list[str]:
        """Evaluate every f-string in `module` that starts with `prefix`."""
        source_path = Path(cast(str, module.__file__))
        tree = ast.parse(source_path.read_text(encoding="utf-8"), filename=str(source_path))
        rendered = []
        for node in ast.walk(tree):
            if not isinstance(node, ast.JoinedStr) or not node.values:
                continue
            head = node.values[0]
            if not (isinstance(head, ast.Constant) and str(head.value).startswith(prefix)):
                continue
            used = {name.id for name in ast.walk(node) if isinstance(name, ast.Name)}
            missing = used - set(names)
            if missing:
                self.fail(
                    self.leak_message(
                        f"{source_path.name}:{node.lineno} now formats {sorted(missing)}"
                    )
                )
            code = compile(ast.Expression(body=node), str(source_path), "eval")
            rendered.append(eval(code, {"__builtins__": {}}, dict(names)))
        if not rendered:
            self.fail(self.leak_message(f"no f-string starting {prefix!r} in {source_path}"))
        return rendered

    def test_fallback_line_matches_for_every_failover_reason(self):
        helpers = hermes["agent.chat_completion_helpers"]
        reasons = [*hermes["agent.error_classifier"].FailoverReason, None]
        for reason in reasons:
            names = {
                "old_model": "openai/gpt-4o-mini",
                "old_provider": "openrouter",
                "_fallback_reason_text": helpers._fallback_reason_text,
                "reason": reason,
                "fb_model": "glm-5-3-flash",
                "fb_provider": "finite-private",
            }
            for line in self.render_upstream(helpers, "⚠️ Model fallback: ", names):
                with self.subTest(reason=reason):
                    self.assertIsNotNone(
                        self.module.FALLBACK_RE.fullmatch(line), self.leak_message(repr(line))
                    )
                    self.assertTrue(self.module._is_upstream_route_status(line))

    def test_fallback_line_is_the_one_the_host_gateway_delivered(self):
        # Host gateway proof 4a, verbatim.
        helpers = hermes["agent.chat_completion_helpers"]
        names = {
            "old_model": "openai/gpt-4o-mini",
            "old_provider": "openrouter",
            "_fallback_reason_text": helpers._fallback_reason_text,
            "reason": hermes["agent.error_classifier"].FailoverReason.billing,
            "fb_model": "glm-5-3-flash",
            "fb_provider": "finite-private",
        }
        self.assertEqual(
            self.render_upstream(helpers, "⚠️ Model fallback: ", names),
            [UpstreamStatusTests.FALLBACK_LINE],
        )

    def test_restore_line_matches(self):
        runtime_helpers = hermes["agent.agent_runtime_helpers"]
        names = {
            "agent": types.SimpleNamespace(model="openai/gpt-4o-mini", provider="openrouter"),
            "previous_model": "glm-5-3-flash",
            "previous_provider": "finite-private",
        }
        lines = self.render_upstream(runtime_helpers, "✅ Primary model restored: ", names)
        self.assertEqual(lines, [UpstreamStatusTests.RESTORE_LINE])
        for line in lines:
            self.assertIsNotNone(
                self.module.RESTORE_RE.fullmatch(line), self.leak_message(repr(line))
            )

    def test_status_lines_still_reach_the_adapter_as_lifecycle(self):
        # Suppression keys on the gateway calling send_or_update_status with
        # status_key "lifecycle"; read both call sites from the pinned source.
        site_packages = Path(cast(str, hermes["agent.error_classifier"].__file__)).parents[1]
        run_agent = (site_packages / "run_agent.py").read_text(encoding="utf-8")
        gateway_run = (site_packages / "gateway" / "run.py").read_text(encoding="utf-8")
        self.assertIn(
            'self.status_callback("lifecycle", message)',
            run_agent,
            self.leak_message("AIAgent._emit_status no longer uses the lifecycle key"),
        )
        self.assertIn(
            "return await sender(chat_id, status_key, content, metadata=metadata)",
            gateway_run,
            self.leak_message("the gateway no longer calls send_or_update_status"),
        )


if __name__ == "__main__":
    unittest.main()

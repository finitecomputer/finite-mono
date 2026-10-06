"""Contract checks for the deterministic real-Hermes interruption smoke."""

from __future__ import annotations

import importlib.util
import subprocess
import unittest
from pathlib import Path
from unittest import mock

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPO_ROOT / "scripts" / "hermes-chat-interruption-docker-smoke.py"
WORKFLOW_PATH = REPO_ROOT.parent / ".github" / "workflows" / "hermes-runtime-smoke.yml"

spec = importlib.util.spec_from_file_location("hermes_chat_interruption_smoke", SCRIPT_PATH)
assert spec is not None and spec.loader is not None
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


class HermesChatInterruptionSmokeTest(unittest.TestCase):
    def test_fake_provider_stall_barrier_is_explicit(self) -> None:
        state = smoke.FakeModelState()
        stall = state.observe(
            {
                "stream": True,
                "messages": [
                    {"role": "user", "content": "FINITE_INTERRUPT_STALL:sigkill keep open"}
                ],
            }
        )

        self.assertEqual(stall, "sigkill")
        self.assertNotIn("sigkill", state.seen)
        state.mark_seen("sigkill")
        state.wait_seen("sigkill", timeout=0.1)

    def test_fake_provider_returns_the_requested_fresh_reply(self) -> None:
        self.assertEqual(
            smoke.expected_reply(
                {"messages": [{"role": "user", "content": "Reply with exactly: fresh chat two ok"}]}
            ),
            "fresh chat two ok",
        )

    def test_canonical_hermes_version_is_parsed_from_runtime_output(self) -> None:
        self.assertEqual(smoke.parse_hermes_version("Hermes Agent v0.18.2 (2026.7.7.2)"), "0.18.2")

    def test_matrix_keeps_the_three_bounded_cases(self) -> None:
        source = SCRIPT_PATH.read_text(encoding="utf-8")
        self.assertIn('interrupt("graceful-stop", kill=False, restore=False)', source)
        self.assertIn('interrupt("sigkill", kill=True, restore=False)', source)
        self.assertIn('interrupt("empty-target-restore", kill=False, restore=True)', source)
        self.assertIn(
            '"interruption_boundary": "SSE headers flushed before first data frame"', source
        )
        self.assertIn("wait_durable_inbox_event(name, queued_message_id)", source)
        self.assertIn('"first_next_ordinary_turn": True', source)
        self.assertIn('"model_handoffs": queued_handoffs', source)
        self.assertIn("expected SIGKILL exit 137", source)
        self.assertIn("escalated to SIGKILL instead of stopping gracefully", source)
        self.assertIn('"production_kata_task_and_stable_manifest_gate": False', source)

    def test_provider_summary_keeps_markers_and_outcomes_without_prompt_bodies(self) -> None:
        state = smoke.FakeModelState()
        stalled, _ = state.record(
            {
                "stream": True,
                "messages": [
                    {"role": "system", "content": "private system prompt"},
                    {"role": "user", "content": "FINITE_INTERRUPT_STALL:graceful-stop keep open"},
                ],
            }
        )
        queued, stall = state.record(
            {
                "stream": True,
                "messages": [
                    {"role": "user", "content": "Reply with exactly: graceful-stop queued ok"}
                ],
            }
        )
        self.assertIsNone(stall)
        state.finish(queued, "client_disconnected")

        summary = state.summary()

        self.assertEqual(summary["count"], 2)
        first, second = summary["requests"]
        self.assertEqual((first["stall"], first["outcome"]), ("graceful-stop", "pending"))
        self.assertEqual(second["latest_user_reply_marker"], "graceful-stop queued ok")
        self.assertEqual(second["outcome"], "client_disconnected")
        self.assertIsInstance(second["finished_at_ms"], int)
        self.assertNotIn("private system prompt", repr(summary))
        self.assertNotIn("keep open", repr(summary))
        self.assertEqual(stalled["outcome"], "pending")

    def test_provider_summary_is_bounded_to_the_latest_requests(self) -> None:
        state = smoke.FakeModelState()
        for index in range(5):
            state.observe(
                {"messages": [{"role": "user", "content": f"Reply with exactly: n{index}"}]}
            )

        summary = state.summary(limit=2)

        self.assertEqual(summary["count"], 5)
        self.assertEqual([request["index"] for request in summary["requests"]], [3, 4])

    def test_inbox_summary_reports_lease_state_without_event_payload(self) -> None:
        inbox = {
            "events": [
                {
                    "key": "room-a:7:msg-7",
                    "room_id": "room-a",
                    "seq": 7,
                    "message_id": "msg-7",
                    "created_at_ms": 900,
                    "event": {"text": "secret message body"},
                    "lease": {"state": "leased", "lease_id": "lease-1", "leased_at_ms": 1_000},
                },
                {
                    "key": "room-a:8:msg-8",
                    "seq": 8,
                    "message_id": "msg-8",
                    "event": {"text": "another body"},
                },
            ],
            "cursors": {"room-a": 8},
            "acked": [{"key": "room-a:6:msg-6", "acked_at_ms": 800}],
        }

        summary = smoke.summarize_hermes_inbox(inbox, now_ms=4_000)

        leased, legacy = summary["events"]
        self.assertEqual(leased["lease_state"], "leased")
        self.assertEqual(leased["lease_id"], "lease-1")
        self.assertEqual(leased["lease_age_ms"], 3_000)
        self.assertEqual(legacy["lease_state"], "pending")
        self.assertIsNone(legacy["lease_age_ms"])
        self.assertEqual(summary["cursors"], {"room-a": 8})
        self.assertEqual(summary["acked_count"], 1)
        self.assertNotIn("body", repr(summary))

    def test_inbox_summary_is_bounded(self) -> None:
        inbox = {"events": [{"key": f"k{index}"} for index in range(10)]}

        summary = smoke.summarize_hermes_inbox(inbox, now_ms=0, limit=3)

        self.assertEqual(summary["event_count"], 10)
        self.assertEqual([event["key"] for event in summary["events"]], ["k7", "k8", "k9"])

    def test_bounded_text_keeps_the_tail(self) -> None:
        self.assertEqual(smoke.bounded_text("short", 10), "short")
        bounded = smoke.bounded_text("a" * 20 + "TAIL", 4)
        self.assertTrue(bounded.endswith("TAIL"))
        self.assertLess(len(bounded), 24)

    def test_stopped_inbox_read_never_creates_a_missing_volume(self) -> None:
        missing = subprocess.CompletedProcess([], 1, "", "no such volume")
        with (
            mock.patch.object(smoke.smoke, "run", return_value=missing) as run,
            self.assertRaises(smoke.SmokeFailure),
        ):
            smoke.read_hermes_inbox(
                image="image", container="agent", home_volume="home", live=False
            )
        self.assertEqual(run.call_count, 1)
        self.assertEqual(run.call_args.args[0][:3], ["docker", "volume", "inspect"])

    def test_stopped_inbox_read_mounts_the_home_volume_read_only(self) -> None:
        ok = subprocess.CompletedProcess([], 0, '{"events":[]}', "")
        with mock.patch.object(smoke.smoke, "run", return_value=ok) as run:
            raw = smoke.read_hermes_inbox(
                image="image", container="agent", home_volume="home", live=False
            )
        self.assertEqual(raw, '{"events":[]}')
        command = run.call_args.args[0]
        self.assertIn("type=volume,src=home,dst=/home/node,readonly", command)
        self.assertEqual(command[-1], "/home/node/.finitechat/agent/hermes-inbox.json")

    def test_diagnostic_capture_records_read_failures_without_raising(self) -> None:
        state = smoke.FakeModelState()
        state.observe({"messages": [{"role": "user", "content": "Reply with exactly: x"}]})
        timeout = subprocess.TimeoutExpired(["docker"], 30)
        with (
            mock.patch.object(smoke.smoke, "run", side_effect=timeout),
            mock.patch.object(smoke.smoke, "agent_log_tail", side_effect=RuntimeError("gone")),
        ):
            snapshot = smoke.capture_diagnostics(
                "on_failure",
                image="image",
                container="agent",
                home_volume="home",
                live=True,
                model_state=state,
            )

        self.assertEqual(snapshot["point"], "on_failure")
        self.assertIn("TimeoutExpired", snapshot["hermes_inbox_error"])
        self.assertEqual(snapshot["agent_log_error"], "RuntimeError: gone")
        self.assertEqual(snapshot["provider"]["count"], 1)

    def test_diagnostic_capture_bounds_the_agent_log_tail(self) -> None:
        inbox = subprocess.CompletedProcess([], 0, '{"events":[]}', "")
        with (
            mock.patch.object(smoke.smoke, "run", return_value=inbox),
            mock.patch.object(smoke.smoke, "agent_log_tail", return_value="x" * 50_000) as tail,
        ):
            snapshot = smoke.capture_diagnostics(
                "before_stop",
                image="image",
                container="agent",
                home_volume="home",
                live=True,
                model_state=smoke.FakeModelState(),
            )

        self.assertEqual(tail.call_args.kwargs["lines"], smoke.DIAGNOSTIC_LOG_LINES)
        self.assertLessEqual(len(snapshot["agent_log_tail"]), smoke.DIAGNOSTIC_TEXT_LIMIT + 20)
        self.assertEqual(snapshot["hermes_inbox"]["event_count"], 0)

    def test_failure_report_records_stage_and_diagnostics_before_reraising(self) -> None:
        source = SCRIPT_PATH.read_text(encoding="utf-8")
        failure = source[source.index('report["status"] = "failed"') :]
        failure = failure[: failure.index("finally:")]
        self.assertIn('report["failure_stage"] = report.get("stage")', failure)
        self.assertIn('diagnose("on_failure", live=False)', failure)
        self.assertTrue(failure.rstrip().endswith("raise"))
        for point in ("before_stop", "before_container_removal", "after_restart"):
            self.assertIn(f'diagnose("{point}"', source)

    def test_dispatch_workflow_runs_and_uploads_the_matrix(self) -> None:
        workflow = WORKFLOW_PATH.read_text(encoding="utf-8")
        self.assertIn("chat_interruption_smoke:", workflow)
        self.assertIn("../#finitechat-server", workflow)
        self.assertIn("FINITECHAT_SERVER_BIN=", workflow)
        self.assertIn("scripts/hermes-chat-interruption-docker-smoke.py", workflow)
        self.assertIn("target/hermes-chat-interruption-docker-smoke/report.json", workflow)


if __name__ == "__main__":
    unittest.main()

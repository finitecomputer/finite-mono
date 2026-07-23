"""Contract checks for the deterministic real-Hermes interruption smoke."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

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

    def test_dispatch_workflow_runs_and_uploads_the_matrix(self) -> None:
        workflow = WORKFLOW_PATH.read_text(encoding="utf-8")
        self.assertIn("chat_interruption_smoke:", workflow)
        self.assertIn("cargo build --locked -p finitechat-server", workflow)
        self.assertIn("scripts/hermes-chat-interruption-docker-smoke.py", workflow)
        self.assertIn("target/hermes-chat-interruption-docker-smoke/report.json", workflow)


if __name__ == "__main__":
    unittest.main()

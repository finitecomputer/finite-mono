"""The rollout idle check reads accepted child work as busy, against the pinned gateway.

Round 09 proved the check blind to /bg and /btw children: Hermes counts no
agent for them, and their command entry was acked at launch. The entry now
stays leased until the work is done, and the real classifier reads that
lease. The classifier ships with the rollout tooling, so this check lives
apart from the child-work tests.
"""

import importlib.util
import json
import shutil
import tempfile
import time
from pathlib import Path
from typing import Any

from gateway.run import GatewayRunner

from tests.hermes.test_pinned_hermes_child_work import (
    COMMANDS,
    ChildHarness,
    GoalScenario,
)

IDLE_CHECK = Path(__file__).resolve().parents[3] / "scripts" / "finite_status_runtime_idle.py"


def idle_report(h: ChildHarness, home: str) -> dict[str, Any]:
    """What the rollout's idle check reads from this runtime right now.

    Hermes's own ``_persist_active_agents`` writes ``gateway_state.json``, and
    the inbox file mirrors the harness sidecar's leases.
    """
    from gateway.status import write_runtime_status

    spec = importlib.util.spec_from_file_location("finite_status_runtime_idle", IDLE_CHECK)
    assert spec is not None and spec.loader is not None, IDLE_CHECK
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


class PinnedHermesChildWorkIdleCheckTests(GoalScenario):
    def test_a_running_child_reads_busy(self):
        for command in COMMANDS:
            with self.subTest(command=command):

                async def scenario(home: str, command: str = command):
                    h = ChildHarness(home, timeline=[])
                    try:
                        await h.seed()
                        self.assertEqual(idle_report(h, home)["verdict"], "idle")
                        await self.launch(h, command)
                        # Hermes itself counts no agent for the child.
                        self.assertEqual(h.runner._active_work_count(), 0)
                        report = idle_report(h, home)
                        self.assertEqual(report["verdict"], "busy", report)
                        self.assertEqual(report["reasons"], ["inbox_leased"], report)
                        self.children.gate.set()
                        await h.wait_settled("msg-2")
                        await h.settle_loop()
                        self.assertEqual(idle_report(h, home)["verdict"], "idle")
                    finally:
                        await h.close()

                self.run_scenario(scenario)

    def test_a_pending_goal_kickoff_reads_busy(self):
        async def scenario(home: str):
            h = ChildHarness(home, timeline=[])
            try:
                await self.prepare(h, "set")
                await self.launch_goal(h, "set")
                self.assertEqual(h.state("msg-2"), "leased")
                self.assertEqual(idle_report(h, home)["verdict"], "busy")
                h.model_gate.set()
                await h.wait_settled("msg-2")
                await h.wait_turns_finished()
                self.assertEqual(idle_report(h, home)["verdict"], "idle")
            finally:
                h.model_gate.set()
                await h.close()

        self.run_scenario(scenario)

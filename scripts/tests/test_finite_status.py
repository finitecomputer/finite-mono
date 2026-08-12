from __future__ import annotations

import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from scripts import finite_status


ROOT = Path(__file__).resolve().parents[2]
COMMAND = ROOT / "scripts" / "finite-status"
FIXTURE = ROOT / "scripts" / "tests" / "fixtures" / "finite_status_aug1.json"


class FiniteStatusTests(unittest.TestCase):
    def fixture_report(self) -> dict[str, object]:
        raw = finite_status.load_fixture(FIXTURE)
        now = finite_status.parse_time(raw["now"])
        self.assertIsNotNone(now)
        return finite_status.build_report(raw, now)

    def test_aug1_convergence_math_and_inactive_exclusion(self) -> None:
        report = self.fixture_report()
        fleet = report["sections"]["fleet_convergence"]
        hosts = {host["source_host_id"]: host for host in fleet["hosts"]}

        lat1 = hosts["finite-lat-1"]
        self.assertEqual((lat1["on_target"], lat1["active_total"]), (21, 28))
        self.assertEqual(lat1["straggler_count"], 7)
        self.assertEqual(lat1["intentionally_inactive_count"], 6)
        self.assertEqual(lat1["unlinked_count"], 0)

        lat3 = hosts["finite-lat-3"]
        self.assertEqual((lat3["on_target"], lat3["active_total"]), (3, 24))
        self.assertEqual(lat3["straggler_count"], 21)
        self.assertEqual(lat3["intentionally_inactive_count"], 0)

        distribution = {
            (row["source_host_id"], row["version_label"]): row["count"]
            for row in fleet["recorded_distribution"]
        }
        self.assertEqual(distribution[("finite-lat-1", "2026-07-22.1")], 13)
        self.assertEqual(
            distribution[("finite-lat-1", "2026-07-22.1")]
            - lat1["straggler_count"],
            6,
        )
        self.assertTrue(fleet["distribution_consistent_with_detail_snapshot"])

    def test_core_queries_share_one_read_only_transaction(self) -> None:
        output = "\n".join(
            [
                "__FINITE_STATUS_ARTIFACTS__",
                "artifact-v2,ghcr.io/finite/runtime@sha256:2222,v2,git-v2,0.2.0,2026-08-01T00:00:00Z,",
                "__FINITE_STATUS_DISTRIBUTION__",
                "finite-lat-1,v2,1",
                "__FINITE_STATUS_RUNTIMES__",
                "finite-lat-1,artifact-v2,runtime-a,project-a,machine-a,Agent A,v2,active",
            ]
        )
        completed = subprocess.CompletedProcess(["psql"], 0, output, "")
        with mock.patch.object(finite_status, "run_read_only", return_value=completed) as run:
            result = finite_status.psql_query_sets({})
        self.assertEqual(len(result["runtimes"]), 1)
        self.assertEqual(result["runtimes"][0]["runtime_artifact_id"], "artifact-v2")
        call = run.call_args
        sql = call.kwargs["input_text"]
        self.assertTrue(sql.startswith("BEGIN TRANSACTION READ ONLY;"))
        self.assertIn(finite_status.ARTIFACTS_QUERY, sql)
        self.assertIn(finite_status.DISTRIBUTION_QUERY, sql)
        self.assertIn(finite_status.RUNTIME_DETAILS_QUERY, sql)
        self.assertEqual(call.args[0].count("psql"), 1)

    def test_human_output_names_every_straggler(self) -> None:
        output = finite_status.render_human(self.fixture_report())
        self.assertIn("21/28 active on target", output)
        self.assertIn("3/24 active on target", output)
        self.assertIn("6 intentionally inactive excluded", output)
        self.assertIn("NOT verified live", output)
        for index in range(1, 8):
            self.assertIn(f"Lat1 Straggler Agent {index:02d}", output)
        for index in range(1, 22):
            self.assertIn(f"Lat3 Straggler Agent {index:02d}", output)

    def test_json_has_four_sections_and_red_exit(self) -> None:
        result = subprocess.run(
            [str(COMMAND), "--json", "--fixture", str(FIXTURE)],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 1, result.stderr)
        payload = json.loads(result.stdout)
        self.assertEqual(
            set(payload["sections"]),
            {
                "fleet_convergence",
                "host_health",
                "recovery_boundary",
                "rollout_state",
            },
        )
        self.assertEqual(payload["overall_status"], "red")
        self.assertEqual(payload["exit_code"], 1)

    def test_exit_precedence_is_red_then_unknown_then_green(self) -> None:
        sections = {
            name: {"status": "green"}
            for name in (
                "fleet_convergence",
                "host_health",
                "recovery_boundary",
                "rollout_state",
            )
        }
        report = {"sections": sections}
        self.assertEqual(finite_status.report_exit_code(report), 0)
        sections["host_health"]["status"] = "unknown"
        self.assertEqual(finite_status.report_exit_code(report), 2)
        sections["fleet_convergence"]["status"] = "red"
        self.assertEqual(finite_status.report_exit_code(report), 1)

    def test_unlinked_runtime_is_unknown_not_intentionally_inactive(self) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        raw["core"]["runtimes"][0]["link_state"] = "unlinked"
        now = finite_status.parse_time(raw["now"])
        fleet = finite_status.build_fleet(raw["core"], now)
        lat1 = next(host for host in fleet["hosts"] if host["source_host_id"] == "finite-lat-1")
        self.assertEqual(lat1["intentionally_inactive_count"], 6)
        self.assertEqual(lat1["unlinked_count"], 1)

    def test_snapshot_manifest_is_checksum_only_and_accepts_sqlite_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            snapshot = root / "20260801T120000Z"
            snapshot.mkdir()
            database = snapshot / "server.sqlite3"
            database.write_bytes(b"not opened as a SQLite database")
            digest = hashlib.sha256(database.read_bytes()).hexdigest()
            (snapshot / "manifest.sha256").write_text(
                f"{digest}  server.sqlite3\n", encoding="utf-8"
            )
            (root / "latest").symlink_to(snapshot.name)

            self.assertEqual(finite_status.safe_snapshot_directory(root), snapshot.resolve())
            checked, failures = finite_status.verify_manifest(snapshot)
            self.assertEqual(checked, 1)
            self.assertEqual(failures, [])

    def test_litestream_recovery_evidence_is_scored_in_the_recovery_boundary(self) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        now = finite_status.parse_time(raw["now"])

        fresh = finite_status.build_recovery(raw["recovery"], now)
        self.assertEqual(fresh["litestream"]["stamp_status"], "green")
        self.assertEqual(fresh["litestream"]["service_status"], "green")
        self.assertEqual(
            fresh["litestream"]["service_unit"], "finite-litestream.service"
        )

        stale = dict(raw["recovery"])
        stale["litestream_last_success_epoch"] = int(now.timestamp()) - 7200
        report = finite_status.build_recovery(stale, now)
        self.assertEqual(report["litestream"]["stamp_status"], "red")
        self.assertEqual(report["status"], "red")

        missing = dict(raw["recovery"])
        del missing["litestream_last_success_epoch"]
        missing["litestream_last_success_error"] = "cannot read stamp"
        missing["litestream_service_unit"] = {"error": "unit not found"}
        report = finite_status.build_recovery(missing, now)
        self.assertEqual(report["litestream"]["stamp_status"], "unknown")
        self.assertNotEqual(report["status"], "green")

    def test_interrupted_rollout_is_reported_without_repair(self) -> None:
        raw = {
            "exists": True,
            "plan_hash": "b" * 64,
            "plan": {"planned": [{}, {}]},
            "events": [
                {"event": "start", "phase": "execute", "timestamp": "2026-08-01T00:00:00Z"},
                {
                    "event": "entry_postflight",
                    "phase": "execute",
                    "status": "succeeded",
                    "agent_runtime_id": "runtime-a",
                    "timestamp": "2026-08-01T00:01:00Z",
                },
            ],
        }
        rollout = finite_status.build_rollout(raw)
        self.assertEqual(rollout["status"], "red")
        self.assertEqual(rollout["planned_entries"], 2)
        self.assertEqual(rollout["completed_entries"], 1)
        self.assertEqual(rollout["terminal_state"], "interrupted-or-incomplete")

    def test_recorded_interrupted_final_is_red_and_named(self) -> None:
        raw = {
            "exists": True,
            "plan_hash": "b" * 64,
            "plan": {"planned": [{}, {}]},
            "events": [
                {"event": "start", "phase": "execute", "run_id": "run-1"},
                {
                    "event": "entry_postflight",
                    "phase": "execute",
                    "status": "succeeded",
                    "agent_runtime_id": "runtime-a",
                    "run_id": "run-1",
                },
                {
                    "event": "final",
                    "phase": "execute",
                    "status": "interrupted",
                    "run_id": "run-1",
                    "resume_point": "project-b/runtime-b",
                },
            ],
        }
        rollout = finite_status.build_rollout(raw)
        self.assertEqual(rollout["status"], "red")
        self.assertEqual(rollout["terminal_state"], "interrupted")

    def test_noop_final_is_never_reported_as_success(self) -> None:
        raw = {
            "exists": True,
            "plan_hash": "c" * 64,
            "plan": {"planned": []},
            "events": [
                {"event": "start", "phase": "execute", "run_id": "run-1"},
                {"event": "final", "phase": "execute", "status": "noop", "run_id": "run-1"},
            ],
        }
        rollout = finite_status.build_rollout(raw)
        self.assertEqual(rollout["status"], "green")
        self.assertEqual(rollout["terminal_state"], "noop")

    def runtime_row(
        self,
        runtime: str,
        *,
        host: str = "finite-lat-1",
        link_state: str = "active",
    ) -> dict[str, str]:
        return {
            "source_host_id": host,
            "agent_runtime_id": runtime,
            "project_id": runtime.replace("runtime", "project"),
            "source_machine_id": runtime.replace("runtime", "machine"),
            "agent_name": runtime,
            "version_label": "v2",
            "link_state": link_state,
        }

    def probe_report(self, verdict: str, reason: str | None = None) -> str:
        return json.dumps(
            {
                "schema": "finite.lifecycle-probe.v1",
                "runtime": {
                    "project_id": "project-a",
                    "agent_runtime_id": "runtime-a",
                    "source_machine_id": "machine-a",
                    "container_name": "machine-a",
                },
                "verdict": verdict,
                "reason": reason,
                "checks": [],
            }
        )

    def test_collect_lifecycle_probe_reports_per_agent_verdicts(self) -> None:
        runtimes = [
            self.runtime_row("runtime-a"),
            self.runtime_row("runtime-b"),
            self.runtime_row("runtime-remote", host="finite-lat-3"),
            self.runtime_row("runtime-inactive", link_state="inactive"),
        ]
        reports = {
            "runtime-a": self.probe_report("operable"),
            "runtime-b": self.probe_report("inoperable", "orphaned_task"),
        }

        def fake_run(command, **kwargs):
            runtime = command[command.index("--agent-runtime-id") + 1]
            return subprocess.CompletedProcess(command, 0, reports[runtime], "")

        with (
            mock.patch.dict(
                finite_status.os.environ,
                {"FINITE_STATUS_LIFECYCLE_PROBE_BIN": "/bin/sh"},
            ),
            mock.patch.object(finite_status, "run_read_only", side_effect=fake_run) as run,
            mock.patch.object(
                finite_status, "read_environment_values", return_value={}
            ),
        ):
            raw = finite_status.collect_lifecycle_probe(runtimes, "finite-lat-1")

        self.assertTrue(raw["available"])
        self.assertEqual(
            raw["agents"]["runtime-a"], {"verdict": "operable", "reason": None}
        )
        self.assertEqual(
            raw["agents"]["runtime-b"],
            {"verdict": "inoperable", "reason": "orphaned_task"},
        )
        # Only this host's active Agents are probed.
        self.assertEqual(set(raw["agents"]), {"runtime-a", "runtime-b"})
        command = run.call_args_list[0].args[0]
        self.assertEqual(command[:2], ["/bin/sh", "lifecycle-probe"])
        self.assertIn("machine-a", command)

    def test_collect_lifecycle_probe_marks_failures_unknown(self) -> None:
        runtimes = [
            self.runtime_row("runtime-a"),
            self.runtime_row("runtime-b"),
            self.runtime_row("runtime-c"),
            self.runtime_row("runtime-d"),
        ]
        outcomes = {
            "runtime-a": subprocess.CompletedProcess([], 1, "", "boom"),
            "runtime-b": subprocess.CompletedProcess([], 0, "not json", ""),
            "runtime-c": subprocess.CompletedProcess(
                [], 0, '{"schema":"other","verdict":"operable"}', ""
            ),
            "runtime-d": finite_status.CollectionError("probe missing"),
        }

        def fake_run(command, **kwargs):
            outcome = outcomes[command[command.index("--agent-runtime-id") + 1]]
            if isinstance(outcome, Exception):
                raise outcome
            return outcome

        with (
            mock.patch.dict(
                finite_status.os.environ,
                {"FINITE_STATUS_LIFECYCLE_PROBE_BIN": "/bin/sh"},
            ),
            mock.patch.object(finite_status, "run_read_only", side_effect=fake_run),
            mock.patch.object(
                finite_status, "read_environment_values", return_value={}
            ),
        ):
            raw = finite_status.collect_lifecycle_probe(runtimes, "finite-lat-1")

        self.assertEqual(raw["agents"]["runtime-a"]["verdict"], "unknown")
        self.assertEqual(raw["agents"]["runtime-a"]["reason"], "probe_unavailable")
        self.assertEqual(raw["agents"]["runtime-b"]["reason"], "probe_invalid")
        self.assertEqual(raw["agents"]["runtime-c"]["reason"], "probe_invalid")
        self.assertEqual(raw["agents"]["runtime-d"]["verdict"], "unknown")
        self.assertEqual(raw["agents"]["runtime-d"]["reason"], "probe_unavailable")

    def test_collect_lifecycle_probe_without_binary_is_unavailable(self) -> None:
        with mock.patch.dict(
            finite_status.os.environ,
            {"FINITE_STATUS_LIFECYCLE_PROBE_BIN": "/nonexistent/lifecycle-probe"},
        ):
            raw = finite_status.collect_lifecycle_probe(
                [self.runtime_row("runtime-a")], "finite-lat-1"
            )
        self.assertFalse(raw["available"])
        self.assertEqual(raw["agents"], {})
        self.assertTrue(raw["errors"])

    def test_lifecycle_health_is_a_separate_displayed_per_agent_field(self) -> None:
        report = self.fixture_report()
        fleet = report["sections"]["fleet_convergence"]
        lat1 = next(
            host for host in fleet["hosts"] if host["source_host_id"] == "finite-lat-1"
        )
        self.assertEqual(lat1["lifecycle_probed_count"], 3)
        attention = {
            row["agent_runtime_id"]: row["lifecycle"]
            for row in lat1["lifecycle_attention"]
        }
        self.assertEqual(
            attention["runtime-lat1-straggler-01"],
            {"verdict": "inoperable", "reason": "orphaned_task"},
        )
        # unknown is a displayed state, not hidden
        self.assertEqual(
            attention["runtime-lat1-target-02"],
            {"verdict": "unknown", "reason": "task_list_error"},
        )
        output = finite_status.render_human(report)
        self.assertIn(
            "LIFECYCLE Lat1 Straggler Agent 01 [runtime-lat1-straggler-01]: inoperable (orphaned_task)",
            output,
        )
        self.assertIn(
            "LIFECYCLE Lat1 Target Agent 02 [runtime-lat1-target-02]: unknown (task_list_error)",
            output,
        )
        self.assertIn("lifecycle 1/3 operable", output)


if __name__ == "__main__":
    unittest.main()

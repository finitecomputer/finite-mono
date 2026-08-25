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
                "finite-lat-1,artifact-v2,runtime-a,project-a,machine-a,Agent A,v2,active,restart,launching,online,2026-08-01T13:59:00Z,t,,60",
            ]
        )
        completed = subprocess.CompletedProcess(["psql"], 0, output, "")
        with mock.patch.object(finite_status, "run_read_only", return_value=completed) as run:
            result = finite_status.psql_query_sets({})
        self.assertEqual(len(result["runtimes"]), 1)
        self.assertEqual(result["runtimes"][0]["runtime_artifact_id"], "artifact-v2")
        # The canonical lifecycle state arrives with the row, unmodified.
        self.assertEqual(result["runtimes"][0]["control_status"], "launching")
        # So do the standing-health columns.
        self.assertEqual(result["runtimes"][0]["health_ready"], "t")
        self.assertEqual(result["runtimes"][0]["runtime_status"], "online")
        call = run.call_args
        sql = call.kwargs["input_text"]
        self.assertTrue(sql.startswith("BEGIN TRANSACTION READ ONLY;"))
        self.assertIn(finite_status.ARTIFACTS_QUERY, sql)
        self.assertIn(finite_status.DISTRIBUTION_QUERY, sql)
        self.assertIn(finite_status.RUNTIME_DETAILS_QUERY, sql)
        self.assertEqual(call.args[0].count("psql"), 1)

    def test_human_output_projects_active_control_state(self) -> None:
        raw = json.loads(FIXTURE.read_text(encoding="utf-8"))
        for group in raw["core"]["runtime_groups"]:
            group["count"] = 0
        raw["core"]["runtime_groups"].append(
            {
                "source_host_id": "finite-lat-1",
                "id_prefix": "ctl-agent",
                "project_prefix": "ctl-project",
                "name_prefix": "Control Agent",
                "version_label": "2026-07-22.1",
                "link_state": "active",
                "count": 1,
                "control_kind": "restart",
                "control_status": "compute_up",
            }
        )
        expanded = finite_status.expand_fixture(raw)
        now = finite_status.parse_time(expanded["now"])
        self.assertIsNotNone(now)
        report = finite_status.build_report(expanded, now)
        output = finite_status.render_human(report)
        self.assertIn("CONTROL Control Agent 01 [ctl-agent-01]: restart compute_up", output)

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
        self.assertIn("model=deepseek-v4-flash-0731 [GREEN]", output)

    def test_runner_glm_override_is_red(self) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        raw["host_health"]["runner_shared_environment"] = {
            "FC_RUNNER_FINITE_PRIVATE_MODEL": "deepseek-v4-flash-0731"
        }
        raw["host_health"]["runner_operator_environment"] = {
            "FC_RUNNER_FINITE_PRIVATE_MODEL": "glm-5-2"
        }
        raw["host_health"]["runner_environment"][
            "FC_RUNNER_FINITE_PRIVATE_MODEL"
        ] = "glm-5-2"
        now = finite_status.parse_time(raw["now"])
        self.assertIsNotNone(now)
        report = finite_status.build_report(raw, now)
        runner = report["sections"]["host_health"]["runner"]
        self.assertEqual(runner["finite_private_model"], "glm-5-2")
        self.assertEqual(runner["finite_private_model_status"], "red")
        self.assertEqual(
            runner["finite_private_model_state"], "stale-operator-override"
        )
        self.assertEqual(report["sections"]["host_health"]["status"], "red")

    def test_runner_mixed_version_alias_is_green_before_canonical_role_deploy(self) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        raw["host_health"]["runner_shared_environment"] = {
            "FC_RUNNER_FINITE_PRIVATE_MODEL": "glm-5-2"
        }
        raw["host_health"]["runner_operator_environment"] = {
            "FC_RUNNER_FINITE_PRIVATE_MODEL": "glm-5-2"
        }
        raw["host_health"]["runner_environment"][
            "FC_RUNNER_FINITE_PRIVATE_MODEL"
        ] = "glm-5-2"
        now = finite_status.parse_time(raw["now"])
        self.assertIsNotNone(now)
        report = finite_status.build_report(raw, now)
        runner = report["sections"]["host_health"]["runner"]
        self.assertEqual(runner["finite_private_model_status"], "green")
        self.assertEqual(
            runner["finite_private_model_state"], "mixed-version-compatibility"
        )

    def test_runner_mixed_version_alias_is_unknown_without_shared_role(self) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        raw["host_health"]["runner_shared_environment"] = {}
        raw["host_health"]["runner_operator_environment"] = {
            "FC_RUNNER_FINITE_PRIVATE_MODEL": "glm-5-2"
        }
        raw["host_health"]["runner_environment"][
            "FC_RUNNER_FINITE_PRIVATE_MODEL"
        ] = "glm-5-2"
        now = finite_status.parse_time(raw["now"])
        self.assertIsNotNone(now)
        report = finite_status.build_report(raw, now)
        runner = report["sections"]["host_health"]["runner"]
        self.assertEqual(runner["finite_private_model_status"], "unknown")
        self.assertEqual(
            runner["finite_private_model_state"], "unresolved-shared-role"
        )

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
            sorted(fresh["litestream"]["service_units"]),
            [
                "finite-litestream-finite-brain.service",
                "finite-litestream-finite-chat-server.service",
            ],
        )
        self.assertEqual(
            set(fresh["litestream"]["service_units"].values()), {"green"}
        )

        one_down = dict(raw["recovery"])
        one_down["litestream_service_units"] = dict(
            raw["recovery"]["litestream_service_units"],
            **{
                "finite-litestream-finite-brain.service": {
                    "LoadState": "loaded",
                    "ActiveState": "inactive",
                    "SubState": "dead",
                    "Result": "success",
                    "ExecMainStatus": "0",
                },
            },
        )
        report = finite_status.build_recovery(one_down, now)
        self.assertNotEqual(report["litestream"]["service_status"], "green")
        self.assertNotEqual(report["status"], "green")

        stale = dict(raw["recovery"])
        stale["litestream_last_success_epoch"] = int(now.timestamp()) - 7200
        report = finite_status.build_recovery(stale, now)
        self.assertEqual(report["litestream"]["stamp_status"], "red")
        self.assertEqual(report["status"], "red")

        missing = dict(raw["recovery"])
        del missing["litestream_last_success_epoch"]
        missing["litestream_last_success_error"] = "cannot read stamp"
        missing["litestream_service_units"] = {
            "finite-litestream-finite-chat-server.service": {"error": "unit not found"}
        }
        report = finite_status.build_recovery(missing, now)
        self.assertEqual(report["litestream"]["stamp_status"], "unknown")
        self.assertNotEqual(report["status"], "green")

    def test_snapshot_and_borg_use_their_deployed_cadences(self) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        now = finite_status.parse_time(raw["now"])
        self.assertIsNotNone(now)
        raw["recovery"]["snapshot"]["created_at"] = "2026-07-30T07:00:00Z"
        raw["recovery"]["borg_last_success_epoch"] = int(now.timestamp()) - 55 * 3600

        recovery = finite_status.build_recovery(raw["recovery"], now)

        self.assertEqual(recovery["snapshot"]["age_seconds"], 55 * 3600)
        self.assertEqual(recovery["snapshot"]["status"], "green")
        self.assertEqual(recovery["borg"]["stamp_status"], "red")

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

    def report_with_health_group(self, group: dict[str, object]) -> dict[str, object]:
        raw = json.loads(FIXTURE.read_text(encoding="utf-8"))
        for existing in raw["core"]["runtime_groups"]:
            existing["count"] = 0
        raw["core"]["runtime_groups"].append(
            {
                "source_host_id": "finite-lat-9",
                "id_prefix": "health-agent",
                "project_prefix": "health-project",
                "name_prefix": "Health Agent",
                "version_label": "2026-08-01.1",
                "link_state": "active",
                "count": 1,
                **group,
            }
        )
        expanded = finite_status.expand_fixture(raw)
        now = finite_status.parse_time(expanded["now"])
        self.assertIsNotNone(now)
        return finite_status.build_report(expanded, now)

    def lat9(self, report: dict[str, object]) -> dict[str, object]:
        fleet = report["sections"]["fleet_convergence"]
        return next(
            host for host in fleet["hosts"] if host["source_host_id"] == "finite-lat-9"
        )

    def test_fresh_ready_health_report_keeps_the_host_green(self) -> None:
        # 30s old at the default 60s cadence is fresh.
        report = self.report_with_health_group(
            {"health_reported_at": "2026-08-01T13:59:30Z", "health_ready": True}
        )
        host = self.lat9(report)
        self.assertEqual(host["status"], "green")
        self.assertEqual((host["health_ready_count"], host["health_tracked_count"]), (1, 1))
        output = finite_status.render_human(report)
        self.assertIn("health 1/1 ready (0 unknown)", output)

    def test_fresh_not_ready_health_report_is_red_and_names_the_reason(self) -> None:
        report = self.report_with_health_group(
            {
                "health_reported_at": "2026-08-01T13:59:30Z",
                "health_ready": False,
                "health_reason": "unreachable",
            }
        )
        host = self.lat9(report)
        self.assertEqual(host["status"], "red")
        self.assertEqual(report["sections"]["fleet_convergence"]["status"], "red")
        entry = host["health_not_ready"][0]
        self.assertEqual(entry["health"]["reason"], "unreachable")
        output = finite_status.render_human(report)
        self.assertIn(
            "HEALTH Health Agent 01 [health-agent-01]: not_ready (unreachable)", output
        )

    def test_stale_health_report_reads_unknown_past_three_cadences(self) -> None:
        # 600s old at a 60s cadence is past the 3x deadline: the "died at 3am"
        # runtime stops displaying its frozen last-known ready.
        report = self.report_with_health_group(
            {"health_reported_at": "2026-08-01T13:50:00Z", "health_ready": True}
        )
        host = self.lat9(report)
        self.assertEqual(host["status"], "unknown")
        entry = host["health_unknown"][0]
        self.assertEqual(entry["health"]["status"], "unknown")
        self.assertEqual(entry["health"]["age_seconds"], 600)
        output = finite_status.render_human(report)
        self.assertIn(
            "HEALTH-UNKNOWN Health Agent 01 [health-agent-01]: no fresh report (last report 10m ago)",
            output,
        )
        # At exactly 3x cadence the report is still fresh.
        fresh_edge = self.report_with_health_group(
            {"health_reported_at": "2026-08-01T13:57:00Z", "health_ready": True}
        )
        self.assertEqual(self.lat9(fresh_edge)["health_ready_count"], 1)
        self.assertEqual(self.lat9(fresh_edge)["status"], "green")
        # A slower reporter gets its own deadline (10m cadence, 600s old).
        slow = self.report_with_health_group(
            {
                "health_reported_at": "2026-08-01T13:50:00Z",
                "health_ready": True,
                "health_report_interval_seconds": 600,
            }
        )
        self.assertEqual(self.lat9(slow)["status"], "green")

    def test_online_runtime_that_never_reported_is_unknown(self) -> None:
        report = self.report_with_health_group({"runtime_status": "online"})
        host = self.lat9(report)
        self.assertEqual(host["status"], "unknown")
        entry = host["health_unknown"][0]
        self.assertEqual(entry["health"]["status"], "unknown")
        self.assertIsNone(entry["health"]["age_seconds"])
        output = finite_status.render_human(report)
        self.assertIn("no fresh report (never reported)", output)

    def test_offline_runtime_health_is_displayed_but_not_tracked(self) -> None:
        # An intentionally stopped runtime carries no standing readiness claim:
        # even a fresh-looking last report projects unknown and is not counted
        # against the host.
        report = self.report_with_health_group(
            {
                "runtime_status": "offline",
                "health_reported_at": "2026-08-01T13:59:30Z",
                "health_ready": True,
            }
        )
        host = self.lat9(report)
        self.assertEqual(host["status"], "green")
        self.assertEqual(host["health_tracked_count"], 0)
        projected = finite_status.project_runtime_health(
            {
                "runtime_status": "offline",
                "health_reported_at": "2026-08-01T13:59:30Z",
                "health_ready": True,
            },
            finite_status.parse_time("2026-08-01T14:00:00Z"),
        )
        self.assertEqual(projected["status"], "unknown")

    def test_single_disk_profile_does_not_imply_raid(self) -> None:
        profile = finite_status.CONTRACT["hosts"]["finite-lat-1"]
        self.assertEqual(profile["storage"], "single-disk")
        self.assertNotIn("mdstat_path", profile)
        self.assertEqual(len(profile["disks"]), 2)

    def test_single_disk_storage_greens_despite_leftover_md_arrays(self) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        raw["host_health"]["storage"] = {
            "mode": "single-disk",
            "md_arrays": ["md127", "md126"],
            "disks": [
                {
                    "path": finite_status.CONTRACT["hosts"]["finite-lat-1"]["disks"][0],
                    "present": True,
                },
                {
                    "path": finite_status.CONTRACT["hosts"]["finite-lat-1"]["disks"][1],
                    "present": True,
                },
            ],
        }
        now = finite_status.parse_time(raw["now"])
        self.assertIsNotNone(now)
        report = finite_status.build_report(raw, now)
        storage = report["sections"]["host_health"]["storage"]
        self.assertEqual(storage["status"], "green")
        output = finite_status.render_human(report)
        self.assertIn("storage: single-disk; expected devices 2/2 present [GREEN]", output)
        self.assertNotIn("MD arrays=", output)

    def test_single_disk_storage_is_red_when_listed_disk_missing(self) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        raw["host_health"]["storage"]["disks"][1]["present"] = False
        now = finite_status.parse_time(raw["now"])
        self.assertIsNotNone(now)
        report = finite_status.build_report(raw, now)
        storage = report["sections"]["host_health"]["storage"]
        self.assertEqual(storage["status"], "red")
        self.assertEqual(report["sections"]["host_health"]["status"], "red")

    def test_raid_storage_uses_storage_health_unit(self) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        unit = finite_status.CONTRACT["hosts"]["finite-lat-3"]["storage_health_unit"]
        self.assertEqual(unit, "finite-storage-health.service")
        raw["host_health"]["storage"] = {"mode": "raid"}
        raw["host_health"]["units"][unit] = {
            "LoadState": "loaded",
            "ActiveState": "inactive",
            "SubState": "dead",
            "Result": "success",
            "ExecMainStatus": "0",
        }
        now = finite_status.parse_time(raw["now"])
        self.assertIsNotNone(now)
        report = finite_status.build_report(raw, now)
        self.assertEqual(report["sections"]["host_health"]["storage"]["status"], "green")

        raw["host_health"]["units"][unit] = {
            "LoadState": "loaded",
            "ActiveState": "inactive",
            "SubState": "dead",
            "Result": "failed",
            "ExecMainStatus": "1",
        }
        report = finite_status.build_report(raw, now)
        self.assertEqual(report["sections"]["host_health"]["storage"]["status"], "red")
        self.assertEqual(report["sections"]["host_health"]["status"], "red")

    def test_collect_single_disk_does_not_read_mdstat(self) -> None:
        stats = mock.Mock(f_blocks=100, f_frsize=1024, f_bavail=50)
        with (
            mock.patch.object(
                finite_status, "systemd_properties", return_value={"LoadState": "loaded"}
            ),
            mock.patch.object(finite_status, "collect_healthcheck_journal", return_value={}),
            mock.patch.object(finite_status.os, "statvfs", return_value=stats),
            mock.patch.object(finite_status.Path, "exists", return_value=True),
            mock.patch.object(
                finite_status.Path,
                "read_text",
                side_effect=AssertionError("single-disk collect must not read mdstat"),
            ),
            mock.patch.object(finite_status, "line_count", return_value=0),
            mock.patch.object(finite_status, "read_environment_values", return_value={}),
        ):
            collected = finite_status.collect_host_health("finite-lat-1")
        self.assertEqual(collected["storage"]["mode"], "single-disk")
        self.assertNotIn("md_arrays", collected["storage"])
        self.assertNotIn("error", collected["storage"])
        self.assertEqual(len(collected["storage"]["disks"]), 2)
        self.assertTrue(all(disk["present"] for disk in collected["storage"]["disks"]))


if __name__ == "__main__":
    unittest.main()

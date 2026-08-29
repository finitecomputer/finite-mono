from __future__ import annotations

import hashlib
import json
import os
from datetime import timedelta
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
            distribution[("finite-lat-1", "2026-07-22.1")] - lat1["straggler_count"],
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
        with mock.patch.object(
            finite_status, "run_read_only", return_value=completed
        ) as run:
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
        self.assertIn(
            "CONTROL Control Agent 01 [ctl-agent-01]: restart compute_up", output
        )

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
        self.assertIn("model=glm-5-3-flash [GREEN]", output)

    def test_runner_glm_override_is_red(self) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        raw["host_health"]["runner_shared_environment"] = {
            "FC_RUNNER_FINITE_PRIVATE_MODEL": "glm-5-3-flash"
        }
        raw["host_health"]["runner_operator_environment"] = {
            "FC_RUNNER_FINITE_PRIVATE_MODEL": "deepseek-v4-flash-0731"
        }
        raw["host_health"]["runner_environment"]["FC_RUNNER_FINITE_PRIVATE_MODEL"] = (
            "deepseek-v4-flash-0731"
        )
        now = finite_status.parse_time(raw["now"])
        self.assertIsNotNone(now)
        report = finite_status.build_report(raw, now)
        runner = report["sections"]["host_health"]["runner"]
        self.assertEqual(runner["finite_private_model"], "deepseek-v4-flash-0731")
        self.assertEqual(runner["finite_private_model_status"], "red")
        self.assertEqual(
            runner["finite_private_model_state"], "stale-operator-override"
        )
        self.assertEqual(report["sections"]["host_health"]["status"], "red")

    def test_runner_mixed_version_alias_is_green_before_canonical_role_deploy(
        self,
    ) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        raw["host_health"]["runner_shared_environment"] = {
            "FC_RUNNER_FINITE_PRIVATE_MODEL": "glm-5-2"
        }
        raw["host_health"]["runner_operator_environment"] = {
            "FC_RUNNER_FINITE_PRIVATE_MODEL": "glm-5-2"
        }
        raw["host_health"]["runner_environment"]["FC_RUNNER_FINITE_PRIVATE_MODEL"] = (
            "glm-5-2"
        )
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
        raw["host_health"]["runner_environment"]["FC_RUNNER_FINITE_PRIVATE_MODEL"] = (
            "glm-5-2"
        )
        now = finite_status.parse_time(raw["now"])
        self.assertIsNotNone(now)
        report = finite_status.build_report(raw, now)
        runner = report["sections"]["host_health"]["runner"]
        self.assertEqual(runner["finite_private_model_status"], "unknown")
        self.assertEqual(runner["finite_private_model_state"], "unresolved-shared-role")

    def write_fixture_variant(self, mutate) -> Path:
        raw = json.loads(FIXTURE.read_text(encoding="utf-8"))
        mutate(raw)
        descriptor, name = tempfile.mkstemp(suffix=".json")
        os.close(descriptor)
        path = Path(name)
        path.write_text(json.dumps(raw), encoding="utf-8")
        self.addCleanup(path.unlink)
        return path

    def fixture_runner(self) -> dict[str, object]:
        raw = finite_status.load_fixture(FIXTURE)
        now = finite_status.parse_time(raw["now"])
        self.assertIsNotNone(now)
        report = finite_status.build_report(raw, now)
        return report["sections"]["host_health"]["runner"]

    def test_runner_pin_on_target_is_green_matched(self) -> None:
        runner = self.fixture_runner()
        self.assertEqual(runner["artifact_pin"], "finite-agent-runtime-2026-08-01.1")
        self.assertEqual(runner["target_artifact_id"], runner["artifact_pin"])
        self.assertEqual(runner["pin_status"], "green")
        self.assertEqual(runner["pin_state"], "matched")
        output = finite_status.render_human(self.fixture_report())
        self.assertIn("pin=finite-agent-runtime-2026-08-01.1 [GREEN] (matched)", output)

    def test_runner_pin_mismatch_stays_red_and_names_mismatched(self) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        raw["host_health"]["runner_environment"]["FC_RUNNER_RUNTIME_ARTIFACT_ID"] = (
            "finite-agent-runtime-2026-07-22.1"
        )
        now = finite_status.parse_time(raw["now"])
        self.assertIsNotNone(now)
        report = finite_status.build_report(raw, now)
        runner = report["sections"]["host_health"]["runner"]
        self.assertEqual(runner["pin_status"], "red")
        self.assertEqual(runner["pin_state"], "mismatched")
        self.assertEqual(report["sections"]["host_health"]["status"], "red")

    def test_absent_pin_with_readable_environment_is_red_not_unknown(self) -> None:
        # kata-runner-host.nix dropped its implicit pin default: an operator
        # runner.env without FC_RUNNER_RUNTIME_ARTIFACT_ID halts new agent
        # creation, so it must not read as probe noise.
        fixture = self.write_fixture_variant(
            lambda raw: raw["host_health"]["runner_environment"].pop(
                "FC_RUNNER_RUNTIME_ARTIFACT_ID"
            )
        )
        result = subprocess.run(
            [str(COMMAND), "--json", "--fixture", str(fixture)],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 1, result.stderr)
        payload = json.loads(result.stdout)
        runner = payload["sections"]["host_health"]["runner"]
        self.assertIsNone(runner["artifact_pin"])
        self.assertEqual(runner["pin_status"], "red")
        self.assertEqual(runner["pin_state"], "absent")
        self.assertEqual(payload["sections"]["host_health"]["status"], "red")
        self.assertEqual(payload["overall_status"], "red")
        # Same entry point as the contract job renders it loudly, too.
        human = subprocess.run(
            [str(COMMAND), "--fixture", str(fixture)],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertIn("pin=unset [RED] (absent)", human.stdout)

    def test_unprobeable_runner_environment_keeps_plain_unknown(self) -> None:
        # No environment evidence at all (both files unreadable): pin absence
        # cannot be distinguished from an unprobeable host, so stay unknown.
        def no_environment_evidence(raw: dict[str, object]) -> None:
            raw["host_health"]["runner_environment"].pop(
                "FC_RUNNER_RUNTIME_ARTIFACT_ID"
            )
            raw["host_health"]["runner_environment_files_read"] = []

        fixture = self.write_fixture_variant(no_environment_evidence)
        result = subprocess.run(
            [str(COMMAND), "--json", "--fixture", str(fixture)],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 1, result.stderr)
        payload = json.loads(result.stdout)
        runner = payload["sections"]["host_health"]["runner"]
        self.assertIsNone(runner["artifact_pin"])
        self.assertEqual(runner["pin_status"], "unknown")
        self.assertEqual(runner["pin_state"], "unresolved")
        # Contrast with the absent case: without environment evidence an
        # otherwise-green host is plain unknown, never red by pin.
        self.assertEqual(payload["sections"]["host_health"]["status"], "unknown")
        human = subprocess.run(
            [str(COMMAND), "--fixture", str(fixture)],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertIn("pin=unknown [UNKNOWN] (unresolved)", human.stdout)
        self.assertNotIn("(absent)", human.stdout)

    def test_legacy_reports_without_collection_marker_stay_conservative(self) -> None:
        # Inputs predating runner_environment_files_read (persisted snapshots,
        # external harnesses) keep the old conservative unknown semantics.
        raw = finite_status.load_fixture(FIXTURE)
        raw["host_health"]["runner_environment"]["FC_RUNNER_RUNTIME_ARTIFACT_ID"] = ""
        del raw["host_health"]["runner_environment_files_read"]
        now = finite_status.parse_time(raw["now"])
        self.assertIsNotNone(now)
        report = finite_status.build_report(raw, now)
        runner = report["sections"]["host_health"]["runner"]
        self.assertEqual(runner["pin_status"], "unknown")
        self.assertEqual(runner["pin_state"], "unresolved")

    def test_json_has_five_sections_and_red_exit(self) -> None:
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
                "chat_plane",
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
                "chat_plane",
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
        lat1 = next(
            host for host in fleet["hosts"] if host["source_host_id"] == "finite-lat-1"
        )
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

            self.assertEqual(
                finite_status.safe_snapshot_directory(root), snapshot.resolve()
            )
            checked, failures = finite_status.verify_manifest(snapshot)
            self.assertEqual(checked, 1)
            self.assertEqual(failures, [])

    def test_litestream_recovery_evidence_is_scored_in_the_recovery_boundary(
        self,
    ) -> None:
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
        self.assertEqual(set(fresh["litestream"]["service_units"].values()), {"green"})

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
                {
                    "event": "start",
                    "phase": "execute",
                    "timestamp": "2026-08-01T00:00:00Z",
                },
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
                {
                    "event": "final",
                    "phase": "execute",
                    "status": "noop",
                    "run_id": "run-1",
                },
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
            mock.patch.object(
                finite_status, "run_read_only", side_effect=fake_run
            ) as run,
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
        self.assertEqual(
            (host["health_ready_count"], host["health_tracked_count"]), (1, 1)
        )
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
        self.assertIn(
            "storage: single-disk; expected devices 2/2 present [GREEN]", output
        )
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
        self.assertEqual(
            report["sections"]["host_health"]["storage"]["status"], "green"
        )

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
                finite_status,
                "systemd_properties",
                return_value={"LoadState": "loaded"},
            ),
            mock.patch.object(
                finite_status, "collect_healthcheck_journal", return_value={}
            ),
            mock.patch.object(finite_status.os, "statvfs", return_value=stats),
            mock.patch.object(finite_status.Path, "exists", return_value=True),
            mock.patch.object(
                finite_status.Path,
                "read_text",
                side_effect=AssertionError("single-disk collect must not read mdstat"),
            ),
            mock.patch.object(finite_status, "line_count", return_value=0),
            mock.patch.object(
                finite_status, "read_environment_values", return_value={}
            ),
        ):
            collected = finite_status.collect_host_health("finite-lat-1")
        self.assertEqual(collected["storage"]["mode"], "single-disk")
        self.assertNotIn("md_arrays", collected["storage"])
        self.assertNotIn("error", collected["storage"])
        self.assertEqual(len(collected["storage"]["disks"]), 2)
        self.assertTrue(all(disk["present"] for disk in collected["storage"]["disks"]))

    # ------------------------------------------------------------------
    # Chat-plane incident probes (2026-08-27..29 outage).

    def chat_raw(
        self,
        *,
        server: dict[str, object] | None = None,
        sync_rate: dict[str, object] | None = None,
        client_stores: dict[str, object] | None = None,
    ) -> dict[str, object]:
        return {
            "server": server
            if server is not None
            else {
                "applicable": True,
                "ops_head": 206563,
                "snapshot_watermark": 204801,
                "room_heads": {"room-a": 206500, "room-b": 206100},
                "errors": [],
            },
            "sync_rate": sync_rate
            if sync_rate is not None
            else {
                "available": True,
                "source": "journal:caddy.service",
                "window_seconds": 5,
                "since": "2026-08-01T13:59:55Z",
                "total": 2,
                "clients": [
                    {
                        "ip": "207.188.7.157",
                        "count": 2,
                        "rate_per_second": 0.4,
                        "attributed_host": "finite-lat-3",
                    }
                ],
                "errors": [],
            },
            "client_stores": client_stores
            if client_stores is not None
            else {
                "stores": [
                    {
                        "kind": "hosted-device",
                        "label": "users-3f2a",
                        "path": "/var/lib/private/finitechat-hosted-device/users/3f2a/chat/client.sqlite3",
                        "age_seconds": 30,
                        "rooms": {"room-a": 206400, "room-b": 206050},
                    }
                ],
                "skipped": [],
                "errors": [],
            },
        }

    def test_watermark_gap_beyond_two_intervals_is_red_with_numbers(self) -> None:
        # The Aug 27-29 freeze class: ops keep being accepted while the
        # durable-state watermark stands still (~8,000 un-snapshotted ops).
        raw = self.chat_raw(
            server={
                "applicable": True,
                "ops_head": 206563,
                "snapshot_watermark": 198000,
                "room_heads": {},
                "errors": [],
            }
        )
        report = finite_status.build_chat_plane(
            raw, finite_status.parse_time(raw_sync_since())
        )
        watermark = report["server_watermark"]
        self.assertEqual(watermark["status"], "red")
        self.assertEqual(watermark["gap_ops"], 206563 - 198000)
        self.assertEqual(watermark["snapshot_interval_ops"], 4096)
        self.assertEqual(watermark["gap_red_ops"], 8192)
        self.assertGreater(watermark["gap_intervals"], 2.0)
        self.assertEqual(report["status"], "red")
        output = finite_status.render_human(
            {
                "generated_at": "2026-08-01T14:00:00Z",
                "exit_code": 1,
                "overall_status": "red",
                "sections": {
                    name: {"status": "green"}
                    for name in (
                        "fleet_convergence",
                        "host_health",
                        "recovery_boundary",
                        "rollout_state",
                    )
                }
                | {"chat_plane": report},
            }
        )
        self.assertIn("gap 8563 ops (~2.09 intervals of 4096) [RED]", output)

    def test_watermark_within_two_intervals_is_green(self) -> None:
        report = finite_status.build_chat_plane(
            self.chat_raw(), finite_status.parse_time(raw_sync_since())
        )
        self.assertEqual(report["server_watermark"]["status"], "green")
        self.assertEqual(report["server_watermark"]["gap_ops"], 1762)

    def test_watermark_probe_errors_read_unknown_never_crash(self) -> None:
        raw = self.chat_raw(
            server={
                "applicable": True,
                "database": "/var/lib/private/finite-chat/data/server.sqlite3",
                "errors": ["read-only sqlite query failed: disk image is malformed"],
            }
        )
        report = finite_status.build_chat_plane(
            raw, finite_status.parse_time(raw_sync_since())
        )
        self.assertEqual(report["server_watermark"]["status"], "unknown")
        self.assertEqual(report["status"], "unknown")

    def test_watermark_not_applicable_off_the_app_host(self) -> None:
        raw = self.chat_raw(
            server={
                "applicable": False,
                "reason": "chat server database not present on this host",
            }
        )
        report = finite_status.build_chat_plane(
            raw, finite_status.parse_time(raw_sync_since())
        )
        self.assertEqual(report["server_watermark"]["status"], "green")
        self.assertEqual(report["server_watermark"]["state"], "not-applicable")

    def test_sync_fetch_rate_red_names_the_livelocked_egress(self) -> None:
        # The Aug 29 quarantine livelock: 13-25 POST /sync/group per second
        # from one runner egress address.
        raw = self.chat_raw(
            sync_rate={
                "available": True,
                "source": "journal:caddy.service",
                "window_seconds": 5,
                "since": "2026-08-01T13:59:55Z",
                "total": 97,
                "clients": [
                    {
                        "ip": "207.188.7.157",
                        "count": 97,
                        "rate_per_second": 19.4,
                        "attributed_host": "finite-lat-3",
                    },
                    {
                        "ip": "198.51.100.7",
                        "count": 3,
                        "rate_per_second": 0.6,
                        "attributed_host": None,
                    },
                ],
                "errors": [],
            }
        )
        report = finite_status.build_chat_plane(
            raw, finite_status.parse_time(raw_sync_since())
        )
        sync = report["sync_fetch_rate"]
        self.assertEqual(sync["status"], "red")
        self.assertEqual(len(sync["over_threshold"]), 1)
        self.assertEqual(sync["over_threshold"][0]["attributed_host"], "finite-lat-3")

    def test_sync_fetch_rate_error_only_sections_never_crash_the_render(self) -> None:
        # The whole-report CollectionError fallback and the no-evidence
        # builder both emit an error-only chat section; rendering it must
        # never raise (a probe error degrades to UNKNOWN, never a crash).
        report = {
            "schema_version": "finite.status.v1",
            "generated_at": "2026-08-29T23:00:00Z",
            "overall_status": "unknown",
            "exit_code": 2,
            "sections": {
                name: {"status": "unknown", "error": "evidence unavailable"}
                for name in (
                    "fleet_convergence",
                    "host_health",
                    "recovery_boundary",
                    "rollout_state",
                    "chat_plane",
                )
            },
        }
        output = finite_status.render_human(report)
        self.assertIn("Chat plane [UNKNOWN]", output)
        self.assertIn("evidence unavailable", output.split("Chat plane", 1)[1])

    def test_collect_sync_fetch_rate_swallows_a_missing_journalctl(self) -> None:
        now = finite_status.parse_time("2026-08-29T23:00:00Z")
        with (
            mock.patch.object(finite_status.glob, "glob", return_value=[]),
            mock.patch.object(
                finite_status,
                "run_read_only",
                side_effect=finite_status.CollectionError(
                    "journalctl unavailable: no such file"
                ),
            ),
        ):
            raw = finite_status.collect_sync_fetch_rate(now)
        self.assertFalse(raw["available"])
        self.assertTrue(raw["errors"])
        self.assertIn("journalctl unavailable", raw["errors"][0])

    def test_sync_fetch_rate_without_evidence_is_unknown_and_actionable(self) -> None:
        raw = self.chat_raw(
            sync_rate={
                "available": False,
                "window_seconds": 5,
                "errors": [
                    "no access-log evidence in the caddy.service journal; the chat"
                    " vhost needs a `log` directive (infra/nixos/modules/caddy.nix)"
                    " for this probe"
                ],
            }
        )
        report = finite_status.build_chat_plane(
            raw, finite_status.parse_time(raw_sync_since())
        )
        self.assertEqual(report["sync_fetch_rate"]["status"], "unknown")
        self.assertIn("log` directive", report["sync_fetch_rate"]["errors"][0])

    def caddy_entry(self, *, ts: float, ip: str, method: str, uri: str) -> str:
        return json.dumps(
            {
                "level": "info",
                "ts": ts,
                "logger": "http.log.access.chat.finite.computer",
                "msg": "handled request",
                "request": {"remote_ip": ip, "method": method, "uri": uri},
            }
        )

    def test_collect_sync_fetch_rate_samples_the_edge_journal(self) -> None:
        now = finite_status.parse_time("2026-08-29T23:00:00Z")
        window = finite_status.CONTRACT["chat_plane"]["sync_rate_window_seconds"]
        journal_lines = [
            json.dumps(
                {
                    "MESSAGE": self.caddy_entry(
                        ts=now.timestamp() - 1,
                        ip="207.188.7.157",
                        method="POST",
                        uri="/sync/group",
                    )
                }
            ),
            json.dumps(
                {
                    "MESSAGE": self.caddy_entry(
                        ts=now.timestamp() - 2,
                        ip="207.188.7.157",
                        method="POST",
                        uri="/sync/group?x=1",
                    )
                }
            ),
            json.dumps(
                {
                    "MESSAGE": self.caddy_entry(
                        ts=now.timestamp() - 3,
                        ip="198.51.100.7",
                        method="POST",
                        uri="/sync/group",
                    )
                }
            ),
            json.dumps(
                {
                    "MESSAGE": self.caddy_entry(
                        ts=now.timestamp() - 1,
                        ip="207.188.7.157",
                        method="GET",
                        uri="/health",
                    )
                }
            ),
            json.dumps(
                {
                    "MESSAGE": self.caddy_entry(
                        ts=now.timestamp() - window - 5,
                        ip="207.188.7.157",
                        method="POST",
                        uri="/sync/group",
                    )
                }
            ),
            "not json",
        ]
        completed = subprocess.CompletedProcess(
            ["journalctl"], 0, "\n".join(journal_lines), ""
        )
        with (
            mock.patch.object(finite_status.glob, "glob", return_value=[]),
            mock.patch.object(
                finite_status, "run_read_only", return_value=completed
            ) as run,
        ):
            raw = finite_status.collect_sync_fetch_rate(now)
        self.assertTrue(raw["available"])
        self.assertEqual(raw["source"], "journal:caddy.service")
        self.assertEqual(raw["total"], 3)
        clients = {client["ip"]: client for client in raw["clients"]}
        self.assertEqual(clients["207.188.7.157"]["count"], 2)
        self.assertEqual(clients["207.188.7.157"]["attributed_host"], "finite-lat-3")
        self.assertEqual(clients["198.51.100.7"]["count"], 1)
        self.assertIsNone(clients["198.51.100.7"]["attributed_host"])
        command = run.call_args.args[0]
        self.assertEqual(command[:3], ["journalctl", "--no-pager", "--output=json"])
        self.assertIn("--unit=caddy.service", command)
        # The window must be bounded and derived from `now`.
        since_flag = next(part for part in command if part.startswith("--since="))
        self.assertEqual(
            since_flag.removeprefix("--since="),
            finite_status.isoformat(now - timedelta(seconds=window)),
        )

    def test_collect_sync_fetch_rate_prefers_newest_access_log_file(self) -> None:
        now = finite_status.parse_time("2026-08-29T23:00:00Z")
        with tempfile.TemporaryDirectory() as directory:
            old_log = Path(directory) / "access-chat.finite.computer.log.1"
            new_log = Path(directory) / "access-chat.finite.computer.log"
            old_log.write_text(
                self.caddy_entry(
                    ts=now.timestamp() - 1,
                    ip="207.188.7.157",
                    method="POST",
                    uri="/sync/group",
                )
                + "\n",
                encoding="utf-8",
            )
            new_log.write_text(
                self.caddy_entry(
                    ts=now.timestamp() - 1,
                    ip="152.236.34.15",
                    method="POST",
                    uri="/sync/group",
                )
                + "\n",
                encoding="utf-8",
            )
            with (
                mock.patch.object(
                    finite_status.glob,
                    "glob",
                    return_value=[str(old_log), str(new_log)],
                ) as glob_call,
                mock.patch.object(
                    finite_status.os.path,
                    "getmtime",
                    side_effect=lambda path: 1 if str(path).endswith(".log.1") else 2,
                ),
                mock.patch.object(finite_status, "run_read_only") as run,
            ):
                raw = finite_status.collect_sync_fetch_rate(now)
            self.assertTrue(raw["available"])
            self.assertEqual(raw["source"], f"file:{new_log}")
            self.assertEqual(raw["clients"][0]["attributed_host"], "finite-lat-4")
            run.assert_not_called()
            self.assertTrue(glob_call.call_args.args[0].endswith("access*.log"))

    def test_cursor_probe_flags_head_ahead_plus_stale_store(self) -> None:
        # Freeze signature: the server head advances while the client store
        # stands still (Aug 27-29 blindness).
        raw = self.chat_raw(
            client_stores={
                "stores": [
                    {
                        "kind": "hosted-device",
                        "label": "users-3f2a",
                        "path": "/var/lib/private/finitechat-hosted-device/users/3f2a/chat/client.sqlite3",
                        "age_seconds": 2700,
                        "rooms": {"room-a": 195000},
                    }
                ],
                "skipped": [],
                "errors": [],
            }
        )
        report = finite_status.build_chat_plane(
            raw, finite_status.parse_time(raw_sync_since())
        )
        cursors = report["agent_cursors"]
        self.assertEqual(cursors["status"], "red")
        self.assertEqual(len(cursors["flagged_rooms"]), 1)
        flagged = cursors["flagged_rooms"][0]
        self.assertEqual(flagged["room_id"], "room-a")
        self.assertEqual(flagged["lag_ops"], 206500 - 195000)
        self.assertEqual(flagged["store_age_seconds"], 2700)
        output = finite_status.render_human(
            {
                "generated_at": "2026-08-29T23:00:00Z",
                "exit_code": 1,
                "overall_status": "red",
                "sections": {
                    name: {"status": "green"}
                    for name in (
                        "fleet_convergence",
                        "host_health",
                        "recovery_boundary",
                        "rollout_state",
                    )
                }
                | {"chat_plane": report},
            }
        )
        self.assertIn(
            "FROZEN hosted-device users-3f2a: room room-a head 206500", output
        )
        self.assertIn("store idle 45m", output)

    def test_cursor_probe_tolerates_standing_lag_when_store_is_fresh(self) -> None:
        # client_app_events lags the true cursor by design (membership and
        # key-package ops never land there); a fresh store means the client
        # is alive and converging, not frozen.
        raw = self.chat_raw(
            client_stores={
                "stores": [
                    {
                        "kind": "hosted-device",
                        "label": "users-3f2a",
                        "path": "/store",
                        "age_seconds": 30,
                        "rooms": {"room-a": 206490},
                    }
                ],
                "skipped": [],
                "errors": [],
            }
        )
        report = finite_status.build_chat_plane(
            raw, finite_status.parse_time(raw_sync_since())
        )
        self.assertEqual(report["agent_cursors"]["status"], "green")
        self.assertIn(
            "client_app_events MAX(seq) can lag",
            report["agent_cursors"]["evidence_note"],
        )

    def test_cursor_probe_unknown_without_server_heads_on_a_runner_host(self) -> None:
        raw = self.chat_raw(
            server={
                "applicable": False,
                "reason": "chat server database not present on this host",
            },
            client_stores={
                "runner_work_root": "/data/finite-saas-runner",
                "stores": [
                    {
                        "kind": "agent",
                        "label": "Lat3 Agent 01",
                        "agent_runtime_id": "runtime-lat3-01",
                        "path": "/data/finite-saas-runner/kata/machine-01/agent/client.sqlite3",
                        "age_seconds": 12,
                        "rooms": {"room-a": 206400},
                    }
                ],
                "skipped": [],
                "errors": [],
            },
        )
        report = finite_status.build_chat_plane(
            raw, finite_status.parse_time(raw_sync_since())
        )
        cursors = report["agent_cursors"]
        self.assertEqual(cursors["status"], "unknown")
        self.assertIn("app host", cursors["reason"])
        # The agent-side evidence is still displayed, not hidden.
        self.assertEqual(cursors["stores"][0]["rooms"][0]["cursor_max_seq"], 206400)

    def test_cursor_probe_not_applicable_without_local_stores(self) -> None:
        raw = self.chat_raw(client_stores={"stores": [], "skipped": [], "errors": []})
        report = finite_status.build_chat_plane(
            raw, finite_status.parse_time(raw_sync_since())
        )
        self.assertEqual(report["agent_cursors"]["status"], "green")
        self.assertEqual(report["agent_cursors"]["state"], "not-applicable")

    def test_collect_chat_server_state_never_opens_the_live_database(self) -> None:
        queries: list[str] = []

        def fake_int(database, sql, timeout=15):
            queries.append(sql)
            if "http_delivery_ops" in sql:
                return 206563
            if "http_state_snapshots_v2" in sql:
                return 204801
            raise AssertionError(f"unexpected int query {sql}")

        def fake_rows(database, sql, timeout=15):
            queries.append(sql)
            if "http_room_memberships" in sql:
                return [{"room_id": "room-a", "last_seq": 206500}]
            raise AssertionError(f"unexpected row query {sql}")

        with (
            mock.patch.object(finite_status.Path, "exists", return_value=True),
            mock.patch.object(finite_status, "scratch_copy_sqlite") as scratch,
            mock.patch.object(finite_status, "sqlite_int_query", side_effect=fake_int),
            mock.patch.object(
                finite_status, "sqlite_json_query", side_effect=fake_rows
            ),
        ):
            scratch.return_value.__enter__ = mock.MagicMock(
                return_value=Path("/tmp/scratch/scratch.sqlite3")
            )
            scratch.return_value.__exit__ = mock.MagicMock(return_value=False)
            raw = finite_status.collect_chat_server_state("finite-lat-2")

        self.assertTrue(raw["applicable"])
        self.assertEqual(raw["ops_head"], 206563)
        self.assertEqual(raw["snapshot_watermark"], 204801)
        self.assertEqual(raw["room_heads"], {"room-a": 206500})
        self.assertEqual(raw["errors"], [])
        scratch.assert_called_once()
        live = scratch.call_args.args[0]
        self.assertEqual(
            str(live), finite_status.CONTRACT["chat_plane"]["server_database"]
        )
        self.assertTrue(all("http_" in sql for sql in queries))

    def test_collect_chat_server_state_marks_missing_database(self) -> None:
        with mock.patch.object(finite_status.Path, "exists", return_value=False):
            raw = finite_status.collect_chat_server_state("finite-lat-3")
        self.assertFalse(raw["applicable"])
        self.assertEqual(raw["reason"], "chat server database not present on this host")

    def test_scratch_copy_sqlite_copies_sidecars_and_cleans_up(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            live = Path(directory) / "server.sqlite3"
            live.write_bytes(b"db")
            Path(f"{live}-wal").write_bytes(b"wal")
            Path(f"{live}-shm").write_bytes(b"shm")
            scratch_root = Path(directory) / "scratch"
            scratch_root.mkdir()
            with (
                mock.patch.object(
                    finite_status.tempfile, "mkdtemp", return_value=str(scratch_root)
                ),
                mock.patch.object(
                    finite_status.shutil,
                    "copyfile",
                    wraps=finite_status.shutil.copyfile,
                ) as copy,
                mock.patch.object(
                    finite_status.shutil, "rmtree", wraps=finite_status.shutil.rmtree
                ),
            ):
                with finite_status.scratch_copy_sqlite(live) as scratch:
                    self.assertEqual(scratch.parent, scratch_root)
                    self.assertTrue(scratch.is_file())
                    self.assertEqual(
                        sorted(path.name for path in scratch_root.iterdir()),
                        [
                            "scratch.sqlite3",
                            "scratch.sqlite3-shm",
                            "scratch.sqlite3-wal",
                        ],
                    )
                self.assertFalse(scratch_root.exists())
            self.assertEqual(copy.call_count, 3)
            # The live tree was never written to.
            self.assertEqual(live.read_bytes(), b"db")
            self.assertEqual(Path(f"{live}-wal").read_bytes(), b"wal")

    def test_sqlite_json_query_runs_read_only_on_the_scratch_copy(self) -> None:
        completed = subprocess.CompletedProcess(
            ["sqlite3"], 0, '[{"room_id":"room-a","last_seq":206500}]', ""
        )
        with mock.patch.object(
            finite_status, "run_read_only", return_value=completed
        ) as run:
            rows = finite_status.sqlite_json_query(
                Path("/tmp/scratch/scratch.sqlite3"), "SELECT 1;"
            )
        self.assertEqual(rows, [{"room_id": "room-a", "last_seq": 206500}])
        command = run.call_args.args[0]
        self.assertEqual(
            command[:6],
            ["sqlite3", "-safe", "-readonly", "-batch", "-init", "/dev/null"],
        )
        self.assertNotIn(
            str(finite_status.CONTRACT["chat_plane"]["server_database"]), command
        )

    def test_collect_local_client_stores_bounds_the_sample(self) -> None:
        runtimes = [
            {
                "source_host_id": "finite-lat-3",
                "agent_runtime_id": f"runtime-{index:02d}",
                "project_id": f"project-{index:02d}",
                "source_machine_id": f"Machine {index:02d}?",
                "agent_name": f"Agent {index:02d}",
                "version_label": "v2",
                "link_state": "active" if index <= 4 else "inactive",
            }
            for index in range(1, 6)
        ]
        with (
            mock.patch.object(finite_status.Path, "glob", return_value=[]),
            mock.patch.object(
                finite_status,
                "runner_work_root",
                return_value=Path("/data/finite-saas-runner"),
            ),
            mock.patch.object(finite_status.Path, "is_file", return_value=True),
            mock.patch.object(finite_status, "store_freshness", return_value=10.0),
            mock.patch.object(
                finite_status,
                "sqlite_json_query",
                return_value=[{"room_id": "room-a", "max_seq": 5}],
            ) as query,
            mock.patch.object(finite_status, "scratch_copy_sqlite") as scratch,
        ):
            scratch.return_value.__enter__ = mock.MagicMock(
                return_value=Path("/tmp/scratch/scratch.sqlite3")
            )
            scratch.return_value.__exit__ = mock.MagicMock(return_value=False)
            raw = finite_status.collect_local_client_stores(runtimes, "finite-lat-3")

        self.assertEqual(len(raw["stores"]), 3)
        # Sandbox names mirror the runner's sanitization (lowercased).
        self.assertEqual(
            raw["stores"][0]["path"],
            "/data/finite-saas-runner/kata/machine-01/agent/client.sqlite3",
        )
        self.assertEqual(raw["skipped"], [])
        self.assertEqual(query.call_count, 3)
        self.assertEqual(raw["stores"][0]["rooms"], {"room-a": 5})

    def test_collect_local_client_stores_records_missing_agent_stores(self) -> None:
        runtimes = [
            {
                "source_host_id": "finite-lat-4",
                "agent_runtime_id": "runtime-01",
                "source_machine_id": "machine-01",
                "agent_name": "Agent 01",
                "link_state": "active",
            }
        ]
        with (
            mock.patch.object(finite_status.Path, "glob", return_value=[]),
            mock.patch.object(
                finite_status,
                "runner_work_root",
                return_value=Path("/data/finite-saas-runner"),
            ),
            mock.patch.object(finite_status.Path, "is_file", return_value=False),
        ):
            raw = finite_status.collect_local_client_stores(runtimes, "finite-lat-4")
        self.assertEqual(raw["stores"], [])
        self.assertEqual(len(raw["skipped"]), 1)
        self.assertIn("no client store at", raw["skipped"][0]["reason"])

    def test_runner_host_health_is_not_scored_against_app_services(self) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        raw["host_health"]["hostname"] = "finite-lat-3"
        raw["host_health"]["roles"] = ["runner"]
        # The runner host has none of the app-plane units observed; that must
        # not drag its health to unknown.
        now = finite_status.parse_time(raw["now"])
        self.assertIsNotNone(now)
        report = finite_status.build_report(raw, now)
        health = report["sections"]["host_health"]
        self.assertEqual(health["services"], [])
        self.assertEqual(health["http_probes"], [])
        self.assertNotEqual(health["status"], "unknown")
        output = finite_status.render_human(report)
        self.assertIn("Host health", output)
        self.assertIn("runner: timer", output)

    def test_app_host_runner_fields_are_not_applicable(self) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        raw["host_health"]["hostname"] = "finite-lat-2"
        raw["host_health"]["roles"] = ["app"]
        # lat2 runs no Runner: a missing runner.env must not read as a red
        # or unknown runner state there (ADR 0007).
        raw["host_health"]["runner_environment"] = {}
        raw["host_health"]["runner_environment_files_read"] = []
        now = finite_status.parse_time(raw["now"])
        self.assertIsNotNone(now)
        report = finite_status.build_report(raw, now)
        runner = report["sections"]["host_health"]["runner"]
        self.assertEqual(runner["applicable"], False)
        output = finite_status.render_human(report)
        self.assertIn("runner: not applicable on this host", output)

    def test_legacy_host_health_input_without_roles_keeps_combined_scoring(
        self,
    ) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        self.assertNotIn("roles", raw["host_health"])
        now = finite_status.parse_time(raw["now"])
        self.assertIsNotNone(now)
        report = finite_status.build_report(raw, now)
        health = report["sections"]["host_health"]
        self.assertTrue(health["services"])
        self.assertTrue(health["http_probes"])
        self.assertNotEqual(health["runner"].get("applicable"), False)
        self.assertIn("timer_status", health["runner"])


def raw_sync_since() -> str:
    return "2026-08-01T14:00:00Z"

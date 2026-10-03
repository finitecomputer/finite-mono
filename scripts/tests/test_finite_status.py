from __future__ import annotations

import contextlib
import hashlib
import io
import json
import os
from datetime import timedelta
from pathlib import Path
import subprocess
import tempfile
import time
import unittest
from unittest import mock

from scripts import finite_status


ROOT = Path(__file__).resolve().parents[2]
COMMAND = ROOT / "scripts" / "finite-status"
FIXTURE = ROOT / "scripts" / "tests" / "fixtures" / "finite_status_aug1.json"


class FiniteStatusTests(unittest.TestCase):
    def _shim_exit_fixture(self, *, duplicate=False, drift=False, unsafe_log=False, missing_handle=False, fail_read=None, publisher=None, publisher_drift=False, extra_options=(), filesystem_hook=None, isolated=False):
        container = "a" * 64
        executable = "/nix/store/" + "b" * 32 + "-kata/bin/containerd-shim-kata-v2"
        report = {"checks": [] if missing_handle else [{"name": "canonical_handle", "status": "pass",
            "evidence": {"container_id": container, "state_root": "/data/finite-saas-runner/kata/runtime_fixture"}}]}
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary).resolve()
            def path(value):
                value = str(value)
                return base / value.lstrip("/") if value.startswith(("/proc", "/var", "/nix")) else Path(value)
            def stat_bytes(state, parent, start, wait=0):
                fields = [state, str(parent)] + ["0"] * 48
                fields[19] = str(start); fields[49] = str(wait)
                return ("99 (PRIVATE_COMM_λ) " + " ".join(fields)).encode()
            path(executable).parent.mkdir(parents=True)
            path(executable).touch(mode=0o755)
            path("/proc/self/ns").mkdir(parents=True)
            path("/proc/self/ns/pid").symlink_to("pid:[100]")
            path("/proc/self/mountinfo").write_text("1 0 8:1 / /var rw - ext4 /dev/example rw\n")
            path("/proc/1/ns").mkdir(parents=True)
            path("/proc/1/ns/pid").symlink_to("pid:[100]")
            path("/proc/1/stat").write_bytes(stat_bytes("S", 0, 1))
            path("/proc/1/comm").write_text("systemd")
            path("/proc/1/exe").symlink_to("/nix/store/systemd/bin/systemd")
            options = ["-namespace", "finite", "-address", "/run/containerd/containerd.sock"]
            if publisher is not None:
                options += ["-publish-binary", publisher]
            options += ["-id", container] + list(extra_options)
            command = ("\0".join([executable] + options) + "\0").encode()
            for pid in ([100, 102] if duplicate else [100]):
                path(f"/proc/{pid}").mkdir()
                path(f"/proc/{pid}/stat").write_bytes(stat_bytes("S", 1, pid))
                path(f"/proc/{pid}/cmdline").write_bytes(command)
                path(f"/proc/{pid}/exe").symlink_to(executable)
            for pid, parent in ((101, 100), (103, 999)):
                path(f"/proc/{pid}").mkdir()
                path(f"/proc/{pid}/stat").write_bytes(stat_bytes("Z", parent, pid, 15))
            log = path("/var/lib/nerdctl/01234567/containers/finite/" + container + "/" + container + "-json.log")
            log.parent.mkdir(parents=True);log.write_text("PRIVATE_LOG_CONTENT")
            if unsafe_log:log.chmod(0o666)
            real_stat, real_lstat, real_open, real_statvfs = Path.stat, Path.lstat, Path.open, os.statvfs
            reads = []
            command_reads = 0
            def root_info(method, value, *args, **kwargs):
                if "PRIVATE_PUBLISHER" in str(value):
                    raise AssertionError("opaque publisher must not be inspected")
                info = method(value, *args, **kwargs)
                fields = list(info); fields[4] = 0
                return os.stat_result(fields)
            def guarded_open(value, *args, **kwargs):
                nonlocal command_reads
                reads.append(str(value))
                if filesystem_hook and value == path("/proc/self/mountinfo"):
                    filesystem_hook("mountinfo", path)
                if fail_read and value == path(fail_read):
                    raise PermissionError(13, "PRIVATE_EXCEPTION_PAYLOAD")
                if "PRIVATE_PUBLISHER" in str(value):
                    raise AssertionError("opaque publisher must not be opened")
                if value == log:
                    raise AssertionError("log contents must never be opened")
                if value == path("/proc/100/cmdline"):
                    command_reads += 1
                    if drift and command_reads > 1:return io.BytesIO(b"changed")
                    if publisher_drift and command_reads > 1:
                        return io.BytesIO(command.replace(publisher.encode(), b"DIFFERENT_OPAQUE_PUBLISHER"))
                return real_open(value, *args, **kwargs)
            def guarded_statvfs(value):
                if filesystem_hook:
                    filesystem_hook("statvfs", path)
                return real_statvfs(value)
            with mock.patch.object(finite_status, "Path", side_effect=path), \
                 mock.patch.object(Path, "stat", lambda value, *a, **kw: root_info(real_stat, value, *a, **kw)), \
                 mock.patch.object(Path, "lstat", lambda value, *a, **kw: root_info(real_lstat, value, *a, **kw)), \
                 mock.patch.object(Path, "open", guarded_open), \
                 mock.patch.object(finite_status.os, "statvfs", guarded_statvfs), \
                 mock.patch.object(finite_status, "run_read_only", side_effect=AssertionError("no runtime commands required")):
                result = (finite_status.collect_runtime_shim_exit_log(report) if isolated else
                    finite_status._collect_runtime_shim_exit_log(report, time.monotonic() + 10))
            return result, reads

    def test_shim_exit_log_isolated_worker_preserves_observation(self) -> None:
        result, _ = self._shim_exit_fixture(isolated=True)
        self.assertEqual(result["status"], "observed")
        self.assertEqual(result["shim"]["starttime_ticks"], "100")
        self.assertEqual(result["log_filesystem_type"], "ext4")

    def test_shim_exit_log_worker_setup_failure_is_sanitized(self) -> None:
        for operation in ("pipe", "fork"):
            with self.subTest(operation=operation):
                with mock.patch.object(finite_status.os, operation, side_effect=OSError(24, "PRIVATE_FAILURE")):
                    result = finite_status.collect_runtime_shim_exit_log({})
                self.assertEqual(result["status"], "unknown")
                self.assertEqual(result["error_kind"], "OSError")
                self.assertEqual(result["error_errno"], 24)
                self.assertNotIn("PRIVATE", json.dumps(result))

    def test_shim_exit_log_rejects_worker_result_after_deadline(self) -> None:
        read = os.read
        def delayed_read(*args):
            data = read(*args)
            time.sleep(0.3)
            return data
        with mock.patch.object(finite_status.os, "read", side_effect=delayed_read), \
             mock.patch.object(finite_status, "SHIM_EXIT_LOG_TIMEOUT", 0.2):
            result, _ = self._shim_exit_fixture(isolated=True)
        self.assertEqual(result["status"], "unknown")
        self.assertEqual(result["error_kind"], "TimeoutError")
        self.assertNotIn("default_json_log", result)

    def test_shim_exit_log_checks_deadline_after_filesystem_collection(self) -> None:
        for phase in ("mountinfo", "statvfs"):
            with self.subTest(phase=phase):
                clock = [0.0]
                def expire(current, _):
                    if current == phase:
                        clock[0] = 11.0
                with mock.patch.object(finite_status.time, "monotonic", side_effect=lambda: clock[0]):
                    result, _ = self._shim_exit_fixture(filesystem_hook=expire)
                self.assertEqual(result["status"], "unknown")
                self.assertEqual(result["exited_children"], [])
                self.assertNotIn("default_json_log", result)

    def test_shim_exit_log_rechecks_lifetime_after_filesystem_collection(self) -> None:
        for phase in ("mountinfo", "statvfs"):
            for identity in ("stat", "exe", "cmdline"):
                with self.subTest(phase=phase, identity=identity):
                    def change(current, path):
                        if current != phase:
                            return
                        target = path("/proc/100/" + identity)
                        if identity == "stat":
                            target.write_bytes(target.read_bytes().replace(b" 100 ", b" 999 "))
                        elif identity == "exe":
                            target.unlink()
                            target.symlink_to("/PRIVATE_REPLACEMENT")
                        else:
                            target.write_bytes(b"PRIVATE_REPLACEMENT\0")
                    result, _ = self._shim_exit_fixture(filesystem_hook=change)
                    self.assertEqual(result["status"], "unknown")
                    self.assertEqual(result["observation_phase"], "final_lifetime")
                    self.assertEqual(result["exited_children"], [])
                    self.assertNotIn("PRIVATE", json.dumps(result))

    def test_shim_exit_log_stalled_filesystem_cannot_hold_caller(self) -> None:
        for phase in ("mountinfo", "statvfs"):
            with self.subTest(phase=phase):
                def stall(current, _):
                    if current == phase:
                        time.sleep(30)
                started = time.monotonic()
                with mock.patch.object(finite_status, "SHIM_EXIT_LOG_TIMEOUT", 0.2):
                    result, _ = self._shim_exit_fixture(filesystem_hook=stall, isolated=True)
                self.assertLess(time.monotonic() - started, 2)
                self.assertEqual(result["status"], "unknown")
                self.assertEqual(result["error_kind"], "TimeoutError")
                self.assertEqual(result["exited_children"], [])
                self.assertNotIn("default_json_log", result)

    def test_shim_exit_log_metadata_is_exact_read_only_and_redacted(self) -> None:
        result, reads = self._shim_exit_fixture()
        self.assertEqual(result["status"], "observed")
        self.assertEqual(result["exited_children"], [{"pid": 101, "parent_pid": 100,
            "starttime_ticks": "101", "leader_state": "Z", "exit_wait_status": 15, "wait_status_scope": "leader_only"}])
        self.assertEqual(result["default_json_log"]["size_bytes"], len("PRIVATE_LOG_CONTENT"))
        self.assertEqual(result["log_filesystem_type"], "ext4")
        self.assertFalse(result["repair_authority"])
        self.assertFalse(result["historical_logger_identity_established"])
        self.assertFalse(result["host_context"]["host_namespace_authority_established"])
        self.assertFalse(any(value.endswith("environ") for value in reads))
        self.assertNotIn("PRIVATE", json.dumps(result))

    def test_shim_exit_log_accepts_opaque_vendor_publisher_without_access(self) -> None:
        for publisher in ("/tmp/PRIVATE_PUBLISHER", "binary://PRIVATE_PUBLISHER?literal=value", "PRIVATE_PUBLISHER --literal", ""):
            with self.subTest(publisher=publisher):
                result, reads = self._shim_exit_fixture(publisher=publisher)
                self.assertEqual(result["status"], "observed")
                self.assertEqual(result["shim"], {"pid": 100, "starttime_ticks": "100", "publisher_argument_present": True,
                    "publisher_argument_sha256": hashlib.sha256(publisher.encode()).hexdigest()})
                self.assertFalse(any("PRIVATE_PUBLISHER" in value for value in reads))
                self.assertNotIn("PRIVATE_PUBLISHER", json.dumps(result))

    def test_shim_exit_log_refuses_oversized_publisher_and_unknown_duplicate_flags(self) -> None:
        cases = [({"publisher": "x" * 4097}, "publisher_argument", "publisher_argument_bound"),
                 ({"extra_options": ("-namespace", "finite")}, "shim_options", "shim_option_shape"),
                 ({"extra_options": ("-unknown", "PRIVATE_VALUE")}, "shim_options", "shim_option_values"),
                 ({"publisher": "opaque", "extra_options": ("-publish-binary", "PRIVATE_VALUE")}, "shim_options", "shim_option_shape")]
        for options, detail, gate in cases:
            with self.subTest(detail=detail, gate=gate):
                result, _ = self._shim_exit_fixture(**options)
                self.assertEqual(result["status"], "unknown")
                self.assertEqual(result["observation_phase"], "shim_bind")
                self.assertEqual((result["observation_detail"], result["failure_gate"]), (detail, gate))
                self.assertNotIn("PRIVATE_VALUE", json.dumps(result))

    def test_shim_exit_log_opaque_publisher_drift_refuses_observation(self) -> None:
        result, _ = self._shim_exit_fixture(publisher="PRIVATE_PUBLISHER", publisher_drift=True)
        self.assertEqual(result["status"], "unknown")
        self.assertEqual(result["observation_phase"], "final_lifetime")
        self.assertNotIn("PRIVATE_PUBLISHER", json.dumps(result))

    def test_shim_exit_log_ambiguity_drift_and_unsafe_layout_are_unknown(self) -> None:
        for options, phase in (({"duplicate": True}, "shim_bind"), ({"drift": True}, "final_lifetime"),
                               ({"unsafe_log": True}, "log_file"), ({"missing_handle": True}, "canonical_handle")):
            with self.subTest(options=options):
                result, _ = self._shim_exit_fixture(**options)
                self.assertEqual(result["status"], "unknown")
                self.assertEqual(result["exited_children"], [])
                self.assertEqual(result["observation_phase"], phase)
                self.assertEqual(result["error_kind"], "ValueError")

    def test_shim_exit_log_failure_phase_and_errno_do_not_leak_exception_payload(self) -> None:
        for target, phase in (("/proc/1/comm", "namespace"), ("/proc/100/stat", "process_inventory"),
                              ("/proc/self/mountinfo", "filesystem")):
            with self.subTest(phase=phase):
                result, _ = self._shim_exit_fixture(fail_read=target)
                self.assertEqual(result["status"], "unknown")
                self.assertEqual(result["observation_phase"], phase)
                self.assertEqual(result["error_kind"], "PermissionError")
                self.assertEqual(result["error_errno"], 13)
                self.assertNotIn("PRIVATE", json.dumps(result))
                self.assertNotIn(target, json.dumps(result))

    def test_exec_observation_hashes_args_and_distinguishes_fifo_writer_from_path_pin(self) -> None:
        container = "a" * 64
        command = b"nerdctl\0--namespace\0finite\0exec\0-i\0" + container.encode() + b"\0python3\0-c\0PRIVATE_PAYLOAD\0"
        with tempfile.TemporaryDirectory() as temporary:
            process = Path(temporary)
            (process / "fd").mkdir()
            (process / "fdinfo").mkdir()
            for number, flags in (("7", getattr(os, "O_PATH", 0o10000000)), ("9", os.O_WRONLY | os.O_NONBLOCK)):
                os.mkfifo(process / "fd" / number)
                (process / "fdinfo" / number).write_text(f"flags:\t{flags:o}\n")
            target = "/run/containerd/fifo/123/exec-" + "b" * 64 + "-stdin"
            with mock.patch.object(finite_status.os, "readlink", return_value=target):
                observation = finite_status.observe_nerdctl_exec(process, command, container)
        self.assertEqual(observation["argv_sha256"], hashlib.sha256(command).hexdigest())
        self.assertTrue(observation["interactive"])
        self.assertEqual(observation["command_classification"], "other")
        self.assertEqual({row["fd"]: row["holds_writer"] for row in observation["stdio_fifos"]}, {"7": False, "9": True})
        self.assertFalse(observation["observation_grants_cancel_authority"])
        self.assertNotIn("PRIVATE_PAYLOAD", json.dumps(observation))

    def test_exec_observation_requires_an_exact_container_argument(self) -> None:
        container = "a" * 64
        for command in (b"nerdctl\0exec\0" + container.encode() + b"-suffix\0fbrain\0--version\0",
                        b"nerdctl\0" + container.encode() + b"\0exec\0fbrain\0--version\0",
                        b"nerdctl\0--namespace\0exec\0run\0" + container.encode() + b"\0fbrain\0--version\0"):
            self.assertIsNone(finite_status.observe_nerdctl_exec(Path("/nonexistent"), command, container))

    def test_exec_version_classification_does_not_accept_extra_shell_commands(self) -> None:
        container = "a" * 64
        with tempfile.TemporaryDirectory() as temporary:
            process = Path(temporary)
            (process / "fd").mkdir()
            prefix = b"nerdctl\0exec\0" + container.encode() + b"\0sh\0-c\0"
            safe = finite_status.observe_nerdctl_exec(process, prefix + b"fbrain --version\0", container)
            other = finite_status.observe_nerdctl_exec(process, prefix + b"fbrain --version; PRIVATE_PAYLOAD\0", container)
        self.assertEqual(safe["command_classification"], "version_only")
        self.assertEqual(other["command_classification"], "other")
        self.assertNotIn("PRIVATE_PAYLOAD", json.dumps(other))

    def test_exec_observation_refuses_redirected_non_fifo_stream(self) -> None:
        container = "a" * 64
        with tempfile.TemporaryDirectory() as temporary:
            process = Path(temporary)
            (process / "fd").mkdir()
            (process / "fd" / "9").write_text("PRIVATE_PAYLOAD")
            target = "/run/containerd/fifo/123/exec-" + "b" * 64 + "-stdin"
            with mock.patch.object(finite_status.os, "readlink", return_value=target):
                with self.assertRaisesRegex(ValueError, "not a FIFO"):
                    finite_status.observe_nerdctl_exec(process, b"nerdctl\0exec\0" + container.encode() + b"\0fbrain\0--version\0", container)

    def test_recovery_receipts_distinguish_absent_empty_and_used(self) -> None:
        for state, count in [("absent", ""), ("present", "0"), ("present", "2")]:
            with self.subTest(state=state, count=count):
                output = f"__FINITE_STATUS_CREDENTIAL_RECOVERY_STATE__\n{state},{count}\n__FINITE_STATUS_RUNTIMES__\n"
                completed = subprocess.CompletedProcess(["psql"], 0, output, "")
                with mock.patch.object(finite_status, "run_read_only", return_value=completed) as run:
                    core = finite_status.psql_query_sets({})
                sql = run.call_args.kwargs["input_text"]
                self.assertTrue(sql.startswith("BEGIN TRANSACTION READ ONLY;"))
                self.assertIn("\\if :finite_has_credential_recoveries", sql)
                self.assertIn("SELECT 'absent',NULL::bigint;", sql)
                self.assertLess(sql.index("\\echo __FINITE_STATUS_CREDENTIAL_RECOVERY_STATE__"), sql.index("\\echo __FINITE_STATUS_RUNTIMES__"))
                raw = finite_status.load_fixture(FIXTURE)
                raw["core"]["credential_recovery_state"] = core["credential_recovery_state"]
                report = finite_status.build_report(raw, finite_status.parse_time(raw["now"]))
                observation = report["sections"]["fleet_convergence"]["credential_recovery_state"]
                self.assertEqual(observation["observations"], [{"schema_state": state, "receipt_count": count}])

    def test_registry_inventory_keeps_retired_and_missing_references(self) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        core = raw["core"]
        old = core["artifacts"][-1]
        old["retired_at"] = "2026-07-31T00:00:00Z"
        old["reference"] = "ghcr.io/finite/runtime@sha256:" + "a" * 64
        core["runtimes"][0]["runtime_artifact_id"] = old["id"]
        core["runtimes"][0]["link_state"] = "inactive"
        core["artifacts"].append(
            {"id": "unknown-reference", "version_label": "unknown"}
        )
        report = finite_status.build_fleet(core, finite_status.parse_time(raw["now"]))
        inventory = {row["id"]: row for row in report["artifact_inventory"]}
        self.assertEqual(inventory[old["id"]]["reference"], old["reference"])
        self.assertGreaterEqual(inventory[old["id"]]["recorded_runtime_count"], 1)
        self.assertEqual(inventory[old["id"]]["retired_at"], old["retired_at"])
        self.assertIsNone(inventory["unknown-reference"]["reference"])
        self.assertEqual(inventory["unknown-reference"]["recorded_runtime_count"], 0)

    def fixture_report(self) -> dict[str, object]:
        raw = finite_status.load_fixture(FIXTURE)
        now = finite_status.parse_time(raw["now"])
        self.assertIsNotNone(now)
        return finite_status.build_report(raw, now)

    @unittest.skipUnless(os.environ.get("FC_CORE_POSTGRES_TEST_URL"), "requires disposable Core Postgres")
    def test_hosted_enrollment_query_matches_primary_and_credential_conflicts(self) -> None:
        with mock.patch.object(finite_status, "run_read_only", return_value=subprocess.CompletedProcess([], 0, "__FINITE_STATUS_RUNTIMES__\n", "")) as run:
            finite_status.psql_query_sets({})
        query = run.call_args.kwargs["input_text"].split("\\echo __FINITE_STATUS_HOSTED_ENROLLMENT__\n", 1)[1].split("\\echo __FINITE_STATUS_ARTIFACTS__", 1)[0]
        fixture = """
BEGIN;
SET LOCAL search_path=pg_temp;
CREATE TEMP TABLE projects(id text,owner_user_id text);
CREATE TEMP TABLE agent_runtimes(id text,project_id text,source_host_id text,source_machine_id text,runtime_artifact_id text);
CREATE TEMP TABLE project_runtime_links(project_id text,agent_runtime_id text,active boolean);
CREATE TEMP TABLE agent_creation_requests(id text,agent_runtime_id text,project_id text,status text,owner_user_id text,runner_class text,relocation_spec jsonb,target_source_host_id text);
INSERT INTO projects VALUES ('project','owner');
INSERT INTO agent_runtimes VALUES ('runtime','project','host','machine','artifact');
INSERT INTO project_runtime_links VALUES ('project','runtime',TRUE);
INSERT INTO agent_creation_requests VALUES ('primary','runtime','project','running','owner','kata',NULL,NULL),('relocation','runtime','project','running','owner','kata','{}','host'),('old-owner','runtime','project','running','previous-owner','kata','{}','host');
"""
        schema = "CREATE TEMP TABLE runtime_core_credentials(creation_request_id text,agent_runtime_id text,source_host_id text,source_machine_id text,owner_user_id text,revoked boolean,activated boolean);\n"
        for setup, expected in [
            ("", "schema_absent"),
            (schema, "missing"),
            (schema + "INSERT INTO runtime_core_credentials VALUES ('primary',NULL,'host','machine','owner',FALSE,TRUE);", "assignment_conflict"),
            (schema + "INSERT INTO runtime_core_credentials VALUES ('relocation','runtime','host','machine','owner',FALSE,TRUE);", "bound"),
            (schema + "INSERT INTO runtime_core_credentials VALUES ('primary',NULL,'source-host','machine','owner',TRUE,FALSE),('relocation','runtime','host','machine','owner',FALSE,TRUE);", "bound"),
            (schema + "UPDATE agent_creation_requests SET target_source_host_id='elsewhere' WHERE id='relocation'; INSERT INTO runtime_core_credentials VALUES ('relocation','runtime','host','machine','owner',FALSE,TRUE);", "assignment_conflict"),
            (schema + "INSERT INTO runtime_core_credentials VALUES ('old-owner','runtime','host','machine','owner',FALSE,TRUE);", "assignment_conflict"),
            (schema + "INSERT INTO runtime_core_credentials VALUES ('primary','runtime','host','machine','owner',TRUE,TRUE);", "revoked"),
            (schema + "INSERT INTO runtime_core_credentials VALUES ('primary','runtime','host','machine','owner',FALSE,TRUE);", "bound"),
            (schema + "UPDATE agent_creation_requests SET owner_user_id='previous-owner' WHERE id='primary';", "primary_conflict"),
        ]:
            with self.subTest(expected=expected, setup=setup):
                result = subprocess.run(["psql", "--no-psqlrc", "--csv", "--tuples-only", "--quiet", "--set", "ON_ERROR_STOP=1", "--dbname", os.environ["FC_CORE_POSTGRES_TEST_URL"]], input=fixture + setup + query + "ROLLBACK;", text=True, capture_output=True, check=True)
                self.assertEqual(len(result.stdout.strip().splitlines()), 1)
                values = result.stdout.strip().split(",")
                self.assertEqual(values[-1], expected)
                if expected != "primary_conflict":
                    self.assertEqual(values[4:6], ["2", "1"])

    @unittest.skipUnless(os.environ.get("FC_CORE_POSTGRES_TEST_URL"), "requires disposable Core Postgres")
    def test_completed_creation_query_distinguishes_retired_from_pending(self) -> None:
        with mock.patch.object(finite_status, "run_read_only", return_value=subprocess.CompletedProcess([], 0, "__FINITE_STATUS_RUNTIMES__\n", "")) as run:
            finite_status.psql_query_sets({})
        query = run.call_args.kwargs["input_text"].split("\\echo __FINITE_STATUS_UNROUTABLE_COMPLETED_CREATIONS__\n", 1)[1].split("\\echo __FINITE_STATUS_AGENT_CREATION_REQUESTS__", 1)[0]
        fixture = """
BEGIN;
SET LOCAL search_path=pg_temp;
CREATE TEMP TABLE projects(id text,owner_user_id text,import_candidate_id text);
CREATE TEMP TABLE project_runtime_links(project_id text,agent_runtime_id text,active boolean);
CREATE TEMP TABLE agent_runtimes(id text,host_facts jsonb);
CREATE TEMP TABLE agent_creation_requests(id text,project_id text,display_name text,owner_user_id text,agent_runtime_id text,status text,relocation_spec jsonb,created_at timestamptz);
INSERT INTO projects VALUES ('retired','owner',NULL),('pending','owner',NULL),('healthy','owner',NULL),('import','owner','candidate'),('changed-owner','new-owner',NULL),('relocation','owner',NULL);
INSERT INTO agent_runtimes VALUES ('live','{"runtime_status":"running"}');
INSERT INTO project_runtime_links VALUES ('retired','old',FALSE),('healthy','live',TRUE),('import','live',TRUE),('changed-owner','live',TRUE);
INSERT INTO agent_creation_requests
SELECT id,id,id,'owner',CASE WHEN id='pending' THEN NULL ELSE 'assigned' END,'running',CASE WHEN id='relocation' THEN '{}'::jsonb ELSE NULL END,now() FROM projects;
"""
        result = subprocess.run(["psql", "--no-psqlrc", "--csv", "--tuples-only", "--quiet", "--set", "ON_ERROR_STOP=1", "--dbname", os.environ["FC_CORE_POSTGRES_TEST_URL"]], input=fixture + query + "ROLLBACK;", text=True, capture_output=True, check=True)
        rows = [line.split(",") for line in result.stdout.strip().splitlines()]
        self.assertEqual([row[0] for row in rows], ["changed-owner", "import", "retired"])
        self.assertEqual(rows[-1][-1], "")
        self.assertEqual(rows[0][-1], "running")

    def test_completed_creation_history_is_informational_and_visible(self) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        now = finite_status.parse_time(raw["now"])
        baseline = finite_status.build_report(raw, now)
        rows = [{"id": "retired-request", "project_id": "retired-project", "display_name": "Retired Agent", "agent_runtime_id": "retired-runtime"}]
        raw["core"]["unroutable_completed_creations"] = rows
        report = finite_status.build_report(raw, now)
        self.assertEqual(report["sections"]["fleet_convergence"]["unroutable_completed_creations"], rows)
        self.assertEqual(report["overall_status"], baseline["overall_status"])
        self.assertEqual(report["exit_code"], baseline["exit_code"])
        human = finite_status.render_human(report)
        self.assertIn("Completed launches without a current owner route: 1", human)
        self.assertIn("Retired Agent [retired-request]: project=retired-project; assigned runtime=retired-runtime", human)

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

    def test_reservation_state_distinguishes_pre_release_schema_and_released_host(self) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        raw["core"]["canary_host_reservations"] = [
            {"source_host_id": "finite-lat-5", "launch_code_id": "root"},
            {"source_host_id": "finite-lat-4", "launch_code_id": "other"},
        ]
        now = finite_status.parse_time(raw["now"])
        fleet = finite_status.build_fleet(raw["core"], now)
        self.assertEqual([row["reservation_state"] for row in fleet["canary_host_reservations"]], ["reserved", "reserved"])
        raw["core"]["launch_host_reservation_releases"] = [{"source_host_id": "finite-lat-5"}]
        fleet = finite_status.build_fleet(raw["core"], now)
        self.assertEqual([row["reservation_state"] for row in fleet["canary_host_reservations"]], ["released", "reserved"])
        self.assertEqual(fleet["launch_host_reservation_releases"], raw["core"]["launch_host_reservation_releases"])

    def test_core_queries_share_one_read_only_transaction(self) -> None:
        output = "\n".join(
            [
                "__FINITE_STATUS_HOSTED_ENROLLMENT__",
                "finite-lat-1,runtime-a,artifact-v2,kata,1,1,missing",
                "__FINITE_STATUS_ARTIFACTS__",
                "artifact-v2,ghcr.io/finite/runtime@sha256:2222,v2,git-v2,0.2.0,2026-08-01T00:00:00Z,",
                "__FINITE_STATUS_DISTRIBUTION__",
                "finite-lat-1,v2,1",
                "__FINITE_STATUS_CANARY_HOST_RESERVATIONS__",
                "finite-lat-5,code-canary,batch-canary,workos-operator,org-canary,f,f,request-done,project-done,running,,runner-4,runtime-done,finite-lat-4,,",
                "__FINITE_STATUS_LAUNCH_HOST_RESERVATION_RELEASES__",
                "finite-lat-5,code-canary,runtime-retry,workos-operator,2026-08-01T00:00:00Z",
                "__FINITE_STATUS_UNUSED_SINGLE_CODE_BATCHES__",
                'code-retry,batch-retry,"Retry, canary",workos-operator,standard,2026-08-02T00:00:00Z',
                "__FINITE_STATUS_LAUNCH_CODE_BATCHES__",
                "batch-cohort,Cohort,workos-operator,standard,17,17,0,2026-08-02T00:00:00Z,f",
                "__FINITE_STATUS_UNROUTABLE_COMPLETED_CREATIONS__",
                "retired-request,retired-project,Retired Agent,owner,retired-runtime,owner,,",
                "__FINITE_STATUS_AGENT_CREATION_REQUESTS__",
                "request-canary,project-canary,Lat5 Canary,requested,finite-lat-5,,",
                "__FINITE_STATUS_RUNTIMES__",
                "finite-lat-1,artifact-v2,runtime-a,project-a,machine-a,Agent A,v2,active,restart,launching,online,2026-08-01T13:59:00Z,t,,60",
            ]
        )
        completed = subprocess.CompletedProcess(["psql"], 0, output, "")
        with mock.patch.object(
            finite_status, "run_read_only", return_value=completed
        ) as run:
            result = finite_status.psql_query_sets({})
        self.assertEqual(result["hosted_enrollment"][0]["bootstrap_state"], "missing")
        self.assertEqual(result["hosted_enrollment"][0]["runner_class"], "kata")
        self.assertEqual(result["canary_host_reservations"][0]["batch_id"], "batch-canary")
        self.assertEqual(result["launch_host_reservation_releases"][0]["canary_runtime_id"], "runtime-retry")
        self.assertEqual(result["unused_single_code_batches"][0], {
            "launch_code_id": "code-retry", "batch_id": "batch-retry", "batch_name": "Retry, canary",
            "issuer_workos_user_id": "workos-operator", "hosting_tier": "standard", "expires_at": "2026-08-02T00:00:00Z",
        })
        self.assertEqual(result["launch_code_batches"][0]["actual_count"], "17")
        self.assertEqual(result["launch_code_batches"][0]["redeemed_count"], "0")
        reservation = result["canary_host_reservations"][0]
        self.assertEqual(reservation["retry_of_launch_code_id"], "")
        # Completed misplaced launches remain visible after leaving the queue.
        self.assertEqual(reservation["request_status"], "running")
        self.assertEqual(reservation["request_target_source_host_id"], "")
        self.assertEqual(reservation["actual_source_host_id"], "finite-lat-4")
        self.assertEqual(result["agent_creation_requests"][0]["target_source_host_id"], "finite-lat-5")
        self.assertEqual(result["agent_creation_requests"][0]["status"], "requested")
        completed = result["unroutable_completed_creations"][0]
        self.assertEqual(completed["id"], "retired-request")
        self.assertEqual(completed["agent_runtime_id"], "retired-runtime")
        self.assertEqual(completed["runtime_status"], "")
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
        self.assertIn("to_regclass('launch_host_reservation_releases')", sql)
        self.assertIn("\\if :finite_has_host_releases", sql)
        self.assertIn(finite_status.ARTIFACTS_QUERY, sql)
        self.assertIn(finite_status.DISTRIBUTION_QUERY, sql)
        self.assertIn(finite_status.RUNTIME_DETAILS_QUERY, sql)
        self.assertEqual(call.args[0].count("psql"), 1)

    def executable_probe(self, *, stale=False, pid="123", after_pid=None, continued=False):
        expected = "/nix/store/candidate-core/bin/finite-saas-core"
        system = Path("/nix/store/candidate-system")
        def resolve(path, strict=False):
            if str(path) == "/run/current-system":
                return system
            if str(path).startswith("/proc/"):
                return Path("/nix/store/old-core/bin/finite-saas-core" if stale else expected)
            return path
        command = expected + (" serve " + chr(92) + "\n  --port 8787" if continued else "")
        properties = {"MainPID": pid, "ActiveState": "active"}
        after = {**properties, "MainPID": after_pid or pid}
        with mock.patch.object(Path, "resolve", autospec=True, side_effect=resolve), \
             mock.patch.object(Path, "read_text", return_value=f"[Service]\nExecStart={command}\n"), \
             mock.patch.object(finite_status, "systemd_properties", side_effect=[properties, after]):
            return finite_status.collect_service_executable("finite-saas-core.service")

    def test_active_candidate_executable_is_green(self):
        result = self.executable_probe()
        self.assertEqual(result["status"], "green")
        self.assertEqual(result["running_executable"], result["expected_executable"])

    def test_continued_execstart_reads_only_the_executable(self):
        self.assertEqual(self.executable_probe(continued=True)["status"], "green")

    def test_active_stale_executable_is_red_in_canonical_status(self):
        result = self.executable_probe(stale=True)
        self.assertEqual(result["status"], "red")
        self.assertNotEqual(result["running_executable"], result["expected_executable"])
        raw = finite_status.load_fixture(FIXTURE)
        raw["host_health"]["service_executables"] = [result]
        report = finite_status.build_report(raw, finite_status.parse_time(raw["now"]))
        row = report["sections"]["host_health"]["service_executables"][0]
        self.assertEqual(row["status"], "red")
        self.assertIn("differs", row["error"])

    def test_missing_process_cannot_pass_executable_check(self):
        self.assertEqual(self.executable_probe(pid="0")["status"], "red")

    def test_process_change_during_observation_requires_repeat(self):
        result = self.executable_probe(after_pid="124")
        self.assertEqual(result["status"], "unknown")
        self.assertIn("repeat status", result["error"])

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
                "checks": [
                    {"name": name, "status": "pass", "detail": "qualified", "evidence": {}}
                    for name in ["canonical_handle", "containerd_task", "sandbox_state", "duplicate_writers", "cni_namespace", "vmm_process"]
                ],
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

    def test_target_lifecycle_preserves_findings_and_exit_severity(self) -> None:
        for verdict, status, code in [("operable", "green", 0), ("inoperable", "red", 1), ("unknown", "unknown", 2)]:
            probe = json.loads(self.probe_report(verdict, None if verdict == "operable" else "control_channel_unavailable"))
            if verdict != "operable":
                probe["checks"][1].update(status="fail", detail="bounded timeout", finding="control_channel_unavailable")
            with (
                mock.patch.dict(finite_status.os.environ, {"FINITE_STATUS_LIFECYCLE_PROBE_BIN": "/bin/sh"}),
                mock.patch.object(finite_status.socket, "gethostname", return_value="finite-lat-1"),
                mock.patch.object(finite_status, "read_environment_values", return_value={}),
                mock.patch.object(finite_status, "run_read_only", return_value=subprocess.CompletedProcess([], 0, json.dumps(probe), "")) as run,
            ):
                report = finite_status.collect_runtime_lifecycle("project-a", "runtime-a", "machine-a", "finite-lat-1")
            self.assertEqual(report["overall_status"], status)
            self.assertEqual(report["exit_code"], code)
            self.assertEqual(report["sections"]["runtime_lifecycle"]["probe"]["report"], probe)
            self.assertEqual(run.call_count, 1)
            self.assertEqual(run.call_args.args[0][1], "lifecycle-probe")

    def test_target_lifecycle_rejects_malformed_or_wrong_target_green_reports(self) -> None:
        wrong = json.loads(self.probe_report("operable"))
        wrong["runtime"]["source_machine_id"] = "other-machine"
        contradictory = json.loads(self.probe_report("operable"))
        contradictory["checks"][1].update(status="fail", finding="orphaned_task")
        empty = json.loads(self.probe_report("operable"))
        empty["checks"] = []
        malformed = json.loads(self.probe_report("operable"))
        malformed["checks"][0] = "invalid"
        duplicate = json.loads(self.probe_report("operable"))
        duplicate["checks"].append(duplicate["checks"][0])
        missing = json.loads(self.probe_report("operable"))
        missing["checks"].pop()
        mismatch = json.loads(self.probe_report("inoperable", "orphaned_task"))
        mismatch["checks"][1].update(status="fail", finding="control_channel_unavailable")
        skipped = json.loads(self.probe_report("operable"))
        skipped["checks"][1]["status"] = "skip"
        for probe in [[], wrong, contradictory, empty, malformed, duplicate, missing, mismatch, skipped, {"schema": finite_status.LIFECYCLE_PROBE_SCHEMA, "verdict": "operable"}]:
            with (
                mock.patch.dict(finite_status.os.environ, {"FINITE_STATUS_LIFECYCLE_PROBE_BIN": "/bin/sh"}),
                mock.patch.object(finite_status, "read_environment_values", return_value={}),
                mock.patch.object(finite_status, "run_read_only", return_value=subprocess.CompletedProcess([], 0, json.dumps(probe), "")),
            ):
                raw = finite_status.collect_lifecycle_probe([self.runtime_row("runtime-a")], "finite-lat-1", retain_report=True)
            self.assertEqual(raw["agents"]["runtime-a"]["verdict"], "unknown")
            self.assertEqual(raw["agents"]["runtime-a"]["reason"], "probe_invalid")

    def test_restart_owner_probe_filters_metadata_without_granting_control_authority(self) -> None:
        cid = "a" * 64
        report = {"checks": [{"name": "canonical_handle", "status": "pass",
                               "evidence": {"container_id": cid}}]}
        metadata = {"ID": cid, "Labels": {"containerd.io/restart.policy": "unless-stopped",
                    "containerd.io/restart.count": "12061", "unrelated-secret": "private"}}
        def respond(command, **kwargs):
            output = json.dumps(metadata) if command[0] == "ctr" else "MainPID=0\nActiveState=failed\nEnvironment=private\n"
            return subprocess.CompletedProcess(command, 0, output, "")
        with mock.patch.object(finite_status, "run_read_only", side_effect=respond) as run:
            result = finite_status.collect_runtime_restart_fence(report)
        self.assertEqual(result["status"], "observed")
        self.assertEqual(result["labels"], {"containerd.io/restart.policy": "unless-stopped",
                                         "containerd.io/restart.count": "12061"})
        self.assertEqual(set(result["units"]), {cid + ".service", "nerdctl-" + cid + ".service"})
        self.assertTrue(all(value == {"MainPID": "0", "ActiveState": "failed"} for value in result["units"].values()))
        self.assertFalse(result["repair_authority"])
        self.assertFalse(result["in_flight_controls_absent"])
        self.assertNotIn("private", json.dumps(result))
        self.assertEqual(run.call_count, 3)

    def test_restart_owner_probe_rejects_wrong_container_before_unit_queries(self) -> None:
        cid = "a" * 64
        report = {"checks": [{"name": "canonical_handle", "status": "pass",
                               "evidence": {"container_id": cid}}]}
        with mock.patch.object(finite_status, "run_read_only", return_value=subprocess.CompletedProcess(
                [], 0, json.dumps({"ID": "b" * 64, "Labels": {}}), "")) as run:
            result = finite_status.collect_runtime_restart_fence(report)
        self.assertEqual(result["status"], "unknown")
        self.assertFalse(result["repair_authority"])
        self.assertEqual(run.call_count, 1)

    def test_retained_port_claims_bind_exact_full_container_and_keep_wildcard(self) -> None:
        cid = "a" * 64
        with mock.patch.object(finite_status, "run_read_only", return_value=subprocess.CompletedProcess([], 0, "8080/tcp -> 0.0.0.0:49155\n", "")) as run:
            result = finite_status.collect_runtime_port_claims(cid)
        self.assertEqual(run.call_args.args[0], ["nerdctl", "--namespace", "finite", "port", cid])
        self.assertEqual(result["bindings"], [{"HostIp": "0.0.0.0", "HostPort": "49155", "ContainerPort": 8080, "Protocol": "tcp"}])
        self.assertFalse(result["repair_authority"])

    def test_retained_port_claims_fail_closed_on_malformed_output(self) -> None:
        with mock.patch.object(finite_status, "run_read_only", return_value=subprocess.CompletedProcess([], 0, "8080/tcp -> arbitrary:49155\n", "")):
            self.assertEqual(finite_status.collect_runtime_port_claims("a" * 64)["status"], "unknown")

    def test_cleanup_layout_selects_network_facts_and_omits_secret_hook_arguments(self) -> None:
        cid = "a" * 64
        report = {"checks": [{"name": "canonical_handle", "status": "pass",
                              "evidence": {"container_id": cid}}]}
        persist = {"SandboxContainer": cid, "Network": {"NetworkID": "/run/netns/exact",
            "NetworkCreated": False, "Endpoints": [{"Type": "virtual", "Veth": {"NetPair": {
                "ID": "pair", "Name": "tap0", "NetInterworkingModel": 2,
                "TAPIface": {"Name": "tap0", "HardAddr": "02:00:00:00:00:01", "Addrs": ["private"]},
                "VirtIface": {"Name": "eth0", "HardAddr": "02:00:00:00:00:02"}}}}]}}
        config = {"process": {"env": ["SECRET=private"]}, "linux": {"namespaces": [
            {"type": "network", "path": "/run/netns/exact"}]}, "hooks": {"poststop": [
                {"path": "/nix/store/nerdctl", "args": ["private"], "env": ["private"]}]}}
        with (mock.patch.object(finite_status, "read_environment_values", return_value={}),
              mock.patch.object(finite_status.Path, "open", side_effect=[
                  io.BytesIO(json.dumps(persist).encode()), io.BytesIO(json.dumps(config).encode())])):
            result = finite_status.collect_runtime_cleanup_layout(report)
        self.assertEqual(result["status"], "observed")
        self.assertFalse(result["repair_authority"])
        self.assertFalse(result["network_created"])
        self.assertEqual(result["endpoints"][0]["veth"]["model"], 2)
        self.assertEqual(result["oci_hook_paths"], {"poststop": ["/nix/store/nerdctl"]})
        self.assertNotIn("private", json.dumps(result))

    def test_cleanup_layout_rejects_cross_container_persist_before_oci_read(self) -> None:
        report = {"checks": [{"name": "canonical_handle", "status": "pass",
                              "evidence": {"container_id": "a" * 64}}]}
        with (mock.patch.object(finite_status, "read_environment_values", return_value={}),
              mock.patch.object(finite_status.Path, "open", return_value=io.BytesIO(
                  json.dumps({"SandboxContainer": "b" * 64}).encode())) as read):
            result = finite_status.collect_runtime_cleanup_layout(report)
        self.assertEqual(result["status"], "unknown")
        self.assertFalse(result["repair_authority"])
        self.assertEqual(read.call_count, 1)

    def test_target_lifecycle_cli_rejects_unsafe_identifiers_and_conflicting_modes(self) -> None:
        with contextlib.redirect_stderr(io.StringIO()):
            for arguments in [["--runtime-lifecycle", "project", "runtime", "../machine", "finite-lat-1"],
                              ["--guest-agent-probe"],
                              ["--runtime-assignment", "project", "--guest-agent-probe"],
                              ["--runtime-lifecycle", "project", "runtime", "machine", "finite-lat-1", "--runtime-route", "machine"],
                              ["--runtime-lifecycle", "project", "runtime", "machine"]]:
                with self.assertRaises(SystemExit) as failure:
                    finite_status.parse_args(arguments)
                self.assertEqual(failure.exception.code, 2)

    def test_operable_guest_probe_collects_direct_shim_exit_log_observation(self) -> None:
        probe = json.loads(self.probe_report("operable", None))
        observation = {"status": "observed", "repair_authority": False, "exited_children": []}
        raw = {"agents": {"runtime-a": {"verdict": "operable", "reason": None, "report": probe}}, "errors": []}
        for enabled in (False, True):
            with (
                self.subTest(guest_agent_probe=enabled),
                mock.patch.object(finite_status.socket, "gethostname", return_value="finite-lat-1"),
                mock.patch.object(finite_status, "collect_lifecycle_probe", return_value=raw),
                mock.patch("scripts.kata_guest_probe.collect_guest_agent_probe", return_value={}),
                mock.patch("scripts.kata_network_probe.collect_kata_network_probe", return_value={}),
                mock.patch.object(finite_status, "collect_runtime_shim_exit_log", return_value=observation) as diagnostic,
            ):
                result = finite_status.collect_runtime_lifecycle("project-a", "runtime-a", "machine-a", "finite-lat-1", guest_agent_probe=enabled)
                section = result["sections"]["runtime_lifecycle"]
                self.assertEqual(result["overall_status"], "green")
                self.assertIsNone(section["writer_topology"])
                self.assertEqual(section["shim_exit_log_observation"], observation if enabled else None)
                if enabled:diagnostic.assert_called_once_with(probe)
                else:diagnostic.assert_not_called()

    def test_target_lifecycle_rejects_wrong_host_before_provider_access(self) -> None:
        with (
            mock.patch.object(finite_status.socket, "gethostname", return_value="finite-lat-5"),
            mock.patch.object(finite_status, "run_read_only") as run,
        ):
            with self.assertRaises(finite_status.CollectionError):
                finite_status.collect_runtime_lifecycle("project-a", "runtime-a", "machine-a", "finite-lat-3",
                                                       guest_agent_probe=True)
        run.assert_not_called()

    def test_assignment_contact_requires_host_bound_endpoint_before_network(self) -> None:
        with mock.patch.object(finite_status, "run_read_only") as run:
            for url in ["http://127.0.0.1:49155/contact", "http://10.254.3.5:49155/contact",
                        "http://user@10.254.3.2:49155/contact", "http://10.254.3.2:4200/contact",
                        "http://10.254.3.2:49155/contact?token=private"]:
                result = finite_status.collect_assignment_contact({"source_host_id": "finite-lat-3", "contact_endpoint": url})
                self.assertEqual(result["status"], "unknown")
            run.assert_not_called()

    def test_assignment_contact_reports_principal_mismatch_without_echoing_document(self) -> None:
        principal = "npub1" + "q" * 58
        assignment = {"source_host_id": "finite-lat-3", "contact_endpoint": "http://10.254.3.2:49155/contact",
                      "expected_agent_npub": principal}
        with mock.patch.object(finite_status, "run_read_only", return_value=subprocess.CompletedProcess(
                [], 0, json.dumps({"agent_npub": principal, "unrelated": "private"}), "")):
            result = finite_status.collect_assignment_contact(assignment)
            self.assertTrue(result["matches_core_principal"])
            assignment["expected_agent_npub"] = "another-principal"
            result = finite_status.collect_assignment_contact(assignment)
            self.assertFalse(result["matches_core_principal"])
        self.assertFalse(result["repair_authority"])
        self.assertNotIn("private", json.dumps(result))
        self.assertNotIn(principal, json.dumps(result))

    def test_assignment_contact_malformed_bracket_returns_unknown_without_network(self) -> None:
        with mock.patch.object(finite_status, "run_read_only") as run:
            result = finite_status.collect_assignment_contact({
                "source_host_id": "finite-lat-3", "contact_endpoint": "http://[",
            })
        run.assert_not_called()
        self.assertEqual(result, {
            "status": "unknown", "repair_authority": False,
            "agent_principal_sha256": None, "matches_core_principal": False,
        })

    def test_runtime_assignment_fails_closed_on_ambiguity_and_mismatched_credential(self) -> None:
        row = {"project_id": "project-a", "owner_user_id": "user-a", "owner_link_status": "linked",
               "agent_runtime_id": "runtime-a", "source_host_id": "finite-lat-3",
               "source_machine_id": "machine-a", "expected_agent_npub": "npub-a",
               "runtime_artifact_id": "artifact-a", "image_reference": "image-a",
               "state_schema_version": "runtime-state-v1", "retirement_snapshots": 0,
               "creation_lineage": [{"id": "creation-a", "status": "running", "relocation": False,
                                     "agent_runtime_id": "runtime-a", "owner_user_id": "user-a"}],
               "credentials": [{"agent_runtime_id": "runtime-a", "source_host_id": "finite-lat-3",
                                "source_machine_id": "machine-a", "owner_user_id": "user-a",
                                "activated": True, "revoked": False, "creation_request_id": "creation-a"}]}
        self.assertEqual(finite_status.build_runtime_assignment("project-a", [row])["exit_code"], 0)
        for rows in [[], [row, row]]:
            self.assertEqual(finite_status.build_runtime_assignment("project-a", rows)["exit_code"], 2)
        for override in [{"project_id": "other-project"}, {"active_controls": [{"kind": "upgrade"}]},
                         {"offboarding_phase": "retirement_requested"}, {"credentials": []},
                         {"creation_lineage": [{"status": "launching", "relocation": True}]},
                         {"credentials": [{**row["credentials"][0], "revoked": True}]},
                         {"credentials": [{**row["credentials"][0], "source_machine_id": "other-machine"}]}]:
            self.assertEqual(finite_status.build_runtime_assignment("project-a", [{**row, **override}])["exit_code"], 2)

    def test_runtime_assignment_query_uses_read_only_transaction_and_safe_metadata(self) -> None:
        with (
            mock.patch.object(finite_status, "postgres_environment", return_value={"PGPASSWORD": "test-secret"}),
            mock.patch.object(finite_status, "run_read_only", return_value=subprocess.CompletedProcess([], 0, "[]", "")) as run,
        ):
            self.assertEqual(finite_status.collect_runtime_assignment("project-a")["exit_code"], 2)
        self.assertNotIn("test-secret", str(run.call_args.args))
        query = run.call_args.kwargs["input_text"]
        self.assertIn("BEGIN READ ONLY", query)
        self.assertIn("ROLLBACK", query)
        self.assertNotIn("bootstrap_secret", query)
        self.assertNotIn("token_sha256", query)
        self.assertNotIn("lease_token", query)

    def test_orphan_vmm_observation_is_bounded_and_does_not_authorize_repair(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            container = "a" * 64
            sandbox = root / "sandbox" / container
            process = root / "proc" / "123"
            sandbox.mkdir(parents=True)
            process.mkdir(parents=True)
            persist = {"SandboxContainer": container, "HypervisorState": {"Pid": 123}}
            (sandbox / "persist.json").write_text(json.dumps(persist))
            (process / "stat").write_text("123 (qemu) S " + "0 " * 18 + "333\n")
            (process / "comm").write_text(".qemu-system-x8\n")
            (process / "cmdline").write_bytes(b"qemu\0-name\0sandbox-" + container.encode())
            (process / "exe").symlink_to("/nix/store/example/bin/.qemu-system-x86_64")
            report = {"checks": [{"name": "canonical_handle", "status": "pass", "evidence": {"container_id": container}}]}
            roots = {"FC_RUNNER_KATA_SANDBOX_ROOT": str(root / "sandbox"), "FC_RUNNER_KATA_PROC_ROOT": str(root / "proc")}
            with mock.patch.object(finite_status, "read_environment_values", return_value=roots):
                result = finite_status.collect_orphan_vmm(report)
                self.assertEqual(result["status"], "observed")
                self.assertTrue(result["matches_sandbox"])
                self.assertEqual(result["starttime_ticks"], "333")
                self.assertFalse(result["repair_authority"])
                real_open = Path.open
                stat_reads = 0

                def reused_pid(path, *args, **kwargs):
                    nonlocal stat_reads
                    if path == process / "stat":
                        stat_reads += 1
                        return io.BytesIO(("123 (qemu) S " + "0 " * 18 + str(333 + stat_reads) + "\n").encode())
                    return real_open(path, *args, **kwargs)

                with mock.patch.object(Path, "open", reused_pid):
                    self.assertEqual(finite_status.collect_orphan_vmm(report)["status"], "unknown")
                # A reused PID or unrelated process is never identified as this sandbox.
                (process / "cmdline").write_bytes(b"qemu\0other-sandbox")
                self.assertFalse(finite_status.collect_orphan_vmm(report)["matches_sandbox"])
                self.assertEqual(finite_status.collect_orphan_vmm(report)["status"], "unknown")
                persist["SandboxContainer"] = "b" * 64
                (sandbox / "persist.json").write_text(json.dumps(persist))
                self.assertEqual(finite_status.collect_orphan_vmm(report)["status"], "unknown")
                (sandbox / "persist.json").write_bytes(b" " * (1024 * 1024 + 1))
                self.assertEqual(finite_status.collect_orphan_vmm(report)["status"], "unknown")

    def test_lifecycle_probe_reads_shared_defaults_then_operator_overrides(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            shared = Path(directory) / "shared.env"
            operator = Path(directory) / "operator.env"
            shared.write_text(
                "FC_RUNNER_SOURCE_HOST_ID=finite-lat-5\n"
                "FC_RUNNER_WORK_ROOT=/shared-root\n"
                "UNRELATED_SECRET=must-not-be-forwarded\n"
            )
            operator.write_text("FC_RUNNER_WORK_ROOT=/operator-root\n")
            with (
                mock.patch.dict(finite_status.os.environ, {
                    "FINITE_STATUS_LIFECYCLE_PROBE_BIN": "/bin/sh",
                }, clear=True),
                mock.patch.dict(finite_status.CONTRACT["runner"], {
                    "shared_environment_file": str(shared),
                    "environment_file": str(operator),
                }),
                mock.patch.object(finite_status, "run_read_only", return_value=
                    subprocess.CompletedProcess([], 0, self.probe_report("operable"), "")
                ) as run,
            ):
                raw = finite_status.collect_lifecycle_probe(
                    [self.runtime_row("runtime-a", host="finite-lat-5")], "finite-lat-5"
                )
            self.assertEqual(raw["errors"], [])
            self.assertEqual(raw["agents"]["runtime-a"]["verdict"], "operable")
            environment = run.call_args.kwargs["environment"]
            self.assertEqual(environment["FC_RUNNER_SOURCE_HOST_ID"], "finite-lat-5")
            self.assertEqual(environment["FC_RUNNER_WORK_ROOT"], "/operator-root")
            self.assertNotIn("UNRELATED_SECRET", environment)

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

    def test_stale_health_report_reads_stale_past_three_cadences(self) -> None:
        # 600s old at a 60s cadence is past the 3x deadline: the "died at 3am"
        # runtime stops displaying its frozen last-known ready and names the
        # lapse as `stale`, which leaves the host unknown.
        report = self.report_with_health_group(
            {"health_reported_at": "2026-08-01T13:50:00Z", "health_ready": True}
        )
        host = self.lat9(report)
        self.assertEqual(host["status"], "unknown")
        entry = host["health_unknown"][0]
        self.assertEqual(entry["health"]["status"], "stale")
        self.assertEqual(entry["health"]["age_seconds"], 600)
        output = finite_status.render_human(report)
        self.assertIn(
            "HEALTH-STALE Health Agent 01 [health-agent-01]: no fresh report (last report 10m ago)",
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

    def test_legacy_pending_first_report_latch_projects_unknown(self) -> None:
        # A row the previous Core latched `pending_first_report`, still
        # carrying a fresh ready report from the incarnation before the
        # control (migration 0024 rewrites it on the next Core start). The
        # report must not speak: unknown regardless, and tracked as such.
        report = self.report_with_health_group(
            {
                "runtime_status": "pending_first_report",
                "health_reported_at": "2026-08-01T13:59:30Z",
                "health_ready": True,
            }
        )
        host = self.lat9(report)
        self.assertEqual(host["status"], "unknown")
        self.assertEqual(host["health_tracked_count"], 1)
        entry = host["health_unknown"][0]
        self.assertEqual(entry["health"]["status"], "unknown")
        projected = finite_status.project_runtime_health(
            {
                "runtime_status": "pending_first_report",
                "health_reported_at": "2026-08-01T13:59:30Z",
                "health_ready": True,
            },
            finite_status.parse_time("2026-08-01T14:00:00Z"),
        )
        self.assertEqual(projected["status"], "unknown")

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

    def test_capacity_probe_reports_bytes_and_pressure_without_qualification(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "meminfo").write_text("MemTotal: 1000 kB\nMemAvailable: 700 kB\nSwapTotal: 200 kB\nSwapFree: 190 kB\n")
            (root / "loadavg").write_text("1.00 2.00 3.00 1/20 123")
            (root / "pressure").mkdir()
            for resource in ("cpu", "memory", "io"):
                (root / "pressure" / resource).write_text("some avg10=0.10 avg60=0.20 avg300=0.30 total=100\n")
            result = finite_status.collect_host_capacity(root)
        self.assertEqual(result["memory_bytes"]["MemAvailable"], 700 * 1024)
        self.assertEqual(result["load_average"], [1.0, 2.0, 3.0])
        self.assertEqual(result["pressure"]["memory"]["some"]["avg10"], 0.1)
        self.assertNotIn("qualified_slots", result)

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
            mock.patch.object(finite_status, "collect_host_capacity", return_value={}),
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
    ) -> dict[str, object]:
        return {
            "server": server
            if server is not None
            else {
                "applicable": True,
                "ops_head": 206563,
                "snapshot_watermark": 204801,
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
        }

    def test_watermark_gap_beyond_two_intervals_is_red_with_numbers(self) -> None:
        # The Aug 27-29 freeze class: ops keep being accepted while the
        # durable-state watermark stands still (~8,000 un-snapshotted ops).
        raw = self.chat_raw(
            server={
                "applicable": True,
                "ops_head": 206563,
                "snapshot_watermark": 198000,
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

    def test_watermark_errored_probe_renders_unknown_never_crashes(self) -> None:
        # 2026-09-01 live-run crash: an applicable-but-errored watermark
        # (the sqlite3 CLI missing from the non-login ssh PATH) carries no
        # ops_head; the human render must degrade to unknown-with-reason
        # instead of raising KeyError. Generalized fail-closed contract:
        # missing evidence scores unknown, never green, never a crash.
        raw = self.chat_raw(
            server={
                "applicable": True,
                "database": "/var/lib/private/finite-chat/data/server.sqlite3",
                "errors": [
                    "sqlite3 CLI not found on PATH or under"
                    " /nix/store/*-sqlite-*-bin"
                ],
            }
        )
        report = finite_status.build_chat_plane(
            raw, finite_status.parse_time(raw_sync_since())
        )
        self.assertEqual(report["server_watermark"]["status"], "unknown")
        self.assertEqual(report["status"], "unknown")
        output = finite_status.render_human(
            {
                "generated_at": "2026-09-01T00:00:00Z",
                "exit_code": 2,
                "overall_status": "unknown",
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
        self.assertIn("server watermark [UNKNOWN]", output)
        self.assertIn("sqlite3 CLI not found", output)

    def test_sqlite3_command_falls_back_to_the_nix_store(self) -> None:
        # Production hosts carry sqlite3 only under /nix/store and a
        # non-login ssh session inherits no profile PATH: locate it there
        # (newest first) or fail closed with a clear reason, never crash.
        nix_sqlite = "/nix/store/abc123-sqlite-3.51.2-bin/bin/sqlite3"
        with (
            mock.patch.object(finite_status.shutil, "which", return_value=None),
            mock.patch.object(
                finite_status.glob,
                "glob",
                return_value=[
                    "/nix/store/000-sqlite-3.40.0-bin/bin/sqlite3",
                    nix_sqlite,
                ],
            ) as glob_call,
        ):
            self.assertEqual(finite_status.sqlite3_command(), nix_sqlite)
        glob_call.assert_called_once_with("/nix/store/*-sqlite-*-bin/bin/sqlite3")
        with (
            mock.patch.object(finite_status.shutil, "which", return_value=None),
            mock.patch.object(finite_status.glob, "glob", return_value=[]),
        ):
            with self.assertRaises(finite_status.CollectionError):
                finite_status.sqlite3_command()

    def test_sync_rate_not_applicable_on_runner_role_host(self) -> None:
        # Runner hosts run no chat edge; the probe reads not-applicable
        # instead of UNKNOWN from an unreadable caddy journal (review #802).
        raw = self.chat_raw(
            sync_rate={
                "applicable": False,
                "reason": (
                    "chat edge is not on this runner-role host (roles: runner)"
                ),
            }
        )
        report = finite_status.build_chat_plane(
            raw, finite_status.parse_time(raw_sync_since())
        )
        sync = report["sync_fetch_rate"]
        self.assertEqual(sync["status"], "green")
        self.assertEqual(sync["state"], "not-applicable")
        self.assertIn("runner-role host", sync["reason"])
        output = finite_status.render_human(
            {
                "generated_at": "2026-08-29T23:00:00Z",
                "exit_code": 0,
                "overall_status": "green",
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
        self.assertIn("sync fetch rate: not applicable", output)

    def test_collect_chat_plane_gates_sync_rate_by_host_role(self) -> None:
        now = finite_status.parse_time(raw_sync_since())
        with mock.patch.object(
            finite_status, "collect_sync_fetch_rate"
        ) as sync, mock.patch.object(
            finite_status,
            "collect_chat_server_state",
            return_value={"applicable": False},
        ):
            collected = finite_status.collect_chat_plane("finite-lat-3", now)
        sync.assert_not_called()
        self.assertFalse(collected["sync_rate"]["applicable"])
        self.assertIn("runner-role host", collected["sync_rate"]["reason"])

        with mock.patch.object(
            finite_status, "collect_sync_fetch_rate", return_value={"available": False}
        ) as sync, mock.patch.object(
            finite_status,
            "collect_chat_server_state",
            return_value={"applicable": True},
        ):
            # App-role host (finite-lat-2) and unprofiled hosts keep
            # collecting; the probe itself degrades to unknown with reasons.
            for hostname in ("finite-lat-2", "dev-laptop"):
                collected = finite_status.collect_chat_plane(hostname, now)
                sync.assert_called()
                self.assertNotIn("applicable", collected["sync_rate"])

    def test_collect_chat_server_state_never_opens_the_live_database(self) -> None:
        queries: list[str] = []

        def fake_int(database, sql, timeout=15):
            queries.append(sql)
            if "http_delivery_ops" in sql:
                return 206563
            if "http_state_snapshots_v2" in sql:
                return 204801
            raise AssertionError(f"unexpected int query {sql}")

        with (
            mock.patch.object(finite_status.Path, "exists", return_value=True),
            mock.patch.object(finite_status, "scratch_copy_sqlite") as scratch,
            mock.patch.object(finite_status, "sqlite_int_query", side_effect=fake_int),
        ):
            scratch.return_value.__enter__ = mock.MagicMock(
                return_value=Path("/tmp/scratch/scratch.sqlite3")
            )
            scratch.return_value.__exit__ = mock.MagicMock(return_value=False)
            raw = finite_status.collect_chat_server_state("finite-lat-2")

        self.assertTrue(raw["applicable"])
        self.assertEqual(raw["ops_head"], 206563)
        self.assertEqual(raw["snapshot_watermark"], 204801)
        self.assertEqual(raw["errors"], [])
        scratch.assert_called_once()
        live = scratch.call_args.args[0]
        self.assertEqual(
            str(live), finite_status.CONTRACT["chat_plane"]["server_database"]
        )
        self.assertTrue(all("http_" in sql for sql in queries))

    def test_collect_chat_server_state_marks_missing_database(self) -> None:
        # Applicability is a role fact, not a filesystem fact: only runner-
        # role hosts read not-applicable (review #802, round two).
        with mock.patch.object(finite_status.Path, "exists", return_value=False):
            raw = finite_status.collect_chat_server_state("finite-lat-3")
        self.assertFalse(raw["applicable"])
        self.assertIn("runner-role host", raw["reason"])

        # On the app-role host a missing database is an evidence failure:
        # the probe stays applicable and fails closed to unknown.
        with mock.patch.object(finite_status.Path, "exists", return_value=False):
            raw = finite_status.collect_chat_server_state("finite-lat-2")
        self.assertTrue(raw["applicable"])
        self.assertEqual(len(raw["errors"]), 1)
        self.assertIn("chat server database not present", raw["errors"][0])
        self.assertIn(
            finite_status.CONTRACT["chat_plane"]["server_database"], raw["errors"][0]
        )

        # Unprofiled hosts (dev machines) fail closed the same way.
        with mock.patch.object(finite_status.Path, "exists", return_value=False):
            raw = finite_status.collect_chat_server_state("dev-laptop")
        self.assertTrue(raw["applicable"])
        self.assertEqual(len(raw["errors"]), 1)

    def test_watermark_missing_database_on_app_host_reads_unknown(self) -> None:
        # finite-lat-2 with a missing/moved/unreadable server database must
        # score unknown with a reason, never green/not-applicable.
        database = finite_status.CONTRACT["chat_plane"]["server_database"]
        raw = self.chat_raw(
            server={
                "applicable": True,
                "database": database,
                "errors": [f"chat server database not present on this host: {database}"],
            }
        )
        report = finite_status.build_chat_plane(
            raw, finite_status.parse_time(raw_sync_since())
        )
        watermark = report["server_watermark"]
        self.assertEqual(watermark["status"], "unknown")
        self.assertNotEqual(watermark.get("state"), "not-applicable")
        self.assertIn("not present", watermark["errors"][0])
        self.assertEqual(report["status"], "unknown")

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
            command[1:6],
            ["-safe", "-readonly", "-batch", "-init", "/dev/null"],
        )
        self.assertEqual(command[0], finite_status.sqlite3_command())
        self.assertNotIn(
            str(finite_status.CONTRACT["chat_plane"]["server_database"]), command
        )

    def test_runner_host_health_is_not_scored_against_app_services(self) -> None:
        raw = finite_status.load_fixture(FIXTURE)
        raw["host_health"]["hostname"] = "finite-lat-3"
        raw["host_health"]["roles"] = ["runner"]
        raw["host_health"]["hosted_hermes"] = {"status": "green", "state": "not-configured"}
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

class HostedHermesStatusTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.now = finite_status.utc_now()
        self.unit = {"LoadState": "loaded", "MainPID": "42", "ActiveState": "active", "InvocationID": "a" * 32}
        self.record = {"version": 1, "state": "serving", "checkedAt": int(self.now.timestamp()),
                       "proxyPid": "42", "proxyInvocation": "a" * 32}

    def collect(self, states=None):
        (self.root / "status.json").write_text(json.dumps(self.record))
        with mock.patch.object(finite_status, "systemd_properties", side_effect=states or [self.unit, self.unit]):
            return finite_status.collect_hosted_hermes(self.now, self.root)

    def test_disabled_empty_and_serving_are_distinct(self):
        self.assertEqual(self.collect()["state"], "serving")
        self.unit.update(MainPID="0", ActiveState="inactive", InvocationID="")
        self.record.update(state="empty", proxyPid="0", proxyInvocation="")
        self.assertEqual(self.collect()["state"], "no-eligible-routes")
        self.assertEqual(self.collect()["status"], "green")
        self.unit["LoadState"] = "not-found"
        result = self.collect()
        self.assertEqual((result["status"], result["state"]), ("green", "not-configured"))

    def test_failure_or_stale_evidence_never_scores_green(self):
        self.record["state"] = "failed"
        self.assertEqual(self.collect()["status"], "red")
        for state in ("reconciling", "unknown"):
            self.record["state"] = state
            self.assertEqual(self.collect()["status"], "unknown")
        self.record["state"] = "serving"
        for delta in (-120, 120):
            self.record["checkedAt"] = int(self.now.timestamp()) + delta
            self.assertEqual(self.collect()["state"], "stale")

    def test_restarted_or_dead_proxy_invalidates_success(self):
        self.unit["InvocationID"] = "b" * 32
        self.assertEqual(self.collect()["state"], "process-changed")
        self.unit["InvocationID"] = "a" * 32
        self.unit["ActiveState"] = "failed"
        self.assertEqual(self.collect()["status"], "red")
        self.record["state"] = "empty"
        self.assertEqual(self.collect()["status"], "red")

    def test_surviving_mutation_and_collection_race_are_unknown(self):
        marker = self.root / "mutation-in-progress"
        marker.touch()
        self.assertEqual(self.collect()["state"], "mutation-unresolved")
        marker.unlink()
        self.assertEqual(self.collect([self.unit, {**self.unit, "MainPID": "43"}])["status"], "unknown")
        calls = 0
        def mutate(_unit):
            nonlocal calls
            calls += 1
            if calls == 2:
                marker.touch()
            return self.unit
        self.assertEqual(self.collect(mutate)["status"], "unknown")

    def test_missing_malformed_or_unreadable_evidence_is_unknown(self):
        with mock.patch.object(finite_status, "systemd_properties", return_value=self.unit):
            self.assertEqual(finite_status.collect_hosted_hermes(self.now, self.root)["status"], "unknown")
            for encoded in ("not-json", "[]", "{}", '"' + "a" * 4097 + '"',
                            json.dumps({**self.record, "checkedAt": 10 ** 350}),
                            "[" * 1500 + "]" * 1500):
                (self.root / "status.json").write_text(encoded)
                self.assertEqual(finite_status.collect_hosted_hermes(self.now, self.root)["status"], "unknown")
            with mock.patch.object(Path, "stat", side_effect=PermissionError()):
                self.assertEqual(finite_status.collect_hosted_hermes(self.now, self.root)["status"], "unknown")

    def test_runner_report_and_text_include_ingress_and_old_evidence_is_unknown(self):
        raw = finite_status.load_fixture(FIXTURE)
        raw["host_health"]["roles"] = ["runner"]
        raw["host_health"]["hosted_hermes"] = {"status": "red", "state": "reconciliation-failed"}
        now = finite_status.parse_time(raw["now"])
        report = finite_status.build_report(raw, now)
        self.assertEqual(report["sections"]["host_health"]["hosted_hermes"]["status"], "red")
        self.assertIn("hosted Hermes:", finite_status.render_human(report))
        self.assertIn("reconciliation-failed", finite_status.render_human(report))
        del raw["host_health"]["hosted_hermes"]
        report = finite_status.build_report(raw, now)
        self.assertEqual(report["sections"]["host_health"]["hosted_hermes"]["state"], "not-observed")


class FinitePrivateUsageStatusTests(unittest.TestCase):
    def test_probe_is_one_read_only_transaction_and_prints_json(self):
        usage = {"profiles": [{"id": "finite-private-generous-v2", "burst_limit_units": 200000000}]}
        completed = subprocess.CompletedProcess([], 0, json.dumps(usage) + "\n", "")
        stdout = io.StringIO()
        with mock.patch.object(finite_status, "postgres_environment", return_value={}), \
                mock.patch.object(finite_status, "run_read_only", return_value=completed) as run, \
                contextlib.redirect_stdout(stdout), \
                self.assertRaises(SystemExit) as exit:
            finite_status.main(["--finite-private-usage"])
        sql = run.call_args.kwargs["input_text"]
        self.assertTrue(sql.startswith("BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;\n"))
        self.assertTrue(sql.endswith("ROLLBACK;\n"))
        self.assertEqual(exit.exception.code, 0)
        report = json.loads(stdout.getvalue())
        self.assertEqual(report["schema_version"], "finite.private-usage-status.v1")
        self.assertEqual(report["usage"], usage)

    def test_query_failure_or_invalid_json_is_a_collection_error(self):
        for completed in (
            subprocess.CompletedProcess([], 1, "", "ERROR: relation does not exist\n"),
            subprocess.CompletedProcess([], 0, "not json", ""),
        ):
            with self.subTest(returncode=completed.returncode), \
                    mock.patch.object(finite_status, "postgres_environment", return_value={}), \
                    mock.patch.object(finite_status, "run_read_only", return_value=completed), \
                    self.assertRaises(finite_status.CollectionError):
                finite_status.collect_finite_private_usage()

    @unittest.skipUnless(os.environ.get("FC_CORE_POSTGRES_TEST_URL"), "requires disposable Core Postgres")
    def test_query_is_profile_relative_and_sees_the_200m_floor(self):
        migration = (
            ROOT / "finitecomputer-v2/crates/finite-saas-core/migrations/0033_finite_private_200m_default.sql"
        ).read_text()
        # Temp tables shadow any real Core tables; the rollback discards the
        # migration's function and trigger too.
        fixture = """
BEGIN;
CREATE TEMP TABLE finite_private_limit_profiles(id text PRIMARY KEY,burst_window_seconds bigint,burst_limit_units bigint,weekly_limit_units bigint,created_at timestamptz,updated_at timestamptz);
CREATE TEMP TABLE finite_private_grants(id text,limit_profile_id text,status text,burst_window_epoch bigint,current_window_started_at timestamptz,current_window_used_units bigint);
CREATE TEMP TABLE finite_private_reservations(grant_id text,burst_window_epoch bigint,model text,usage_formula_version text,status text,settlement_kind text,upstream_status integer,upstream_error_class text,reserved_usage_units bigint,settled_usage_units bigint,created_at timestamptz);
INSERT INTO finite_private_limit_profiles VALUES
  ('finite-private-generous-v2',18000,100000000,NULL,now(),now()),
  ('finite-private-generous-5x-v1',18000,500000000,NULL,now(),now());
"""
        seed = """
INSERT INTO finite_private_grants VALUES
  ('near','finite-private-generous-v2','active',3,now()-interval '1 hour',190000000),
  ('drift','finite-private-generous-v2','active',1,now()-interval '10 minutes',5000000),
  ('extended','finite-private-generous-5x-v1','active',2,now()-interval '1 hour',190000000),
  ('expired','finite-private-generous-v2','active',1,now()-interval '6 hours',199000000),
  ('revoked','finite-private-generous-v2','revoked',1,now()-interval '1 hour',0);
INSERT INTO finite_private_reservations VALUES
  ('near',3,'glm','v1','settled','actual',200,NULL,160000000,150000000,now()-interval '50 minutes'),
  ('near',3,'glm','v1','reserved',NULL,NULL,NULL,40000000,NULL,now()-interval '20 minutes'),
  ('near',3,'glm','v1','denied',NULL,NULL,NULL,999,NULL,now()-interval '5 minutes'),
  ('drift',1,'glm','v1','settled','actual',200,NULL,4000000,4000000,now()-interval '5 minutes'),
  ('extended',2,'glm','v1','settled','estimate',502,'upstream_http',190000000,190000000,now()-interval '30 minutes'),
  ('expired',1,'glm','v1','settled','actual',200,NULL,199000000,199000000,now()-interval '6 hours');
"""
        result = subprocess.run(
            ["psql", "--no-psqlrc", "--tuples-only", "--no-align", "--quiet", "--set", "ON_ERROR_STOP=1",
             "--dbname", os.environ["FC_CORE_POSTGRES_TEST_URL"]],
            input=fixture + migration + seed + finite_status.FINITE_PRIVATE_USAGE_QUERY + "ROLLBACK;\n",
            text=True, capture_output=True, check=True,
        )
        usage = json.loads(result.stdout)
        self.assertEqual(
            [(p["id"], p["burst_limit_units"], p["active_grants"]) for p in usage["profiles"]],
            [("finite-private-generous-5x-v1", 500000000, 1), ("finite-private-generous-v2", 200000000, 3)],
        )
        self.assertEqual(usage["limit_profile_triggers"], ["preserve_finite_private_200m_default"])
        self.assertEqual(
            usage["current_windows"], {"active_windows": 3, "at_least_90_percent": 1, "at_or_over_limit": 0}
        )
        self.assertEqual(usage["current_counter_mismatches"], 1)
        self.assertEqual(
            usage["reserved_over_15_minutes_in_current_windows"], {"requests": 1, "held_units": 40000000}
        )
        outcomes = {
            (row["status"], row["upstream_status"]): row["charged_or_held_units"]
            for row in usage["last_7_days_by_outcome"]
        }
        self.assertEqual(outcomes[("settled", 502)], 190000000)
        self.assertEqual(outcomes[("reserved", None)], 40000000)
        self.assertEqual(
            {row["limit_profile_id"]: (row["observed_epochs"], row["at_least_90_percent"], row["at_least_98_percent"])
             for row in usage["last_7_days_epochs_by_current_profile"]},
            {"finite-private-generous-5x-v1": (1, 0, 0), "finite-private-generous-v2": (3, 2, 1)},
        )

from __future__ import annotations

import contextlib
from datetime import datetime, timezone
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from scripts import finite_status
from scripts import finite_status_runtime_idle as idle

NOW_MS = 1_790_000_000_000
SECRET = "PRIVATE_CHAT_TEXT_room_secret_msg_secret_lease_secret_nsec1secret"
CHECKS = ["canonical_handle", "containerd_task", "sandbox_state",
          "duplicate_writers", "cni_namespace", "vmm_process"]


def event(lease: dict | None = None, created_ms: int = NOW_MS - 30_000) -> dict:
    item = {"key": SECRET, "room_id": SECRET, "seq": 1, "message_id": SECRET,
            "created_at_ms": created_ms, "event": {"text": SECRET}}
    if lease is not None:
        item["lease"] = lease
    return item


class AgentRoot:
    """A synthetic `<work_root>/kata/<runtime>` durable root."""

    def __init__(self, base: Path, runtime: str = "runtime-a") -> None:
        self.root = base / "kata" / runtime
        self.agent = self.root / "agent"
        (self.agent / "hermes-home").mkdir(parents=True)
        self.gateway()
        self.write("hermes-inbox.json", {"events": [], "cursors": {SECRET: 4},
                                         "acked": [{"key": SECRET, "acked_at_ms": 1}]})

    def write(self, name: str, document: object) -> Path:
        path = self.agent / name
        path.write_text(document if isinstance(document, str) else json.dumps(document))
        return path

    def gateway(self, state: object = "running", active: object = 0, **extra: object) -> None:
        self.write("hermes-home/gateway_state.json",
                   {"pid": 7, "gateway_state": state, "active_agents": active,
                    "updated_at": datetime.fromtimestamp(NOW_MS / 1000 - 600, timezone.utc).isoformat(), "platforms": {SECRET: {}}, **extra})


class ObserveTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.home = AgentRoot(Path(self.temporary.name).resolve())

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def observe(self) -> dict:
        result = idle.observe(self.home.root, NOW_MS)
        self.assertNotIn(SECRET, json.dumps(result))
        return result

    def test_running_gateway_with_absent_agentd_and_marker_files_is_idle(self) -> None:
        result = self.observe()
        self.assertEqual((result["verdict"], result["reasons"]), ("idle", []))
        self.assertEqual(result["gateway"], {"state": "running", "active_agents": 0, "updated_age_s": 600})
        self.assertEqual(result["agentd_inbox"], {"present": False, "events": 0})
        self.assertEqual(result["running_markers"], {"present": False, "messages": 0})
        (self.home.agent / "hermes-inbox.json").unlink()
        self.assertEqual(self.observe()["verdict"], "idle")

    def test_each_turn_signal_is_busy_with_counts_and_ages_only(self) -> None:
        cases = {
            "active_agents": lambda: self.home.gateway(active=2),
            "inbox_pending": lambda: self.home.write("hermes-inbox.json", {"events": [
                event(), event({"state": "pending"}, created_ms=NOW_MS - 90_000)]}),
            "inbox_leased": lambda: self.home.write("hermes-inbox.json", {"events": [
                event({"state": "leased", "lease_id": SECRET, "leased_at_ms": NOW_MS - 840_000}),
                event({"state": "leased", "lease_id": SECRET, "leased_at_ms": NOW_MS - 5_000})]}),
            "agentd_inbox": lambda: self.home.write("agentd-inbox.json", {"events": [
                {"key": SECRET, "room_id": SECRET, "seq": 2, "message_id": SECRET, "delivery": {}}]}),
        }
        for reason, arrange in cases.items():
            with self.subTest(reason):
                self.tearDown(); self.setUp()
                arrange()
                result = self.observe()
                self.assertEqual((result["verdict"], result["reasons"]), ("busy", [reason]))
        self.assertEqual(result["agentd_inbox"], {"present": True, "events": 1})
        self.tearDown(); self.setUp()
        cases["inbox_leased"]()
        self.assertEqual(self.observe()["hermes_inbox"], {
            "present": True, "pending": 0, "leased": 2, "oldest_pending_age_s": None,
            "oldest_lease_age_s": 840, "newest_lease_age_s": 5})
        cases["inbox_pending"]()
        self.assertEqual(self.observe()["hermes_inbox"]["oldest_pending_age_s"], 90)

    def test_leftover_running_markers_are_reported_separately_from_busy(self) -> None:
        self.home.write("hermes-running.json", {"messages": [
            {"room_id": SECRET, "conversation_id": None, "message_id": SECRET}] * 3})
        result = self.observe()
        self.assertEqual((result["verdict"], result["reasons"]), ("unfinished_markers", ["running_markers"]))
        self.assertEqual(result["running_markers"], {"present": True, "messages": 3})
        self.home.gateway(active=1)
        self.assertEqual(self.observe()["reasons"], ["active_agents", "running_markers"])

    def test_missing_or_non_running_gateway_is_never_idle(self) -> None:
        (self.home.agent / "hermes-home" / "gateway_state.json").unlink()
        self.assertEqual(self.observe()["reasons"], ["gateway_missing"])
        for state in ("starting", "draining", "stopped", "degraded", "startup_failed", "new_state"):
            with self.subTest(state):
                self.home.gateway(state=state)
                result = self.observe()
                self.assertEqual((result["verdict"], result["reasons"]), ("unknown", ["gateway_not_running"]))
        self.assertEqual(result["gateway"]["state"], "unrecognized")

    def test_empty_objects_load_as_empty_but_null_or_mistyped_fields_fail(self) -> None:
        # The finitechat serde loaders default absent fields and ignore unknown
        # ones, but reject a JSON null document or a present null field.
        for name in ("hermes-inbox.json", "agentd-inbox.json", "hermes-running.json"):
            self.home.write(name, {"future_field": SECRET})
        self.assertEqual(self.observe()["verdict"], "idle")
        cases = [
            ("hermes_inbox_malformed", "hermes-inbox.json", "null"),
            ("agentd_inbox_malformed", "agentd-inbox.json", "null"),
            ("running_markers_malformed", "hermes-running.json", "null"),
            ("hermes_inbox_malformed", "hermes-inbox.json", {"events": None}),
            ("hermes_inbox_malformed", "hermes-inbox.json", {"events": [], "cursors": None}),
            ("hermes_inbox_malformed", "hermes-inbox.json", {"events": [], "cursors": {"r": -1}}),
            ("hermes_inbox_malformed", "hermes-inbox.json", {"events": [], "acked": None}),
            ("hermes_inbox_malformed", "hermes-inbox.json", {"events": [], "acked": [{"key": "k"}]}),
            ("agentd_inbox_malformed", "agentd-inbox.json", {"events": [], "cursors": []}),
            ("running_markers_malformed", "hermes-running.json", {"messages": [], "x": 1} | {"messages": None}),
        ]
        for reason, name, document in cases:
            with self.subTest(reason=reason, document=document):
                self.tearDown(); self.setUp()
                self.home.write(name, document)
                result = self.observe()
                self.assertEqual((result["verdict"], result["reasons"]), ("unknown", [reason]))
        self.tearDown(); self.setUp()
        self.home.write("hermes-home/gateway_state.json", "null")
        self.assertEqual(self.observe()["reasons"], ["gateway_malformed"])

    def test_malformed_state_and_unknown_leases_fail_unknown(self) -> None:
        cases = [
            ("gateway_malformed", lambda: self.home.gateway(active="0")),
            ("gateway_malformed", lambda: self.home.gateway(active=False)),
            ("gateway_malformed", lambda: self.home.gateway(active=-1)),
            ("gateway_malformed", lambda: self.home.gateway(active=None)),
            ("gateway_malformed", lambda: self.home.gateway(state=None)),
            ("gateway_malformed", lambda: self.home.write("hermes-home/gateway_state.json", "{")),
            ("gateway_malformed", lambda: self.home.write("hermes-home/gateway_state.json", "")),
            ("hermes_inbox_unknown_lease", lambda: self.home.write(
                "hermes-inbox.json", {"events": [event({"state": "expired"})]})),
            ("hermes_inbox_unknown_lease", lambda: self.home.write(
                "hermes-inbox.json", {"events": [event({"state": "leased", "leased_at_ms": 1})]})),
            ("hermes_inbox_malformed", lambda: self.home.write(
                "hermes-inbox.json", {"events": [event({"state": "leased", "lease_id": "x", "leased_at_ms": "1"})]})),
            ("hermes_inbox_malformed", lambda: self.home.write("hermes-inbox.json", {"events": [event(None) | {"lease": None}]})),
            ("hermes_inbox_malformed", lambda: self.home.write("hermes-inbox.json", {"events": {}})),
            ("hermes_inbox_malformed", lambda: self.home.write("hermes-inbox.json", {"events": [7]})),
            ("agentd_inbox_malformed", lambda: self.home.write("agentd-inbox.json", [])),
            ("agentd_inbox_malformed", lambda: self.home.write("agentd-inbox.json", "0")),
            ("running_markers_malformed", lambda: self.home.write("hermes-running.json", {"messages": None})),
            ("running_markers_malformed", lambda: self.home.write("hermes-running.json", b"\xff".decode("latin-1"))),
        ]
        for reason, arrange in cases:
            with self.subTest(reason):
                self.tearDown(); self.setUp()
                arrange()
                result = self.observe()
                self.assertEqual((result["verdict"], result["reasons"]), ("unknown", [reason]))

    def test_unknown_keeps_observed_busy_signals(self) -> None:
        self.home.gateway(active=3)
        self.home.write("agentd-inbox.json", "not json")
        result = self.observe()
        self.assertEqual((result["verdict"], result["reasons"]), ("unknown", ["agentd_inbox_malformed", "active_agents"]))

    def test_symlinks_special_files_oversize_and_missing_roots_fail_unknown(self) -> None:
        outside = Path(self.temporary.name).resolve() / "outside.json"
        outside.write_text(json.dumps({"events": []}))
        cases = [
            ("gateway_symlink", lambda: self._replace_with_symlink("hermes-home/gateway_state.json", outside)),
            ("hermes_inbox_symlink", lambda: self._replace_with_symlink("hermes-inbox.json", outside)),
            ("agentd_inbox_symlink", lambda: (self.home.agent / "agentd-inbox.json").symlink_to(outside)),
            ("running_markers_symlink", lambda: (self.home.agent / "hermes-running.json").symlink_to(
                Path(self.temporary.name) / "dangling")),
            ("agentd_inbox_not_regular", lambda: os.mkfifo(self.home.agent / "agentd-inbox.json")),
            ("running_markers_not_regular", lambda: (self.home.agent / "hermes-running.json").mkdir()),
            ("agent_root_symlink", lambda: self._replace_with_symlink("hermes-home", outside.parent)),
            ("agent_root_missing", lambda: os.rename(self.home.agent, self.home.root / "moved")),
        ]
        for reason, arrange in cases:
            with self.subTest(reason):
                self.tearDown(); self.setUp()
                outside.parent.mkdir(exist_ok=True); outside.write_text(json.dumps({"events": []}))
                arrange()
                result = self.observe()
                self.assertEqual((result["verdict"], result["reasons"]), ("unknown", [reason]))
        self.tearDown(); self.setUp()
        with mock.patch.object(idle, "MAX_STATE_BYTES", 64):
            self.home.write("hermes-inbox.json", {"events": [], "pad": "x" * 64})
            self.assertEqual(self.observe()["reasons"], ["gateway_oversize", "hermes_inbox_oversize"])

    @unittest.skipIf(os.geteuid() == 0, "root bypasses file modes")
    def test_permission_denied_is_unknown(self) -> None:
        path = self.home.agent / "hermes-home" / "gateway_state.json"
        path.chmod(0)
        try:
            self.assertEqual(self.observe()["reasons"], ["gateway_unreadable"])
        finally:
            path.chmod(0o600)

    def _replace_with_symlink(self, relative: str, target: Path) -> None:
        path = self.home.agent / relative
        if path.is_dir():
            for child in path.iterdir():
                child.unlink()
            path.rmdir()
        else:
            path.unlink()
        path.symlink_to(target)


class TargetTests(unittest.TestCase):
    """The exact assignment is proven by the lifecycle probe before any read."""

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.home = AgentRoot(Path(self.temporary.name).resolve())
        self.home.gateway(active=1)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def probe(self, verdict: str = "operable", state_root: object = None, machine: str = "machine-a") -> dict:
        checks = [{"name": name, "status": "pass", "detail": "qualified", "evidence": {}} for name in CHECKS]
        checks[0]["evidence"] = {"container_id": "a" * 64, "container_name": machine,
                                 "state_root": str(self.home.root) if state_root is None else state_root}
        if verdict != "operable":
            checks[1].update(status="fail", finding="control_channel_unavailable")
        return {"schema": "finite.lifecycle-probe.v1", "verdict": verdict,
                "reason": None if verdict == "operable" else "control_channel_unavailable",
                "runtime": {"project_id": "project-a", "agent_runtime_id": "runtime-a",
                            "source_machine_id": machine, "container_name": machine},
                "checks": checks}

    def run_idle(self, probe: dict, host: str = "finite-lat-3") -> dict:
        with (
            mock.patch.dict(finite_status.os.environ, {"FINITE_STATUS_LIFECYCLE_PROBE_BIN": "/bin/sh"}),
            mock.patch.object(finite_status.socket, "gethostname", return_value="finite-lat-3"),
            mock.patch.object(finite_status, "read_environment_values", return_value={}),
            mock.patch.object(finite_status, "run_read_only",
                              return_value=subprocess.CompletedProcess([], 0, json.dumps(probe), "")) as run,
        ):
            report = finite_status.collect_runtime_idle("project-a", "runtime-a", "machine-a", host)
        command = run.call_args.args[0]
        self.assertEqual(command[1:], ["lifecycle-probe", "--project-id", "project-a",
                                       "--agent-runtime-id", "runtime-a", "--source-machine-id", "machine-a"])
        self.assertNotIn(SECRET, json.dumps(report))
        return report

    def test_operable_exact_assignment_reads_its_durable_root(self) -> None:
        report = self.run_idle(self.probe())
        section = report["sections"]["runtime_idle"]
        self.assertEqual((report["overall_status"], report["exit_code"]), ("red", 1))
        self.assertEqual((section["verdict"], section["reasons"]), ("busy", ["active_agents"]))
        self.assertEqual(section["lifecycle"], {"verdict": "operable", "reason": None})
        self.home.gateway(active=0)
        report = self.run_idle(self.probe())
        self.assertEqual((report["overall_status"], report["exit_code"]), ("green", 0))
        self.assertEqual(set(report["sections"]["runtime_idle"]), {
            "status", "project_id", "agent_runtime_id", "source_machine_id", "source_host_id", "lifecycle",
            "verdict", "reasons", "gateway", "hermes_inbox", "agentd_inbox", "running_markers"})

    def test_wrong_target_or_unproven_root_is_unknown_without_reading_state(self) -> None:
        other = AgentRoot(Path(self.temporary.name).resolve() / "other", "runtime-b")
        link = Path(self.temporary.name).resolve() / "linked" / "kata" / "runtime-a"
        link.parent.mkdir(parents=True)
        link.symlink_to(self.home.root)
        cases = [
            (self.probe(machine="machine-b"), "lifecycle_not_operable"),
            (self.probe("degraded"), "lifecycle_not_operable"),
            (self.probe(state_root=str(other.root)), "state_root_mismatch"),
            (self.probe(state_root=str(self.home.agent)), "state_root_mismatch"),
            (self.probe(state_root="kata/runtime-a"), "state_root_mismatch"),
            (self.probe(state_root=str(link)), "state_root_mismatch"),
            (self.probe(state_root=str(self.home.root / ".." / "runtime-a")), "state_root_mismatch"),
            (self.probe(state_root=None) | {"checks": []}, "lifecycle_not_operable"),
        ]
        for probe, reason in cases:
            with self.subTest(reason), mock.patch.object(idle, "observe") as observe:
                report = self.run_idle(probe)
                observe.assert_not_called()
                section = report["sections"]["runtime_idle"]
                self.assertEqual((report["exit_code"], section["verdict"], section["reasons"]),
                                 (2, "unknown", [reason]))

    def test_wrong_host_refuses_before_provider_access(self) -> None:
        with (
            mock.patch.object(finite_status.socket, "gethostname", return_value="finite-lat-4"),
            mock.patch.object(finite_status, "run_read_only") as run,
        ):
            with self.assertRaises(finite_status.CollectionError):
                finite_status.collect_runtime_idle("project-a", "runtime-a", "machine-a", "finite-lat-3")
        run.assert_not_called()

    def test_cli_emits_json_and_requires_four_simple_identifiers(self) -> None:
        stdout = io.StringIO()
        with (
            mock.patch.object(finite_status, "collect_runtime_idle",
                              return_value={"exit_code": 0, "sections": {}}) as collect,
            contextlib.redirect_stdout(stdout),
            self.assertRaises(SystemExit) as exit_,
        ):
            finite_status.main(["--runtime-idle", "project-a", "runtime-a", "machine-a", "finite-lat-3"])
        collect.assert_called_once_with("project-a", "runtime-a", "machine-a", "finite-lat-3")
        self.assertEqual((exit_.exception.code, json.loads(stdout.getvalue())["exit_code"]), (0, 0))
        for arguments in (["project-a", "../runtime", "machine-a", "finite-lat-3"],
                          ["project-a", "runtime-a", "machine-a"]):
            with self.subTest(arguments), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                finite_status.parse_args(["--runtime-idle", *arguments])
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            finite_status.parse_args(["--runtime-idle", "p", "r", "m", "h", "--runtime-lifecycle", "p", "r", "m", "h"])


if __name__ == "__main__":
    unittest.main()

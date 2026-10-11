"""Unit checks for the Agent Runtime's narrow health/contact server."""

from __future__ import annotations

import importlib.util
import json
import os
import sys
import tempfile
import time
import unittest
from pathlib import Path
from types import ModuleType

REPO_ROOT = Path(__file__).resolve().parents[2]
HEALTH_SERVER = REPO_ROOT / "containers" / "agent" / "health_server.py"
GATEWAY = REPO_ROOT / "containers" / "agent" / "run_hermes_gateway.sh"

FAKE_FINITECHAT = """#!/usr/bin/env python3
import json
import sys

if sys.argv[1:] == ["auth", "status"]:
    print(json.dumps({"npub": "npub1agent", "account_id": "a" * 64}))
    sys.exit(0)
sys.exit(2)
"""


UPDATED_AT_MS = 1_760_000_000_000


def pipe_observation(buffered_bytes: int) -> dict[str, object]:
    return {
        "kind": "pipe",
        "observed_at_ms": UPDATED_AT_MS - 1000,
        "buffered_bytes": buffered_bytes,
        "capacity_bytes": 65536,
        "first_observed_at_ms": UPDATED_AT_MS - 60_000,
        "sample_count": 60,
        "observed_decrease_bytes": 0,
    }


class AgentHealthServerTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        tmp = Path(self._tmp.name)
        self.agent_home = tmp / "agent"
        self.agent_home.mkdir()
        (self.agent_home / "config.json").write_text("{}", encoding="utf-8")
        self.fake_bin = tmp / "finitechat"
        self.fake_bin.write_text(FAKE_FINITECHAT, encoding="utf-8")
        self.fake_bin.chmod(0o755)
        self.health = self._load_health_server()

    def _load_health_server(self) -> ModuleType:
        env_before = {
            name: os.environ.get(name)
            for name in (
                "FINITECHAT_HOME",
                "FINITECHAT_BIN",
                "FINITE_AGENT_BOOT_INTENT_JSON",
            )
        }
        os.environ["FINITECHAT_HOME"] = str(self.agent_home)
        os.environ["FINITECHAT_BIN"] = str(self.fake_bin)
        os.environ.pop("FINITE_AGENT_BOOT_INTENT_JSON", None)
        try:
            spec = importlib.util.spec_from_file_location(
                "agent_health_server_under_test", HEALTH_SERVER
            )
            assert spec is not None and spec.loader is not None
            module = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = module
            self.addCleanup(sys.modules.pop, spec.name, None)
            spec.loader.exec_module(module)
            return module
        finally:
            for name, value in env_before.items():
                if value is None:
                    os.environ.pop(name, None)
                else:
                    os.environ[name] = value

    def write_bridge(self, payload: object) -> None:
        (self.agent_home / "hermes-bridge-status.json").write_text(
            json.dumps(payload), encoding="utf-8"
        )

    def write_startup_report(self, payload: object) -> None:
        (self.agent_home / "startup-report.json").write_text(json.dumps(payload), encoding="utf-8")

    def test_contact_is_the_agent_principal_not_an_invite_session(self) -> None:
        self.write_bridge({"status": "connected", "ok": True})

        payload = self.health.runtime_health()

        self.assertTrue(self.health.runtime_ready(payload))
        self.assertEqual(payload["npub"], "npub1agent")
        self.assertEqual(payload["agent_npub"], "npub1agent")
        self.assertEqual(payload["account_id"], "a" * 64)
        self.assertEqual(payload["bridge"], {"status": "connected", "ok": True})
        self.assertEqual(payload["agentd"], {"status": "starting", "ok": True})
        self.assertNotIn("startup", payload)
        for deleted_field in ("url", "invite_id", "room_id", "paired"):
            self.assertNotIn(deleted_field, payload)
        self.assertFalse(hasattr(self.health, "mint_invite"))
        self.assertFalse((self.agent_home / "current-invite.json").exists())

    def test_identity_or_bridge_failure_is_not_ready(self) -> None:
        missing_bridge = self.health.runtime_health()
        self.assertFalse(self.health.runtime_ready(missing_bridge))
        self.assertEqual(missing_bridge["bridge"]["status"], "starting")

        (self.agent_home / "config.json").unlink()
        self.write_bridge({"status": "connected", "ok": True})
        missing_identity = self.health.runtime_health()
        self.assertFalse(self.health.runtime_ready(missing_identity))
        self.assertIsNone(missing_identity["agent_npub"])

    def test_agentd_required_waits_for_all_supervised_processes(self) -> None:
        self.health.__dict__["AGENTD_REQUIRED"] = True
        self.addCleanup(setattr, self.health, "AGENTD_REQUIRED", False)
        self.write_bridge({"status": "connected", "ok": True})
        starting = self.health.runtime_health()
        self.assertFalse(self.health.runtime_ready(starting))

        status_path = self.agent_home / "agentd" / "status.json"
        status_path.parent.mkdir()
        status_path.write_text(
            json.dumps(
                {
                    "version": "0.1.0",
                    "processes": {
                        "processes": {
                            name: {"state": "running"}
                            for name in ("finitechat", "health", "hermes")
                        }
                    },
                }
            ),
            encoding="utf-8",
        )
        healthy = self.health.runtime_health()
        self.assertTrue(self.health.runtime_ready(healthy))
        self.assertEqual(healthy["agentd"]["version"], "0.1.0")

    def write_agentd_status(self, age_secs: float = 0, **extra: object) -> None:
        status_path = self.agent_home / "agentd" / "status.json"
        status_path.parent.mkdir(exist_ok=True)
        status_path.write_text(
            json.dumps(
                {
                    "version": "0.1.0",
                    "processes": {
                        "processes": {
                            name: {"state": "running"}
                            for name in ("finitechat", "health", "hermes")
                        }
                    },
                    **extra,
                }
            ),
            encoding="utf-8",
        )
        if age_secs:
            old = time.time() - age_secs
            os.utime(status_path, (old, old))

    def test_frozen_agentd_heartbeat_is_not_ready(self) -> None:
        # A status file a dead (SIGKILLed) agentd left behind keeps green
        # contents; the freshness gate must override them (PR 440 backport).
        self.write_bridge({"status": "connected", "ok": True})
        self.write_agentd_status(age_secs=3600)

        payload = self.health.runtime_health()

        self.assertFalse(self.health.runtime_ready(payload))
        self.assertFalse(payload["ready"])
        self.assertEqual(payload["ready_reason"], "agentd_status_stale")
        self.assertFalse(payload["agentd"]["ok"])
        self.assertIs(payload["agentd"]["stale"], True)
        self.assertEqual(payload["agentd"]["status"], "unavailable")

    def write_output_report(self, report: object, updated_at_ms: object = UPDATED_AT_MS) -> None:
        self.write_agentd_status(container_output=report, updated_at_ms=updated_at_ms)

    def test_output_observations_pass_through_a_strict_allowlist(self) -> None:
        self.write_bridge({"status": "connected", "ok": True})
        full = {**pipe_observation(65536), "path": "/secret", "pid": 42}
        self.write_output_report(
            {
                "sample_interval_ms": 1000,
                "stdout": full,
                "stderr": {"kind": "same_pipe_as_stdout", "note": "must-not-escape"},
                "collector": "must-not-escape",
            }
        )

        payload = self.health.runtime_health()

        self.assertEqual(
            payload["agentd"]["container_output"],
            {
                "state": "reported",
                "sample_interval_ms": 1000,
                "stdout": pipe_observation(65536),
                "stderr": {"kind": "same_pipe_as_stdout"},
            },
        )
        serialized = json.dumps(payload)
        for leaked in ("must-not-escape", "/secret", '"pid"', "collector"):
            self.assertNotIn(leaked, serialized)

    def test_every_reported_stream_kind_is_projected(self) -> None:
        observed = {"observed_at_ms": UPDATED_AT_MS - 1000}
        for stream in (
            {"kind": "not_pipe", **observed, "file_type": "character_device"},
            {"kind": "not_pipe", **observed, "file_type": "regular_file"},
            {"kind": "unavailable", **observed, "error": "EBADF"},
            {"kind": "unsupported_platform"},
            {"kind": "sampler_not_started"},
            pipe_observation(0),
        ):
            with self.subTest(stream=stream):
                self.write_output_report(
                    {"sample_interval_ms": 1000, "stdout": stream, "stderr": stream}
                )
                projected = self.health.agentd_status()["container_output"]
                self.assertEqual(projected["stdout"], stream)
                self.assertEqual(projected["stderr"], stream)

    def test_older_agentd_reports_no_output_observations(self) -> None:
        self.write_agentd_status()
        self.assertEqual(self.health.agentd_status()["container_output"], {"state": "not_reported"})
        self.write_agentd_status(container_output=None)
        self.assertEqual(self.health.agentd_status()["container_output"], {"state": "not_reported"})

    def test_malformed_output_reports_are_unknown(self) -> None:
        stream = pipe_observation(4096)
        valid = {"sample_interval_ms": 1000, "stdout": stream, "stderr": stream}
        for report in (
            "pipe",
            [],
            {**valid, "stdout": None},
            {key: value for key, value in valid.items() if key != "stderr"},
            {key: value for key, value in valid.items() if key != "sample_interval_ms"},
            {**valid, "sample_interval_ms": True},
            {**valid, "stdout": {"kind": "same_pipe_as_stdout"}},
            {**valid, "stdout": {**stream, "kind": "stalled"}},
            {**valid, "stdout": {**stream, "buffered_bytes": -1}},
            {**valid, "stdout": {**stream, "buffered_bytes": "4096"}},
            {**valid, "stdout": {**stream, "buffered_bytes": 4096.0}},
            {**valid, "stdout": {**stream, "capacity_bytes": 2**53}},
            {**valid, "stdout": {**stream, "sample_count": False}},
            {**valid, "stdout": {k: v for k, v in stream.items() if k != "observed_at_ms"}},
            {**valid, "stdout": {k: v for k, v in stream.items() if k != "capacity_bytes"}},
            {**valid, "stdout": {"kind": "not_pipe", "observed_at_ms": 1, "file_type": "tty"}},
            {**valid, "stdout": {"kind": "unavailable", "observed_at_ms": 1, "error": "x" * 64}},
        ):
            with self.subTest(report=report):
                self.write_output_report(report)
                self.assertEqual(
                    self.health.agentd_status()["container_output"],
                    {"state": "unknown", "reason": "malformed"},
                )
        for updated_at_ms in (None, "1", -1, True):
            with self.subTest(updated_at_ms=updated_at_ms):
                self.write_output_report(valid, updated_at_ms=updated_at_ms)
                self.assertEqual(
                    self.health.agentd_status()["container_output"],
                    {"state": "unknown", "reason": "malformed"},
                )

    def test_samples_outside_the_heartbeat_limit_are_unknown(self) -> None:
        limit_ms = int(self.health.AGENTD_STATUS_STALE_SECS * 1000)
        for observed_at_ms, current in (
            (UPDATED_AT_MS, True),
            (UPDATED_AT_MS - limit_ms, True),
            (UPDATED_AT_MS - limit_ms - 1, False),
            (UPDATED_AT_MS + 1, False),
        ):
            with self.subTest(observed_at_ms=observed_at_ms):
                stream = {"kind": "unavailable", "observed_at_ms": observed_at_ms, "error": "EIO"}
                self.write_output_report(
                    {
                        "sample_interval_ms": 1000,
                        "stdout": stream,
                        "stderr": {"kind": "same_pipe_as_stdout"},
                    }
                )
                projected = self.health.agentd_status()["container_output"]
                self.assertEqual(
                    projected["stdout"],
                    stream if current else {"kind": "unknown", "reason": "observation_stale"},
                )
                self.assertEqual(projected["stderr"], {"kind": "same_pipe_as_stdout"})

    def test_frozen_heartbeat_makes_output_observations_unknown(self) -> None:
        self.write_bridge({"status": "connected", "ok": True})
        self.write_agentd_status(
            age_secs=3600,
            updated_at_ms=UPDATED_AT_MS,
            container_output={
                "sample_interval_ms": 1000,
                "stdout": pipe_observation(65536),
                "stderr": pipe_observation(0),
            },
        )

        payload = self.health.runtime_health()

        self.assertEqual(
            payload["agentd"]["container_output"],
            {"state": "unknown", "reason": "agentd_status_stale"},
        )
        self.assertFalse(payload["ready"])
        self.assertEqual(payload["ready_reason"], "agentd_status_stale")
        self.assertIs(payload["agentd"]["stale"], True)

    def test_output_observations_never_change_readiness(self) -> None:
        reports: list[object] = [
            {"sample_interval_ms": 1000, "stdout": stream, "stderr": stream}
            for stream in (
                pipe_observation(65536),
                pipe_observation(0),
                {**pipe_observation(65536), "observed_at_ms": 1},
                {"kind": "unavailable", "observed_at_ms": UPDATED_AT_MS, "error": "EBADF"},
                {"kind": "not_pipe", "observed_at_ms": UPDATED_AT_MS, "file_type": "socket"},
                {"kind": "unsupported_platform"},
                {"kind": "sampler_not_started"},
            )
        ]
        reports.extend(("malformed", None))
        for bridge in ({"status": "connected", "ok": True}, None):
            if bridge is None:
                (self.agent_home / "hermes-bridge-status.json").unlink()
            else:
                self.write_bridge(bridge)
            self.write_agentd_status()
            baseline = self.health.runtime_health()
            for report in reports:
                with self.subTest(bridge=bridge, report=report):
                    self.write_output_report(report)
                    payload = self.health.runtime_health()
                    for key in ("ready", "ready_reason"):
                        self.assertEqual(payload.get(key), baseline.get(key))
                    for key in ("ok", "status", "processes"):
                        self.assertEqual(payload["agentd"][key], baseline["agentd"][key])
                    self.assertEqual(
                        self.health.runtime_ready(payload), self.health.runtime_ready(baseline)
                    )
            self.assertIs(baseline["ready"], bridge is not None)

    def test_output_projection_stays_small_whatever_agentd_writes(self) -> None:
        self.write_bridge({"status": "connected", "ok": True})
        largest = {
            "kind": "pipe",
            **dict.fromkeys(
                ("observed_at_ms", *self.health.CONTAINER_OUTPUT_PIPE_FIELDS),
                2**53 - 1,
            ),
            "padding": "x" * 100_000,
        }
        self.write_agentd_status(
            updated_at_ms=2**53 - 1,
            container_output={
                "sample_interval_ms": 2**53 - 1,
                "stdout": largest,
                "stderr": largest,
                "padding": ["x" * 100_000],
            },
        )

        payload = self.health.runtime_health()

        projected = payload["agentd"]["container_output"]
        self.assertEqual(projected["state"], "reported")
        self.assertLessEqual(len(json.dumps(projected, sort_keys=True)), 1024)
        self.assertLessEqual(len(json.dumps(payload, sort_keys=True)), 4096)

    def test_fresh_agentd_heartbeat_stays_ready_without_a_reason(self) -> None:
        self.write_bridge({"status": "connected", "ok": True})
        self.write_agentd_status()

        payload = self.health.runtime_health()

        self.assertTrue(self.health.runtime_ready(payload))
        self.assertTrue(payload["ready"])
        self.assertNotIn("ready_reason", payload)

    def test_not_ready_bodies_name_the_cause(self) -> None:
        missing_bridge = self.health.runtime_health()
        self.assertEqual(missing_bridge["ready_reason"], "bridge_not_connected")
        self.assertFalse(missing_bridge["ready"])

        self.write_bridge({"status": "connected", "ok": True})
        self.write_startup_report(
            {
                "schema_version": 1,
                "report_kind": "finite_agent_startup",
                "boot_mode": "recover_known_good",
                "status": "refused",
                "phase": "blocked",
                "error_code": "identity_missing_or_corrupt",
            }
        )
        refused = self.health.runtime_health()
        self.assertEqual(refused["ready_reason"], "bootstrap_failed")
        (self.agent_home / "startup-report.json").unlink()

        self.health.__dict__["AGENTD_REQUIRED"] = True
        self.addCleanup(setattr, self.health, "AGENTD_REQUIRED", False)
        agentd_missing = self.health.runtime_health()
        self.assertEqual(agentd_missing["ready_reason"], "agentd_not_running")
        self.assertFalse(agentd_missing["agentd"]["ok"])

        self.write_agentd_status()
        (self.agent_home / "config.json").unlink()
        missing_identity = self.health.runtime_health()
        self.assertEqual(missing_identity["ready_reason"], "identity_not_ready")

    def test_runtime_startup_never_calls_deleted_invite_cli(self) -> None:
        gateway = GATEWAY.read_text(encoding="utf-8")
        self.assertNotIn(' hermes --agent-home "$agent_home" invite', gateway)
        self.assertNotIn("current-invite.json", gateway)
        self.assertIn("deleted invite-session protocol", gateway)

    def test_completed_startup_report_is_redacted_and_ready(self) -> None:
        self.write_bridge({"status": "connected", "ok": True})
        self.write_startup_report(
            {
                "schema_version": 1,
                "report_kind": "finite_agent_startup",
                "boot_mode": "recover_known_good",
                "status": "completed",
                "phase": "complete",
                "operation_id_hash": "sha256:" + "b" * 64,
                "operation_id": "must-not-escape",
                "actions": [
                    {
                        "action": "canonical_plugin_reinstall",
                        "status": "changed",
                        "count": 1,
                        "path": "/data/secret-path",
                        "stderr": "must-not-escape",
                    }
                ],
                "refusals": [],
                "identity": {"npub": "npub1agent", "secret": "must-not-escape"},
                "state_roots": {
                    "/data": {"present": True, "writable": True, "path": "/secret"},
                    "/data/agent": {"present": True, "writable": True},
                    "/unapproved": {"present": True, "writable": True},
                },
                "idempotency": {
                    "same_operation_replay": "no_op_after_terminal_state",
                    "resumed_after_interruption": False,
                    "journal_path": "/secret",
                },
                "acceptance_scope": {
                    "runtime_spec_delivery": "not_proven",
                    "provider_conformance": "not_proven",
                    "phala_acceptance": "not_proven",
                    "claimed": "accepted",
                },
                "secret": "must-not-escape",
            }
        )

        payload = self.health.runtime_health()

        self.assertTrue(self.health.runtime_ready(payload))
        startup = payload["startup"]
        self.assertTrue(startup["ok"])
        self.assertEqual(startup["operation_id_hash"], "sha256:" + "b" * 64)
        self.assertEqual(
            startup["actions"],
            [
                {
                    "action": "canonical_plugin_reinstall",
                    "status": "changed",
                    "count": 1,
                }
            ],
        )
        serialized = json.dumps(startup)
        self.assertNotIn("must-not-escape", serialized)
        self.assertNotIn("secret-path", serialized)
        self.assertNotIn("/unapproved", startup["state_roots"])
        self.assertNotIn("claimed", startup["acceptance_scope"])

    def test_refused_or_invalid_startup_report_is_not_ready(self) -> None:
        self.write_bridge({"status": "connected", "ok": True})
        self.write_startup_report(
            {
                "schema_version": 1,
                "report_kind": "finite_agent_startup",
                "boot_mode": "recover_known_good",
                "status": "refused",
                "phase": "blocked",
                "error_code": "identity_missing_or_corrupt",
                "refusals": [
                    {
                        "code": "identity_missing_or_corrupt",
                        "remediation": "restore_or_escalate",
                        "detail": "must-not-escape",
                    }
                ],
            }
        )
        refused = self.health.runtime_health()
        self.assertFalse(self.health.runtime_ready(refused))
        self.assertEqual(refused["startup"]["status"], "refused")
        self.assertNotIn("must-not-escape", json.dumps(refused["startup"]))

        (self.agent_home / "startup-report.json").write_text("not-json\n", encoding="utf-8")
        invalid = self.health.runtime_health()
        self.assertFalse(self.health.runtime_ready(invalid))
        self.assertEqual(invalid["startup"]["error_code"], "startup_report_invalid")

    def test_active_recovery_intent_requires_a_startup_report(self) -> None:
        previous = self.health.RECOVERY_BOOT_INTENT_ACTIVE
        self.health.__dict__["RECOVERY_BOOT_INTENT_ACTIVE"] = True
        self.addCleanup(setattr, self.health, "RECOVERY_BOOT_INTENT_ACTIVE", previous)
        self.write_bridge({"status": "connected", "ok": True})

        payload = self.health.runtime_health()

        self.assertFalse(self.health.runtime_ready(payload))
        self.assertEqual(
            payload["startup"]["error_code"],
            "startup_report_missing_for_recovery_intent",
        )


if __name__ == "__main__":
    unittest.main()

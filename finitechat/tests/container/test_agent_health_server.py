"""Unit checks for the Agent Runtime's narrow health/contact server."""

from __future__ import annotations

import importlib.util
import json
import os
import sys
import tempfile
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

    def test_runtime_startup_never_calls_deleted_invite_cli(self) -> None:
        gateway = GATEWAY.read_text(encoding="utf-8")
        self.assertNotIn(' hermes --home "$agent_home" invite', gateway)
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

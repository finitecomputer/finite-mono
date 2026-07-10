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
        env_before = {name: os.environ.get(name) for name in ("FINITECHAT_HOME", "FINITECHAT_BIN")}
        os.environ["FINITECHAT_HOME"] = str(self.agent_home)
        os.environ["FINITECHAT_BIN"] = str(self.fake_bin)
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

    def test_contact_is_the_agent_principal_not_an_invite_session(self) -> None:
        self.write_bridge({"status": "connected", "ok": True})

        payload = self.health.runtime_health()

        self.assertTrue(self.health.runtime_ready(payload))
        self.assertEqual(payload["npub"], "npub1agent")
        self.assertEqual(payload["agent_npub"], "npub1agent")
        self.assertEqual(payload["account_id"], "a" * 64)
        self.assertEqual(payload["bridge"], {"status": "connected", "ok": True})
        self.assertEqual(payload["agentd"], {"status": "starting", "ok": True})
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


if __name__ == "__main__":
    unittest.main()

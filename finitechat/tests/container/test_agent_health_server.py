"""Unit checks for the container health server's single-use invite state machine.

The health server is a thin shim over `finitechat hermes invite` /
`finitechat hermes invite-status`; these tests fake the CLI binary and
assert the shim's serve/refresh/admission decisions and its cache writes.
"""

from __future__ import annotations

import importlib.util
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from types import ModuleType
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[2]
HEALTH_SERVER = REPO_ROOT / "containers" / "agent" / "health_server.py"

FAKE_FINITECHAT = """#!/usr/bin/env python3
import json
import sys
from pathlib import Path

control = Path(__file__).resolve().parent
args = sys.argv[1:]
with (control / "calls.jsonl").open("a", encoding="utf-8") as log:
    log.write(json.dumps(args) + "\\n")


def next_response(name: str) -> object:
    counter = control / f"{name}-count"
    index = int(counter.read_text(encoding="utf-8")) if counter.exists() else 0
    counter.write_text(str(index + 1), encoding="utf-8")
    responses = json.loads((control / f"{name}-responses.json").read_text(encoding="utf-8"))
    return responses[min(index, len(responses) - 1)]


if "auth" in args and "status" in args:
    print(json.dumps({"npub": "npub1agent", "account_id": "a" * 64}))
    sys.exit(0)
if "invite-status" in args:
    response = next_response("status")
    if response == "fail":
        sys.exit(1)
    print(json.dumps(response))
    sys.exit(0)
if "invite" in args:
    print(json.dumps(next_response("mint")))
    sys.exit(0)
sys.exit(2)
"""


def open_status(expires_at_ms: int = 4_000_000) -> dict[str, Any]:
    return {
        "invite_id": "invite-1",
        "room_id": "room-1",
        "state": "open",
        "max_joins": 1,
        "accepted_joins": 0,
        "expires_at_ms": expires_at_ms,
        "consumed": False,
        "expired": False,
        "joinable": True,
    }


def minted_invite(invite_id: str = "invite-1", url_suffix: str = "1") -> dict[str, Any]:
    return {
        "url": f"finitechat://invite?v=1&i={url_suffix}",
        "qr": "qr",
        "invite_id": invite_id,
        "room_id": "room-1",
        "npub": "npub1agent",
    }


class AgentHealthServerInviteTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        tmp = Path(self._tmp.name)
        self.agent_home = tmp / "agent"
        self.agent_home.mkdir()
        (self.agent_home / "config.json").write_text("{}", encoding="utf-8")
        self.control = tmp / "control"
        self.control.mkdir()
        self.fake_bin = self.control / "finitechat"
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

    def set_mints(self, responses: list[dict[str, Any]]) -> None:
        (self.control / "mint-responses.json").write_text(json.dumps(responses), encoding="utf-8")

    def set_statuses(self, responses: list[Any]) -> None:
        (self.control / "status-responses.json").write_text(json.dumps(responses), encoding="utf-8")

    def calls(self) -> list[list[str]]:
        log = self.control / "calls.jsonl"
        if not log.exists():
            return []
        return [json.loads(line) for line in log.read_text(encoding="utf-8").splitlines()]

    def mint_calls(self) -> list[list[str]]:
        return [call for call in self.calls() if "invite" in call]

    def cached_invite(self) -> dict[str, Any]:
        return json.loads((self.agent_home / "current-invite.json").read_text(encoding="utf-8"))

    def test_first_probe_mints_single_use_short_ttl_invite(self) -> None:
        self.set_mints([minted_invite()])
        self.set_statuses([open_status()])

        payload = self.health.invite()

        self.assertTrue(payload["ready"])
        self.assertFalse(payload["paired"])
        self.assertEqual(payload["url"], "finitechat://invite?v=1&i=1")
        self.assertEqual(payload["invite_state"], "open")
        self.assertEqual(payload["expires_at_ms"], 4_000_000)
        self.assertEqual(payload["room_id"], "room-1")
        self.assertEqual(payload["invite_id"], "invite-1")
        self.assertEqual(payload["agent_npub"], "npub1agent")
        self.assertEqual(self.cached_invite()["invite_id"], "invite-1")

        (mint,) = self.mint_calls()
        self.assertIn("--max-joins", mint)
        self.assertEqual(mint[mint.index("--max-joins") + 1], "1")
        self.assertIn("--ttl-ms", mint)
        self.assertEqual(mint[mint.index("--ttl-ms") + 1], "3600000")
        self.assertIn("--room-name", mint)

    def test_consumed_invite_reports_pending_admission_without_url_and_never_remints(self) -> None:
        (self.agent_home / "current-invite.json").write_text(
            json.dumps(minted_invite()), encoding="utf-8"
        )
        consumed = open_status()
        consumed.update({"state": "closed", "accepted_joins": 1, "consumed": True})
        self.set_statuses([consumed])

        payload = self.health.invite()

        self.assertTrue(payload["ready"])
        self.assertFalse(payload["paired"])
        self.assertNotIn("url", payload)
        self.assertEqual(payload["invite_state"], "consumed_pending_admission")
        self.assertEqual(payload["room_id"], "room-1")
        self.assertEqual(self.cached_invite()["invite_state"], "consumed_pending_admission")
        self.assertNotIn("paired", self.cached_invite())

        # The spent verdict is durable enough to avoid re-minting, but it does
        # not claim pairing. Until finitechat can prove room admission from MLS
        # member state, the dashboard must keep showing a non-paired state.
        self.set_statuses(["fail"])
        repeat = self.health.invite()
        self.assertFalse(repeat["paired"])
        self.assertNotIn("url", repeat)
        self.assertEqual(repeat["invite_state"], "consumed_pending_admission")
        self.assertEqual(self.mint_calls(), [])

    def test_legacy_cached_paired_flag_is_downgraded_to_pending_admission(self) -> None:
        cached = minted_invite()
        cached["paired"] = True
        (self.agent_home / "current-invite.json").write_text(json.dumps(cached), encoding="utf-8")

        payload = self.health.invite()

        self.assertTrue(payload["ready"])
        self.assertFalse(payload["paired"])
        self.assertNotIn("url", payload)
        self.assertEqual(payload["invite_state"], "consumed_pending_admission")
        self.assertEqual(self.calls(), [["auth", "status"]])

    def test_expired_unconsumed_invite_remints_for_the_same_room(self) -> None:
        (self.agent_home / "current-invite.json").write_text(
            json.dumps(minted_invite()), encoding="utf-8"
        )
        expired = open_status()
        expired.update({"expired": True, "joinable": False})
        fresh = open_status()
        fresh["invite_id"] = "invite-2"
        self.set_statuses([expired, fresh])
        self.set_mints([minted_invite(invite_id="invite-2", url_suffix="2")])

        payload = self.health.invite()

        self.assertTrue(payload["ready"])
        self.assertFalse(payload["paired"])
        self.assertEqual(payload["invite_id"], "invite-2")
        self.assertEqual(payload["url"], "finitechat://invite?v=1&i=2")
        self.assertEqual(self.cached_invite()["invite_id"], "invite-2")

        (mint,) = self.mint_calls()
        self.assertIn("--room-id", mint)
        self.assertEqual(mint[mint.index("--room-id") + 1], "room-1")
        self.assertNotIn("--room-name", mint)

    def test_status_probe_failure_serves_cached_invite_as_unknown(self) -> None:
        (self.agent_home / "current-invite.json").write_text(
            json.dumps(minted_invite()), encoding="utf-8"
        )
        self.set_statuses(["fail"])

        payload = self.health.invite()

        self.assertTrue(payload["ready"])
        self.assertFalse(payload["paired"])
        self.assertEqual(payload["url"], "finitechat://invite?v=1&i=1")
        self.assertEqual(payload["invite_state"], "unknown")
        self.assertEqual(self.mint_calls(), [])

    def test_not_found_session_serves_no_url_and_does_not_remint(self) -> None:
        (self.agent_home / "current-invite.json").write_text(
            json.dumps(minted_invite()), encoding="utf-8"
        )
        not_found = {
            "invite_id": "invite-1",
            "room_id": "room-1",
            "state": "not_found",
            "consumed": False,
            "expired": False,
            "joinable": False,
        }
        self.set_statuses([not_found])

        payload = self.health.invite()

        self.assertTrue(payload["ready"])
        self.assertFalse(payload["paired"])
        self.assertNotIn("url", payload)
        self.assertEqual(payload["invite_state"], "not_found")
        self.assertEqual(self.mint_calls(), [])

    def test_identity_failure_stays_not_ready(self) -> None:
        (self.agent_home / "config.json").unlink()
        payload = self.health.invite()
        self.assertFalse(payload["ready"])
        self.assertEqual(self.calls(), [])


if __name__ == "__main__":
    unittest.main()

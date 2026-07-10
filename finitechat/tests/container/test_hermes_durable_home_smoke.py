"""Regression checks for the canonical runtime's durable Welcome smoke."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path
from unittest import mock

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPO_ROOT / "scripts" / "hermes-durable-home-docker-smoke.py"
WORKFLOW_PATH = REPO_ROOT.parent / ".github" / "workflows" / "hermes-runtime-smoke.yml"

spec = importlib.util.spec_from_file_location("hermes_durable_home_docker_smoke", SCRIPT_PATH)
assert spec is not None
smoke = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(smoke)


class HermesDurableHomeSmokeTest(unittest.TestCase):
    def test_welcome_flow_publishes_keys_then_commits_mls_add(self) -> None:
        responses = [
            {"status": "running"},
            {"selected_room_id": "room-welcome"},
            {"status": "people added"},
        ]
        with mock.patch.object(smoke, "docker_user_app", side_effect=responses) as app:
            result = smoke.create_welcome_room(
                image="finite-agent-runtime:test",
                user_volume="probe-state",
                server_url="https://chat.example",
                agent_account_id="agent-account",
                env={},
            )

        self.assertEqual(result, {"room_id": "room-welcome", "add_status": "people added"})
        self.assertEqual(app.call_args_list[0].kwargs["args"], ["state", "--start-runtime"])
        self.assertEqual(
            app.call_args_list[1].kwargs["args"],
            ["create-room", "--display-name", "Finite Durable Docker Smoke"],
        )
        self.assertEqual(
            app.call_args_list[2].kwargs["args"],
            [
                "add-member",
                "--room-id",
                "room-welcome",
                "--account-id",
                "agent-account",
                "--display-name",
                "Finite Agent",
            ],
        )

    def test_probe_identity_and_app_state_share_one_durable_volume(self) -> None:
        captured: list[str] = []

        def fake_run_json(args: list[str], **_kwargs: object) -> dict[str, bool]:
            captured.extend(args)
            return {"ok": True}

        with mock.patch.object(smoke, "run_json", side_effect=fake_run_json):
            smoke.docker_user_app(
                image="finite-agent-runtime:test",
                user_volume="probe-state",
                server_url="https://chat.example",
                args=["state", "--start-runtime"],
                env={},
            )

        self.assertIn("type=volume,src=probe-state,dst=/data/user", captured)
        self.assertIn("FINITE_HOME=/data/user", captured)
        self.assertIn("--data-dir", captured)
        self.assertIn("/data/user", captured)

    def test_room_wait_observes_resident_sidecar_before_collecting_status(self) -> None:
        state = {"rooms": [{"room_id": "room-welcome", "state": "Connected"}]}
        status = {"room_id": "room-welcome", "connected": True, "paired": True}
        with (
            mock.patch.object(smoke, "docker_agent_app_state", return_value=state) as app_state,
            mock.patch.object(smoke, "docker_agent_hermes", return_value=status) as hermes,
        ):
            result = smoke.wait_agent_room_connected(
                "agent-container",
                "room-welcome",
                "https://chat.example",
                timeout=1,
            )

        self.assertEqual(result, status)
        app_state.assert_called_once_with(
            container="agent-container", server_url="https://chat.example"
        )
        self.assertEqual(
            hermes.call_args.kwargs["args"],
            ["room-status", "--room-id", "room-welcome", "--json"],
        )

    def test_default_dispatch_is_welcome_first_and_pins_hermes_0182(self) -> None:
        script = SCRIPT_PATH.read_text(encoding="utf-8")
        workflow = WORKFLOW_PATH.read_text(encoding="utf-8")

        self.assertNotIn("wait_fresh_invite", script)
        self.assertNotIn("docker_user_hermes", script)
        self.assertNotIn('args=["join"', script)
        self.assertIn('FINITE_HERMES_AGENT_VERSION: "0.18.2"', workflow)
        self.assertIn("default: true", workflow)
        self.assertIn("scripts/hermes-durable-home-docker-smoke.py", workflow)


if __name__ == "__main__":
    unittest.main()

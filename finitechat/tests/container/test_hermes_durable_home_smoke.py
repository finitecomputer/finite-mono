"""Regression checks for the canonical runtime's durable Welcome smoke."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import unittest
from collections.abc import Iterator
from pathlib import Path
from typing import Any
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
        # Hermes version facts are flake.lock-derived; never a literal here.
        self.assertNotIn("0.20.0", workflow)
        self.assertIn("default: true", workflow)
        self.assertIn("scripts/hermes-durable-home-docker-smoke.py", workflow)

    def test_finite_private_key_selects_the_product_inference_profile(self) -> None:
        captured: list[list[str]] = []
        done = mock.Mock(stdout="container-id\n")

        def fake_run(command: list[str], **kwargs: object) -> mock.Mock:
            captured.append(command)
            return done

        with (
            mock.patch.object(smoke, "docker_container_rm"),
            mock.patch.object(smoke, "run", side_effect=fake_run),
        ):
            smoke.start_agent_container(
                image="finite-agent-runtime:test",
                container="agent-container",
                home_volume="home-vol",
                server_url="https://chat.example",
                env={"FINITE_PRIVATE_API_KEY": "fpk_test"},
            )
        command = captured[0]
        self.assertIn("FINITE_DEFAULT_INFERENCE_PROFILE=finite-private", command)
        self.assertNotIn("FINITE_DEFAULT_INFERENCE_PROFILE=openrouter", command)

    def test_without_finite_private_key_openrouter_profile_is_kept(self) -> None:
        captured: list[list[str]] = []
        done = mock.Mock(stdout="container-id\n")

        def fake_run(command: list[str], **kwargs: object) -> mock.Mock:
            captured.append(command)
            return done

        with (
            mock.patch.object(smoke, "docker_container_rm"),
            mock.patch.object(smoke, "run", side_effect=fake_run),
        ):
            smoke.start_agent_container(
                image="finite-agent-runtime:test",
                container="agent-container",
                home_volume="home-vol",
                server_url="https://chat.example",
                env={},
            )
        self.assertIn("FINITE_DEFAULT_INFERENCE_PROFILE=openrouter", captured[0])

    def test_probe_path_may_only_name_read_only_hermes_commands(self) -> None:
        with mock.patch.object(smoke, "docker_agent_hermes", return_value={"ok": True}) as hermes:
            result = smoke.docker_agent_hermes_probe(
                container="agent-container",
                args=["room-status", "--room-id", "room-welcome", "--json"],
            )
        self.assertEqual(result, {"ok": True})
        hermes.assert_called_once_with(
            container="agent-container",
            args=["room-status", "--room-id", "room-welcome", "--json"],
            timeout=30,
        )

        for writer_args in (
            ["send"],
            ["poll"],
            ["edit"],
            ["recover"],
            ["activity"],
            ["serve"],
            ["init"],
            ["home-channel"],
            [],
        ):
            with self.assertRaises(smoke.SmokeFailure) as raised:
                smoke.docker_agent_hermes_probe(container="agent-container", args=list(writer_args))
            message = str(raised.exception)
            self.assertIn("read-only probe path", message)
            self.assertIn("room-status", message)

    def test_agent_app_state_probe_cannot_name_a_writer_open(self) -> None:
        captured: list[str] = []

        def fake_run_json(args: list[str], **_kwargs: object) -> dict[str, bool]:
            captured.extend(args)
            return {"ok": True}

        with mock.patch.object(smoke, "run_json", side_effect=fake_run_json):
            smoke.docker_agent_app_state(
                container="agent-container", server_url="https://chat.example"
            )

        # The probe exec is exactly `app state`: the CLI classifies only this
        # flag-free form ReadOnly, and this helper accepts no args that could
        # turn it into a writer (`--start-runtime`, `--wait-update-ms`,
        # `--room-id`).
        self.assertEqual(captured[-1], "state")
        self.assertNotIn("--start-runtime", captured)
        self.assertNotIn("--wait-update-ms", captured)
        self.assertNotIn("--room-id", captured)

    def test_reply_timeout_includes_agent_container_log_tail(self) -> None:
        clock = iter([0.0, 0.0, 200.0])
        with (
            mock.patch.object(smoke, "docker_user_app", return_value={"messages": []}),
            mock.patch.object(smoke, "agent_log_tail", return_value="AGENT-LOG-TAIL"),
            mock.patch.object(smoke.time, "monotonic", side_effect=lambda: next(clock)),
            self.assertRaises(smoke.SmokeFailure) as raised,
        ):
            smoke.run_model_smoke(
                image="finite-agent-runtime:test",
                user_volume="user-vol",
                server_url="https://chat.example",
                room_id="room-x",
                expected="hello",
                env={},
                agent_container="agent-container",
            )
        self.assertIn("AGENT-LOG-TAIL", str(raised.exception))

    def test_command_failure_keeps_exit_code_and_stderr(self) -> None:
        failed = smoke.subprocess.CompletedProcess(
            ["docker"], 1, stdout="", stderr=smoke.MISSED_HEARTBEAT_STDERR + "\n"
        )
        with (
            mock.patch.object(smoke.subprocess, "run", return_value=failed),
            self.assertRaises(smoke.CommandFailed) as raised,
        ):
            smoke.run(["docker", "run"])
        self.assertIsInstance(raised.exception, smoke.SmokeFailure)
        self.assertEqual(raised.exception.returncode, 1)
        self.assertEqual(raised.exception.stderr.strip(), smoke.MISSED_HEARTBEAT_STDERR)
        self.assertIn("exit=1", str(raised.exception))

    def test_reply_poll_reads_again_after_a_missed_heartbeat_without_resending(self) -> None:
        reply = {"is_mine": False, "text": "hello", "message_id": "reply"}
        poll = ReplyPoll(
            [
                (1, {"messages": []}),
                (4, {"messages": []}),
                (18, missed_heartbeat()),
                (4, {"messages": [reply]}),
            ],
        )
        with poll.patched() as alive:
            result = poll.run()
        self.assertEqual(result["status"], "passed")
        self.assertEqual(result["reply_message_id"], "reply")
        self.assertEqual(result["missed_heartbeat_reads_s"], [18.0])
        self.assertEqual(poll.commands(), ["send", "state", "state", "state"])
        alive.assert_called_once_with("agent")
        self.assertEqual(
            poll.stderr.getvalue(),
            "reply poll missed the server heartbeat after 18.0s (1 of 2 allowed this phase);"
            " reading state again\n",
        )

    def test_reply_poll_fails_past_the_missed_heartbeat_cap(self) -> None:
        reply = {"is_mine": False, "text": "hello", "message_id": "reply"}
        poll = ReplyPoll(
            [
                (1, {"messages": []}),
                (18, missed_heartbeat()),
                (18, missed_heartbeat()),
                (18, missed_heartbeat()),
                (4, {"messages": [reply]}),
            ],
        )
        with poll.patched(), self.assertRaises(smoke.SmokeFailure) as raised:
            poll.run()
        self.assertIn("missed the server heartbeat 3 times", str(raised.exception))
        self.assertIn("AGENT-LOG-TAIL", str(raised.exception))
        self.assertEqual(poll.commands(), ["send", "state", "state", "state"])

    def test_reply_poll_keeps_other_read_failures_fatal(self) -> None:
        cases = {
            "before the read budget": (16.9, missed_heartbeat()),
            "another exit code": (18, missed_heartbeat(returncode=2)),
            "another error": (18, missed_heartbeat(stderr="delivery error: server error 500")),
            "extra stderr": (
                18,
                missed_heartbeat(stderr="warning\n" + smoke.MISSED_HEARTBEAT_STDERR),
            ),
            "not a command failure": (1, smoke.SmokeFailure(smoke.MISSED_HEARTBEAT_STDERR)),
        }
        for name, (seconds, error) in cases.items():
            with self.subTest(name):
                poll = ReplyPoll([(1, {"messages": []}), (seconds, error)])
                with poll.patched() as alive, self.assertRaises(smoke.SmokeFailure) as raised:
                    poll.run()
                self.assertIs(raised.exception, error)
                self.assertEqual(poll.commands(), ["send", "state"])
                alive.assert_not_called()

    def test_reply_poll_keeps_its_deadline_across_missed_heartbeats(self) -> None:
        reply = {"is_mine": False, "text": "hello", "message_id": "reply"}
        poll = ReplyPoll(
            [
                (1, {"messages": []}),
                (58, missed_heartbeat()),
                (58, missed_heartbeat()),
                (58, {"messages": []}),
                (4, {"messages": [reply]}),
            ],
        )
        with poll.patched(), self.assertRaises(smoke.SmokeFailure) as raised:
            poll.run()
        self.assertIn("expected Hermes reply 'hello' not found", str(raised.exception))
        self.assertIn("missed heartbeat read seconds=[58.0, 58.0]", str(raised.exception))
        self.assertEqual(poll.commands(), ["send", "state", "state", "state"])


def missed_heartbeat(
    *, returncode: int = 1, stderr: str = smoke.MISSED_HEARTBEAT_STDERR + "\n"
) -> Exception:
    return smoke.CommandFailed(
        ["docker", "run"],
        smoke.subprocess.CompletedProcess(["docker"], returncode, stdout="", stderr=stderr),
    )


class ReplyPoll:
    """Runs `run_model_smoke` against scripted CLI results on a fake clock.

    Each step is (seconds the command takes, state dict or exception).
    """

    def __init__(self, steps: list[tuple[float, object]]) -> None:
        self.steps = list(steps)
        self.now = 0.0
        self.app = mock.Mock(side_effect=self._app)
        self.stderr = io.StringIO()

    def _app(self, **_kwargs: object) -> object:
        seconds, result = self.steps.pop(0)
        self.now += seconds
        if isinstance(result, Exception):
            raise result
        return result

    def _sleep(self, seconds: float) -> None:
        self.now += seconds

    @contextlib.contextmanager
    def patched(self) -> Iterator[mock.Mock]:
        with (
            mock.patch.object(smoke, "docker_user_app", self.app),
            mock.patch.object(smoke, "agent_log_tail", return_value="AGENT-LOG-TAIL"),
            mock.patch.object(smoke.time, "monotonic", lambda: self.now),
            mock.patch.object(smoke.time, "sleep", self._sleep),
            mock.patch.object(smoke.sys, "stderr", self.stderr),
            mock.patch.object(smoke, "ensure_container_running") as alive,
        ):
            yield alive

    def run(self) -> dict[str, Any]:
        return smoke.run_model_smoke(
            image="test",
            user_volume="user",
            server_url="https://chat.example",
            room_id="room",
            expected="hello",
            env={},
            agent_container="agent",
        )

    def commands(self) -> list[str]:
        return [call.kwargs["args"][0] for call in self.app.call_args_list]


if __name__ == "__main__":
    unittest.main()

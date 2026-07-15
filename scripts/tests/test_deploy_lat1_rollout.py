import json
import os
from pathlib import Path
import subprocess
import tempfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[2]
ROLLOUT = ROOT / "scripts" / "rollout-lat1-runtime-artifact"


def plan_entry(project: str, runtime: str, machine: str) -> dict[str, str]:
    return {
        "project_id": project,
        "agent_runtime_id": runtime,
        "project_display_name": project,
        "source_host_id": "finite-lat-1",
        "source_machine_id": machine,
        "target_artifact_id": "artifact-v2",
    }


def rollout_report(
    planned: list[dict[str, str]],
    *,
    plan_only: bool = True,
    halted: bool = False,
    halted_reason: str | None = None,
) -> dict[str, object]:
    outcomes = []
    if not plan_only and planned:
        outcomes = [
            {
                "project_id": planned[0]["project_id"],
                "agent_runtime_id": planned[0]["agent_runtime_id"],
                "request_id": "runtime_ctl_test",
                "status": "succeeded",
                "detail": None,
            }
        ]
    return {
        "target_artifact_id": "artifact-v2",
        "source_host_id": "finite-lat-1",
        "plan_only": plan_only,
        "planned": planned,
        "skipped": [],
        "outcomes": outcomes,
        "halted": halted,
        "halted_reason": halted_reason,
    }


class RuntimeRolloutScriptTests(unittest.TestCase):
    def run_rollout(
        self, *args: str, env: dict[str, str] | None = None
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(ROLLOUT), *args],
            cwd=ROOT,
            env={**os.environ, **(env or {})},
            text=True,
            capture_output=True,
            check=False,
        )

    def actor_args(self) -> list[str]:
        return [
            "--roll-runtime-artifact",
            "artifact-v2",
            "--roll-admin-email",
            "admin@finite.vip",
            "--roll-admin-workos-user-id",
            "workos-admin",
        ]

    def fake_ssh_environment(
        self,
        temp: Path,
        plan: dict[str, object],
        *,
        execution_status: int = 0,
        execution_result: dict[str, object] | None = None,
    ) -> tuple[dict[str, str], Path]:
        log = temp / "ssh.log"
        fake_ssh = temp / "ssh"
        fake_ssh.write_text(
            textwrap.dedent(
                """\
                #!/usr/bin/env bash
                set -euo pipefail
                command="${*: -1}"
                printf '%s\n' "$command" >>"$FAKE_SSH_LOG"
                if [[ $command == *"--plan-only"* ]]; then
                  printf '%s\n' "$FAKE_ROLLOUT_PLAN"
                  exit "${FAKE_PLAN_STATUS:-0}"
                fi
                if [[ $command == *"nerdctl --namespace finite inspect"* ]]; then
                  if [[ $command == *"missing-kata"* ]]; then
                    exit 1
                  fi
                  exit 0
                fi
                if [[ ${FAKE_SSH_DRAIN_STDIN:-false} == true && " $* " != *" -n "* ]]; then
                  cat >/dev/null
                fi
                if [[ -n ${FAKE_EXEC_RESULT:-} ]]; then
                  printf '%s\n' "$FAKE_EXEC_RESULT"
                  exit "${FAKE_EXEC_STATUS:-0}"
                fi
                [[ $command =~ --project-id[[:space:]]+([A-Za-z0-9_.-]+) ]]
                project="${BASH_REMATCH[1]}"
                [[ $command =~ --expected-agent-runtime-id[[:space:]]+([A-Za-z0-9_.-]+) ]]
                runtime="${BASH_REMATCH[1]}"
                [[ $command =~ --expected-source-machine-id[[:space:]]+([A-Za-z0-9_.-]+) ]]
                machine="${BASH_REMATCH[1]}"
                printf '{"target_artifact_id":"artifact-v2","source_host_id":"finite-lat-1","plan_only":false,"planned":[{"project_id":"%s","agent_runtime_id":"%s","project_display_name":"%s","source_host_id":"finite-lat-1","source_machine_id":"%s","target_artifact_id":"artifact-v2"}],"skipped":[],"outcomes":[{"project_id":"%s","agent_runtime_id":"%s","request_id":"runtime_ctl_test","status":"succeeded","detail":null}],"halted":false,"halted_reason":null}\n' "$project" "$runtime" "$project" "$machine" "$project" "$runtime"
                exit "${FAKE_EXEC_STATUS:-0}"
                """
            ),
            encoding="utf-8",
        )
        fake_ssh.chmod(0o755)
        environment = {
            "PATH": f"{temp}:{os.environ['PATH']}",
            "LAT1": "root@test-lat1",
            "FAKE_SSH_LOG": str(log),
            "FAKE_ROLLOUT_PLAN": json.dumps(plan),
            "FAKE_EXEC_STATUS": str(execution_status),
        }
        if execution_result is not None:
            environment["FAKE_EXEC_RESULT"] = json.dumps(execution_result)
        return environment, log

    def test_validate_only_accepts_explicit_projects(self) -> None:
        result = self.run_rollout(
            "--validate-only",
            *self.actor_args(),
            "--roll-project-id",
            "project-canary",
            "--roll-project-id",
            "project-next",
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_all_requires_canary_and_cannot_mix_explicit_projects(self) -> None:
        missing_canary = self.run_rollout(
            "--validate-only", *self.actor_args(), "--roll-all"
        )
        self.assertEqual(missing_canary.returncode, 64)

        mixed_scope = self.run_rollout(
            "--validate-only",
            *self.actor_args(),
            "--roll-all",
            "--roll-canary-project-id",
            "project-canary",
            "--roll-project-id",
            "project-other",
        )
        self.assertEqual(mixed_scope.returncode, 64)

    def test_all_plan_is_explicitly_scoped_to_finite_lat_1(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temp = Path(directory)
            env, log = self.fake_ssh_environment(temp, rollout_report([]))
            result = self.run_rollout(
                *self.actor_args(),
                "--roll-all",
                "--roll-canary-project-id",
                "project-canary",
                env=env,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = log.read_text(encoding="utf-8").splitlines()
            self.assertEqual(len(calls), 1, calls)
            self.assertIn("--all", calls[0])
            self.assertIn("--source-host-id finite-lat-1", calls[0])

    def test_missing_canonical_aborts_before_execution(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temp = Path(directory)
            plan = rollout_report(
                [plan_entry("project-canary", "runtime-canary", "missing-kata")]
            )
            env, log = self.fake_ssh_environment(temp, plan)
            result = self.run_rollout(
                *self.actor_args(), "--roll-project-id", "project-canary", env=env
            )
            self.assertNotEqual(result.returncode, 0)
            calls = log.read_text(encoding="utf-8").splitlines()
            self.assertEqual(len(calls), 2, calls)
            self.assertIn("--plan-only", calls[0])
            self.assertIn("missing-kata", calls[1])
            self.assertFalse(any("--expected-agent-runtime-id" in call for call in calls))

    def test_explicit_order_is_preserved_for_exact_execution(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temp = Path(directory)
            plan = rollout_report(
                [
                    plan_entry("project-z-canary", "runtime-z", "kata-z"),
                    plan_entry("project-a-next", "runtime-a", "kata-a"),
                ]
            )
            env, log = self.fake_ssh_environment(temp, plan)
            env["FAKE_SSH_DRAIN_STDIN"] = "true"
            result = self.run_rollout(
                *self.actor_args(),
                "--roll-project-id",
                "project-z-canary",
                "--roll-project-id",
                "project-a-next",
                env=env,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = log.read_text(encoding="utf-8").splitlines()
            exact_calls = [call for call in calls if "--expected-agent-runtime-id" in call]
            self.assertEqual(len(exact_calls), 2, calls)
            self.assertIn("--project-id project-z-canary", exact_calls[0])
            self.assertIn("--project-id project-a-next", exact_calls[1])
            self.assertTrue(
                all("--source-host-id finite-lat-1" in call for call in exact_calls)
            )

    def test_metacharacter_is_rejected_before_first_ssh(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temp = Path(directory)
            env, log = self.fake_ssh_environment(temp, rollout_report([]))
            result = self.run_rollout(
                *self.actor_args(),
                "--roll-project-id",
                "project;touch-pwned",
                env=env,
            )
            self.assertEqual(result.returncode, 64)
            self.assertFalse(log.exists())

    def test_halted_execution_json_is_printed_before_nonzero_propagates(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temp = Path(directory)
            entry = plan_entry("project-canary", "runtime-canary", "kata-canary")
            halted = rollout_report(
                [entry], plan_only=False, halted=True, halted_reason="binding changed"
            )
            halted["outcomes"] = [
                {
                    "project_id": "project-canary",
                    "agent_runtime_id": "runtime-canary",
                    "request_id": None,
                    "status": "enqueue_failed",
                    "detail": "binding changed",
                }
            ]
            env, _ = self.fake_ssh_environment(
                temp, rollout_report([entry]), execution_status=17, execution_result=halted
            )
            result = self.run_rollout(
                *self.actor_args(), "--roll-project-id", "project-canary", env=env
            )
            self.assertEqual(result.returncode, 17, result.stderr)
            printed = "\n".join(
                line for line in result.stdout.splitlines() if not line.startswith("==>")
            )
            self.assertIn('"halted": true', printed)
            self.assertIn('"halted_reason": "binding changed"', printed)

    def test_halted_plan_json_is_printed_before_nonzero_propagates(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temp = Path(directory)
            halted_plan = rollout_report(
                [], halted=True, halted_reason="canary is unavailable on lat1"
            )
            env, _ = self.fake_ssh_environment(temp, halted_plan)
            env["FAKE_PLAN_STATUS"] = "19"
            result = self.run_rollout(
                *self.actor_args(), "--roll-project-id", "project-canary", env=env
            )
            self.assertEqual(result.returncode, 19, result.stderr)
            self.assertIn('"halted": true', result.stdout)
            self.assertIn("canary is unavailable on lat1", result.stdout)

    def test_just_variadic_arguments_are_forwarded_positionally(self) -> None:
        result = subprocess.run(
            [
                "just",
                "--dry-run",
                "deploy-lat1",
                "0" * 40,
                "--roll-project-id",
                "project;touch-pwned",
            ],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        dry_run = result.stdout + result.stderr
        self.assertIn('exec scripts/deploy-lat1 "$@"', dry_run)
        self.assertNotIn("project;touch-pwned", dry_run)


if __name__ == "__main__":
    unittest.main()

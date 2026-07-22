import json
import hashlib
import os
from pathlib import Path
import re
import stat
import subprocess
import tempfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[2]
ROLLOUT = ROOT / "scripts" / "rollout-lat1-runtime-artifact"
TARGET_IMAGE = "ghcr.io/finitecomputer/agent-runtime:v2@sha256:" + "b" * 64
OLD_IMAGE = "ghcr.io/finitecomputer/agent-runtime:v1@sha256:" + "a" * 64


def plan_entry(project: str, runtime: str, machine: str) -> dict[str, str]:
    return {
        "project_id": project,
        "agent_runtime_id": runtime,
        "project_display_name": project,
        "source_host_id": "finite-lat-1",
        "source_machine_id": machine,
        "target_artifact_id": "artifact-v2",
    }


def skipped_entry(
    project: str,
    runtime: str | None,
    machine: str | None,
    reason: str,
) -> dict[str, str | None]:
    return {
        "project_id": project,
        "agent_runtime_id": runtime,
        "project_display_name": project if runtime else None,
        "source_host_id": "finite-lat-1" if runtime else None,
        "source_machine_id": machine,
        "reason": reason,
    }


def rollout_report(
    planned: list[dict[str, str]],
    *,
    skipped: list[dict[str, str | None]] | None = None,
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
        "skipped": skipped or [],
        "outcomes": outcomes,
        "halted": halted,
        "halted_reason": halted_reason,
    }


def provider_fact(
    project: str,
    runtime: str,
    machine: str,
    *,
    artifact: str = "artifact-v1",
    image: str = OLD_IMAGE,
    state: str = "running",
) -> dict[str, object]:
    principal = f"npub1{runtime}principal"
    return {
        "project_id": project,
        "agent_runtime_id": runtime,
        "source_machine_id": machine,
        "current_artifact_id": artifact,
        "image": image,
        "state_schema_version": "state-v1",
        "state": state,
        "data_source": f"/var/lib/finite-saas-runner/kata/{runtime}",
        "ownership": {
            "runtime": "true",
            "source_host_id": "finite-lat-1",
            "source_machine_id": machine,
            "project_id": project,
        },
        "source_machine_owner_count": 1,
        "durable_root_owner_count": 1,
        "agent_principal_sha256": hashlib.sha256(principal.encode()).hexdigest(),
        "test_agent_principal": principal,
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
        facts: list[dict[str, object]],
        *,
        execution_status: int = 0,
        execution_result: dict[str, object] | None = None,
        provider_failure: str | None = None,
    ) -> tuple[dict[str, str], Path, Path]:
        temp.mkdir(parents=True, exist_ok=True)
        log = temp / "ssh.log"
        state_dir = temp / "provider-state"
        state_dir.mkdir()
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
                if [[ $command == *"provider-snapshot-v1"* ]]; then
                  suffix="${command##*provider-snapshot-v1}"
                  read -r -a values <<<"$suffix"
                  (( ${#values[@]} % 3 == 0 ))
                  printf 'SYSTEM\t%s\n' "$FAKE_REMOTE_SYSTEM_CLOSURE"
                  for ((index=0; index<${#values[@]}; index+=3)); do
                    runtime="${values[index]}"
                    machine="${values[index+1]}"
                    project="${values[index+2]}"
                    fact="$(jq -ce --arg runtime "$runtime" --arg machine "$machine" --arg project "$project" '.[] | select(.agent_runtime_id == $runtime and .source_machine_id == $machine and .project_id == $project)' <<<"$FAKE_PROVIDER_FACTS")"
                    if [[ -f "$FAKE_STATE_DIR/upgraded-$machine" ]]; then
                      fact="$(jq -c --arg artifact artifact-v2 --arg image "$FAKE_TARGET_IMAGE" '.current_artifact_id = $artifact | .image = $image' <<<"$fact")"
                    fi
                    state="$(jq -r '.state' <<<"$fact")"
                    if [[ ${FAKE_PROVIDER_FAILURE:-} == "canonical is stopped" ]]; then
                      state=stopped
                    fi
                    image="$(jq -r '.image' <<<"$fact")"
                    artifact="$(jq -r '.current_artifact_id' <<<"$fact")"
                    schema="$(jq -r '.state_schema_version' <<<"$fact")"
                    root="$(jq -r '.data_source' <<<"$fact")"
                    mounts="$(jq -cn --arg root "$root" '[{Source:$root,Destination:"/data",RW:true}]')"
                    ports='{"8080/tcp":[{"HostIp":"127.0.0.1","HostPort":"41001"}]}'
                    printf 'CANONICAL\t%s\t%s\t%s\t%s\t%s\t%s\t%s\ttrue\tfinite-lat-1\t%s\t%s\t%s\t%s\n' "$runtime" "$machine" "$project" "$state" "$image" "$artifact" "$schema" "$machine" "$project" "$mounts" "$ports"
                  done
                  while IFS=$'\t' read -r runtime machine project root; do
                    if [[ ${FAKE_PROVIDER_FAILURE:-} == "missing container topology" ]]; then
                      continue
                    fi
                    mounts="$(jq -cn --arg root "$root" '[{Source:$root,Destination:"/data",RW:true}]')"
                    printf 'TOPOLOGY\t%s\t%s\t%s\n' "$machine" "$machine" "$mounts"
                    if [[ ${FAKE_PROVIDER_FAILURE:-} == "ambiguous container topology" ]]; then
                      printf 'TOPOLOGY\thelper-%s\t%s\t%s\n' "$machine" "$machine" "$mounts"
                    fi
                  done < <(jq -r '.[] | [.agent_runtime_id,.source_machine_id,.project_id,.data_source] | @tsv' <<<"$FAKE_PROVIDER_FACTS")
                  exit 0
                fi
                if [[ $command == *"provider-contact-v1"* ]]; then
                  suffix="${command##*provider-contact-v1}"
                  read -r -a values <<<"$suffix"
                  (( ${#values[@]} % 4 == 0 ))
                  for ((index=0; index<${#values[@]}; index+=4)); do
                    runtime="${values[index]}"
                    machine="${values[index+1]}"
                    project="${values[index+2]}"
                    principal="$(jq -er --arg runtime "$runtime" '.[] | select(.agent_runtime_id == $runtime) | .test_agent_principal' <<<"$FAKE_PROVIDER_FACTS")"
                    printf '\036CONTACT\t%s\t%s\t%s\n{"agent_npub":"%s"}\n' "$runtime" "$machine" "$project" "$principal"
                  done
                  exit 0
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
                touch "$FAKE_STATE_DIR/upgraded-$machine"
                printf '{"target_artifact_id":"artifact-v2","source_host_id":"finite-lat-1","plan_only":false,"planned":[{"project_id":"%s","agent_runtime_id":"%s","project_display_name":"%s","source_host_id":"finite-lat-1","source_machine_id":"%s","target_artifact_id":"artifact-v2"}],"skipped":[],"outcomes":[{"project_id":"%s","agent_runtime_id":"%s","request_id":"runtime_ctl_test","status":"succeeded","detail":null}],"halted":false,"halted_reason":null}\n' "$project" "$runtime" "$project" "$machine" "$project" "$runtime"
                exit "${FAKE_EXEC_STATUS:-0}"
                """
            ),
            encoding="utf-8",
        )
        fake_ssh.chmod(0o755)
        state_root = temp / "rollout-evidence"
        environment = {
            "PATH": f"{temp}:{os.environ['PATH']}",
            "LAT1": "root@test-lat1",
            "ROLLOUT_STATE_ROOT": str(state_root),
            "FAKE_SSH_LOG": str(log),
            "FAKE_STATE_DIR": str(state_dir),
            "FAKE_ROLLOUT_PLAN": json.dumps(plan),
            "FAKE_PROVIDER_FACTS": json.dumps(facts),
            "FAKE_TARGET_IMAGE": TARGET_IMAGE,
            "FAKE_REMOTE_SYSTEM_CLOSURE": "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-nixos-system-lat1-test",
            "FAKE_EXEC_STATUS": str(execution_status),
        }
        if execution_result is not None:
            environment["FAKE_EXEC_RESULT"] = json.dumps(execution_result)
        if provider_failure is not None:
            environment["FAKE_PROVIDER_FAILURE"] = provider_failure
        return environment, log, state_root

    def prepare(
        self,
        env: dict[str, str],
        *scope: str,
    ) -> tuple[subprocess.CompletedProcess[str], str]:
        result = self.run_rollout("--prepare", *self.actor_args(), *scope, env=env)
        match = re.search(r"plan hash: ([0-9a-f]{64})", result.stdout)
        return result, match.group(1) if match else ""

    def test_mode_is_required_exclusive_and_hash_is_exact_lowercase(self) -> None:
        no_mode = self.run_rollout(
            "--validate-only", *self.actor_args(), "--roll-project-id", "project-a"
        )
        self.assertEqual(no_mode.returncode, 64)

        both = self.run_rollout(
            "--validate-only",
            "--prepare",
            "--execute-plan-hash",
            "0" * 64,
            *self.actor_args(),
            "--roll-project-id",
            "project-a",
        )
        self.assertEqual(both.returncode, 64)

        for invalid_hash in ("0" * 63, "A" * 64, "g" * 64):
            invalid = self.run_rollout(
                "--validate-only",
                "--execute-plan-hash",
                invalid_hash,
                *self.actor_args(),
                "--roll-project-id",
                "project-a",
            )
            self.assertEqual(invalid.returncode, 64)

    def test_validate_only_checks_complete_argument_shapes_without_ssh(self) -> None:
        explicit = self.run_rollout(
            "--validate-only",
            "--prepare",
            *self.actor_args(),
            "--roll-project-id",
            "project-a",
        )
        self.assertEqual(explicit.returncode, 0, explicit.stderr)

        all_scope = self.run_rollout(
            "--validate-only",
            "--execute-plan-hash",
            "0" * 64,
            *self.actor_args(),
            "--roll-all",
            "--roll-canary-project-id",
            "project-canary",
        )
        self.assertEqual(all_scope.returncode, 0, all_scope.stderr)

        incomplete = self.run_rollout(
            "--validate-only", "--prepare", *self.actor_args(), "--roll-all"
        )
        self.assertEqual(incomplete.returncode, 64)

    def test_prepare_sorts_and_persists_canonical_plan_without_enqueue(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temp = Path(directory)
            entries = [
                plan_entry("project-z", "runtime-z", "kata-z"),
                plan_entry("project-a", "runtime-a", "kata-a"),
            ]
            facts = [
                provider_fact("project-z", "runtime-z", "kata-z"),
                provider_fact("project-a", "runtime-a", "kata-a"),
            ]
            env, log, state_root = self.fake_ssh_environment(
                temp, rollout_report(entries), facts
            )
            result, plan_hash = self.prepare(
                env,
                "--roll-project-id",
                "project-z",
                "--roll-project-id",
                "project-a",
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertRegex(plan_hash, r"^[0-9a-f]{64}$")
            self.assertIn("--execute-plan-hash", result.stdout)

            calls = log.read_text(encoding="utf-8").splitlines()
            self.assertEqual(len(calls), 3, calls)
            self.assertTrue(any("--plan-only" in call for call in calls))
            self.assertFalse(any("--expected-agent-runtime-id" in call for call in calls))

            evidence_dir = state_root / plan_hash
            plan_path = evidence_dir / "plan.json"
            events_path = evidence_dir / "events.jsonl"
            saved = json.loads(plan_path.read_text(encoding="utf-8"))
            self.assertNotIn("actor", saved)
            self.assertNotIn("wait_timeout_seconds", saved)
            self.assertNotIn("project_display_name", plan_path.read_text(encoding="utf-8"))
            self.assertRegex(saved["repo_revision"], r"^[0-9a-f]{40}$")
            self.assertEqual(
                saved["remote_system_closure"], env["FAKE_REMOTE_SYSTEM_CLOSURE"]
            )
            self.assertNotIn("npub1", plan_path.read_text(encoding="utf-8"))
            self.assertEqual(
                [entry["project_id"] for entry in saved["planned"]],
                ["project-a", "project-z"],
            )
            self.assertEqual(stat.S_IMODE(evidence_dir.stat().st_mode), 0o700)
            self.assertEqual(stat.S_IMODE(plan_path.stat().st_mode), 0o600)
            self.assertEqual(stat.S_IMODE(events_path.stat().st_mode), 0o600)
            events = [
                json.loads(line) for line in events_path.read_text(encoding="utf-8").splitlines()
            ]
            self.assertEqual([event["event"] for event in events], ["start", "final"])
            self.assertEqual(events[-1]["status"], "success")

    def test_all_requires_already_target_canary_and_provider_preflights_it(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temp = Path(directory)
            planned = [plan_entry("project-next", "runtime-next", "kata-next")]
            canary_skip = skipped_entry(
                "project-canary",
                "runtime-canary",
                "kata-canary",
                "already_on_target_artifact",
            )
            facts = [
                provider_fact("project-next", "runtime-next", "kata-next"),
                provider_fact(
                    "project-canary",
                    "runtime-canary",
                    "kata-canary",
                    artifact="artifact-v2",
                    image=TARGET_IMAGE,
                ),
            ]
            env, log, _ = self.fake_ssh_environment(
                temp, rollout_report(planned, skipped=[canary_skip]), facts
            )
            result, _ = self.prepare(
                env,
                "--roll-all",
                "--roll-canary-project-id",
                "project-canary",
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            provider_call = next(
                call for call in log.read_text(encoding="utf-8").splitlines()
                if "provider-snapshot-v1" in call
            )
            self.assertIn("runtime-canary", provider_call)
            self.assertIn("runtime-next", provider_call)

            bad_env, _, _ = self.fake_ssh_environment(
                temp / "bad",
                rollout_report(planned),
                [provider_fact("project-next", "runtime-next", "kata-next")],
            )
            rejected, _ = self.prepare(
                bad_env,
                "--roll-all",
                "--roll-canary-project-id",
                "project-canary",
            )
            self.assertNotEqual(rejected.returncode, 0)

    def test_explicit_core_skip_hard_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temp = Path(directory)
            plan = rollout_report(
                [],
                skipped=[skipped_entry("project-a", None, None, "project_not_found")],
            )
            env, log, _ = self.fake_ssh_environment(temp, plan, [])
            result, _ = self.prepare(env, "--roll-project-id", "project-a")
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(len(log.read_text(encoding="utf-8").splitlines()), 1)

    def test_all_excludes_nonrunning_runtime_without_contact_or_enqueue(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temp = Path(directory)
            running = plan_entry("project-running", "runtime-running", "kata-running")
            offline = plan_entry("project-offline", "runtime-offline", "kata-offline")
            canary_skip = skipped_entry(
                "project-canary",
                "runtime-canary",
                "kata-canary",
                "already_on_target_artifact",
            )
            facts = [
                provider_fact("project-running", "runtime-running", "kata-running"),
                provider_fact(
                    "project-offline",
                    "runtime-offline",
                    "kata-offline",
                    state="exited",
                ),
                provider_fact(
                    "project-canary",
                    "runtime-canary",
                    "kata-canary",
                    artifact="artifact-v2",
                    image=TARGET_IMAGE,
                ),
            ]
            env, log, state_root = self.fake_ssh_environment(
                temp,
                rollout_report([running, offline], skipped=[canary_skip]),
                facts,
            )
            scope = (
                "--roll-all",
                "--roll-canary-project-id",
                "project-canary",
            )
            prepared, plan_hash = self.prepare(env, *scope)
            self.assertEqual(prepared.returncode, 0, prepared.stderr)
            self.assertIn("provider_not_running=1", prepared.stdout)
            saved = json.loads(
                (state_root / plan_hash / "plan.json").read_text(encoding="utf-8")
            )
            self.assertEqual(
                [entry["agent_runtime_id"] for entry in saved["planned"]],
                ["runtime-running"],
            )
            self.assertEqual(saved["excluded"][0]["agent_runtime_id"], "runtime-offline")
            self.assertEqual(saved["excluded"][0]["reason"], "provider_not_running")
            self.assertEqual(saved["excluded"][0]["provider_facts"]["state"], "exited")
            self.assertIsNone(
                saved["excluded"][0]["provider_facts"]["agent_principal_sha256"]
            )
            prepare_contacts = [
                call
                for call in log.read_text(encoding="utf-8").splitlines()
                if "provider-contact-v1" in call
            ]
            self.assertTrue(prepare_contacts)
            self.assertTrue(all("runtime-offline" not in call for call in prepare_contacts))

            log.write_text("", encoding="utf-8")
            executed = self.run_rollout(
                "--execute-plan-hash",
                plan_hash,
                *self.actor_args(),
                *scope,
                env=env,
            )
            self.assertEqual(executed.returncode, 0, executed.stderr)
            exact = [
                call
                for call in log.read_text(encoding="utf-8").splitlines()
                if "--expected-agent-runtime-id" in call
            ]
            self.assertEqual(len(exact), 1)
            self.assertIn("runtime-running", exact[0])
            self.assertNotIn("runtime-offline", exact[0])

    def test_execute_recomputes_hash_then_rechecks_each_entry_and_postflight(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temp = Path(directory)
            entries = [
                plan_entry("project-z", "runtime-z", "kata-z"),
                plan_entry("project-a", "runtime-a", "kata-a"),
            ]
            facts = [
                provider_fact("project-z", "runtime-z", "kata-z", artifact="artifact-v2", image=TARGET_IMAGE),
                provider_fact("project-a", "runtime-a", "kata-a", artifact="artifact-v2", image=TARGET_IMAGE),
            ]
            env, log, state_root = self.fake_ssh_environment(
                temp, rollout_report(entries), facts
            )
            scope = (
                "--roll-project-id",
                "project-z",
                "--roll-project-id",
                "project-a",
            )
            prepared, plan_hash = self.prepare(env, *scope)
            self.assertEqual(prepared.returncode, 0, prepared.stderr)
            log.write_text("", encoding="utf-8")

            executed = self.run_rollout(
                "--execute-plan-hash",
                plan_hash,
                *self.actor_args(),
                *scope,
                env=env,
            )
            self.assertEqual(executed.returncode, 0, executed.stderr)
            calls = log.read_text(encoding="utf-8").splitlines()
            exact = [call for call in calls if "--expected-agent-runtime-id" in call]
            self.assertEqual(len(exact), 2, calls)
            self.assertIn("--project-id project-a", exact[0])
            self.assertIn("--project-id project-z", exact[1])
            provider = [call for call in calls if "provider-snapshot-v1" in call]
            self.assertEqual(len(provider), 5, calls)  # full recompute, then pre/post per entry

            events = [
                json.loads(line)
                for line in (state_root / plan_hash / "events.jsonl")
                .read_text(encoding="utf-8")
                .splitlines()
            ]
            execute_events = [event for event in events if event["phase"] == "execute"]
            self.assertEqual(execute_events[0]["event"], "start")
            self.assertEqual(execute_events[-1]["event"], "final")
            self.assertEqual(execute_events[-1]["status"], "success")
            self.assertEqual(
                sum(event["event"] == "entry_postflight" for event in execute_events), 2
            )
            postflights = [
                event for event in execute_events if event["event"] == "entry_postflight"
            ]
            self.assertTrue(all("provider_facts" in event for event in postflights))
            self.assertNotIn("npub1", (state_root / plan_hash / "events.jsonl").read_text(encoding="utf-8"))

    def test_hash_drift_refuses_before_first_enqueue(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temp = Path(directory)
            entry = plan_entry("project-a", "runtime-a", "kata-a")
            original = provider_fact("project-a", "runtime-a", "kata-a")
            env, log, _ = self.fake_ssh_environment(
                temp, rollout_report([entry]), [original]
            )
            scope = ("--roll-project-id", "project-a")
            prepared, plan_hash = self.prepare(env, *scope)
            self.assertEqual(prepared.returncode, 0, prepared.stderr)

            drifted = {**original, "test_agent_principal": "npub1changedprincipal"}
            env["FAKE_PROVIDER_FACTS"] = json.dumps([drifted])
            log.write_text("", encoding="utf-8")
            executed = self.run_rollout(
                "--execute-plan-hash",
                plan_hash,
                *self.actor_args(),
                *scope,
                env=env,
            )
            self.assertNotEqual(executed.returncode, 0)
            self.assertIn("does not match approved hash", executed.stderr)
            self.assertFalse(
                any(
                    "--expected-agent-runtime-id" in call
                    for call in log.read_text(encoding="utf-8").splitlines()
                )
            )

    def test_explicit_nonrunning_or_invalid_topology_fails_during_prepare(self) -> None:
        for failure, expected, state in (
            (None, "explicitly requested Runtime was excluded", "exited"),
            ("ambiguous container topology", "ambiguous container topology", "running"),
            ("missing container topology", "ambiguous container topology", "running"),
        ):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                temp = Path(directory)
                entry = plan_entry("project-a", "runtime-a", "kata-a")
                env, _, _ = self.fake_ssh_environment(
                    temp,
                    rollout_report([entry]),
                    [provider_fact("project-a", "runtime-a", "kata-a", state=state)],
                    provider_failure=failure,
                )
                result, _ = self.prepare(env, "--roll-project-id", "project-a")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(expected, result.stderr)

    def test_core_failure_is_retained_and_final_event_is_failure(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temp = Path(directory)
            entry = plan_entry("project-a", "runtime-a", "kata-a")
            facts = [
                provider_fact(
                    "project-a",
                    "runtime-a",
                    "kata-a",
                    artifact="artifact-v2",
                    image=TARGET_IMAGE,
                )
            ]
            halted = rollout_report(
                [entry], plan_only=False, halted=True, halted_reason="binding changed"
            )
            halted["outcomes"] = [
                {
                    "project_id": "project-a",
                    "agent_runtime_id": "runtime-a",
                    "request_id": None,
                    "status": "enqueue_failed",
                    "detail": "binding changed",
                }
            ]
            env, _, state_root = self.fake_ssh_environment(
                temp,
                rollout_report([entry]),
                facts,
                execution_status=17,
                execution_result=halted,
            )
            scope = ("--roll-project-id", "project-a")
            prepared, plan_hash = self.prepare(env, *scope)
            self.assertEqual(prepared.returncode, 0, prepared.stderr)
            executed = self.run_rollout(
                "--execute-plan-hash",
                plan_hash,
                *self.actor_args(),
                *scope,
                env=env,
            )
            self.assertEqual(executed.returncode, 17, executed.stderr)
            events = [
                json.loads(line)
                for line in (state_root / plan_hash / "events.jsonl")
                .read_text(encoding="utf-8")
                .splitlines()
            ]
            self.assertEqual(events[-1]["event"], "final")
            self.assertEqual(events[-1]["status"], "failure")
            self.assertEqual(events[-1]["exit_code"], 17)

    def test_metacharacter_is_rejected_before_first_ssh(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temp = Path(directory)
            env, log, _ = self.fake_ssh_environment(temp, rollout_report([]), [])
            result = self.run_rollout(
                "--prepare",
                *self.actor_args(),
                "--roll-project-id",
                "project;touch-pwned",
                env=env,
            )
            self.assertEqual(result.returncode, 64)
            self.assertFalse(log.exists())

    def test_every_ssh_call_uses_n(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temp = Path(directory)
            entry = plan_entry("project-a", "runtime-a", "kata-a")
            env, _, _ = self.fake_ssh_environment(
                temp,
                rollout_report([entry]),
                [provider_fact("project-a", "runtime-a", "kata-a")],
            )
            prepared, _ = self.prepare(env, "--roll-project-id", "project-a")
            self.assertEqual(prepared.returncode, 0, prepared.stderr)
            invocations = (temp / "ssh.log").read_text(encoding="utf-8")
            self.assertNotIn("secret", invocations.lower())
            provider_commands = [
                call
                for call in invocations.splitlines()
                if "provider-snapshot-v1" in call or "provider-contact-v1" in call
            ]
            self.assertTrue(provider_commands)
            self.assertTrue(
                all("jq" not in call and "sha256sum" not in call for call in provider_commands)
            )
            # The fake receives only the remote command, so inspect the script source for
            # the invariant that protects loops/process substitutions from SSH stdin.
            source = ROLLOUT.read_text(encoding="utf-8")
            self.assertIn('ssh -n -o BatchMode=yes -- "$LAT1"', source)


if __name__ == "__main__":
    unittest.main()

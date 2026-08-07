#!/usr/bin/env python3
"""Synthetic safety tests for finite-lat-2 runner operator scripts."""

from __future__ import annotations

import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest


ROOT = Path(__file__).resolve().parents[2]
MAINTENANCE = ROOT / "infra/hosts/lat2/runner-maintenance"
RESTART = ROOT / "infra/hosts/lat2/restart-idle-runner"
SERVICE = ROOT / "infra/hosts/lat2/systemd/finite-lat2-runner-maintenance.service"
WORKFLOW = ROOT / ".github/workflows/ci.yml"


class Lat2RunnerGuardrailTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.temp = Path(self.temporary.name)
        self.bin = self.temp / "bin"
        self.bin.mkdir()
        self.command_log = self.temp / "commands.log"
        self._write_fake_commands()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def _write_executable(self, name: str, body: str) -> None:
        path = self.bin / name
        path.write_text("#!/usr/bin/env bash\nset -euo pipefail\n" + body, encoding="utf-8")
        path.chmod(0o755)

    def _write_fake_commands(self) -> None:
        self._write_executable(
            "df",
            "printf 'Filesystem 1024-blocks Used Available Capacity Mounted on\\n'\n"
            "printf '/dev/fake 100 1 99 %s%% /\\n' \"${FAKE_DISK_PERCENT:-50}\"\n",
        )
        self._write_executable(
            "pgrep", '[[ "${FAKE_WORKER_ACTIVE:-0}" == "1" ]]\n'
        )
        self._write_executable(
            "docker",
            'printf \'docker %s\\n\' "$*" >>"$FAKE_COMMAND_LOG"\n'
            '[[ "${FAKE_DOCKER_FAIL:-0}" != "1" ]]\n',
        )
        self._write_executable(
            "systemctl",
            'printf \'systemctl %s\\n\' "$*" >>"$FAKE_COMMAND_LOG"\n'
            '[[ "$1" == "is-active" || "$1" == "restart" ]]\n',
        )
        self._write_executable("sudo", 'exec "$@"\n')
        self._write_executable(
            "timeout",
            'while [[ "$1" == -* ]]; do shift; done\nshift\nexec "$@"\n',
        )
        self._write_executable(
            "gh",
            "endpoint=\"${2:-}\"\n"
            "if [[ \"$endpoint\" == *'/actions/runners?'* ]]; then\n"
            "  printf '{\"runners\":[{\"name\":\"finite-lat-2-mono\",\"status\":\"online\",\"busy\":%s}]}' \"${FAKE_RUNNER_BUSY:-false}\"\n"
            "elif [[ \"$endpoint\" == *'/actions/runs/42/jobs?'* ]]; then\n"
            "  printf '{\"total_count\":1,\"jobs\":[{\"id\":7,\"status\":\"in_progress\",\"runner_name\":\"finite-lat-2-mono\",\"html_url\":\"https://example.invalid/job/7\"}]}'\n"
            "elif [[ \"${FAKE_ASSIGNED_JOB:-0}\" == \"1\" && \"$endpoint\" == *'status=in_progress'* ]]; then\n"
            "  printf '{\"total_count\":1,\"workflow_runs\":[{\"id\":42}]}'\n"
            "else\n"
            "  printf '{\"total_count\":0,\"workflow_runs\":[]}'\n"
            "fi\n",
        )

    def environment(self, **extra: str) -> dict[str, str]:
        return {
            **os.environ,
            "PATH": f"{self.bin}:{os.environ['PATH']}",
            "FAKE_COMMAND_LOG": str(self.command_log),
            **extra,
        }

    def run_maintenance(
        self, runner_root: Path, **extra: str
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(MAINTENANCE)],
            env=self.environment(
                FINITE_LAT2_RUNNER_ROOT=str(runner_root),
                FINITE_LAT2_DISK_PATH=str(self.temp),
                FINITE_LAT2_SCRATCH_MINUTES="60",
                **extra,
            ),
            check=False,
            capture_output=True,
            text=True,
        )

    def test_idle_cleanup_removes_only_aged_named_checkouts(self) -> None:
        workspace = self.temp / "runners/mono/_work/finite-mono/finite-mono"
        aged_brain = workspace / "brain-matrix-1-1"
        aged_nix = workspace / "nix-packages-1-1"
        fresh_brain = workspace / "brain-matrix-2-1"
        unrelated = workspace / "keep-me"
        for path in (aged_brain, aged_nix, fresh_brain, unrelated):
            path.mkdir(parents=True)
            (path / "artifact").write_text("synthetic", encoding="utf-8")
        old = time.time() - 7200
        for path in (aged_brain, aged_nix):
            os.utime(path / "artifact", (old, old))
            os.utime(path, (old, old))

        result = self.run_maintenance(self.temp / "runners")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(aged_brain.exists())
        self.assertFalse(aged_nix.exists())
        self.assertTrue(fresh_brain.exists())
        self.assertTrue(unrelated.exists())
        commands = self.command_log.read_text(encoding="utf-8")
        self.assertIn("docker container prune --force --filter until=24h", commands)
        self.assertIn("docker image prune --all --force --filter until=168h", commands)
        self.assertIn("--keep-storage 32GB", commands)

    def test_active_worker_skips_every_destructive_operation(self) -> None:
        scratch = self.temp / "runners/mono/_work/repo/repo/brain-matrix-1-1"
        scratch.mkdir(parents=True)
        old = time.time() - 7200
        os.utime(scratch, (old, old))

        result = self.run_maintenance(
            self.temp / "runners", FAKE_WORKER_ACTIVE="1"
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(scratch.exists())
        self.assertFalse(self.command_log.exists())
        self.assertIn("skipped all destructive cleanup", result.stdout)

    def test_disk_watermarks_warn_and_fail_critically(self) -> None:
        warning = self.run_maintenance(
            self.temp / "missing-runners", FAKE_DISK_PERCENT="80"
        )
        self.assertEqual(warning.returncode, 0, warning.stderr)
        self.assertIn("WARNING", warning.stderr)

        critical = self.run_maintenance(
            self.temp / "missing-runners", FAKE_DISK_PERCENT="90"
        )
        self.assertEqual(critical.returncode, 2)
        self.assertIn("CRITICAL", critical.stderr)

    def test_systemd_service_runs_cleanup_as_root(self) -> None:
        unit = SERVICE.read_text(encoding="utf-8")
        self.assertIn("User=root", unit)
        self.assertIn("ExecStart=/usr/local/sbin/finite-lat2-runner-maintenance", unit)

    def test_default_scratch_grace_exceeds_job_timeout_without_one_day_burst(self) -> None:
        maintenance = MAINTENANCE.read_text(encoding="utf-8")
        self.assertIn('FINITE_LAT2_SCRATCH_MINUTES:-180', maintenance)

    def test_self_hosted_jobs_remove_only_their_exact_checkouts(self) -> None:
        workflow = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("Remove Nix package checkout", workflow)
        self.assertIn("Remove Brain matrix checkout", workflow)
        self.assertEqual(workflow.count('if [[ "$CHECKOUT_PATH" != "$expected"'), 2)
        self.assertEqual(
            workflow.count('sudo -n find "$CHECKOUT_PATH" -xdev -depth -delete'),
            2,
        )

    def run_restart(self, **extra: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                "bash",
                str(RESTART),
                "finitecomputer/finite-mono",
                "finite-lat-2-mono",
                "actions.runner.finitecomputer-finite-mono.finite-lat-2-mono.service",
            ],
            env=self.environment(**extra),
            check=False,
            capture_output=True,
            text=True,
        )

    def test_stale_busy_lease_restarts_when_no_job_or_worker_exists(self) -> None:
        result = self.run_restart(FAKE_RUNNER_BUSY="true")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("busy lease is stale", result.stdout)
        self.assertIn(
            "systemctl restart actions.runner",
            self.command_log.read_text(encoding="utf-8"),
        )

    def test_assigned_job_aborts_listener_restart(self) -> None:
        result = self.run_restart(
            FAKE_RUNNER_BUSY="true", FAKE_ASSIGNED_JOB="1"
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("active job(s) are assigned", result.stderr)
        commands = (
            self.command_log.read_text(encoding="utf-8")
            if self.command_log.exists()
            else ""
        )
        self.assertNotIn("systemctl restart", commands)

    def test_local_worker_aborts_listener_restart(self) -> None:
        result = self.run_restart(
            FAKE_RUNNER_BUSY="false", FAKE_WORKER_ACTIVE="1"
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("local Runner.Worker process is active", result.stderr)
        commands = (
            self.command_log.read_text(encoding="utf-8")
            if self.command_log.exists()
            else ""
        )
        self.assertNotIn("systemctl restart", commands)


if __name__ == "__main__":
    unittest.main()

"""Unit checks for the container entrypoint restore behavior."""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
ENTRYPOINT = REPO_ROOT / "containers" / "agent" / "entrypoint.sh"


def install_gnu_realpath_fixture(fake_bin: Path) -> None:
    """Provide the Linux `realpath -m` contract when unit tests run on macOS."""
    realpath = fake_bin / "realpath"
    realpath.write_text(
        "#!/usr/bin/env python3\n"
        "import os\n"
        "import sys\n"
        "paths = [value for value in sys.argv[1:] if value != '-m']\n"
        "for value in paths:\n"
        "    print(os.path.realpath(value))\n",
        encoding="utf-8",
    )
    realpath.chmod(0o755)


class AgentEntrypointTest(unittest.TestCase):
    def test_runs_command_without_restore(self) -> None:
        result = subprocess.run(
            [str(ENTRYPOINT), "sh", "-c", "echo command-ran"],
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertIn("command-ran", result.stdout)

    def test_finite_home_defaults_to_agent_home(self) -> None:
        # The shared Finite identity (identity/identity.json) must land on the
        # durable agent mount so the account key survives restarts and rides
        # along with restic backup/restore of the agent home.
        with tempfile.TemporaryDirectory() as tmp_value:
            home = Path(tmp_value) / "agent"
            env = os.environ.copy()
            env.pop("FINITE_HOME", None)
            env["FINITECHAT_HOME"] = str(home)
            result = subprocess.run(
                [str(ENTRYPOINT), "sh", "-c", 'echo "finite-home=$FINITE_HOME"'],
                capture_output=True,
                text=True,
                env=env,
                check=True,
            )
        self.assertIn(f"finite-home={home}", result.stdout)

    def test_finite_home_override_is_honored(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            env = os.environ.copy()
            env.update(
                {
                    "FINITECHAT_HOME": str(tmp / "agent"),
                    "FINITE_HOME": str(tmp / "identity-home"),
                }
            )
            result = subprocess.run(
                [str(ENTRYPOINT), "sh", "-c", 'echo "finite-home=$FINITE_HOME"'],
                capture_output=True,
                text=True,
                env=env,
                check=True,
            )
        self.assertIn(f"finite-home={tmp / 'identity-home'}", result.stdout)

    def test_restore_requires_exact_snapshot_id(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            env = os.environ.copy()
            env.update(
                {
                    "FINITECHAT_HOME": str(Path(tmp_value) / "agent"),
                    "FINITE_AGENT_RESTORE_ON_START": "1",
                    "FINITE_AGENT_RESTIC_REPOSITORY": "s3:https://example.invalid/bucket/prefix",
                    "FINITE_AGENT_RESTIC_PASSWORD": "secret",
                }
            )
            result = subprocess.run(
                [str(ENTRYPOINT), "sh", "-c", "echo should-not-run"],
                capture_output=True,
                text=True,
                env=env,
                check=False,
            )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "missing FINITE_AGENT_RESTIC_SNAPSHOT_ID or FINITE_AGENT_RESTORE_LATEST=1",
            result.stderr,
        )
        self.assertNotIn("should-not-run", result.stdout)

    def test_restore_runs_before_command(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            fake_bin = tmp / "bin"
            fake_bin.mkdir()
            fake_restic = fake_bin / "restic"
            log_path = tmp / "restic.log"
            home = tmp / "agent"
            fake_restic.write_text(
                "#!/usr/bin/env sh\n"
                'echo "$@" > "$RESTIC_FAKE_LOG"\n'
                'test "$RESTIC_PASSWORD" = secret\n'
                'mkdir -p "$FINITECHAT_HOME"\n'
                "echo '{}' > \"$FINITECHAT_HOME/config.json\"\n",
                encoding="utf-8",
            )
            fake_restic.chmod(0o755)
            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{fake_bin}:{env['PATH']}",
                    "RESTIC_FAKE_LOG": str(log_path),
                    "FINITECHAT_HOME": str(home),
                    "FINITE_AGENT_RESTORE_ON_START": "1",
                    "FINITE_AGENT_RESTIC_REPOSITORY": "s3:https://example.invalid/bucket/prefix",
                    "FINITE_AGENT_RESTIC_PASSWORD": "secret",
                    "FINITE_AGENT_RESTIC_SNAPSHOT_ID": "snapshot-123",
                }
            )
            result = subprocess.run(
                [
                    str(ENTRYPOINT),
                    "sh",
                    "-c",
                    'test -f "$FINITECHAT_HOME/config.json" && echo restored',
                ],
                capture_output=True,
                text=True,
                env=env,
                check=True,
            )
            self.assertIn("FINITE_AGENT_RESTORE_COMPLETE", result.stdout)
            self.assertIn("restored", result.stdout)
            self.assertEqual(
                log_path.read_text(encoding="utf-8").strip(),
                "-r s3:https://example.invalid/bucket/prefix restore snapshot-123 --target /",
            )

    def test_restore_latest_uses_backup_tag(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            fake_bin = tmp / "bin"
            fake_bin.mkdir()
            fake_restic = fake_bin / "restic"
            log_path = tmp / "restic.log"
            home = tmp / "agent"
            fake_restic.write_text(
                "#!/usr/bin/env sh\n"
                'echo "$@" > "$RESTIC_FAKE_LOG"\n'
                'test "$RESTIC_PASSWORD" = secret\n'
                'mkdir -p "$FINITECHAT_HOME"\n'
                "echo '{}' > \"$FINITECHAT_HOME/config.json\"\n",
                encoding="utf-8",
            )
            fake_restic.chmod(0o755)
            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{fake_bin}:{env['PATH']}",
                    "RESTIC_FAKE_LOG": str(log_path),
                    "FINITECHAT_HOME": str(home),
                    "FINITE_AGENT_RESTORE_ON_START": "1",
                    "FINITE_AGENT_RESTORE_LATEST": "1",
                    "FINITE_AGENT_RESTIC_REPOSITORY": "s3:https://example.invalid/bucket/prefix",
                    "FINITE_AGENT_RESTIC_PASSWORD": "secret",
                    "FINITE_AGENT_RESTIC_BACKUP_TAG": "finite-agent-state",
                }
            )
            result = subprocess.run(
                [
                    str(ENTRYPOINT),
                    "sh",
                    "-c",
                    'test -f "$FINITECHAT_HOME/config.json" && echo restored',
                ],
                capture_output=True,
                text=True,
                env=env,
                check=True,
            )
            self.assertIn("FINITE_AGENT_RESTORE_COMPLETE snapshot=latest", result.stdout)
            self.assertIn("restored", result.stdout)
            self.assertEqual(
                log_path.read_text(encoding="utf-8").strip(),
                "-r s3:https://example.invalid/bucket/prefix restore latest --tag finite-agent-state --target /",
            )

    def test_existing_state_skips_restore_without_force(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            home = tmp / "agent"
            home.mkdir()
            (home / "config.json").write_text("{}", encoding="utf-8")
            env = os.environ.copy()
            env.update(
                {
                    "FINITECHAT_HOME": str(home),
                    "FINITE_AGENT_RESTORE_ON_START": "1",
                    "FINITE_AGENT_RESTIC_REPOSITORY": "s3:https://example.invalid/bucket/prefix",
                    "FINITE_AGENT_RESTIC_PASSWORD": "secret",
                    "FINITE_AGENT_RESTIC_SNAPSHOT_ID": "snapshot-123",
                }
            )
            result = subprocess.run(
                [str(ENTRYPOINT), "sh", "-c", "echo skipped-ok"],
                capture_output=True,
                text=True,
                env=env,
                check=True,
            )
        self.assertIn("FINITE_AGENT_RESTORE_SKIPPED", result.stdout)
        self.assertIn("skipped-ok", result.stdout)

    def test_backup_runs_after_command_exit(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            fake_bin = tmp / "bin"
            fake_bin.mkdir()
            install_gnu_realpath_fixture(fake_bin)
            fake_restic = fake_bin / "restic"
            log_path = tmp / "restic.log"
            state_root = tmp / "data"
            home = state_root / "agent"
            workspace = state_root / "workspace"
            home.mkdir(parents=True)
            workspace.mkdir()
            (home / "config.json").write_text("{}", encoding="utf-8")
            (workspace / "project.txt").write_text("workspace", encoding="utf-8")
            fake_restic.write_text(
                "#!/usr/bin/env sh\n"
                'echo "$@" > "$RESTIC_FAKE_LOG"\n'
                'test "$RESTIC_PASSWORD" = secret\n'
                'test -f "$FINITECHAT_WORKSPACE/project.txt"\n'
                'printf \'{"message_type":"summary","snapshot_id":"snapshot-456"}\\n\'\n',
                encoding="utf-8",
            )
            fake_restic.chmod(0o755)
            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{fake_bin}:{env['PATH']}",
                    "RESTIC_FAKE_LOG": str(log_path),
                    "FINITECHAT_HOME": str(home),
                    "FINITECHAT_WORKSPACE": str(workspace),
                    "FINITE_AGENT_STATE_ROOT": str(state_root),
                    "FINITE_AGENT_BACKUP_ON_EXIT": "1",
                    "FINITE_AGENT_RESTIC_REPOSITORY": "s3:https://example.invalid/bucket/prefix",
                    "FINITE_AGENT_RESTIC_PASSWORD": "secret",
                    "FINITE_AGENT_RESTIC_BACKUP_TAG": "finite-agent-state",
                }
            )
            result = subprocess.run(
                [str(ENTRYPOINT), "sh", "-c", "echo child-done"],
                capture_output=True,
                text=True,
                env=env,
                check=True,
            )
            log_text = log_path.read_text(encoding="utf-8").strip()
            self.assertIn("child-done", result.stdout)
            self.assertIn("FINITE_AGENT_BACKUP_COMPLETE", result.stdout)
            self.assertEqual(
                log_text,
                f"-r s3:https://example.invalid/bucket/prefix backup {state_root} --tag finite-agent-state --json",
            )

    def test_periodic_backup_runs_while_command_is_alive(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            fake_bin = tmp / "bin"
            fake_bin.mkdir()
            install_gnu_realpath_fixture(fake_bin)
            fake_restic = fake_bin / "restic"
            log_path = tmp / "restic.log"
            state_root = tmp / "data"
            home = state_root / "agent"
            workspace = state_root / "workspace"
            home.mkdir(parents=True)
            workspace.mkdir()
            (home / "config.json").write_text("{}", encoding="utf-8")
            fake_restic.write_text(
                "#!/usr/bin/env sh\n"
                'echo "$@" >> "$RESTIC_FAKE_LOG"\n'
                'test "$RESTIC_PASSWORD" = secret\n'
                'printf \'{"message_type":"summary","snapshot_id":"snapshot-periodic"}\\n\'\n',
                encoding="utf-8",
            )
            fake_restic.chmod(0o755)
            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{fake_bin}:{env['PATH']}",
                    "RESTIC_FAKE_LOG": str(log_path),
                    "FINITECHAT_HOME": str(home),
                    "FINITECHAT_WORKSPACE": str(workspace),
                    "FINITE_AGENT_STATE_ROOT": str(state_root),
                    "FINITE_AGENT_BACKUP_ON_EXIT": "1",
                    "FINITE_AGENT_BACKUP_INTERVAL_SECS": "1",
                    "FINITE_AGENT_RESTIC_REPOSITORY": "s3:https://example.invalid/bucket/prefix",
                    "FINITE_AGENT_RESTIC_PASSWORD": "secret",
                    "FINITE_AGENT_RESTIC_BACKUP_TAG": "finite-agent-state",
                }
            )
            result = subprocess.run(
                [str(ENTRYPOINT), "sh", "-c", "sleep 2; echo child-done"],
                capture_output=True,
                text=True,
                env=env,
                check=True,
            )

            log_lines = log_path.read_text(encoding="utf-8").splitlines()
            self.assertIn("FINITE_AGENT_BACKUP_PERIODIC_START", result.stdout)
            self.assertIn("child-done", result.stdout)
            self.assertGreaterEqual(len(log_lines), 2)
            self.assertTrue(
                all(
                    line
                    == f"-r s3:https://example.invalid/bucket/prefix backup {state_root} --tag finite-agent-state --json"
                    for line in log_lines
                )
            )

    def test_backup_requires_repository(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            home = Path(tmp_value) / "agent"
            home.mkdir()
            env = os.environ.copy()
            env.update(
                {
                    "FINITECHAT_HOME": str(home),
                    "FINITE_AGENT_BACKUP_ON_EXIT": "1",
                    "FINITE_AGENT_RESTIC_PASSWORD": "secret",
                }
            )
            result = subprocess.run(
                [str(ENTRYPOINT), "sh", "-c", "echo child-done"],
                capture_output=True,
                text=True,
                env=env,
                check=False,
            )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("child-done", result.stdout)
        self.assertIn("missing FINITE_AGENT_RESTIC_REPOSITORY", result.stderr)

    def test_backup_skips_while_finitechat_activity_marker_is_fresh(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            fake_bin = tmp / "bin"
            fake_bin.mkdir()
            install_gnu_realpath_fixture(fake_bin)
            fake_restic = fake_bin / "restic"
            log_path = tmp / "restic.log"
            state_root = tmp / "data"
            home = state_root / "agent"
            workspace = state_root / "workspace"
            home.mkdir(parents=True)
            workspace.mkdir()
            (home / "config.json").write_text("{}", encoding="utf-8")
            (home / ".finitechat-backup-active").write_text("active", encoding="utf-8")
            fake_restic.write_text(
                '#!/usr/bin/env sh\necho "$@" >> "$RESTIC_FAKE_LOG"\nexit 9\n',
                encoding="utf-8",
            )
            fake_restic.chmod(0o755)
            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{fake_bin}:{env['PATH']}",
                    "RESTIC_FAKE_LOG": str(log_path),
                    "FINITECHAT_HOME": str(home),
                    "FINITECHAT_WORKSPACE": str(workspace),
                    "FINITE_AGENT_STATE_ROOT": str(state_root),
                    "FINITE_AGENT_BACKUP_ON_EXIT": "1",
                    "FINITE_AGENT_RESTIC_REPOSITORY": "s3:https://example.invalid/bucket/prefix",
                    "FINITE_AGENT_RESTIC_PASSWORD": "secret",
                    "FINITE_AGENT_RESTIC_BACKUP_TAG": "finite-agent-state",
                }
            )
            result = subprocess.run(
                [str(ENTRYPOINT), "sh", "-c", "echo child-done"],
                capture_output=True,
                text=True,
                env=env,
                check=True,
            )

            self.assertFalse(log_path.exists())
            self.assertIn("child-done", result.stdout)
            self.assertIn("FINITE_AGENT_BACKUP_SKIPPED activity_active=true", result.stdout)

    def test_backup_rejects_workspace_outside_state_root(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            fake_bin = tmp / "bin"
            fake_bin.mkdir()
            install_gnu_realpath_fixture(fake_bin)
            state_root = tmp / "data"
            home = state_root / "agent"
            home.mkdir(parents=True)
            (home / "config.json").write_text("{}", encoding="utf-8")
            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{fake_bin}:{env['PATH']}",
                    "FINITECHAT_HOME": str(home),
                    "FINITECHAT_WORKSPACE": str(tmp / "outside-workspace"),
                    "FINITE_AGENT_STATE_ROOT": str(state_root),
                    "FINITE_AGENT_BACKUP_ON_EXIT": "1",
                    "FINITE_AGENT_RESTIC_REPOSITORY": "s3:https://example.invalid/bucket/prefix",
                    "FINITE_AGENT_RESTIC_PASSWORD": "secret",
                }
            )
            result = subprocess.run(
                [str(ENTRYPOINT), "sh", "-c", "echo child-done"],
                capture_output=True,
                text=True,
                env=env,
                check=False,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("child-done", result.stdout)
        self.assertIn("workspace is outside state root", result.stderr)


if __name__ == "__main__":
    unittest.main()

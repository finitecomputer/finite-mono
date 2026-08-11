from __future__ import annotations

import contextlib
import importlib.machinery
import importlib.util
import io
from pathlib import Path
import shlex
import socket
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
COMMAND = ROOT / "scripts" / "devfinity-restart-process"


def load_command_module():
    loader = importlib.machinery.SourceFileLoader(
        "devfinity_restart_process", str(COMMAND)
    )
    spec = importlib.util.spec_from_loader(loader.name, loader)
    assert spec is not None
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


restart = load_command_module()


class DevfinityRestartProcessTests(unittest.TestCase):
    def setUp(self) -> None:
        self.scratch = tempfile.TemporaryDirectory(prefix="dfr-", dir="/tmp")
        self.root = Path(self.scratch.name)
        self.addCleanup(self.scratch.cleanup)

    def write_default_run(self, *, processes: list[str]) -> Path:
        run_dir = self.root / ".local-state/devfinity/runs/default"
        run_dir.mkdir(parents=True)
        process_compose_file = run_dir / "process-compose.yaml"
        process_compose_socket = run_dir / "pc.sock"
        process_lines = "\n".join(
            f"  {process}:\n    command: sleep 1000" for process in processes
        )
        process_compose_file.write_text(
            f'version: "0.5"\nprocesses:\n{process_lines}\n',
            encoding="utf-8",
        )
        run_dir.joinpath("env").write_text(
            "\n".join(
                [
                    "export DEVFINITY_PROCESS_COMPOSE_FILE="
                    f"{shlex.quote(str(process_compose_file))}",
                    "export DEVFINITY_PROCESS_COMPOSE_SOCKET="
                    f"{shlex.quote(str(process_compose_socket))}",
                    "export UNRELATED_SECRET='not inherited by the test runner'",
                    "",
                ]
            ),
            encoding="utf-8",
        )
        return process_compose_socket

    def bind_socket(self, path: Path) -> socket.socket:
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            sock.bind(str(path))
        except OSError:
            sock.close()
            raise
        self.addCleanup(sock.close)
        return sock

    def run_command(self, process: str, runner):
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            code = restart.main([process], repo_root=self.root, runner=runner)
        return code, stderr.getvalue()

    def test_restarts_known_process_through_default_socket(self) -> None:
        socket_path = self.write_default_run(processes=["core", "dashboard"])
        self.bind_socket(socket_path)
        calls = []

        def runner(argv: list[str], cwd: Path) -> int:
            calls.append((argv, cwd))
            return 0

        code, stderr = self.run_command("core", runner)

        self.assertEqual(code, 0, stderr)
        self.assertEqual(stderr, "")
        self.assertEqual(
            calls,
            [
                (
                    [
                        "process-compose",
                        "-U",
                        "-u",
                        str(socket_path),
                        "process",
                        "restart",
                        "core",
                    ],
                    self.root,
                )
            ],
        )

    def test_missing_socket_fails_before_process_compose(self) -> None:
        socket_path = self.write_default_run(processes=["core"])
        calls = []

        def runner(argv: list[str], cwd: Path) -> int:
            calls.append((argv, cwd))
            return 0

        code, stderr = self.run_command("core", runner)

        self.assertEqual(code, restart.EX_UNAVAILABLE)
        self.assertEqual(calls, [])
        self.assertIn("devfinity is not running", stderr)
        self.assertIn(f"process-compose socket is missing at {socket_path}", stderr)

    def test_unknown_process_fails_before_process_compose(self) -> None:
        socket_path = self.write_default_run(processes=["core", "finite-brain"])
        self.bind_socket(socket_path)
        calls = []

        def runner(argv: list[str], cwd: Path) -> int:
            calls.append((argv, cwd))
            return 0

        code, stderr = self.run_command("runner", runner)

        self.assertEqual(code, restart.EX_DATAERR)
        self.assertEqual(calls, [])
        self.assertIn("unknown devfinity process `runner`", stderr)
        self.assertIn("known processes: core, finite-brain", stderr)

    def test_missing_default_env_reports_devfinity_not_running(self) -> None:
        calls = []

        def runner(argv: list[str], cwd: Path) -> int:
            calls.append((argv, cwd))
            return 0

        code, stderr = self.run_command("core", runner)

        self.assertEqual(code, restart.EX_UNAVAILABLE)
        self.assertEqual(calls, [])
        self.assertIn("default run env is missing", stderr)
        self.assertIn("start it with `just dev up`", stderr)


if __name__ == "__main__":
    unittest.main()

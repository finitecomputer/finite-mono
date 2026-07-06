import json
import os
import socket
import subprocess
import tempfile
import time
import unittest
import urllib.error
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]


def _target_debug_dir() -> Path:
    target_dir = Path(os.environ.get("CARGO_TARGET_DIR", REPO_ROOT / "target"))
    if not target_dir.is_absolute():
        target_dir = REPO_ROOT / target_dir
    return target_dir / "debug"


def _binary_path(name: str) -> Path:
    suffix = ".exe" if os.name == "nt" else ""
    return _target_debug_dir() / f"{name}{suffix}"


def _free_local_addr() -> str:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        host, port = sock.getsockname()
    return f"{host}:{port}"


def _typed_event(payload: str) -> dict:
    sender = {"account_id": "alice", "device_id": "alice-laptop"}
    return {
        "event": {
            "room_id": "process-room",
            "sender": sender,
            "envelope": {
                "room_id": "process-room",
                "mls_group_id": "process-mls",
                "epoch": 0,
                "sender": sender,
                "kind": "application",
                "payload": list(payload.encode("utf-8")),
            },
            "idempotency_key": "process-idempotency",
        },
        "delivery_policy": {
            "push": "default",
            "unread": "default",
            "command_inbox": "never",
        },
    }


class ProcessBinarySmokeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        result = subprocess.run(
            ["cargo", "build", "-p", "finitechat-server", "-p", "finitechat-cli"],
            cwd=REPO_ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )
        if result.returncode != 0:
            raise AssertionError(f"cargo build failed:\n{result.stdout}")
        cls.server_bin = _binary_path("finitechat-server")
        cls.cli_bin = _binary_path("finitechat")

    def test_server_and_cli_binaries_typed_event_replay_restart_and_sync(self):
        with tempfile.TemporaryDirectory() as temp:
            db_path = Path(temp) / "process-smoke.sqlite3"
            addr = _free_local_addr()
            server_url = f"http://{addr}"
            server = self._start_server(addr, db_path)
            try:
                self._wait_for_health(server, server_url)
                health = self._cli_json(server_url, "health")
                self.assertEqual(health["status"], "ok")

                bootstrap = self._cli_json(
                    server_url,
                    "account-room-bootstrap",
                    "--room-id",
                    "process-room",
                    "--mls-group-id",
                    "process-mls",
                    "--account-id",
                    "alice",
                    "--device-id",
                    "alice-laptop",
                )
                self.assertTrue(bootstrap["bootstrapped"])

                event_request = json.dumps(_typed_event("smoke-payload"))
                accepted = self._cli_json(
                    server_url,
                    "append-event",
                    "--request-json",
                    event_request,
                )
                self.assertEqual(accepted["seq"], 1)

                server = self._restart_server(server, addr, db_path)
                self._wait_for_health(server, server_url)

                page = self._cli_json(
                    server_url,
                    "sync-group",
                    "--group-id",
                    "process-room",
                    "--limit",
                    "10",
                )
                self.assertEqual(page["next_after_seq"], 1)
                self.assertEqual(len(page["entries"]), 1)
                self.assertEqual(page["entries"][0]["seq"], 1)

                replayed = self._cli_json(
                    server_url,
                    "append-event",
                    "--request-json",
                    event_request,
                )
                self.assertEqual(replayed, accepted)

                conflict_request = json.dumps(_typed_event("different-payload"))
                conflict = self._cli(
                    server_url,
                    "append-event",
                    "--request-json",
                    conflict_request,
                    check=False,
                )
                self.assertEqual(conflict.returncode, 1)
                self.assertIn("409 Conflict", conflict.stderr)
            finally:
                self._stop_server(server)

    def _start_server(self, addr: str, db_path: Path) -> subprocess.Popen:
        return subprocess.Popen(
            [str(self.server_bin), "serve", addr, "--sqlite", str(db_path)],
            cwd=REPO_ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )

    def _restart_server(
        self, server: subprocess.Popen, addr: str, db_path: Path
    ) -> subprocess.Popen:
        self._stop_server(server)
        return self._start_server(addr, db_path)

    def _stop_server(self, server: subprocess.Popen) -> None:
        if server.poll() is not None:
            return
        server.terminate()
        try:
            server.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
            server.communicate(timeout=5)

    def _wait_for_health(self, server: subprocess.Popen, server_url: str) -> None:
        health_url = f"{server_url}/health"
        deadline = time.monotonic() + 10
        last_error = None
        while time.monotonic() < deadline:
            if server.poll() is not None:
                output, _ = server.communicate(timeout=1)
                self.fail(f"server exited before becoming healthy:\n{output}")
            try:
                with urllib.request.urlopen(health_url, timeout=0.5) as response:
                    body = json.loads(response.read().decode("utf-8"))
                    if response.status == 200 and body.get("status") == "ok":
                        return
            except (OSError, TimeoutError, urllib.error.URLError) as error:
                last_error = error
            time.sleep(0.05)
        self.fail(f"server did not become healthy at {health_url}: {last_error}")

    def _cli_json(self, server_url: str, *args: str) -> dict:
        result = self._cli(server_url, *args)
        return json.loads(result.stdout)

    def _cli(self, server_url: str, *args: str, check: bool = True) -> subprocess.CompletedProcess:
        result = subprocess.run(
            [str(self.cli_bin), "http", "--server", server_url, *args],
            cwd=REPO_ROOT,
            text=True,
            capture_output=True,
        )
        if check and result.returncode != 0:
            self.fail(
                "CLI command failed with "
                f"{result.returncode}\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}"
            )
        return result


if __name__ == "__main__":
    unittest.main()

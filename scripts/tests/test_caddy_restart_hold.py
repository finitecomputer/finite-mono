"""Exercise the lat2 edge's restart hold against real Caddy (FIN-156).

A request that arrives while an upstream is restarting must wait for it to
come back instead of failing with 502, and a POST must reach the upstream
exactly once.
"""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import http.client
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
# Every app-plane upstream and the hold window its restart needs.
EXPECTED_WINDOWS = {
    "finite.computer": {"127.0.0.1:4200": "30s", "127.0.0.1:3000": "15s"},
    "brain.finite.computer": {"127.0.0.1:3015": "15s"},
    "identity.finite.vip": {"127.0.0.1:8791": "15s"},
    "chat.finite.computer": {"127.0.0.1:8788": "30s"},
}
RESTART_SECONDS = 1.5


def free_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def proxy_blocks(handler: str) -> dict[str, str]:
    """Map each `reverse_proxy UPSTREAM {` to the text of its block."""
    blocks = {}
    lines = handler.splitlines()
    for index, line in enumerate(lines):
        words = line.split()
        if words[:1] != ["reverse_proxy"]:
            continue
        depth, body = 0, []
        for inner in lines[index:]:
            depth += inner.count("{") - inner.count("}")
            body.append(inner)
            if depth == 0:
                break
        blocks[words[1]] = "\n".join(body)
    return blocks


class RecordingUpstream(BaseHTTPRequestHandler):
    hits: list[tuple[str, str]] = []

    def _answer(self) -> None:
        length = int(self.headers.get("Content-Length") or 0)
        self.rfile.read(length)
        RecordingUpstream.hits.append((self.command, self.path))
        self.send_response(200)
        self.send_header("Content-Length", "2")
        self.end_headers()
        self.wfile.write(b"ok")

    do_GET = _answer
    do_POST = _answer

    def log_message(self, *_args) -> None:
        pass


class CaddyRestartHold(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.hosts = json.loads(subprocess.check_output([
            "nix", "eval", "--json",
            ".#nixosConfigurations.finite-lat-2.config.services.caddy.virtualHosts",
            "--apply", "hosts: builtins.mapAttrs (_: host: host.extraConfig) hosts",
        ], cwd=ROOT, text=True))

    def test_every_app_plane_upstream_holds_during_restart(self) -> None:
        for host, windows in EXPECTED_WINDOWS.items():
            blocks = proxy_blocks(self.hosts[host])
            self.assertEqual(set(blocks), set(windows), host)
            for upstream, window in windows.items():
                self.assertIn(f"lb_try_duration {window}", blocks[upstream], host)
                self.assertIn("lb_try_interval 250ms", blocks[upstream], host)

    def test_requests_wait_for_a_restarting_upstream(self) -> None:
        # Real finite.computer and chat handlers, with their upstreams moved to
        # free loopback ports that start closed, as during a restart.
        chat_upstream, dashboard_upstream, edge = free_port(), free_port(), free_port()
        rewrites = {"127.0.0.1:8788": chat_upstream, "127.0.0.1:3000": dashboard_upstream}
        blocks = []
        for host in ["finite.computer", "chat.finite.computer"]:
            handler = self.hosts[host]
            for original, port in rewrites.items():
                handler = handler.replace(original, f"127.0.0.1:{port}")
            blocks.append(f"http://{host}:{edge} {{\n bind 127.0.0.1\n{handler}\n}}")

        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            config = root / "Caddyfile"
            config.write_text("{\n admin off\n auto_https off\n}\n" + "\n".join(blocks))
            servers = []

            def start_upstreams() -> None:
                time.sleep(RESTART_SECONDS)
                for port in rewrites.values():
                    server = ThreadingHTTPServer(("127.0.0.1", port), RecordingUpstream)
                    servers.append(server)
                    threading.Thread(target=server.serve_forever, daemon=True).start()

            def request(host: str, method: str, path: str) -> tuple[int, float]:
                started = time.monotonic()
                conn = http.client.HTTPConnection("127.0.0.1", edge, timeout=20)
                conn.request(method, path, body=b"{}" if method == "POST" else None,
                             headers={"Host": host})
                response = conn.getresponse()
                response.read()
                conn.close()
                return response.status, time.monotonic() - started

            with (root / "caddy.log").open("w") as log:
                process = subprocess.Popen(
                    ["caddy", "run", "--config", str(config), "--adapter", "caddyfile"],
                    stdout=log, stderr=log,
                    env={**os.environ, "XDG_CONFIG_HOME": scratch, "XDG_DATA_HOME": scratch},
                )
                try:
                    for _ in range(100):
                        if process.poll() is not None:
                            self.fail((root / "caddy.log").read_text())
                        try:
                            socket.create_connection(("127.0.0.1", edge), timeout=1).close()
                            break
                        except OSError:
                            time.sleep(0.05)
                    RecordingUpstream.hits = []
                    threading.Thread(target=start_upstreams, daemon=True).start()
                    results = {}
                    threads = [
                        threading.Thread(target=lambda key=key, args=args: results.__setitem__(key, request(*args)))
                        for key, args in {
                            "dashboard": ("finite.computer", "GET", "/"),
                            "chat": ("chat.finite.computer", "POST", "/sync/group"),
                        }.items()
                    ]
                    for thread in threads:
                        thread.start()
                    for thread in threads:
                        thread.join(timeout=30)
                    for key, (status, elapsed) in results.items():
                        self.assertEqual(status, 200, key)
                        self.assertGreaterEqual(elapsed, RESTART_SECONDS - 0.2, key)
                    self.assertEqual(len(results), 2)
                    self.assertEqual(
                        sorted(RecordingUpstream.hits),
                        [("GET", "/"), ("POST", "/sync/group")],
                    )
                finally:
                    process.terminate()
                    process.wait(timeout=10)
                    for server in servers:
                        server.shutdown()
                        server.server_close()


if __name__ == "__main__":
    unittest.main()

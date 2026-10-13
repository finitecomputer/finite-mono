"""Finite Chat dashboard plugin through the real pinned Hermes native app.

A disposable Hermes home holds the source plugin where `finitechat hermes
install` writes it. The native web app serves it from a subprocess, with
native auth and middleware intact, and a fake loopback bridge records exactly
what reaches it.
"""

import http.client
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import ClassVar

PLUGIN = Path(__file__).resolve().parents[2] / "integrations/hermes/finitechat"
TOKEN = "synthetic-native-session-token"
BASE = "/api/plugins/finitechat/v1/app"
MAX_BODY = 70 * 1024 * 1024
ATTACHMENT = bytes(range(256)) * 4096
ERROR = b'{"error":"bad assertion","error_kind":"chat_authorization_invalid","retryable":false}'
FIRST_EVENT = b'event: state\nid: 1\ndata: {"rev":1}\n\n'
SECOND_EVENT = b'event: state\nid: 2\ndata: {"rev":2}\n\n'
ATTACHMENT_HEADERS = {
    "content-type": "image/png",
    "content-disposition": 'attachment; filename="photo.png"',
    "x-content-type-options": "nosniff",
    "content-security-policy": "sandbox",
    "cache-control": "private, no-store",
}
SERVE = """
import pathlib, socket, sys
import uvicorn
from hermes_cli import web_server
listener = socket.socket()
listener.bind(("127.0.0.1", 0))
listener.listen(64)
pathlib.Path(sys.argv[1]).write_text(str(listener.getsockname()[1]))
config = uvicorn.Config(web_server.app, lifespan="off", log_level="warning")
uvicorn.Server(config).run(sockets=[listener])
"""


class Bridge(BaseHTTPRequestHandler):
    seen: ClassVar[list[dict]] = []
    release = threading.Event()
    disconnected = threading.Event()

    def do_GET(self):
        self.reply()

    def do_POST(self):
        self.reply()

    def reply(self):
        body = self.rfile.read(int(self.headers.get("content-length") or 0))
        headers = {key.lower(): value for key, value in self.headers.items()}
        Bridge.seen.append(
            {"method": self.command, "path": self.path, "headers": headers, "body": body}
        )
        route = self.path.partition("?")[0]
        if route == "/v1/app/actions":
            extra = {"cache-control": "no-store", "set-cookie": "bridge=1", "x-bridge-private": "1"}
            self.send(200, b'{"ok":true}', {"content-type": "application/json", **extra})
        elif route == "/v1/app/state":
            self.send(401, ERROR, {"content-type": "application/json"})
        elif route.startswith("/v1/app/attachments/"):
            self.send(200, ATTACHMENT, {**ATTACHMENT_HEADERS, "x-bridge-private": "1"})
        elif route == "/v1/app/updates":
            self.stream("forever" in self.path)
        else:
            self.send(404, b'{"error":"unknown"}', {"content-type": "application/json"})

    def send(self, status, body, headers):
        self.send_response(status)
        for key, value in headers.items():
            self.send_header(key, value)
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def stream(self, forever):
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("cache-control", "no-store")
        self.end_headers()
        try:
            self.wfile.write(FIRST_EVENT)
            if not forever:
                # The upstream stays open until the client proves it saw event 1.
                Bridge.release.wait(10)
                self.wfile.write(SECOND_EVENT)
                return
            while True:
                time.sleep(0.05)
                self.wfile.write(b": keepalive\n\n")
        except OSError:
            Bridge.disconnected.set()

    def log_message(self, format, *args):
        pass


class DashboardBridgeProxyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.scratch = tempfile.TemporaryDirectory()
        root = Path(cls.scratch.name)
        home = root / "hermes"
        shutil.copytree(
            PLUGIN, home / "plugins/finitechat", ignore=shutil.ignore_patterns("__pycache__")
        )
        (home / "config.yaml").write_text("plugins:\n  enabled: [finitechat]\n")
        cls.start_bridge(0)
        port_file = root / "port"
        cls.log = (root / "hermes.log").open("w+")
        env = dict(
            os.environ,
            HERMES_HOME=str(home),
            HERMES_DASHBOARD_SESSION_TOKEN=TOKEN,
            FINITECHAT_HERMES_SERVICE_URL=f"http://127.0.0.1:{cls.bridge_port}",
            PYTHONDONTWRITEBYTECODE="1",
        )
        cls.hermes = subprocess.Popen(
            [sys.executable, "-c", SERVE, str(port_file)],
            env=env,
            stdout=cls.log,
            stderr=subprocess.STDOUT,
        )
        deadline = time.monotonic() + 90
        while not port_file.exists() or not port_file.read_text():
            if cls.hermes.poll() is not None or time.monotonic() > deadline:
                cls.tearDownClass()
                raise RuntimeError("native Hermes did not start")
            time.sleep(0.05)
        cls.port = int(port_file.read_text())

    @classmethod
    def tearDownClass(cls):
        cls.hermes.kill()
        cls.hermes.wait()
        cls.bridge.shutdown()
        cls.bridge.server_close()
        cls.log.close()
        cls.scratch.cleanup()

    @classmethod
    def start_bridge(cls, port):
        cls.bridge = ThreadingHTTPServer(("127.0.0.1", port), Bridge)
        cls.bridge.daemon_threads = True
        cls.bridge_port = cls.bridge.server_address[1]
        threading.Thread(target=cls.bridge.serve_forever, daemon=True).start()

    def setUp(self):
        Bridge.seen.clear()
        Bridge.release.clear()
        Bridge.disconnected.clear()

    def connect(self):
        return http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)

    def request(self, method, path, body=None, headers=None, auth=True):
        connection = self.connect()
        sent = {"Authorization": f"Bearer {TOKEN}"} if auth else {}
        connection.request(method, path, body=body, headers={**sent, **(headers or {})})
        response = connection.getresponse()
        data = response.read()
        connection.close()
        return response, data

    def assert_log(self, condition, message=""):
        if not condition:
            self.log.seek(0)
            self.fail(f"{message}\n{self.log.read()[-4000:]}")

    def test_native_auth_runs_before_the_bridge(self):
        for headers in ({}, {"Authorization": "Bearer wrong"}, {"Cookie": "session=wrong"}):
            response, _ = self.request("GET", f"{BASE}/info", headers=headers, auth=False)
            self.assertEqual(response.status, 401)
        self.assertEqual(Bridge.seen, [])

    def test_forwards_only_allowlisted_headers_with_exact_query_and_body(self):
        body = b'{"SendMessage":{"text":"hi"}}\x00\xff'
        query = "room_id=room-1&topic_id=a%2Fb%20c&flag"
        headers = {
            "Content-Type": "application/json",
            "X-Finite-Chat-Authorization": "eyJraW5kIjoyNzIzNX0",
            "X-Finite-Requester-Email": "owner@example.test",
            "X-Finite-Sites-Requester-Assertion": "sites-assertion",
            "X-Hermes-Session-Token": TOKEN,
            "Cookie": "hermes_session_at=private",
            "X-Forwarded-For": "203.0.113.9",
        }
        response, data = self.request("POST", f"{BASE}/actions?{query}", body, headers)
        self.assert_log(response.status == 200, data.decode(errors="replace"))
        self.assertEqual(data, b'{"ok":true}')
        self.assertEqual(response.getheader("content-type"), "application/json")
        self.assertEqual(response.getheader("cache-control"), "no-store")
        self.assertIsNone(response.getheader("set-cookie"))
        self.assertIsNone(response.getheader("x-bridge-private"))
        [seen] = Bridge.seen
        self.assertEqual(seen["method"], "POST")
        self.assertEqual(seen["path"], f"/v1/app/actions?{query}")
        self.assertEqual(seen["body"], body)
        forwarded = {key.lower(): value for key, value in headers.items()}
        for key in (
            "content-type",
            "x-finite-chat-authorization",
            "x-finite-requester-email",
            "x-finite-sites-requester-assertion",
        ):
            self.assertEqual(seen["headers"][key], forwarded[key])
        for key in ("authorization", "cookie", "x-hermes-session-token", "x-forwarded-for"):
            self.assertNotIn(key, seen["headers"])

    def test_bridge_errors_pass_through_unchanged(self):
        response, data = self.request("GET", f"{BASE}/state?room_id=room-1")
        self.assertEqual(response.status, 401)
        self.assertEqual(data, ERROR)
        self.assertEqual(response.getheader("content-type"), "application/json")
        self.assertEqual(len(Bridge.seen), 1)

    def test_update_stream_flushes_each_event_before_the_bridge_finishes(self):
        connection = self.connect()
        connection.request("GET", f"{BASE}/updates?room_id=room-1", headers=self.auth())
        response = connection.getresponse()
        self.assert_log(response.status == 200)
        self.assertEqual(response.getheader("content-type"), "text/event-stream")
        self.assertEqual(response.getheader("cache-control"), "no-store")
        first = self.read_event(response)
        self.assertEqual(first, FIRST_EVENT)
        self.assertFalse(Bridge.release.is_set())
        Bridge.release.set()
        self.assertEqual(response.read(), SECOND_EVENT)
        connection.close()

    def test_client_disconnect_releases_the_bridge_stream(self):
        connection = self.connect()
        connection.request("GET", f"{BASE}/updates?mode=forever", headers=self.auth())
        response = connection.getresponse()
        self.assertEqual(self.read_event(response), FIRST_EVENT)
        response.close()
        connection.close()
        self.assertTrue(Bridge.disconnected.wait(10), "bridge stream was not closed")

    def test_attachment_bytes_and_safety_headers_pass_through(self):
        # Percent-encoding reaches the bridge as sent: the assertion signs it.
        path = "/attachments/room-1/msg-1/att%2F1%20b"
        response, data = self.request("GET", f"{BASE}{path}")
        self.assertEqual(response.status, 200)
        self.assertEqual(data, ATTACHMENT)
        for key, value in ATTACHMENT_HEADERS.items():
            self.assertEqual(response.getheader(key), value)
        self.assertIsNone(response.getheader("x-bridge-private"))
        self.assertEqual(Bridge.seen[0]["path"], f"/v1/app{path}")

    def test_oversized_bodies_are_rejected_before_the_bridge(self):
        connection = self.connect()
        connection.putrequest("POST", f"{BASE}/attachments")
        for key, value in self.auth().items():
            connection.putheader(key, value)
        connection.putheader("Content-Length", str(MAX_BODY + 1))
        connection.endheaders()
        self.assert_too_large(connection)

        connection = self.connect()
        connection.putrequest("POST", f"{BASE}/attachments")
        for key, value in self.auth().items():
            connection.putheader(key, value)
        connection.putheader("Transfer-Encoding", "chunked")
        connection.endheaders()
        chunk = b"x" * (1024 * 1024)
        for _ in range(MAX_BODY // len(chunk)):
            connection.send(b"%x\r\n%s\r\n" % (len(chunk), chunk))
        time.sleep(0.2)
        connection.send(b"1\r\nx\r\n0\r\n\r\n")
        self.assert_too_large(connection)
        self.assertEqual(Bridge.seen, [])

    def test_unreachable_bridge_is_a_retryable_503(self):
        type(self).bridge.shutdown()
        type(self).bridge.server_close()
        try:
            response, data = self.request("GET", f"{BASE}/info")
        finally:
            type(self).start_bridge(self.bridge_port)
        self.assertEqual(response.status, 503)
        self.assertEqual(
            json.loads(data),
            {
                "error": "Finite Chat bridge is unavailable",
                "error_kind": "bridge_unavailable",
                "retryable": True,
            },
        )

    def test_only_bridge_app_routes_are_reachable(self):
        for path in (
            f"{BASE}/../hermes/poll",
            f"{BASE}/%2e%2e/agentd/deliveries",
            f"{BASE}/attachments/%5C..%5Chermes",
            f"{BASE}/state/",
        ):
            response, data = self.request("GET", path)
            self.assertEqual(response.status, 404, path)
            self.assertEqual(json.loads(data)["error_kind"], "not_found")
        response, _ = self.request("GET", "/api/plugins/finitechat/v1/hermes/poll")
        self.assertNotEqual(response.status, 200)
        response, _ = self.request("PUT", f"{BASE}/actions", b"{}")
        self.assertEqual(response.status, 405)
        self.assertEqual(Bridge.seen, [])

    def auth(self):
        return {"Authorization": f"Bearer {TOKEN}"}

    def read_event(self, response):
        received = b""
        while b"\n\n" not in received:
            chunk = response.read1(65536)
            if not chunk:
                break
            received += chunk
        return received

    def assert_too_large(self, connection):
        response = connection.getresponse()
        self.assertEqual(response.status, 413)
        self.assertEqual(json.loads(response.read())["error_kind"], "payload_too_large")
        connection.close()


if __name__ == "__main__":
    unittest.main()

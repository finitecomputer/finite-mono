from __future__ import annotations

import json
import os
import re
import subprocess
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
OPS = ROOT / "infra/runbooks/finite-private-ops.sh"


class MockFinitePrivateHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    fail_request_two = False

    def log_message(self, format: str, *args: object) -> None:
        pass

    def do_GET(self) -> None:
        if self.path != "/health":
            self.send_error(404)
            return
        body = b'{"status":"ok"}'
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self) -> None:
        if self.path != "/v1/chat/completions":
            self.send_error(404)
            return
        length = int(self.headers.get("content-length", "0"))
        payload = json.loads(self.rfile.read(length))
        if self.fail_request_two and self.headers.get("x-request-id", "").endswith("_2"):
            body = b'{"error":"synthetic tier failure"}'
            self.send_response(503)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if payload.get("stream") is not True:
            self.send_error(400, "streaming required")
            return
        if payload.get("stream_options") != {"include_usage": True}:
            self.send_error(400, "streaming usage required")
            return
        chunks = [
            'data: {"id":"mock","choices":[{"index":0,"delta":{"content":"ok"}}]}\n\n',
            'data: {"id":"mock","choices":[],"usage":{"prompt_tokens":12,"completion_tokens":16,"total_tokens":28}}\n\n',
            "data: [DONE]\n\n",
        ]
        body = "".join(chunks).encode()
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class FinitePrivateOpsLoadTests(unittest.TestCase):
    def setUp(self) -> None:
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), MockFinitePrivateHandler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.environment = os.environ.copy()
        self.environment.update(
            {
                "FINITE_PRIVATE_ENDPOINT": f"http://127.0.0.1:{self.server.server_port}",
                "FINITE_PRIVATE_MODEL": "deepseek-v4-flash-0731",
                "FINITE_PRIVATE_CANARY_API_KEY": "synthetic-test-key",
                "FINITE_PRIVATE_CANARY_ENV_FILE": "/nonexistent/finite-private-canary.env",
                "FINITE_PRIVATE_CANARY_TIMEOUT_SECS": "10",
                "FINITE_PRIVATE_LOAD_MAX_TOKENS": "16",
            }
        )

    def tearDown(self) -> None:
        MockFinitePrivateHandler.fail_request_two = False
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)

    def run_ops(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(OPS), *arguments],
            cwd=ROOT,
            env=self.environment,
            text=True,
            capture_output=True,
            check=False,
        )

    def test_parameterized_load_canary_reports_streaming_metrics(self) -> None:
        result = self.run_ops("load-canary", "4")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("requests=4", result.stdout)
        self.assertIn("completion_tokens=64", result.stdout)
        self.assertIn("time_to_first_byte_seconds p50=", result.stdout)
        self.assertIn("completion_seconds p50=", result.stdout)
        self.assertIn("generation_tokens_per_second", result.stdout)
        self.assertIn("aggregate=", result.stdout)
        batch_match = re.search(r"batch_seconds=([0-9.]+)", result.stdout)
        aggregate_match = re.search(r"aggregate=([0-9.]+)", result.stdout)
        self.assertIsNotNone(batch_match, result.stdout)
        self.assertIsNotNone(aggregate_match, result.stdout)
        self.assertGreater(float(batch_match.group(1)), 0)
        self.assertGreater(float(aggregate_match.group(1)), 0)

    def test_high_concurrency_requires_exact_sweep_approval(self) -> None:
        result = self.run_ops("load-canary", "64")

        self.assertEqual(result.returncode, 64)
        self.assertIn("refusing high-concurrency load", result.stderr)
        self.assertIn("1,4,8,16,32,64,128,256", result.stderr)

    def test_load_sweep_requires_exact_approval(self) -> None:
        result = self.run_ops("load-sweep")

        self.assertEqual(result.returncode, 64)
        self.assertIn("refusing high-concurrency load", result.stderr)

    def test_load_sweep_stops_at_first_failed_tier(self) -> None:
        MockFinitePrivateHandler.fail_request_two = True
        self.environment["FINITE_PRIVATE_LOAD_SWEEP_APPROVED"] = (
            "1,4,8,16,32,64,128,256"
        )

        result = self.run_ops("load-sweep")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stopping sweep at failed tier 4", result.stderr)
        self.assertNotIn("concurrency tier 8", result.stdout)


if __name__ == "__main__":
    unittest.main()

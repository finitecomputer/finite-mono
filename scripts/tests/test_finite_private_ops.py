from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
OPS = ROOT / "infra/runbooks/finite-private-ops.sh"


class MockFinitePrivateHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    fail_request_two = False
    required_model: str | None = None

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
        if self.path not in ("/v1/chat/completions", "/v1/responses"):
            self.send_error(404)
            return
        length = int(self.headers.get("content-length", "0"))
        payload = json.loads(self.rfile.read(length))
        if (
            self.required_model is not None
            and payload.get("model") != self.required_model
        ):
            body = b'{"error":"unexpected model"}'
            self.send_response(400)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if self.path == "/v1/responses":
            body = b'{"id":"response-mixed-version"}'
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if self.fail_request_two and self.headers.get("x-request-id", "").endswith(
            "_2"
        ):
            body = b'{"error":"synthetic tier failure"}'
            self.send_response(503)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if payload.get("stream") is not True:
            body = b'{"choices":[{"message":{"content":"finite private ok"}}]}'
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
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
                "FINITE_PRIVATE_CANARY_API_KEY": "synthetic-test-key",
                "FINITE_PRIVATE_CANARY_ENV_FILE": "/nonexistent/finite-private-canary.env",
                "FINITE_PRIVATE_CANARY_TIMEOUT_SECS": "10",
                "FINITE_PRIVATE_LOAD_MAX_TOKENS": "16",
            }
        )

    def tearDown(self) -> None:
        MockFinitePrivateHandler.fail_request_two = False
        MockFinitePrivateHandler.required_model = None
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

    def test_parameterized_load_canary_reports_metrics(self) -> None:
        result = self.run_ops("load-canary", "4")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("requests=4", result.stdout)
        self.assertIn("completion_tokens=64", result.stdout)
        self.assertIn("time_to_first_byte_seconds p50=", result.stdout)
        self.assertIn("generation_tokens_per_second", result.stdout)
        self.assertIsNotNone(re.search(r"aggregate=([0-9.]+)", result.stdout))

    def test_load_canary_uses_one_parallel_curl_process(self) -> None:
        real_curl = shutil.which("curl")
        self.assertIsNotNone(real_curl)
        with tempfile.TemporaryDirectory() as temporary_directory:
            invocation_log = Path(temporary_directory) / "curl-invocations"
            curl = Path(temporary_directory) / "curl"
            curl.write_text(
                "#!/bin/sh\n"
                'printf "invoked\\n" >>"$CURL_INVOCATION_LOG"\n'
                f'exec "{real_curl}" "$@"\n',
                encoding="utf-8",
            )
            curl.chmod(0o700)
            self.environment["CURL_INVOCATION_LOG"] = str(invocation_log)
            self.environment["PATH"] = (
                temporary_directory + os.pathsep + self.environment["PATH"]
            )
            result = self.run_ops("load-canary", "4")
            invocations = invocation_log.read_text(encoding="utf-8")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(invocations, "invoked\n")

    def test_high_concurrency_requires_exact_approval(self) -> None:
        result = self.run_ops("load-canary", "64")
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
        self.assertIn(
            "no further inference requests were issued after failure", result.stderr
        )

    def test_mixed_version_canary_exercises_the_historical_request_alias(self) -> None:
        MockFinitePrivateHandler.required_model = "glm-5-2"
        result = self.run_ops("mixed-version-canary")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Mixed-version Finite Private compatibility passed", result.stdout)

    def test_settlement_status_accepts_preexisting_but_not_rollout_reserved_rows(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            ssh = Path(temporary_directory) / "ssh"
            ssh.write_text(
                "#!/bin/sh\n"
                "printf '%s\\n' "
                "'deepseek-v4-flash-0731|settled|actual|12' "
                "'preexisting_reserved|50|rollout_reserved|0'\n",
                encoding="utf-8",
            )
            ssh.chmod(0o700)
            self.environment["PATH"] = (
                temporary_directory + os.pathsep + self.environment["PATH"]
            )
            result = self.run_ops(
                "settlement-status", "2026-08-14T04:00:00Z"
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("preexisting_reserved|50|rollout_reserved|0", result.stdout)
        self.assertIn("rollout-era canary settlements passed", result.stdout)

    def test_settlement_status_rejects_rollout_reserved_rows(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            ssh = Path(temporary_directory) / "ssh"
            ssh.write_text(
                "#!/bin/sh\n"
                "printf '%s\\n' 'preexisting_reserved|50|rollout_reserved|1'\n",
                encoding="utf-8",
            )
            ssh.chmod(0o700)
            self.environment["PATH"] = (
                temporary_directory + os.pathsep + self.environment["PATH"]
            )
            result = self.run_ops(
                "settlement-status", "2026-08-14T04:00:00Z"
            )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("created during this rollout remain reserved", result.stderr)


if __name__ == "__main__":
    unittest.main()

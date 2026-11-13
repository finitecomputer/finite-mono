from __future__ import annotations

from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import threading
import unittest

from scripts.check_finite_private_inkling_protocol import (
    Client,
    MARKER,
    MODEL,
    payload,
    run,
    tool_calls_valid,
    validate_endpoint,
)


def tool_call(index=0, city="Austin", state="Texas"):
    return {
        "index": index,
        "id": f"call_{index}",
        "type": "function",
        "function": {
            "name": "get_weather",
            "arguments": json.dumps({"city": city, "state": state}),
        },
    }


@contextmanager
def endpoint(*, fault=None):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            if fault == "http":
                self.send_response(400)
                self.end_headers()
                self.wfile.write(b"secret response body: synthetic-key")
                return
            if fault == "redirect":
                self.send_response(307)
                self.send_header("Location", "https://example.com/v1/chat/completions")
                self.end_headers()
                return
            messages = body["messages"]
            prompt = str(messages[-1].get("content", ""))
            calls = []
            content = MARKER
            if body.get("tools") and body.get("tool_choice") != "none":
                calls = [tool_call()]
                content = None
                if "Boston" in prompt:
                    calls.append(tool_call(1, "Boston", "Massachusetts"))
            elif body.get("tool_choice") == "none":
                content = "91"
            elif "Multiply" in prompt:
                content = "12"
            elif "17 * 19" in prompt:
                content = "323"
            think = (
                body.get("reasoning_effort") != "none"
                and body.get("chat_template_kwargs", {}).get("enable_thinking")
                is not False
            )
            if (
                fault == "legacy"
                and body.get("chat_template_kwargs", {}).get("enable_thinking") is False
            ):
                think = True
            msg = {"role": "assistant", "content": content}
            if think:
                msg["reasoning_content"] = "Synthetic separated reasoning."
            if calls:
                msg["tool_calls"] = calls
            finish = "tool_calls" if calls else "stop"
            model = "wrong-model" if fault == "identity" else MODEL
            usage = {"prompt_tokens": 100, "completion_tokens": 20}
            if body.get("stream"):
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                events = []
                if calls:
                    for call in calls:
                        first = dict(call, function=dict(call["function"]))
                        args = first["function"]["arguments"]
                        first["function"]["arguments"] = args[:10]
                        events.append({"choices": [{"delta": {"tool_calls": [first]}}]})
                        events.append(
                            {
                                "choices": [
                                    {
                                        "delta": {
                                            "tool_calls": [
                                                {
                                                    "index": call["index"],
                                                    "function": {
                                                        "arguments": args[10:]
                                                    },
                                                }
                                            ]
                                        }
                                    }
                                ]
                            }
                        )
                else:
                    events.append({"choices": [{"delta": {"content": content}}]})
                events.append(
                    {
                        "choices": [
                            {
                                "delta": {},
                                "finish_reason": "length"
                                if fault == "length"
                                else finish,
                            }
                        ]
                    }
                )
                if fault != "usage":
                    events.append({"choices": [], "usage": usage})
                try:
                    for event in events:
                        self.wfile.write(
                            (
                                "data: " + json.dumps(dict(event, model=model)) + "\n\n"
                            ).encode()
                        )
                    if fault != "done":
                        self.wfile.write(b"data: [DONE]\n\n")
                except (BrokenPipeError, ConnectionResetError):
                    pass
            else:
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(
                    json.dumps(
                        {
                            "model": model,
                            "usage": usage,
                            "choices": [{"message": msg, "finish_reason": finish}],
                        }
                    ).encode()
                )

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}/v1"
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


class ProtocolTests(unittest.TestCase):
    def test_real_http_and_fragmented_sse_complete_full_protocol_suite(self):
        with endpoint() as url:
            results = run(Client(url, "synthetic-key", MODEL, 5), MODEL)
        self.assertGreaterEqual(len(results), 12)
        self.assertTrue(all(row["passed"] for row in results), results)

    def test_ignored_legacy_fast_control_is_a_failed_gate(self):
        with endpoint(fault="legacy") as url:
            results = run(Client(url, "synthetic-key", MODEL, 5), MODEL)
        self.assertFalse(
            next(row for row in results if row["case"] == "legacy_fast_control")[
                "passed"
            ]
        )

    def test_long_context_records_actual_counts_and_rejects_wrong_target(self):
        class CountedClient(Client):
            wrong_target = False

            def post(self, body):
                msg, response = super().post(body)
                content = body["messages"][0]["content"]
                units = content.count(" x")
                if units:
                    response["usage"]["prompt_tokens"] = (
                        360000 if self.wrong_target and units > 1024 else units + 20
                    )
                return msg, response

        with endpoint() as url:
            client = CountedClient(url, "synthetic-key", MODEL, 5)
            results = run(client, MODEL, context_tokens=128000)
            long = next(row for row in results if row["case"] == "context_128000")
            self.assertTrue(long["passed"], long)
            self.assertGreater(long["prompt_tokens"], 115200)
            client.wrong_target = True
            results = run(client, MODEL, context_tokens=128000)
            long = next(row for row in results if row["case"] == "context_128000")
            self.assertFalse(long["passed"], long)

    def test_missing_terminal_marker_usage_and_truncation_fail(self):
        for fault in ("done", "usage", "length", "identity"):
            with self.subTest(fault=fault), endpoint(fault=fault) as url:
                with self.assertRaises(ValueError):
                    Client(url, "synthetic-key", MODEL, 5).stream(
                        payload(MODEL, MARKER)
                    )

    def test_error_reports_never_echo_upstream_body_or_key(self):
        with endpoint(fault="http") as url:
            results = run(Client(url, "synthetic-key", MODEL, 5), MODEL)
        self.assertTrue(all(not row["passed"] for row in results))
        report = json.dumps(results)
        self.assertNotIn("synthetic-key", report)
        self.assertNotIn("secret response body", report)
        self.assertTrue(all(row["detail"] == "HTTP 400" for row in results))

    def test_redirect_cannot_forward_credentials_to_another_host(self):
        with endpoint(fault="redirect") as url:
            with self.assertRaisesRegex(ValueError, "redirect refused"):
                Client(url, "synthetic-key", MODEL, 5).post(payload(MODEL, MARKER))

    def test_no_remote_http_credentials_query_or_default_endpoint(self):
        self.assertEqual(
            validate_endpoint("http://127.0.0.1:8001/v1", allow_remote=False),
            "http://127.0.0.1:8001/v1",
        )
        for url in (
            "https://example.com/v1",
            "http://user:key@localhost/v1",
            "http://localhost/v1?key=x",
            "http://localhost/v1#x",
            "file:///v1",
        ):
            with self.subTest(url=url), self.assertRaises(ValueError):
                validate_endpoint(url, allow_remote=False)
        with self.assertRaises(ValueError):
            validate_endpoint("http://example.com/v1", allow_remote=True)

    def test_tool_gate_rejects_wrong_state_extra_arguments_and_duplicate_ids(self):
        self.assertTrue(tool_calls_valid([tool_call()], {"austin"}))
        wrong = tool_call(state="Massachusetts")
        self.assertFalse(tool_calls_valid([wrong], {"austin"}))
        extra = tool_call()
        extra["function"]["arguments"] = (
            '{"city":"Austin","state":"Texas","command":"x"}'
        )
        self.assertFalse(tool_calls_valid([extra], {"austin"}))
        self.assertFalse(tool_calls_valid([tool_call(), tool_call()], {"austin"}))


if __name__ == "__main__":
    unittest.main()

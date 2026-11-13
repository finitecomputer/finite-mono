"""Loopback fakes for a real Hermes gateway: a Telegram Bot API and a model.

`FakeTelegramBotApi` answers python-telegram-bot's calls for one token only,
keeps a server-side update queue (dropped when `deleteWebhook` asks for it,
as Telegram does) and records every message the bot sends. `StubModel` is an
OpenAI-compatible chat-completions endpoint that records each request.
"""

from __future__ import annotations

import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any
from urllib.parse import parse_qs


def _reply(handler: BaseHTTPRequestHandler, body: Any, status: int = 200) -> None:
    data = json.dumps(body).encode()
    handler.send_response(status)
    handler.send_header("content-type", "application/json")
    handler.send_header("content-length", str(len(data)))
    handler.end_headers()
    handler.wfile.write(data)


class FakeTelegramBotApi:
    def __init__(self, token: str, bot_id: int = 999000111):
        self.token = token
        self.bot_id = bot_id
        self.condition = threading.Condition()
        self.queue: list[dict[str, Any]] = []
        self.next_update_id = 1000
        self.calls: list[str] = []
        self.sent: list[dict[str, Any]] = []
        self.dropped_updates = 0
        self.unauthorized_calls = 0
        self.message_id = 5000
        fake = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args: Any) -> None:
                return None

            def do_GET(self) -> None:
                self.handle_call()

            def do_POST(self) -> None:
                self.handle_call()

            def handle_call(self) -> None:
                parts = self.path.strip("/").split("/")
                if len(parts) < 2 or not parts[0].startswith("bot"):
                    self.send_error(404)
                    return
                length = int(self.headers.get("content-length") or 0)
                raw = self.rfile.read(length) if length else b""
                kind = self.headers.get("content-type", "")
                params: dict[str, Any] = {}
                if raw and "json" in kind:
                    params = json.loads(raw)
                elif raw and "form" in kind:
                    params = {key: value[0] for key, value in parse_qs(raw.decode()).items()}
                if parts[0][3:] != fake.token:
                    with fake.condition:
                        fake.unauthorized_calls += 1
                    _reply(
                        self, {"ok": False, "error_code": 401, "description": "Unauthorized"}, 401
                    )
                    return
                with fake.condition:
                    fake.calls.append(parts[1])
                _reply(self, {"ok": True, "result": fake.dispatch(parts[1], params)})

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    @property
    def base_url(self) -> str:
        return f"http://127.0.0.1:{self.server.server_address[1]}/bot"

    def __enter__(self) -> FakeTelegramBotApi:
        self.thread.start()
        return self

    def __exit__(self, *exc: Any) -> None:
        self.server.shutdown()
        self.server.server_close()

    def queue_message(self, user_id: int, text: str, first_name: str) -> int:
        with self.condition:
            self.next_update_id += 1
            self.queue.append(
                {
                    "update_id": self.next_update_id,
                    "message": {
                        "message_id": self.next_update_id,
                        "date": int(time.time()),
                        "chat": {"id": user_id, "type": "private", "first_name": first_name},
                        "from": {"id": user_id, "is_bot": False, "first_name": first_name},
                        "text": text,
                    },
                }
            )
            self.condition.notify_all()
            return self.next_update_id

    def dispatch(self, method: str, params: dict[str, Any]) -> Any:
        if method == "getMe":
            return {
                "id": self.bot_id,
                "is_bot": True,
                "first_name": "Legacy Bot",
                "username": "legacy_fixture_bot",
                "can_join_groups": True,
                "can_read_all_group_messages": False,
                "supports_inline_queries": False,
            }
        if method in ("deleteWebhook", "setWebhook"):
            if str(params.get("drop_pending_updates", "")).lower() in ("true", "1"):
                with self.condition:
                    self.dropped_updates += len(self.queue)
                    self.queue.clear()
            return True
        if method == "getUpdates":
            offset = int(params.get("offset") or 0)
            deadline = time.time() + min(float(params.get("timeout") or 0), 2.0)
            with self.condition:
                if offset:
                    self.queue = [item for item in self.queue if item["update_id"] >= offset]
                while not self.queue and time.time() < deadline:
                    self.condition.wait(deadline - time.time())
                return list(self.queue)
        if method in ("sendMessage", "editMessageText"):
            with self.condition:
                self.message_id += 1
                self.sent.append({"method": method, **params})
                self.condition.notify_all()
            return {
                "message_id": self.message_id,
                "date": int(time.time()),
                "chat": {"id": int(params.get("chat_id") or 0), "type": "private"},
                "text": params.get("text", ""),
                "from": {"id": self.bot_id, "is_bot": True, "first_name": "Legacy Bot"},
            }
        if method == "getMyCommands":
            return []
        return True

    def messages_to(self, chat_id: int) -> list[str]:
        with self.condition:
            return [
                str(item.get("text", ""))
                for item in self.sent
                if str(item.get("chat_id")) == str(chat_id)
            ]

    def wait_for_message(self, chat_id: int, timeout: float) -> list[str]:
        deadline = time.time() + timeout
        with self.condition:
            while time.time() < deadline:
                found = [
                    str(item.get("text", ""))
                    for item in self.sent
                    if str(item.get("chat_id")) == str(chat_id)
                ]
                if found:
                    return found
                self.condition.wait(min(0.5, max(0.0, deadline - time.time())))
        return []


class StubModel:
    def __init__(self, reply: str):
        self.reply = reply
        self.requests: list[dict[str, Any]] = []
        self.lock = threading.Lock()
        stub = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args: Any) -> None:
                return None

            def do_GET(self) -> None:
                _reply(
                    self, {"object": "list", "data": [{"id": "glm-5-3-flash", "object": "model"}]}
                )

            def do_POST(self) -> None:
                length = int(self.headers.get("content-length") or 0)
                body = json.loads(self.rfile.read(length) or b"{}")
                with stub.lock:
                    stub.requests.append(body)
                created = int(time.time())
                usage = {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
                if not body.get("stream"):
                    _reply(
                        self,
                        {
                            "id": "stub",
                            "object": "chat.completion",
                            "created": created,
                            "model": body.get("model"),
                            "choices": [
                                {
                                    "index": 0,
                                    "message": {"role": "assistant", "content": stub.reply},
                                    "finish_reason": "stop",
                                }
                            ],
                            "usage": usage,
                        },
                    )
                    return
                self.send_response(200)
                self.send_header("content-type", "text/event-stream")
                self.end_headers()
                for delta, finish in (
                    ({"role": "assistant", "content": stub.reply}, None),
                    ({}, "stop"),
                ):
                    chunk = {
                        "id": "stub",
                        "object": "chat.completion.chunk",
                        "created": created,
                        "model": body.get("model"),
                        "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
                    }
                    if finish:
                        chunk["usage"] = usage
                    self.wfile.write(b"data: " + json.dumps(chunk).encode() + b"\n\n")
                self.wfile.write(b"data: [DONE]\n\n")
                self.wfile.flush()

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    @property
    def base_url(self) -> str:
        return f"http://127.0.0.1:{self.server.server_address[1]}/v1"

    def __enter__(self) -> StubModel:
        self.thread.start()
        return self

    def __exit__(self, *exc: Any) -> None:
        self.server.shutdown()
        self.server.server_close()

    def system_prompts(self) -> list[str]:
        with self.lock:
            requests = list(self.requests)
        prompts = []
        for body in requests:
            prompts.append(
                "\n".join(
                    message["content"]
                    if isinstance(message.get("content"), str)
                    else json.dumps(message.get("content"))
                    for message in body.get("messages", [])
                    if message.get("role") == "system"
                )
            )
        return prompts

    def user_texts(self) -> list[str]:
        with self.lock:
            requests = list(self.requests)
        return [
            json.dumps(
                [m.get("content") for m in body.get("messages", []) if m.get("role") == "user"]
            )
            for body in requests
        ]

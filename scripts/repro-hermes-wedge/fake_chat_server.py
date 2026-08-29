#!/usr/bin/env python3
"""Minimal fake finitechat server for the hermes sidecar wedge repro.

Answers just enough of the chat HTTP contract for `finitechat hermes init` +
`serve` to reach steady state, then can be flipped into `stall` mode, where:

  * already-open `/sync/stream` SSE connections stop sending heartbeats
    (bytes stop flowing, the connection stays ESTABLISHED, no FIN/RST), and
  * every new request is read and never answered.

That is the production failure shape seen by the sidecar: the network path is
"healthy" for anyone opening a *new* connection, but an established stream or
in-flight request simply goes silent forever.

Control: write `healthy` or `stall` into the file named by --mode-file
(default: <log-dir>/mode). Default mode is healthy.
"""

from __future__ import annotations

import argparse
import json
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

CONTRACT_VERSION = 6


def read_mode(mode_file: Path) -> str:
    try:
        return mode_file.read_text(encoding="utf-8").strip() or "healthy"
    except OSError:
        return "healthy"


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    # --- plumbing -----------------------------------------------------------

    def log_message(self, fmt: str, *args: object) -> None:
        stamp = time.strftime("%H:%M:%S")
        sys.stdout.write(f"{stamp} {self.address_string()} {fmt % args}\n")
        sys.stdout.flush()

    def _read_body(self) -> bytes:
        length = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(length) if length else b""

    def _json(self, payload: object, status: int = 200) -> None:
        body = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _stall_forever(self) -> None:
        # Read whatever the client sent, then hold the connection open without
        # ever responding. Matches "request in flight, connection half-open".
        self.log_message("STALL (holding connection open, never responding)")
        while True:
            time.sleep(60)

    # --- routes -------------------------------------------------------------

    def do_GET(self):
        if read_mode(self.server.mode_file) == "stall":  # type: ignore[attr-defined]
            self._read_body()
            self._stall_forever()
            return
        if self.path == "/health":
            self._json(
                {
                    "status": "ok",
                    "server_contract_version": CONTRACT_VERSION,
                    "server_version": "repro-fake/1",
                }
            )
            return
        # Generic empty success for every other GET (room lists, inbox, ...).
        self.log_message("GET %s -> 200 {}", self.path)
        self._json({})

    def do_PUT(self):
        self._read_body()
        if read_mode(self.server.mode_file) == "stall":  # type: ignore[attr-defined]
            self._stall_forever()
            return
        self.log_message("PUT %s -> 200 {}", self.path)
        self._json({})

    def do_POST(self):
        body = self._read_body()
        if read_mode(self.server.mode_file) == "stall":  # type: ignore[attr-defined]
            self._stall_forever()
            return
        if self.path == "/sync/stream":
            self._sync_stream(body)
            return
        self.log_message("POST %s -> 200 {}", self.path)
        self._json({})

    def _sync_stream(self, body: bytes) -> None:
        try:
            request = json.loads(body or b"{}")
        except ValueError:
            request = {}
        heartbeat_ms = max(250, int(request.get("heartbeat_ms") or 3000))
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.log_message("POST /sync/stream -> SSE (heartbeat %d ms)", heartbeat_ms)
        while True:
            if read_mode(self.server.mode_file) == "stall":
                # Established stream goes silent: no bytes, no EOF, no error.
                self.log_message("SSE stream going SILENT (stall mode)")
                return
            self.wfile.write(b'data: {"type":"heartbeat"}\n\n')
            self.wfile.flush()
            time.sleep(heartbeat_ms / 1000)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=18180)
    parser.add_argument("--mode-file", required=True)
    args = parser.parse_args()

    mode_file = Path(args.mode_file)
    mode_file.parent.mkdir(parents=True, exist_ok=True)
    mode_file.write_text("healthy\n", encoding="utf-8")

    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    server.daemon_threads = False  # stalled handlers must hold their sockets
    server.mode_file = mode_file  # type: ignore[attr-defined]
    print(f"fake chat server on http://127.0.0.1:{args.port} (mode file: {mode_file})")
    server.serve_forever()


if __name__ == "__main__":
    main()

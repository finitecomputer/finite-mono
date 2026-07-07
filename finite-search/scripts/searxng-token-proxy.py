#!/usr/bin/env python3
"""Local bearer-token proxy for stock Hermes SearXNG clients.

Hermes' bundled SearXNG provider only sends ordinary JSON requests to
SEARXNG_URL. For gated deployments, run this process on localhost and point
Hermes at it. The token stays in this proxy process, not in Hermes config.
"""

from __future__ import annotations

import argparse
import http.server
import os
import sys
import urllib.error
import urllib.request


HOP_BY_HOP_HEADERS = {
    "connection",
    "content-encoding",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
}


def env_int(name: str, default: int) -> int:
    value = os.environ.get(name)
    if not value:
        return default
    try:
        return int(value)
    except ValueError:
        raise SystemExit(f"{name} must be an integer, got {value!r}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Inject Authorization: Bearer for localhost SearXNG clients.",
    )
    parser.add_argument(
        "--listen-host",
        default=os.environ.get("SEARXNG_PROXY_HOST", "127.0.0.1"),
        help="Address to bind. Defaults to 127.0.0.1.",
    )
    parser.add_argument(
        "--port",
        type=int,
        default=env_int("SEARXNG_PROXY_PORT", 18999),
        help="Local listen port. Defaults to 18999.",
    )
    parser.add_argument(
        "--upstream-url",
        default=os.environ.get("SEARXNG_UPSTREAM_URL", "http://127.0.0.1:3399"),
        help="Upstream SearXNG base URL. Defaults to http://127.0.0.1:3399.",
    )
    return parser.parse_args()


def token_from_env() -> str:
    token = os.environ.get("SEARXNG_TOKEN") or os.environ.get("FINITE_SEARCH_TOKEN")
    if not token:
        raise SystemExit("SEARXNG_TOKEN or FINITE_SEARCH_TOKEN is required")
    return token.strip()


def make_handler(upstream_url: str, token: str) -> type[http.server.BaseHTTPRequestHandler]:
    upstream = upstream_url.rstrip("/")

    class SearXNGTokenProxy(http.server.BaseHTTPRequestHandler):
        server_version = "finite-search-searxng-token-proxy/0.1"

        def log_message(self, fmt: str, *args: object) -> None:
            sys.stderr.write("%s - %s\n" % (self.address_string(), fmt % args))

        def do_GET(self) -> None:
            if self.path == "/healthz":
                self._send_bytes(200, b"ok\n", "text/plain")
                return

            if not (self.path == "/search" or self.path.startswith("/search?")):
                self._send_bytes(404, b"not found\n", "text/plain")
                return

            request = urllib.request.Request(
                upstream + self.path,
                headers={
                    "Accept": self.headers.get("Accept", "application/json"),
                    "Authorization": "Bearer " + token,
                    "User-Agent": self.headers.get(
                        "User-Agent",
                        "finite-search-searxng-token-proxy/0.1",
                    ),
                },
                method="GET",
            )

            try:
                with urllib.request.urlopen(request, timeout=25) as response:
                    body = response.read()
                    self.send_response(response.status)
                    for key, value in response.headers.items():
                        if key.lower() not in HOP_BY_HOP_HEADERS:
                            self.send_header(key, value)
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
            except urllib.error.HTTPError as exc:
                body = exc.read()
                self._send_bytes(exc.code, body, "text/plain")
            except Exception as exc:
                body = f"upstream error: {exc}\n".encode("utf-8")
                self._send_bytes(502, body, "text/plain")

        def _send_bytes(self, status: int, body: bytes, content_type: str) -> None:
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    return SearXNGTokenProxy


def main() -> int:
    args = parse_args()
    token = token_from_env()
    handler = make_handler(args.upstream_url, token)
    server = http.server.ThreadingHTTPServer((args.listen_host, args.port), handler)
    print(
        f"searxng token proxy listening on http://{args.listen_host}:{args.port}; "
        f"upstream={args.upstream_url}",
        flush=True,
    )
    server.serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
import hmac
import json
import os
import sys
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


TOKEN = os.environ.get("FINITE_SEARCH_TOKEN", "")
UPSTREAM = os.environ.get("FINITE_SEARCH_UPSTREAM", "http://127.0.0.1:8080").rstrip("/")
PORT = int(os.environ.get("FINITE_SEARCH_PROXY_PORT", "8081"))
TIMEOUT = float(os.environ.get("FINITE_SEARCH_UPSTREAM_TIMEOUT", "20"))


def respond_json(handler, status, payload, headers=None):
    body = json.dumps(payload, sort_keys=True).encode("utf-8")
    handler.send_response(status)
    handler.send_header("Content-Type", "application/json")
    handler.send_header("Content-Length", str(len(body)))
    if headers:
        for key, value in headers.items():
            handler.send_header(key, value)
    handler.end_headers()
    if handler.command != "HEAD":
        handler.wfile.write(body)


def authorized(headers):
    auth = headers.get("Authorization", "")
    candidate = ""
    if auth.startswith("Bearer "):
        candidate = auth.removeprefix("Bearer ").strip()
    elif headers.get("X-Finite-Search-Token"):
        candidate = headers.get("X-Finite-Search-Token", "").strip()

    if not candidate:
        return False, "missing"
    if not hmac.compare_digest(candidate, TOKEN):
        return False, "invalid"
    return True, ""


class SearchProxy(BaseHTTPRequestHandler):
    server_version = "finite-search-auth-proxy/0.1"

    def do_HEAD(self):
        self.handle_request()

    def do_GET(self):
        self.handle_request()

    def handle_request(self):
        if not TOKEN:
            respond_json(self, 500, {"error": "auth token is not configured"})
            return

        path = self.path.split("?", 1)[0]
        if path == "/healthz":
            self.handle_healthz()
            return

        if path != "/search" and not path.startswith("/search/"):
            respond_json(self, 404, {"error": "not found"})
            return

        ok, reason = authorized(self.headers)
        if not ok:
            status = 401 if reason == "missing" else 403
            headers = {"WWW-Authenticate": 'Bearer realm="finite-search"'} if status == 401 else None
            respond_json(self, status, {"error": f"{reason} bearer token"}, headers)
            return

        self.forward_to_searxng()

    def handle_healthz(self):
        try:
            with urllib.request.urlopen(f"{UPSTREAM}/healthz", timeout=5) as response:
                if 200 <= response.status < 300:
                    respond_json(self, 200, {"ok": True})
                    return
                respond_json(self, 502, {"ok": False, "upstream_status": response.status})
        except Exception as exc:
            respond_json(self, 502, {"ok": False, "error": str(exc)})

    def forward_to_searxng(self):
        target = f"{UPSTREAM}{self.path}"
        request = urllib.request.Request(target, method=self.command)
        for header in ("Accept", "Accept-Language", "User-Agent"):
            value = self.headers.get(header)
            if value:
                request.add_header(header, value)

        try:
            with urllib.request.urlopen(request, timeout=TIMEOUT) as response:
                body = b"" if self.command == "HEAD" else response.read()
                self.send_response(response.status)
                for key, value in response.headers.items():
                    lowered = key.lower()
                    if lowered in {"connection", "content-length", "transfer-encoding"}:
                        continue
                    self.send_header(key, value)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                if body:
                    self.wfile.write(body)
        except urllib.error.HTTPError as exc:
            body = b"" if self.command == "HEAD" else exc.read()
            self.send_response(exc.code)
            content_type = exc.headers.get("Content-Type")
            if content_type:
                self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            if body:
                self.wfile.write(body)
        except Exception as exc:
            respond_json(self, 502, {"error": "upstream request failed", "detail": str(exc)})

    def log_message(self, fmt, *args):
        sys.stderr.write("%s - %s\n" % (self.address_string(), fmt % args))


def main():
    if not TOKEN:
        sys.stderr.write("FINITE_SEARCH_TOKEN is required\n")
        return 1

    server = ThreadingHTTPServer(("0.0.0.0", PORT), SearchProxy)
    sys.stderr.write(f"finite-search auth proxy listening on :{PORT}, upstream={UPSTREAM}\n")
    server.serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

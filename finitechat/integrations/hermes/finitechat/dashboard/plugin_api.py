"""Direct-transport Finite Chat proxy. Native Hermes authenticates before routing.

The loopback chat bridge verifies the user-signed `x-finite-chat-authorization`
assertion on every viewer request, so a native session alone cannot act as a
chat user. Hermes credentials and cookies are never forwarded to the bridge.
"""

import os
from urllib.parse import unquote

import anyio
import httpx
from fastapi import APIRouter, Request
from fastapi.responses import JSONResponse, StreamingResponse
from starlette.types import Receive, Scope, Send

router = APIRouter()
DEFAULT_BRIDGE_URL = "http://127.0.0.1:37633"
# The bridge accepts 64 MiB of attachments; the rest is multipart framing.
MAX_BODY_BYTES = 70 * 1024 * 1024
REQUEST_HEADERS = (
    "content-type",
    "x-finite-chat-authorization",
    "x-finite-requester-email",
    "x-finite-sites-requester-assertion",
)
RESPONSE_HEADERS = (
    "content-type",
    "content-disposition",
    "x-content-type-options",
    "content-security-policy",
    "cache-control",
)
# Runtime-command waits stop at 60 s and update streams send state every 30 s.
TIMEOUT = httpx.Timeout(120, connect=5)
_client: httpx.AsyncClient | None = None


def error(status: int, message: str, kind: str, retryable: bool) -> JSONResponse:
    body = {"error": message, "error_kind": kind, "retryable": retryable}
    return JSONResponse(body, status_code=status)


def bridge_client() -> httpx.AsyncClient:
    global _client
    if _client is None:
        # Loopback only, so ignore proxy settings. Unbounded connections: a
        # leaked stream must never exhaust a pool and take Chat down with it.
        limits = httpx.Limits(max_connections=None)
        _client = httpx.AsyncClient(timeout=TIMEOUT, limits=limits, trust_env=False)
    return _client


def bridge_target(request: Request, path: str) -> str | None:
    """The bridge path and query exactly as sent; the viewer assertion signs them."""
    if "\\" in path or any(part in ("", ".", "..") for part in path.split("/")):
        return None
    suffix = f"/v1/app/{path}"
    prefix = request.scope["path"].removesuffix(suffix)
    # `path` is percent-decoded; `raw_path` keeps the client's encoding.
    raw = request.scope.get("raw_path", b"").decode("latin-1")
    target = raw[len(prefix) :]
    if not raw.startswith(prefix) or unquote(target) != suffix:
        return None
    query = request.scope.get("query_string", b"").decode("latin-1")
    return f"{target}?{query}" if query else target


async def read_body(request: Request) -> bytes | None:
    """The complete request body (the assertion signs its hash), or None if too large."""
    declared = request.headers.get("content-length", "")
    if declared.isdigit() and int(declared) > MAX_BODY_BYTES:
        return None
    body = bytearray()
    async for chunk in request.stream():
        body += chunk
        if len(body) > MAX_BODY_BYTES:
            return None
    return bytes(body)


class BridgeResponse(StreamingResponse):
    """Relays each bridge chunk as it arrives (SSE events, attachment bytes)."""

    def __init__(self, upstream: httpx.Response):
        headers = {
            key: upstream.headers[key] for key in RESPONSE_HEADERS if key in upstream.headers
        }
        super().__init__(upstream.aiter_bytes(), upstream.status_code, headers)
        self.upstream = upstream

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        try:
            await super().__call__(scope, receive, send)
        finally:
            # Release the bridge stream on completion, disconnect or cancellation.
            with anyio.CancelScope(shield=True):
                await self.upstream.aclose()


@router.api_route("/v1/app/{path:path}", methods=["GET", "POST"])
async def bridge(request: Request, path: str):
    target = bridge_target(request, path)
    if target is None:
        return error(404, "Unknown Finite Chat route", "not_found", False)
    body = await read_body(request)
    if body is None:
        return error(413, "Request body exceeds 70 MiB", "payload_too_large", False)
    headers = {key: request.headers[key] for key in REQUEST_HEADERS if key in request.headers}
    headers["accept-encoding"] = "identity"
    base = os.environ.get("FINITECHAT_HERMES_SERVICE_URL") or DEFAULT_BRIDGE_URL
    client = bridge_client()
    outgoing = client.build_request(
        request.method, base.rstrip("/") + target, headers=headers, content=body
    )
    try:
        upstream = await client.send(outgoing, stream=True)
    except httpx.TransportError:
        return error(503, "Finite Chat bridge is unavailable", "bridge_unavailable", True)
    if upstream.status_code == 404 and not is_json(upstream):
        # A bridge without the app API answers its routes with a bare 404.
        # That is an older sidecar under this plugin, which stays in the
        # Hermes home when the agent is rolled back to an image without
        # direct chat. Say so in the bridge's own envelope, so the dashboard
        # keeps that agent's chat on the hosted device.
        await upstream.aclose()
        return error(404, "Finite Chat direct transport is not served", "not_found", False)
    return BridgeResponse(upstream)


def is_json(response: httpx.Response) -> bool:
    content_type = response.headers.get("content-type", "")
    return content_type.split(";")[0].strip().lower() == "application/json"

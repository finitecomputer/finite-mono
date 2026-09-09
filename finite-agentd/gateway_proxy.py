#!/usr/bin/env python3
"""Public Hermes protocol proxy; the runner owns routing, the runtime owns access.

No credential store, user data store, per-agent configuration, or protocol
translation. The private listener exposes Caddy's certificate permission API.
"""

import asyncio
import ipaddress
import json
import os
import re
import time
from contextlib import suppress
from pathlib import Path

from aiohttp import (
    ClientError,
    ClientSession,
    ClientTimeout,
    WSMsgType,
    WSServerHandshakeError,
    web,
)
from yarl import URL

HOP_HEADERS = {
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
}


def runtime_for_host(host, domain):
    match = re.fullmatch(r"r-([0-9a-f]{20})\." + re.escape(domain), host.lower())
    return f"runtime_{match[1]}" if match else None


def forwarding_headers(headers):
    excluded = HOP_HEADERS | {
        part.strip().lower() for part in headers.get("Connection", "").split(",")
    }
    return {
        key: value
        for key, value in headers.items()
        if key.lower() not in excluded
        and not key.lower().startswith("x-forwarded-")
        and key.lower() not in {"forwarded", "x-finite-agent-account-id"}
    }


class GatewayProxy:
    def __init__(self, routes_path, domain):
        self.routes_path = Path(routes_path)
        self.domain = domain
        self.networks = tuple(
            ipaddress.ip_network(value) for value in ("10.4.0.0/24", "10.89.0.0/16")
        )
        self.session = None

    async def start(self):
        self.session = ClientSession(
            auto_decompress=False, timeout=ClientTimeout(total=None, connect=5)
        )

    async def close(self):
        await self.session.close()

    async def resolve(self, host):
        runtime_id = runtime_for_host(host, self.domain)
        if not runtime_id:
            raise web.HTTPNotFound()
        try:
            inventory = json.loads(self.routes_path.read_text())
            age = time.time() - inventory["generated_at"]
            if not 0 <= age < 15:
                raise ValueError("stale inventory")
            route = inventory["routes"].get(runtime_id)
            if route is None:
                raise web.HTTPNotFound()
            address = ipaddress.ip_address(route["address"])
            account_id = route["account_id"]
            if not any(address in network for network in self.networks) or not re.fullmatch(
                r"[0-9a-f]{64}", account_id
            ):
                raise ValueError("invalid ingress coordinates")
            return f"http://{address}:8080", account_id
        except (OSError, ValueError, KeyError, TypeError):
            raise web.HTTPServiceUnavailable(text="Runtime ingress is unavailable") from None

    async def certificate_permission(self, request):
        await self.resolve(request.query.get("domain", ""))
        return web.Response(status=204)

    async def forward(self, request):
        target, principal = await self.resolve(request.host.split(":", 1)[0])
        url = URL(f"{target}/gateway{request.raw_path}", encoded=True)
        headers = forwarding_headers(request.headers)
        headers["X-Finite-Agent-Account-Id"] = principal
        try:
            if request.headers.get("Upgrade", "").lower() == "websocket":
                return await self.websocket(request, url, headers)
            async with self.session.request(
                request.method,
                url,
                headers=headers,
                data=request.content.iter_chunked(65536),
                allow_redirects=False,
            ) as upstream:
                response = web.StreamResponse(
                    status=upstream.status, headers=forwarding_headers(upstream.headers)
                )
                await response.prepare(request)
                async for chunk in upstream.content.iter_chunked(65536):
                    await response.write(chunk)
                await response.write_eof()
                return response
        except WSServerHandshakeError as error:
            return web.Response(status=error.status if 400 <= error.status < 600 else 502)
        except (TimeoutError, ClientError, OSError):
            raise web.HTTPBadGateway(text="Gateway is unavailable") from None

    async def websocket(self, request, url, headers):
        protocols = [
            part.strip()
            for part in request.headers.get("Sec-WebSocket-Protocol", "").split(",")
            if part.strip()
        ]
        # aiohttp creates its own handshake keys. Payloads are relayed verbatim.
        headers = {
            key: value
            for key, value in headers.items()
            if not key.lower().startswith("sec-websocket-")
        }
        async with self.session.ws_connect(
            url, headers=headers, protocols=protocols, autoping=False, max_msg_size=32 * 1024 * 1024
        ) as upstream:
            downstream = web.WebSocketResponse(
                protocols=[upstream.protocol] if upstream.protocol else (),
                autoping=False,
                max_msg_size=32 * 1024 * 1024,
            )
            await downstream.prepare(request)

            async def copy(source, destination):
                try:
                    async for message in source:
                        if message.type == WSMsgType.TEXT:
                            await destination.send_str(message.data)
                        elif message.type == WSMsgType.BINARY:
                            await destination.send_bytes(message.data)
                        elif message.type == WSMsgType.PING:
                            await destination.ping(message.data)
                        elif message.type == WSMsgType.PONG:
                            await destination.pong(message.data)
                        else:
                            break
                finally:
                    code = source.close_code or 1001
                    if code in {1004, 1005, 1006, 1015}:
                        code = 1001
                    await destination.close(code=code)

            tasks = [
                asyncio.create_task(copy(upstream, downstream)),
                asyncio.create_task(copy(downstream, upstream)),
            ]
            try:
                await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
            finally:
                for task in tasks:
                    task.cancel()
                await asyncio.gather(*tasks, return_exceptions=True)
            return downstream

    def public_app(self):
        app = web.Application(client_max_size=64 * 1024 * 1024)
        app.router.add_route("*", "/{path:.*}", self.forward)
        return app

    def private_app(self):
        app = web.Application()
        app.router.add_get("/certificate-permission", self.certificate_permission)
        app.router.add_get("/healthz", lambda _: web.Response(text="ok"))
        return app


async def main():
    proxy = GatewayProxy(os.environ["FINITE_GATEWAY_ROUTES"], os.environ["FINITE_GATEWAY_DOMAIN"])
    await proxy.start()
    runners = []
    try:
        for app, port in [(proxy.public_app(), 8792), (proxy.private_app(), 8793)]:
            # Never access-log query tokens, authorization headers, or transcripts.
            runner = web.AppRunner(app, access_log=None)
            await runner.setup()
            runners.append(runner)
            await web.TCPSite(runner, "127.0.0.1", port).start()
        await asyncio.Event().wait()
    finally:
        for runner in runners:
            await runner.cleanup()
        await proxy.close()


if __name__ == "__main__":
    with suppress(KeyboardInterrupt):
        asyncio.run(main())

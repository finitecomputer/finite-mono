#!/usr/bin/env python3
"""Small least-active streaming router for four local Laguna vLLM replicas.

The router deliberately retries only before an upstream response begins. Once
OpenAI SSE bytes have been returned, replaying a request would duplicate model
output and potentially tool side effects.
"""

from __future__ import annotations

import asyncio
import os
from dataclasses import dataclass

from aiohttp import ClientError, ClientSession, ClientTimeout, TCPConnector, web


@dataclass
class Backend:
    url: str
    active: int = 0
    failures: int = 0
    served: int = 0


BACKENDS = [
    Backend(url)
    for url in os.environ.get(
        "LAGUNA_BACKENDS",
        "http://127.0.0.1:8001,http://127.0.0.1:8002,"
        "http://127.0.0.1:8003,http://127.0.0.1:8004",
    ).split(",")
]
MAX_ACTIVE_PER_BACKEND = int(os.environ.get("LAGUNA_MAX_ACTIVE_PER_BACKEND", "128"))
CAPACITY_CHANGED = asyncio.Condition()
HOP_BY_HOP = {
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
}


async def acquire_backend(excluded: set[str]) -> Backend:
    async with CAPACITY_CHANGED:
        await CAPACITY_CHANGED.wait_for(
            lambda: any(
                backend.url not in excluded
                and backend.active < MAX_ACTIVE_PER_BACKEND
                for backend in BACKENDS
            )
        )
        candidates = [
            backend
            for backend in BACKENDS
            if backend.url not in excluded
            and backend.active < MAX_ACTIVE_PER_BACKEND
        ]
        backend = min(
            candidates, key=lambda candidate: (candidate.failures, candidate.active)
        )
        backend.active += 1
        return backend


async def release_backend(backend: Backend) -> None:
    async with CAPACITY_CHANGED:
        backend.active -= 1
        CAPACITY_CHANGED.notify_all()


async def proxy(request: web.Request) -> web.StreamResponse:
    body = await request.read()
    headers = {
        key: value
        for key, value in request.headers.items()
        if key.lower() not in HOP_BY_HOP and key.lower() != "host"
    }
    attempted: set[str] = set()
    last_error: Exception | None = None

    for _ in range(len(BACKENDS)):
        backend = await acquire_backend(attempted)
        attempted.add(backend.url)
        upstream = None
        try:
            upstream = await request.app["client"].request(
                request.method,
                backend.url + request.rel_url.path_qs,
                data=body,
                headers=headers,
            )
        except (ClientError, asyncio.TimeoutError) as error:
            last_error = error
            backend.failures += 1
            await release_backend(backend)
            continue

        # An upstream response means model execution may already have occurred.
        # Never replay after this point, even if the client disconnects before
        # aiohttp can prepare the downstream response.
        try:
            backend.served += 1
            response_headers = {
                key: value
                for key, value in upstream.headers.items()
                if key.lower() not in HOP_BY_HOP
            }
            downstream = web.StreamResponse(
                status=upstream.status,
                reason=upstream.reason,
                headers=response_headers,
            )
            await downstream.prepare(request)
            async for chunk in upstream.content.iter_any():
                await downstream.write(chunk)
            await downstream.write_eof()
            backend.failures = 0
            return downstream
        finally:
            upstream.release()
            await release_backend(backend)

    raise web.HTTPBadGateway(text=f"all Laguna replicas unavailable: {last_error}")


async def health(request: web.Request) -> web.Response:
    checks = await asyncio.gather(
        *(
            request.app["client"].get(backend.url + "/health")
            for backend in BACKENDS
        ),
        return_exceptions=True,
    )
    healthy = [
        not isinstance(result, Exception) and result.status == 200
        for result in checks
    ]
    for backend, result, is_healthy in zip(BACKENDS, checks, healthy, strict=True):
        if is_healthy:
            backend.failures = 0
        if not isinstance(result, Exception):
            result.release()
    status = 200 if all(healthy) else 503
    return web.json_response(
        {
            "healthy": sum(healthy),
            "total": len(BACKENDS),
            "replicas": [
                {
                    "active": backend.active,
                    "failures": backend.failures,
                    "served": backend.served,
                }
                for backend in BACKENDS
            ],
        },
        status=status,
    )


async def start_client(app: web.Application) -> None:
    app["client"] = ClientSession(
        connector=TCPConnector(limit=0, ttl_dns_cache=300),
        timeout=ClientTimeout(total=None, connect=10, sock_read=900),
    )


async def stop_client(app: web.Application) -> None:
    await app["client"].close()


def main() -> None:
    app = web.Application(client_max_size=32 * 1024 * 1024)
    app.on_startup.append(start_client)
    app.on_cleanup.append(stop_client)
    app.router.add_get("/health", health)
    app.router.add_route("*", "/{path:.*}", proxy)
    web.run_app(app, host="0.0.0.0", port=int(os.environ.get("PORT", "8000")))


if __name__ == "__main__":
    main()

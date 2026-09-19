"""Bounded CLI reads for bundled, product-owned Hermes dashboard APIs.

This module is sealed into Hermes' Python package. Product routers choose the
commands; no request can supply argv, cwd, environment, identity or an upstream.
"""

import asyncio
import json
from collections.abc import Awaitable, Callable

from fastapi import Request
from fastapi.responses import JSONResponse

# Leave room inside the dashboard's 15-second owner-grant/read budget. These
# bounds apply across both products, including simultaneous browser tabs.
READ_SECONDS = 10
MAX_OUTPUT_BYTES = 512 * 1024
_PROCESSES = asyncio.Semaphore(4)
_REQUESTS = asyncio.Semaphore(2)


class AccessUnverified(Exception):
    """CLI failed; its human-readable stderr is not an authorization contract."""


class InvalidInventory(Exception):
    """Unexpected or oversized product response; never return a partial list."""


async def read_json(*argv: str):
    async with _PROCESSES:
        process = await asyncio.create_subprocess_exec(
            *argv,
            stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.DEVNULL,
            cwd="/",
            limit=64 * 1024,
        )
        try:
            output = bytearray()
            while chunk := await process.stdout.read(64 * 1024):
                if len(output) + len(chunk) > MAX_OUTPUT_BYTES:
                    raise InvalidInventory()
                output.extend(chunk)
            if await process.wait() != 0:
                raise AccessUnverified()
            try:
                return json.loads(output)
            except (ValueError, UnicodeDecodeError, RecursionError) as error:
                raise InvalidInventory() from error
        finally:
            # Native CLIs do not spawn child processes. Reap on output limits,
            # deadlines and cancellation; abandoning a request must not leak CLI work.
            if process.returncode is None:
                process.kill()
            await process.wait()


async def inventory_response(request: Request, read: Callable[[], Awaitable[dict]]):
    headers = {"Cache-Control": "no-store"}
    if request.query_params or request.headers.get("content-length", "0") != "0":
        return JSONResponse({"error": "This read takes no parameters."}, 400, headers)
    try:
        async with asyncio.timeout(READ_SECONDS):
            async with _REQUESTS:
                result = await read()
        response = JSONResponse(result, headers=headers)
        if len(response.body) > MAX_OUTPUT_BYTES:
            raise InvalidInventory()
        return response
    except AccessUnverified:
        # Neither CLI promises machine-readable errors. Never parse stderr or
        # retain old inventory when signer/service authorization is unverified.
        return JSONResponse({"error": "Product access could not be verified."}, 403, headers)
    except (TimeoutError, OSError, InvalidInventory):
        return JSONResponse({"error": "Product inventory is unavailable."}, 503, headers)


def record(value):
    if not isinstance(value, dict):
        raise InvalidInventory()
    return value


def text(value, maximum=512):
    if not isinstance(value, str) or not value.strip() or len(value) > maximum:
        raise InvalidInventory()
    return value


def records(value, maximum):
    if not isinstance(value, list) or len(value) > maximum:
        raise InvalidInventory()
    return [record(item) for item in value]

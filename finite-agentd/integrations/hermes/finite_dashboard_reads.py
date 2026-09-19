"""Bounded CLI reads for bundled, product-owned Hermes dashboard APIs.

This module is sealed into Hermes' Python package. Product routers choose the
commands; no request can supply argv, cwd, environment, identity or an upstream.
"""

import asyncio
import json
from collections.abc import Awaitable, Callable
from contextlib import suppress

from fastapi import Request
from fastapi.responses import JSONResponse

# Leave room inside the dashboard's 15-second owner-grant/read budget. These
# bounds apply across both products, including simultaneous browser tabs.
READ_SECONDS = 10
MAX_OUTPUT_BYTES = 512 * 1024
_PROCESSES = asyncio.Semaphore(4)
_REQUESTS = asyncio.Semaphore(2)


class AccessUnverified(Exception):
    """CLI access failed, or its error contract is unrecognized."""


class InvalidInventory(Exception):
    """Unexpected or oversized product response; never return a partial list."""


class ProductUnavailable(Exception):
    """A typed CLI transport/service failure allows a visibly stale result."""


async def _read_pipe(stream, limit):
    output = bytearray()
    while chunk := await stream.read(64 * 1024):
        if len(output) + len(chunk) > limit:
            raise InvalidInventory()
        output.extend(chunk)
    return output


async def _read_diagnostics(stream):
    try:
        return await _read_pipe(stream, 8192)
    except InvalidInventory as error:
        # Oversized diagnostics cannot attest a typed retryable failure.
        raise AccessUnverified() from error


async def read_json(*argv: str):
    async with _PROCESSES:
        process = await asyncio.create_subprocess_exec(
            *argv,
            stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            cwd="/",
            limit=64 * 1024,
        )
        stdout = asyncio.create_task(_read_pipe(process.stdout, MAX_OUTPUT_BYTES))
        stderr = asyncio.create_task(_read_diagnostics(process.stderr))
        try:
            output, diagnostics = await asyncio.gather(stdout, stderr)
            if await process.wait() != 0:
                try:
                    failure = json.loads(diagnostics)
                except (ValueError, UnicodeDecodeError, RecursionError):
                    failure = None
                if (
                    isinstance(failure, dict)
                    and type(failure.get("inventory_error_version")) is int
                    and failure["inventory_error_version"] == 1
                    and failure.get("kind") == "request"
                ):
                    raise ProductUnavailable()
                # Unknown/older CLI error contracts fail closed, never by
                # heuristically parsing human-readable diagnostic text.
                raise AccessUnverified()
            try:
                return json.loads(output)
            except (ValueError, UnicodeDecodeError, RecursionError) as error:
                raise InvalidInventory() from error
        finally:
            # Native CLIs do not spawn child processes. Reap on output limits,
            # deadlines and cancellation; abandoning a request must not leak CLI work.
            if process.returncode is None:
                with suppress(ProcessLookupError):
                    process.kill()
            stdout.cancel()
            stderr.cancel()
            await asyncio.gather(stdout, stderr, return_exceptions=True)
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
        # Do not expose diagnostics or retain inventory on unverified access.
        return JSONResponse({"error": "Product access could not be verified."}, 403, headers)
    except (TimeoutError, OSError, InvalidInventory, ProductUnavailable):
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

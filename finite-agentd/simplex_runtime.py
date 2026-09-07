#!/usr/bin/env python3
"""Managed SimpleX lifecycle/control. Uses the unmodified Hermes adapter.

No arbitrary command endpoint. The CLI is called only by agentd using fixed
operations; pairing remains Hermes-owned. The daemon alone opens its databases.
"""

import argparse
import asyncio
import contextlib
import json
import os
from pathlib import Path
import signal
import sys
import uuid

import websockets
import yaml

WS_URL = "ws://127.0.0.1:5225"


def settings():
    home = Path(os.environ.get("HERMES_HOME", "/data/agent/hermes-home"))
    doc = yaml.safe_load((home / "config.yaml").read_text()) or {}
    return doc.get("gateway", {}).get("platforms", {}).get("simplex", {})


def managed_enabled():
    cfg = settings()
    return (
        cfg.get("enabled") is True
        and cfg.get("extra", {}).get("finite_managed") is True
    )


async def command(text, timeout=5):
    # Require our response, not the first unsolicited broadcast on the socket.
    corr = "finite-" + uuid.uuid4().hex
    async with asyncio.timeout(timeout):
        async with websockets.connect(
            WS_URL, max_size=2**20, open_timeout=timeout
        ) as ws:
            await ws.send(json.dumps({"corrId": corr, "cmd": text}))
            async for raw in ws:
                event = json.loads(raw)
                if event.get("corrId") == corr:
                    resp = event.get("resp", {})
                    if resp.get("type") in ("chatCmdError", "chatError"):
                        raise RuntimeError("SimpleX rejected the control operation")
                    return resp
    raise RuntimeError("SimpleX did not answer the control operation")


def address_from(resp):
    # v7.0.2: create returns connLinkContact; show nests it in contactLink.
    container = resp.get("contactLink", resp)
    link = container.get("connLinkContact", {})
    if not isinstance(link, dict):
        raise RuntimeError("SimpleX returned an unsupported contact address")
    address = link.get("connShortLink") or link.get("connFullLink", "")
    if (
        not isinstance(address, str)
        or not address.startswith(("https://", "simplex:/"))
        or len(address) > 4096
    ):
        raise RuntimeError("SimpleX returned an unsupported contact address")
    return address


def state_dir():
    return Path(os.environ.get("FINITECHAT_HOME", "/data/agent")) / "simplex"


def saved_address():
    for suffix in ("_chat.db", "_agent.db"):
        if not (state_dir() / ("identity" + suffix)).is_file():
            raise RuntimeError("Retained SimpleX identity is incomplete")
    value = json.loads((state_dir() / "address.json").read_text())
    return address_from({"connLinkContact": {"connShortLink": value["address"]}})


async def tcp_ready():
    # Deliberately no WebSocket handshake: additional WS clients steal events
    # from the daemon's shared queue. TCP liveness is not message-path health.
    try:
        _, writer = await asyncio.wait_for(
            asyncio.open_connection("127.0.0.1", 5225), 1
        )
        writer.close()
        await writer.wait_closed()
        return True
    except (OSError, TimeoutError):
        return False


async def status():
    result = {"enabled": managed_enabled(), "ready": False, "address": None}
    if result["enabled"]:
        result["address"] = saved_address()
        try:
            pid = int((state_dir() / "daemon.pid").read_text())
            if pid <= 1:
                raise ValueError("Invalid daemon PID")
            os.kill(pid, 0)
            result["ready"] = await tcp_ready()
        except (OSError, ValueError):
            pass
    return result


async def start_child(home):
    for folder in [home, home / "files", home / "tmp"]:
        folder.mkdir(mode=0o700, parents=True, exist_ok=True)
        os.chmod(folder, 0o700)
    profile_args = (
        []
        if (home / "identity_chat.db").exists()
        else ["--user-display-name", "FiniteAgent"]
    )
    return await asyncio.create_subprocess_exec(
        "simplex-chat",
        "-d",
        str(home / "identity"),
        "-p",
        "5225",
        "--mute",
        *profile_args,
        "--files-folder",
        str(home / "files"),
        "--temp-folder",
        str(home / "tmp"),
        stdin=asyncio.subprocess.DEVNULL,
        stdout=asyncio.subprocess.DEVNULL,
        stderr=asyncio.subprocess.DEVNULL,
    )


async def create_address():
    home = state_dir()
    if (home / "address.json").exists():
        return {"address": saved_address()}
    if managed_enabled() or any(home.glob("identity*")):
        raise RuntimeError(
            "Retained SimpleX state has no saved address; review before setup"
        )
    if await tcp_ready():
        raise RuntimeError("SimpleX port is occupied by an existing daemon")
    # Bootstrap only a brand-new, unexposed identity. No owner can have paired
    # yet, and Hermes has not been enabled. Never open this control WS again.
    child = await start_child(home)
    try:
        for attempt in range(20):
            if child.returncode is not None:
                raise RuntimeError("SimpleX could not start")
            if await tcp_ready():
                break
            await asyncio.sleep(0.25)
        address = address_from(await command("/address", timeout=20))
        # v7 emits receivedContactRequest, which the released Hermes adapter's
        # legacy contactRequest handler misses. Let the daemon accept transport
        # connections; Hermes still gates every DM with explicit owner pairing.
        accepted = await command("/auto_accept on", timeout=20)
        if accepted.get("type") != "userContactLinkUpdated":
            raise RuntimeError("SimpleX could not enable contact acceptance")
        # Atomically persist before enabling Hermes or exposing the address.
        temporary = home / "address.json.tmp"
        with temporary.open("w") as out:
            json.dump({"address": address}, out)
            out.flush()
            os.fsync(out.fileno())
        temporary.replace(home / "address.json")
        return {"address": address}
    finally:
        await stop_child(child)


async def stop_child(child):
    if child and child.returncode is None:
        child.terminate()
        try:
            await asyncio.wait_for(child.wait(), 10)
        except TimeoutError:
            child.kill()
            await child.wait()


async def supervise():
    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(sig, stop.set)
    child = None
    pid_file = state_dir() / "daemon.pid"
    pid_file.unlink(missing_ok=True)
    try:
        while not stop.is_set():
            try:
                enabled = managed_enabled()
                if enabled:
                    saved_address()  # Fail closed if retained state is incomplete.
            except (OSError, ValueError, RuntimeError, AttributeError, yaml.YAMLError):
                enabled = False
            if not enabled:
                await stop_child(child)
                child = None
                pid_file.unlink(missing_ok=True)
            elif child is None or child.returncode is not None:
                # Do not attach control clients to a running daemon; Hermes
                # must be its sole event consumer, including during restarts.
                pid_file.unlink(missing_ok=True)
                if not await tcp_ready():
                    child = await start_child(state_dir())
                    pid_file.write_text(str(child.pid))
                    print("SimpleX daemon started", file=sys.stderr, flush=True)
            with contextlib.suppress(TimeoutError):
                await asyncio.wait_for(stop.wait(), 2)
    finally:
        await stop_child(child)
        pid_file.unlink(missing_ok=True)


def gateway():
    # This only exports the managed platform configuration. User-installed
    # adapters/profiles remain untouched. No daemon URL is exported by default.
    if settings().get("extra", {}).get("finite_managed") is True:
        if managed_enabled():
            os.environ["SIMPLEX_WS_URL"] = WS_URL
        else:
            os.environ.pop("SIMPLEX_WS_URL", None)
        os.environ["SIMPLEX_AUTO_ACCEPT"] = "true"
        os.environ["SIMPLEX_ALLOW_ALL_USERS"] = "false"
        os.environ["SIMPLEX_ALLOWED_USERS"] = ""
        os.environ["SIMPLEX_GROUP_ALLOWED"] = ""
    os.execvp("hermes", ["hermes", "gateway", "run", "--replace"])


def pending_requests():
    from gateway.pairing import PairingStore

    return [
        {
            "request_id": row["request_id"],
            "user_id": str(row["user_id"]),
            "name": (row.get("user_name") or "")[:128],
            "age_minutes": max(0, row["age_minutes"]),
        }
        for row in PairingStore().list_pending("simplex")
        if PairingStore.looks_like_request_id(row.get("request_id", ""))
        and row.get("user_id")
    ]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "operation",
        choices=[
            "supervise",
            "status",
            "address",
            "gateway",
            "approve",
            "approve-request",
        ],
    )
    operation = parser.parse_args().operation
    os.umask(0o077)
    if operation == "gateway":
        gateway()
    elif operation == "supervise":
        asyncio.run(supervise())
    else:
        try:
            if operation == "approve-request":
                from gateway.pairing import PairingStore

                request_id = sys.stdin.read(64).strip().lower()
                if not PairingStore.looks_like_request_id(request_id):
                    raise RuntimeError("Invalid SimpleX connection request")
                if PairingStore().approve_request("simplex", request_id) is None:
                    raise RuntimeError(
                        "This connection request expired or was already handled. Refresh and try again."
                    )
                result = {"approved": True}
            elif operation == "approve":
                from gateway.pairing import PairingStore

                code = sys.stdin.read(64).strip().upper()
                if len(code) != 8 or any(
                    c not in "ABCDEFGHJKLMNPQRSTUVWXYZ23456789" for c in code
                ):
                    raise RuntimeError(
                        "Enter the eight-character pairing code from SimpleX"
                    )
                approved = PairingStore().approve_code("simplex", code)
                if approved is None:
                    raise RuntimeError(
                        "Pairing code is invalid, expired, or temporarily locked. Request a new code and try again."
                    )
                result = {"approved": True}
            else:
                result = asyncio.run(
                    status() if operation == "status" else create_address()
                )
            if operation == "status":
                result["pending"] = pending_requests()
            print(json.dumps(result))
        except Exception as error:
            # Only our fixed RuntimeError messages are user-visible; never
            # emit raw protocol events, third-party errors, or chat contents.
            message = (
                str(error)
                if type(error) is RuntimeError
                else "SimpleX is unavailable or returned an unsupported response"
            )
            print(json.dumps({"error": message}))
            sys.exit(1)


if __name__ == "__main__":
    main()

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
import re
from pathlib import Path
import signal
import shutil
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
    if reset_marker().exists():
        result["reset_pending"] = True
        return result
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
    if reset_marker().exists():
        raise RuntimeError("SimpleX disconnect is unfinished; retry Disconnect first")
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


def reset_marker():
    return state_dir().parent / "simplex-reset.json"


def atomic_json(path, value):
    temporary = path.with_suffix(path.suffix + ".tmp")
    with temporary.open("w") as out:
        json.dump(value, out)
        out.flush()
        os.fsync(out.fileno())
    temporary.replace(path)


def prepare_reset():
    if settings().get("extra", {}).get("finite_managed") is not True or managed_enabled():
        raise RuntimeError("Disable managed SimpleX before disconnecting")
    # Durable intent survives interruption. Never bootstrap or approve while set.
    if not reset_marker().exists():
        atomic_json(reset_marker(), {"reset": True})
    return {"reset_pending": True}


async def finish_reset():
    """Run only in the gateway wrapper, after the old gateway has stopped.

    Native trials call this after joining their gateway child. No live database
    is removed, and the SimpleX supervisor observes disabled config before this.
    """
    if not reset_marker().exists():
        return
    if managed_enabled():
        raise RuntimeError("SimpleX disconnect is unfinished; keep it disabled")
    for _ in range(60):
        if not (state_dir() / "daemon.pid").exists() and not await tcp_ready():
            break
        await asyncio.sleep(.25)
    else:
        raise RuntimeError("SimpleX did not stop; disconnect has not cleared its data")
    home = Path(os.environ["HERMES_HOME"])
    # Clear both layouts: Hermes can merge the legacy directory back on startup.
    # The old gateway is stopped, so no other adapter can race the shared file.
    for folder in (home / "pairing", home / "platforms" / "pairing"):
        for suffix in ("pending", "approved"):
            (folder / f"simplex-{suffix}.json").unlink(missing_ok=True)
        limits = folder / "_rate_limits.json"
        if limits.exists():
            values = json.loads(limits.read_text())
            atomic_json(limits, {k: v for k, v in values.items()
                                if not k.startswith("simplex:")
                                and k not in ("_lockout:simplex", "_failures:simplex")})
    clear_simplex_sessions(home)
    if state_dir().exists():
        shutil.rmtree(state_dir())
    (state_dir().parent / "pairing.png").unlink(missing_ok=True)
    reset_marker().unlink()


def clear_simplex_sessions(home):
    # Use pinned upstream APIs; never rewrite shared SQLite tables. Journal
    # transcript IDs before deletion so a file error can be retried after its
    # database row is gone. The marker is also the interrupted-reset barrier.
    from gateway.config import load_gateway_config
    from hermes_state import SessionDB

    sessions = load_gateway_config().sessions_dir
    mirror = sessions / "sessions.json"
    entries = json.loads(mirror.read_text()) if mirror.exists() else {}
    def is_simplex(entry):
        # Hermes writes _README metadata and tolerates non-record sentinels.
        if not isinstance(entry, dict):
            return False
        origin = entry.get("origin")
        return entry.get("platform") == "simplex" or (
            isinstance(origin, dict) and origin.get("platform") == "simplex"
        )
    selected = {k: v for k, v in entries.items() if not k.startswith("_") and is_simplex(v)}
    plan = json.loads(reset_marker().read_text())
    ids = set(plan.get("session_ids", []))
    ids.update(v["session_id"] for v in selected.values())
    db = SessionDB() if (home / "state.db").exists() else None
    try:
        scope = str(sessions.resolve())
        keys = []
        roots = []
        if db:
            for key, raw in db.load_gateway_routing_entries(scope=scope).items():
                entry = json.loads(raw)
                if is_simplex(entry):
                    keys.append(key)
                    ids.add(entry["session_id"])
            offset = 0
            while True:
                rows = db.list_sessions_rich(source="simplex", limit=100, offset=offset,
                    include_children=True, include_archived=True, include_hidden=True,
                    project_compression_tips=False, compact_rows=True)
                if not rows:
                    break
                roots.extend(row["id"] for row in rows)
                offset += len(rows)
            for sid in roots:
                ids.update(db.get_session_delete_targets(sid))
        if any(not isinstance(sid, str) or not re.fullmatch(r"[A-Za-z0-9_-]+", sid) for sid in ids):
            raise RuntimeError("SimpleX session metadata needs review before disconnecting")
        plan["session_ids"] = sorted(ids)
        atomic_json(reset_marker(), plan)
        if db:
            db.delete_gateway_routing_entries(keys, scope=scope)
            db.delete_sessions(roots, sessions_dir=sessions)
        if mirror.exists():
            atomic_json(mirror, {k: v for k, v in entries.items() if k not in selected})
        for sid in ids:
            for suffix in (".json", ".jsonl"):
                (sessions / (sid + suffix)).unlink(missing_ok=True)
            for path in sessions.glob(f"request_dump_{sid}_*.json"):
                path.unlink()
    finally:
        if db:
            db.close()


async def wait_reset():
    for _ in range(100):
        if not reset_marker().exists():
            return {"disconnected": True}
        await asyncio.sleep(.25)
    raise RuntimeError("SimpleX disconnect is unfinished; retry Disconnect")


def gateway():
    try:
        asyncio.run(finish_reset())
    except Exception:
        # A failed SimpleX cleanup must not take other chat platforms offline.
        # Keep durable intent for retry; managed config is already disabled.
        if managed_enabled():
            raise
        print("SimpleX disconnect is unfinished; retry Disconnect", file=sys.stderr)
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
            "prepare-reset",
            "wait-reset",
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
            if operation == "prepare-reset":
                result = prepare_reset()
            elif operation == "wait-reset":
                result = asyncio.run(wait_reset())
            elif operation == "approve-request":
                from gateway.pairing import PairingStore

                if reset_marker().exists():
                    raise RuntimeError("SimpleX disconnect is unfinished")
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

                if reset_marker().exists():
                    raise RuntimeError("SimpleX disconnect is unfinished")
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

#!/usr/bin/env python3
"""Managed SimpleX lifecycle/control. Owns daemon settings; Hermes owns message events.

No arbitrary command endpoint. The CLI is called only by agentd using fixed
operations; pairing remains Hermes-owned. The daemon alone opens its databases.
"""

import argparse
import asyncio
import contextlib
import copy
import json
import os
import re
import shutil
import signal
import sys
import uuid
from pathlib import Path

import websockets
import yaml

CHAT_PORT = 5225
WS_URL = f"ws://127.0.0.1:{CHAT_PORT}"
SETUP_PORT = 5226
SETUP_TIMEOUT = 5
# Official SimpleX v7.0.2 Flux identities, including their certificate fingerprints.
FLUX_SERVERS = (
    "xftp://92Sctlc09vHl_nAqF2min88zKyjdYJ9mgxRCJns5K2U=@xftp1.simplexonflux.com,apl3pumq3emwqtrztykyyoomdx4dg6ysql5zek2bi3rgznz7ai3odkid.onion",
    "xftp://YBXy4f5zU1CEhnbbCzVWTNVNsaETcAGmYqGNxHntiE8=@xftp2.simplexonflux.com,c5jjecisncnngysah3cz2mppediutfelco4asx65mi75d44njvua3xid.onion",
    "xftp://ARQO74ZSvv2OrulRF3CdgwPz_AMy27r0phtLSq5b664=@xftp3.simplexonflux.com,dc4mohiubvbnsdfqqn7xhlhpqs5u4tjzp7xpz6v6corwvzvqjtaqqiqd.onion",
    "xftp://ub2jmAa9U0uQCy90O-fSUNaYCj6sdhl49Jh3VpNXP58=@xftp4.simplexonflux.com,4qq5pzier3i4yhpuhcrhfbl6j25udc4czoyascrj4yswhodhfwev3nyd.onion",
    "xftp://Rh19D5e4Eez37DEE9hAlXDB3gZa1BdFYJTPgJWPO9OI=@xftp5.simplexonflux.com,q7itltdn32hjmgcqwhow4tay5ijetng3ur32bolssw32fvc5jrwvozad.onion",
    "xftp://0AznwoyfX8Od9T_acp1QeeKtxUi676IBIiQjXVwbdyU=@xftp6.simplexonflux.com,upvzf23ou6nrmaf3qgnhd6cn3d74tvivlmz3p7wdfwq6fhthjrjiiqid.onion",
)


def settings():
    home = Path(os.environ.get("HERMES_HOME", "/data/agent/hermes-home"))
    doc = yaml.safe_load((home / "config.yaml").read_text()) or {}
    return doc.get("gateway", {}).get("platforms", {}).get("simplex", {})


def managed_enabled():
    cfg = settings()
    return cfg.get("enabled") is True and cfg.get("extra", {}).get("finite_managed") is True


async def command(text, timeout=5, *, url=None):
    # Require our response, not the first unsolicited broadcast on the socket.
    corr = "finite-" + uuid.uuid4().hex
    async with asyncio.timeout(timeout):
        async with websockets.connect(url or WS_URL, max_size=2**20, open_timeout=timeout) as ws:
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


async def tcp_ready(port=None):
    # Deliberately no WebSocket handshake: additional WS clients steal events
    # from the daemon's shared queue. TCP liveness is not message-path health.
    try:
        _, writer = await asyncio.wait_for(
            asyncio.open_connection("127.0.0.1", CHAT_PORT if port is None else port), 1
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


async def start_child(home, *, maintenance=False):
    for folder in [home, home / "files", home / "tmp"]:
        folder.mkdir(mode=0o700, parents=True, exist_ok=True)
        os.chmod(folder, 0o700)
    profile_args = (
        [] if (home / "identity_chat.db").exists() else ["--user-display-name", "FiniteAgent"]
    )
    return await asyncio.create_subprocess_exec(
        "simplex-chat",
        "-d",
        str(home / "identity"),
        "-p",
        str(SETUP_PORT if maintenance else CHAT_PORT),
        "--mute",
        *(["--maintenance"] if maintenance else []),
        *profile_args,
        "--files-folder",
        str(home / "files"),
        "--temp-folder",
        str(home / "tmp"),
        stdin=asyncio.subprocess.DEVNULL,
        stdout=asyncio.subprocess.DEVNULL,
        stderr=asyncio.subprocess.DEVNULL,
    )


def relay_settings(groups):
    """Preserve owner policy; only extend the default managed server layout."""
    operators = [g for g in groups if g.get("operator")]
    customs = [g for g in groups if not g.get("operator")]
    if (
        len(operators) != 1
        or operators[0]["operator"].get("operatorTag") != "simplex"
        or operators[0]["operator"].get("enabled") is not True
        or len(customs) != 1
        or customs[0].get("smpServers")
        or customs[0].get("chatRelays")
        or customs[0].get("xftpServers")
    ):
        return groups
    known = {s["server"] for g in groups for s in g["xftpServers"]}
    updated = copy.deepcopy(groups)
    custom = next(g for g in updated if not g.get("operator"))
    for server in FLUX_SERVERS:
        if server not in known:
            custom["xftpServers"].append(
                {
                    "serverId": None,
                    "server": server,
                    "preset": False,
                    "enabled": False,
                    "roles": {},
                    "deleted": False,
                }
            )
    return updated


async def configure_relays(home, *, url=None):
    user = (await command("/u", url=url))["user"]["userId"]
    get = f"/_servers {user}"
    current = (await command(get, url=url))["userServers"]
    backup = home / "flux-relays-before.json"
    # The backup is also durable intent. After interruption, compare the actual
    # state with that intent instead of treating our inserted rows as owner policy.
    before = json.loads(backup.read_text()) if backup.exists() else current
    after = relay_settings(before)
    recovering = current != before
    if not recovering and after == before:
        atomic_json(home / "flux-relays-ready.json", {"changed": False})
        return
    if not recovering:
        encoded = json.dumps(after)
        valid = await command(f"/_validate_servers {user} {encoded}", url=url)
        if (
            valid.get("serverErrors")
            or valid.get("serverWarnings")
            or valid.get("type") != "userServersValidation"
        ):
            raise RuntimeError("SimpleX relay settings failed validation")
        if not backup.exists():
            atomic_json(backup, before)
        await command(f"{get} {encoded}", url=url)
        applied = (await command(get, url=url))["userServers"]
    else:
        applied = current
    atomic_json(home / "flux-relays-after.json", applied)
    rollback = copy.deepcopy(before)
    old_ids = {s["serverId"] for g in before for s in g["xftpServers"]}
    inserted = [
        s
        for g in applied
        for s in g["xftpServers"]
        if s["serverId"] not in old_ids and s["server"] in FLUX_SERVERS
    ]
    custom = next(g for g in rollback if not g.get("operator"))
    custom["xftpServers"].extend(dict(s, deleted=True) for s in inserted)
    atomic_json(home / "flux-relays-rollback.json", rollback)
    normalized = copy.deepcopy(applied)
    for group in normalized:
        for entry in group["xftpServers"]:
            if entry["serverId"] not in old_ids:
                entry["serverId"] = None
    if normalized != after:
        if recovering:
            # An owner may have edited settings since the interrupted attempt.
            raise RuntimeError("Interrupted SimpleX relay setup needs operator review")
        await command(f"{get} {json.dumps(rollback)}", url=url)
        restored = (await command(get, url=url))["userServers"]
        if restored != before:
            raise RuntimeError("SimpleX settings rollback needs operator review")
        raise RuntimeError("SimpleX settings read-back differed; restored original settings")
    atomic_json(home / "flux-relays-ready.json", {"changed": True})


async def prepare_relays(home):
    if (home / "flux-relays-ready.json").exists():
        return
    # Configure on a separate port without subscribing contacts or starting file workers.
    # Hermes can keep reconnecting to 5225 without consuming setup responses.
    if await tcp_ready() or await tcp_ready(SETUP_PORT):
        raise RuntimeError("SimpleX setup requires both daemon ports to be idle")
    if not all((home / ("identity" + suffix)).is_file() for suffix in ("_chat.db", "_agent.db")):
        raise RuntimeError("SimpleX relay preparation requires an existing identity")
    child = await start_child(home, maintenance=True)
    try:
        async with asyncio.timeout(SETUP_TIMEOUT):
            for _ in range(40):
                await asyncio.sleep(0.25)
                if child.returncode is not None:
                    raise RuntimeError("SimpleX settings daemon could not start")
                if await tcp_ready(SETUP_PORT):
                    break
            else:
                raise RuntimeError("SimpleX settings daemon did not become ready")
            url = f"ws://127.0.0.1:{SETUP_PORT}"
            await command("/_start main=off snd_files=off", url=url)
            await configure_relays(home, url=url)
    finally:
        await stop_child(child, timeout=2)


async def create_address():
    if reset_marker().exists():
        raise RuntimeError("SimpleX disconnect is unfinished; retry Disconnect first")
    home = state_dir()
    if (home / "address.json").exists():
        return {"address": saved_address()}
    if managed_enabled() or any(home.glob("identity*")):
        raise RuntimeError("Retained SimpleX state has no saved address; review before setup")
    if await tcp_ready():
        raise RuntimeError("SimpleX port is occupied by an existing daemon")
    # Bootstrap only a brand-new, unexposed identity. No owner can have paired
    # yet, and Hermes has not been enabled. Never open this control WS again.
    child = await start_child(home)
    try:
        for _ in range(20):
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
        try:
            await configure_relays(home)
        except Exception:
            # A failed settings update must not strand a new pairing identity.
            print("SimpleX relay setup failed; keeping current settings", file=sys.stderr)
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


async def stop_child(child, timeout=10):
    if child and child.returncode is None:
        child.terminate()
        try:
            await asyncio.wait_for(child.wait(), timeout)
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
                    try:
                        await prepare_relays(state_dir())
                    except Exception:
                        # Existing chat must remain available if setup needs review.
                        print(
                            "SimpleX relay setup failed; keeping current settings", file=sys.stderr
                        )
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
        await asyncio.sleep(0.25)
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
            atomic_json(
                limits,
                {
                    k: v
                    for k, v in values.items()
                    if not k.startswith("simplex:")
                    and k not in ("_lockout:simplex", "_failures:simplex")
                },
            )
    clear_simplex_sessions(home)
    if state_dir().exists():
        shutil.rmtree(state_dir())
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
                rows = db.list_sessions_rich(
                    source="simplex",
                    limit=100,
                    offset=offset,
                    include_children=True,
                    include_archived=True,
                    include_hidden=True,
                    project_compression_tips=False,
                    compact_rows=True,
                )
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
        await asyncio.sleep(0.25)
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
            os.environ["SIMPLEX_FILES_FOLDER"] = str(state_dir() / "files")
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
        if PairingStore.looks_like_request_id(row.get("request_id", "")) and row.get("user_id")
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
            else:
                result = asyncio.run(status() if operation == "status" else create_address())
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

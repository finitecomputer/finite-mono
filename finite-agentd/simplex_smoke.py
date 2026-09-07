"""Opt-in real relay test; uses two disposable identities, no inference or real users.

Run scripts/simplex-smoke. This proves adapter routing, not gateway authorization,
inference, phone UX, or messages arriving during a gateway outage.
"""

import asyncio
import importlib.util
import json
import os
import socket
import sys
import tempfile
from pathlib import Path
import websockets


BIN = "simplex-chat"
PLUGINS = Path(os.environ["HERMES_BUNDLED_PLUGINS"])


def port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Peer:
    def __init__(self, p):
        self.port = p
        self.events = asyncio.Queue()
        self.pending = {}

    async def open(self):
        for i in range(40):
            try:
                self.ws = await websockets.connect(
                    f"ws://127.0.0.1:{self.port}", max_size=2**22
                )
                break
            except OSError:
                await asyncio.sleep(0.25)
        else:
            raise RuntimeError("daemon not ready")
        self.task = asyncio.create_task(self.listen())

    async def listen(self):
        async for raw in self.ws:
            e = json.loads(raw)
            c = e.get("corrId")
            r = e.get("resp", {})
            if c in self.pending:
                self.pending.pop(c).set_result(r)
            else:
                await self.events.put(r)

    async def cmd(self, c):
        ident = str(id(c)) + str(asyncio.get_running_loop().time())
        f = asyncio.get_running_loop().create_future()
        self.pending[ident] = f
        await self.ws.send(json.dumps({"corrId": ident, "cmd": c}))
        return await asyncio.wait_for(f, 30)

    async def until(self, pred):
        async with asyncio.timeout(90):
            while True:
                e = await self.events.get()
                if pred(e):
                    return e

    async def close(self):
        await self.ws.close()
        await self.task


async def run():
    with tempfile.TemporaryDirectory(prefix="finite-simplex-smoke-") as td:
        os.environ["HERMES_HOME"] = td + "/hermes"
        os.environ["HERMES_BUNDLED_PLUGINS"] = str(PLUGINS)
        os.environ["SIMPLEX_AUTO_ACCEPT"] = "false"
        from gateway.config import PlatformConfig

        spec = importlib.util.spec_from_file_location(
            "finite_test_simplex", PLUGINS / "platforms/simplex/adapter.py"
        )
        mod = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = mod
        spec.loader.exec_module(mod)
        ports = [port(), port()]
        procs = []
        peers = []
        adapter = None
        try:
            for i, p in enumerate(ports):
                procs.append(
                    await asyncio.create_subprocess_exec(
                        BIN,
                        "-d",
                        td + f"/peer{i}",
                        "-p",
                        str(p),
                        "--mute",
                        "--user-display-name",
                        f"FiniteTest{i}",
                        stdout=asyncio.subprocess.DEVNULL,
                        stderr=asyncio.subprocess.DEVNULL,
                    )
                )
                peer = Peer(p)
                await peer.open()
                peers.append(peer)
            invitation = await peers[0].cmd("/address")
            link = invitation["connLinkContact"]["connShortLink"]
            acceptance = await peers[0].cmd("/auto_accept on")
            assert acceptance["type"] == "userContactLinkUpdated"
            await peers[0].close()
            adapter = mod.SimplexAdapter(
                PlatformConfig(
                    enabled=True, extra={"ws_url": f"ws://127.0.0.1:{ports[0]}"}
                )
            )
            incoming = asyncio.Queue()

            async def handle(event):
                await incoming.put(event)
                return "synthetic adapter reply"

            adapter.set_message_handler(handle)
            assert await adapter.connect()
            await asyncio.sleep(0.5)
            result = await peers[1].cmd("/connect " + link)
            assert result["type"] != "chatCmdError", result["type"]
            other = await peers[1].until(lambda e: e.get("type") == "contactConnected")
            human_contact = str(other["contact"]["contactId"])
            msg = json.dumps(
                [{"msgContent": {"type": "text", "text": "synthetic inbound"}}]
            )
            response = await peers[1].cmd(f"/_send @{human_contact} json {msg}")
            assert response["type"] != "chatCmdError", response["type"]
            event = await asyncio.wait_for(incoming.get(), 30)
            assert event.source.chat_id.isdecimal()
            assert event.text == "synthetic inbound"

            def reply(e):
                items = (
                    e.get("chatItems", []) if e.get("type") == "newChatItems" else []
                )
                return any(
                    i.get("chatItem", {})
                    .get("content", {})
                    .get("msgContent", {})
                    .get("text")
                    == "synthetic adapter reply"
                    for i in items
                )

            await peers[1].until(reply)
            print(
                "PASS: actual released Hermes adapter received numeric-contact DM and replied through two real SimpleX daemons"
            )
        finally:
            if adapter:
                await adapter.disconnect()
            for peer in peers:
                await peer.close()
            for proc in procs:
                if proc.returncode is None:
                    proc.terminate()
            for proc in procs:
                try:
                    await asyncio.wait_for(proc.wait(), 10)
                except TimeoutError:
                    proc.kill()
                    await proc.wait()


asyncio.run(run())

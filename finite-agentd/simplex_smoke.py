"""Opt-in real relay test; uses three disposable identities, no inference or real users.

Run just computer simplex-smoke. This proves topic tool/admission and adapter routing, not full gateway inference,
phone UX, or messages arriving during a gateway outage.
"""

import asyncio
import json
import os
import shutil
import socket
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
        for _ in range(40):
            try:
                self.ws = await websockets.connect(f"ws://127.0.0.1:{self.port}", max_size=2**22)
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
        from gateway.config import GatewayConfig, Platform, PlatformConfig
        from gateway.pairing import PairingStore
        from gateway.platform_registry import platform_registry
        from gateway.run import GatewayRunner
        from hermes_cli.plugins import get_plugin_manager

        os.environ["FINITECHAT_HOME"] = td + "/agent"
        home = Path(os.environ["HERMES_HOME"])
        home.mkdir(exist_ok=True)
        (home / "config.yaml").write_text("plugins:\n  enabled: [finitechat]\n")
        shutil.copytree(
            Path(__file__).resolve().parents[1] / "finitechat/integrations/hermes/finitechat",
            home / "plugins/finitechat",
        )
        get_plugin_manager().discover_and_load()
        runner = GatewayRunner(config=GatewayConfig())

        ports = [port(), port(), port()]
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
            adapter = platform_registry.get("simplex").adapter_factory(
                PlatformConfig(
                    enabled=True,
                    extra={"ws_url": f"ws://127.0.0.1:{ports[0]}", "finite_managed": True},
                )
            )
            incoming = asyncio.Queue()

            async def handle(event):
                await incoming.put(event)
                return "synthetic adapter reply"

            adapter.set_message_handler(handle)
            assert await adapter.connect()
            runner.adapters[Platform("simplex")] = adapter
            await asyncio.sleep(0.5)
            result = await peers[1].cmd("/connect " + link)
            assert result["type"] != "chatCmdError", result["type"]
            other = await peers[1].until(lambda e: e.get("type") == "contactConnected")
            human_contact = str(other["contact"]["contactId"])
            msg = json.dumps([{"msgContent": {"type": "text", "text": "synthetic inbound"}}])
            response = await peers[1].cmd(f"/_send @{human_contact} json {msg}")
            assert response["type"] != "chatCmdError", response["type"]
            event = await asyncio.wait_for(incoming.get(), 30)
            assert event.source.chat_id.isdecimal()
            assert event.text == "synthetic inbound"

            def reply(e):
                items = e.get("chatItems", []) if e.get("type") == "newChatItems" else []
                return any(
                    i.get("chatItem", {}).get("content", {}).get("msgContent", {}).get("text")
                    == "synthetic adapter reply"
                    for i in items
                )

            await peers[1].until(reply)
            # Owner-only topics use the exact same live adapter/socket.
            owner = event.source.user_id
            pairing = PairingStore()
            code = pairing.generate_code("simplex", owner, "Synthetic owner")
            assert pairing.approve_code("simplex", code)
            from gateway.session_context import clear_session_vars, set_session_vars
            from model_tools import handle_function_call
            from tools.registry import registry

            tool = registry.get_entry("simplex_create_topic")
            assert "error" in json.loads(
                await asyncio.to_thread(tool.handler, {"topic": "gardening"})
            )
            tokens = set_session_vars(
                platform="simplex", chat_id=owner, chat_type="dm", user_id=owner
            )
            try:
                result = json.loads(
                    await asyncio.to_thread(
                        handle_function_call, "simplex_create_topic", {"topic": "gardening"}
                    )
                )
            finally:
                clear_session_vars(tokens)
            gid = result["group_id"]
            assert (await adapter.create_topic("Gardening", owner))["group_id"] == gid
            invitation = await peers[1].until(lambda e: e.get("type") == "receivedGroupInvitation")
            invited = invitation["groupInfo"]
            phone_gid = str(invited["groupId"])
            joined = await peers[1].cmd(f"/_join #{phone_gid}")
            assert joined["type"] != "chatCmdError", joined
            await peers[1].until(lambda e: e.get("type") == "userJoinedGroup")
            await asyncio.sleep(1)
            await peers[1].cmd(f"/_send #{phone_gid} json " + msg)
            topic_event = await asyncio.wait_for(incoming.get(), 30)
            assert topic_event.source.chat_id == f"group:{gid}"
            assert topic_event.source.user_id == owner
            assert topic_event.source.chat_type == "group"
            await peers[1].until(reply)
            # Reconstruct adapter against retained daemon + registry.
            cfg = adapter.config
            await adapter.disconnect()
            stock_class = next(c for c in type(adapter).__mro__ if c.__name__ == "SimplexAdapter")
            adapter = stock_class(cfg)
            assert not adapter.group_allow_from
            adapter.set_message_handler(handle)
            assert await adapter.connect()
            runner.adapters[Platform("simplex")] = adapter
            await asyncio.sleep(0.5)
            await peers[1].cmd(f"/_send #{phone_gid} json " + msg)
            await peers[1].cmd(f"/_send @{human_contact} json " + msg)
            rollback_event = await asyncio.wait_for(incoming.get(), 30)
            assert rollback_event.source.chat_type == "dm"
            await peers[1].until(reply)
            await adapter.disconnect()
            adapter = platform_registry.get("simplex").adapter_factory(cfg)
            adapter.set_message_handler(handle)
            assert gid in adapter.group_allow_from
            assert await adapter.connect()
            runner.adapters[Platform("simplex")] = adapter
            await asyncio.sleep(0.5)
            assert (await adapter.create_topic("gardening", owner))["group_id"] == gid
            await peers[1].cmd(f"/_send #{phone_gid} json " + msg)
            assert (await asyncio.wait_for(incoming.get(), 30)).source.user_id == owner
            await peers[1].until(reply)
            # A third party added outside the supported tool must close the
            # private topic before any response can leave the adapter.
            await peers[2].cmd("/connect " + link)
            await peers[2].until(lambda e: e.get("type") == "contactConnected")
            contacts = (await adapter.command("/contacts"))["contacts"]
            stranger = next(
                c["contactId"] for c in contacts if c["localDisplayName"] == "FiniteTest2"
            )
            await adapter.command(f"/_add #{gid} {stranger} member")
            result = await adapter.send(f"group:{gid}", "must never be sent")
            assert not result.success, "private reply escaped after membership changed"
            assert any(r.get("blocked") for r in adapter.topics.values())
            tokens = set_session_vars(
                platform="simplex", chat_id=owner, chat_type="dm", user_id=owner
            )
            try:
                recovery = json.loads(
                    await asyncio.to_thread(
                        handle_function_call,
                        "simplex_create_topic",
                        {"topic": "gardening", "replace_blocked": True},
                    )
                )
            finally:
                clear_session_vars(tokens)
            replacement_gid = recovery["group_id"]
            assert replacement_gid != gid and recovery["replaced_group_id"] == gid
            invitation = await peers[1].until(lambda e: e.get("type") == "receivedGroupInvitation")
            replacement_phone_gid = str(invitation["groupInfo"]["groupId"])
            await peers[1].cmd(f"/_join #{replacement_phone_gid}")
            await peers[1].until(lambda e: e.get("type") == "userJoinedGroup")
            await peers[1].cmd(f"/_send #{replacement_phone_gid} json " + msg)
            replacement_event = await asyncio.wait_for(incoming.get(), 30)
            assert replacement_event.source.chat_id == f"group:{replacement_gid}"
            await peers[1].until(reply)
            await adapter.disconnect()
            adapter = platform_registry.get("simplex").adapter_factory(cfg)
            adapter.set_message_handler(handle)
            assert await adapter.connect()
            runner.adapters[Platform("simplex")] = adapter
            await asyncio.sleep(0.5)
            retried = await adapter.create_topic("gardening", owner, replace_blocked=True)
            assert retried["group_id"] == replacement_gid
            assert not (await adapter.send(f"group:{gid}", "old topic stays disabled")).success
            assert adapter.topics[f"retired:{gid}"]["blocked"]
            print("PASS: explicit replacement survives restart; old topic stays disabled")
            print("PASS: unexpected third member blocks private outbound traffic")
            print(
                "PASS: owner topic created, invitation accepted, contact identity mapped, round trip and adapter restart/retry preserve group"
            )
            print("PASS: pinned Hermes adapter extension preserved the private DM round trip")
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

import asyncio
import copy
import json
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest.mock import patch

from aiohttp import ClientSession, WSServerHandshakeError, web
from aiohttp.test_utils import TestServer
from yarl import URL

import gateway_ingress
from gateway_inventory import routes_from_containers
from gateway_proxy import GatewayProxy, runtime_for_host

RUNTIME = "runtime_" + "a" * 20
ACCOUNT = "b" * 64
TOKEN = "c" * 64
DOMAIN = "agents.lat3.finite.computer"
HOST = "r-" + "a" * 20 + "." + DOMAIN


class InventoryTests(unittest.TestCase):
    def test_only_unambiguous_canonical_owned_running_container_is_routable(self):
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "kata" / RUNTIME
            (state / "agent").mkdir(parents=True)
            (state / "agent/config.json").write_text(json.dumps({"account_id": ACCOUNT}))
            container = {
                "Name": "/machine-a",
                "State": {"Status": "running"},
                "Config": {
                    "Labels": {
                        "computer.finite.v2." + key: value
                        for key, value in {
                            "runtime": "true",
                            "source_host_id": "finite-lat-3",
                            "source_machine_id": "machine-a",
                            "project_id": "project-a",
                        }.items()
                    }
                },
                "Mounts": [{"Source": str(state), "Destination": "/data", "RW": True}],
                "NetworkSettings": {"Networks": {"finite": {"IPAddress": "10.89.0.2"}}},
            }

            def routes(records):
                return routes_from_containers(records, directory, "finite-lat-3")

            self.assertEqual(
                routes([container]), {RUNTIME: {"address": "10.89.0.2", "account_id": ACCOUNT}}
            )
            self.assertEqual(routes([container, container]), {})
            for mutation in (
                lambda c: c.update(Name="/recovery-helper"),
                lambda c: c["State"].update(Status="exited"),
                lambda c: c["Config"]["Labels"].update(
                    {"computer.finite.v2.source_host_id": "other"}
                ),
                lambda c: c["Mounts"][0].update(Source="/different/" + RUNTIME),
                lambda c: c["NetworkSettings"]["Networks"]["finite"].update(IPAddress="10.254.3.2"),
            ):
                candidate = copy.deepcopy(container)
                mutation(candidate)
                self.assertEqual(routes([candidate]), {})
            deployed = copy.deepcopy(container)
            deployed["NetworkSettings"]["Networks"] = {
                "unknown-eth0": {"IPAddress": "10.4.0.217"},
                "unknown-tap0_kata": {"IPAddress": ""},
            }
            self.assertEqual(routes([deployed])[RUNTIME]["address"], "10.4.0.217")
            deployed["NetworkSettings"]["Networks"]["another"] = {"IPAddress": "10.89.0.3"}
            self.assertEqual(routes([deployed]), {})
            (state / "agent/config.json").unlink()
            self.assertEqual(routes([container]), {})

    def test_exact_hostnames(self):
        self.assertEqual(runtime_for_host(HOST, DOMAIN), RUNTIME)
        for host in ("evil." + HOST, HOST + ".evil.test", DOMAIN, "r-123." + DOMAIN):
            self.assertIsNone(runtime_for_host(host, DOMAIN))


class ForwardingTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.home = Path(self.directory.name)
        (self.home / "agentd").mkdir()
        (self.home / "config.json").write_text(json.dumps({"account_id": ACCOUNT}))
        self.set_config(True)
        self.requests = []

        async def upstream(request):
            self.requests.append((request.method, request.raw_path, dict(request.headers)))
            if (
                request.headers.get("Host") != "127.0.0.1:9120"
                or request.headers.get("X-Hermes-Session-Token") != TOKEN
            ):
                return web.Response(status=403)
            if request.headers.get("Upgrade", "").lower() == "websocket":
                ws = web.WebSocketResponse()
                await ws.prepare(request)
                await ws.send_str("ready immediately")
                async for message in ws:
                    if message.type.name == "TEXT":
                        await ws.send_str(message.data)
                    elif message.type.name == "BINARY":
                        await ws.send_bytes(message.data)
                return ws
            return web.Response(body=await request.read() or b"private html containing a token")

        app = web.Application()
        app.router.add_route("*", "/{path:.*}", upstream)
        self.backend = TestServer(app)
        await self.backend.start_server()
        # Redirect only the fixed loopback dial to our ephemeral test server.
        original_connect = gateway_ingress.socket.create_connection

        def connect(address, **kwargs):
            self.assertEqual(address, ("127.0.0.1", 9120))
            return original_connect(("127.0.0.1", self.backend.port), **kwargs)

        self.dial = patch.object(gateway_ingress.socket, "create_connection", connect)
        self.dial.start()
        home = self.home

        class Handler(BaseHTTPRequestHandler):
            rbufsize = 0

            def do_GET(self):
                gateway_ingress.forward(self, home)

            do_POST = do_GET

            def log_message(self, *_args):
                pass

        self.ingress = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.ingress.serve_forever, daemon=True)
        self.thread.start()
        self.proxy = GatewayProxy(self.home / "routes.json", DOMAIN)
        await self.proxy.start()

        # Exercise real public HTTP/WS forwarding; route target is local fixture.
        async def resolve(host):
            if host != HOST:
                raise web.HTTPNotFound()
            return f"http://127.0.0.1:{self.ingress.server_port}", ACCOUNT

        self.proxy.resolve = resolve
        self.public = TestServer(self.proxy.public_app())
        await self.public.start_server()
        self.client = ClientSession()

    def set_config(self, enabled):
        (self.home / "agentd/hosted-gateway.json").write_text(
            json.dumps({"enabled": enabled, "token": TOKEN if enabled else None})
        )

    async def asyncTearDown(self):
        await self.client.close()
        await self.public.close()
        await self.proxy.close()
        await asyncio.to_thread(self.ingress.shutdown)
        self.ingress.server_close()
        self.thread.join()
        self.dial.stop()
        await self.backend.close()
        self.directory.cleanup()

    async def test_all_routes_require_auth_and_disable_refuses_new_clients(self):
        for path in ("/", "/api/sessions", "/assets/main.js"):
            async with self.client.get(
                self.public.make_url(path), headers={"Host": HOST}
            ) as response:
                self.assertEqual(response.status, 401)
        self.assertEqual(self.requests, [])
        async with self.client.post(
            URL(str(self.public.make_url("/api/example")) + "?a=%2F", encoded=True),
            data=b"payload",
            headers={
                "Host": HOST,
                "Authorization": "Bearer " + TOKEN,
                "Origin": "http://localhost:3000",
            },
        ) as response:
            self.assertEqual(response.status, 200)
            self.assertEqual(await response.read(), b"payload")
        method, path, headers = self.requests[-1]
        self.assertEqual((method, path), ("POST", "/api/example?a=%2F"))
        self.assertNotIn("Origin", headers)
        self.assertNotIn("X-Finite-Agent-Account-Id", headers)
        self.set_config(False)
        async with self.client.get(
            self.public.make_url("/"), headers={"Host": HOST, "Authorization": "Bearer " + TOKEN}
        ) as response:
            self.assertEqual(response.status, 401)

    async def test_websocket_first_event_binary_and_failed_auth(self):
        with self.assertRaises(WSServerHandshakeError) as caught:
            await self.client.ws_connect(self.public.make_url("/api/ws"), headers={"Host": HOST})
        self.assertEqual(caught.exception.status, 401)
        async with self.client.ws_connect(
            self.public.make_url("/api/ws?token=" + TOKEN), headers={"Host": HOST}
        ) as ws:
            self.assertEqual(await ws.receive_str(timeout=3), "ready immediately")
            await ws.send_str('{"method":"session.list","id":1}')
            self.assertEqual(await ws.receive_str(timeout=3), '{"method":"session.list","id":1}')
            await ws.send_bytes(b"\x00\xff")
            self.assertEqual(await ws.receive_bytes(timeout=3), b"\x00\xff")

    async def test_reused_address_with_different_principal_fails_closed(self):
        async with self.client.get(
            f"http://127.0.0.1:{self.ingress.server_port}/gateway/",
            headers={"X-Finite-Agent-Account-Id": "d" * 64, "Authorization": "Bearer " + TOKEN},
        ) as response:
            self.assertEqual(response.status, 421)
        self.assertEqual(self.requests, [])


class ResolutionTests(unittest.IsolatedAsyncioTestCase):
    async def test_inventory_expiry_missing_and_outside_network_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "routes.json"
            proxy = GatewayProxy(path, DOMAIN)
            for age, address, valid in [
                (0, "10.89.0.2", True),
                (30, "10.89.0.2", False),
                (0, "10.254.3.2", False),
                (-30, "10.89.0.2", False),
            ]:
                path.write_text(
                    json.dumps(
                        {
                            "generated_at": time.time() - age,
                            "routes": {RUNTIME: {"address": address, "account_id": ACCOUNT}},
                        }
                    )
                )
                if valid:
                    self.assertEqual(await proxy.resolve(HOST), ("http://10.89.0.2:8080", ACCOUNT))
                else:
                    with self.assertRaises(web.HTTPServiceUnavailable):
                        await proxy.resolve(HOST)
            path.unlink()
            with self.assertRaises(web.HTTPServiceUnavailable):
                await proxy.resolve(HOST)

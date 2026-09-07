import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import AsyncMock, patch

import websockets
import simplex_runtime as runtime


class ControlTests(unittest.IsolatedAsyncioTestCase):
    async def test_correlates_response_and_ignores_broadcast(self):
        async def server(ws):
            request = json.loads(await ws.recv())
            await ws.send(
                json.dumps({"resp": {"type": "newChatItems", "chatItems": []}})
            )
            await ws.send(
                json.dumps({"corrId": "someone-else", "resp": {"type": "cmdOk"}})
            )
            await ws.send(
                json.dumps(
                    {
                        "corrId": request["corrId"],
                        "resp": {"type": "chats", "chats": []},
                    }
                )
            )

        async with websockets.serve(server, "127.0.0.1", 0) as service:
            port = service.sockets[0].getsockname()[1]
            with patch.object(runtime, "WS_URL", f"ws://127.0.0.1:{port}"):
                self.assertEqual((await runtime.command("/chats"))["type"], "chats")

    async def test_daemon_error_is_failure(self):
        async def server(ws):
            request = json.loads(await ws.recv())
            await ws.send(
                json.dumps(
                    {"corrId": request["corrId"], "resp": {"type": "chatCmdError"}}
                )
            )

        async with websockets.serve(server, "127.0.0.1", 0) as service:
            port = service.sockets[0].getsockname()[1]
            with patch.object(runtime, "WS_URL", f"ws://127.0.0.1:{port}"):
                with self.assertRaises(RuntimeError):
                    await runtime.command("/address")

    async def test_open_socket_without_command_response_times_out(self):
        async def server(ws):
            await ws.recv()
            await ws.wait_closed()

        async with websockets.serve(server, "127.0.0.1", 0) as service:
            port = service.sockets[0].getsockname()[1]
            with patch.object(runtime, "WS_URL", f"ws://127.0.0.1:{port}"):
                with self.assertRaises(TimeoutError):
                    await runtime.command("/chats", timeout=0.05)

    async def test_disabled_connection_does_not_contact_daemon(self):
        with patch.object(runtime, "managed_enabled", return_value=False):
            self.assertEqual(
                await runtime.status(),
                {"enabled": False, "ready": False, "address": None},
            )

    async def test_live_status_never_opens_a_websocket(self):
        with (
            patch.object(runtime, "managed_enabled", return_value=True),
            patch.object(
                runtime, "saved_address", return_value="https://smp.example/a#synthetic"
            ),
            patch.object(
                runtime,
                "command",
                new=AsyncMock(
                    side_effect=AssertionError("must not consume live events")
                ),
            ),
        ):
            result = await runtime.status()
            self.assertEqual(result["address"], "https://smp.example/a#synthetic")


class AddressTests(unittest.TestCase):
    def test_real_v7_create_and_show_shapes(self):
        link = {
            "connShortLink": "https://smp.example/a#synthetic",
            "connFullLink": "simplex:/contact#synthetic",
        }
        self.assertEqual(
            runtime.address_from({"connLinkContact": link}), link["connShortLink"]
        )
        self.assertEqual(
            runtime.address_from({"contactLink": {"connLinkContact": link}}),
            link["connShortLink"],
        )

    def test_malformed_address_does_not_become_a_link(self):
        for value in [
            {},
            {"connLinkContact": "bad"},
            {"connLinkContact": {"connShortLink": "javascript:alert(1)"}},
        ]:
            with self.assertRaises(RuntimeError):
                runtime.address_from(value)


class PairingTests(unittest.TestCase):
    def test_request_approval_is_exact_and_stale_requests_fail(self):
        with tempfile.TemporaryDirectory() as home:
            env = {**os.environ, "HERMES_HOME": home}
            created = subprocess.run(
                [
                    sys.executable,
                    "-c",
                    "import json; from gateway.pairing import PairingStore; s=PairingStore(); s.generate_code('simplex','3','Owner'); s.generate_code('simplex','4','Another contact'); print(json.dumps(s.list_pending('simplex')))",
                ],
                env=env,
                capture_output=True,
                text=True,
                check=True,
            )
            pending = json.loads(created.stdout)
            request_id = next(
                row["request_id"] for row in pending if row["user_id"] == "3"
            )
            self.assertTrue(
                all("hash" not in row and "salt" not in row for row in pending)
            )
            helper = str(Path(__file__).with_name("simplex_runtime.py"))
            invalid = subprocess.run(
                [sys.executable, helper, "approve-request"],
                input="0" * 16,
                env=env,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(invalid.returncode, 0)
            approved = subprocess.run(
                [sys.executable, helper, "approve-request"],
                input=request_id,
                env=env,
                capture_output=True,
                text=True,
                check=True,
            )
            self.assertEqual(json.loads(approved.stdout), {"approved": True})
            replay = subprocess.run(
                [sys.executable, helper, "approve-request"],
                input=request_id,
                env=env,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(replay.returncode, 0)
            verify = subprocess.run(
                [
                    sys.executable,
                    "-c",
                    "from gateway.pairing import PairingStore; s=PairingStore(); assert s.is_approved('simplex','3'); assert not s.is_approved('simplex','4'); assert not s.is_approved('telegram','3'); assert len(s.list_pending('simplex')) == 1",
                ],
                env=env,
                capture_output=True,
                text=True,
            )
            self.assertEqual(verify.returncode, 0, verify.stderr)

    def test_released_hermes_grants_only_the_approved_contact(self):
        with tempfile.TemporaryDirectory() as home:
            env = {**os.environ, "HERMES_HOME": home}
            create = subprocess.run(
                [
                    sys.executable,
                    "-c",
                    "from gateway.pairing import PairingStore; print(PairingStore().generate_code('simplex', '3', 'Test owner'))",
                ],
                env=env,
                capture_output=True,
                text=True,
                check=True,
            )
            code = create.stdout.strip()
            helper = str(Path(__file__).with_name("simplex_runtime.py"))
            invalid = "AAAAAAAA" if code != "AAAAAAAA" else "BBBBBBBB"
            result = subprocess.run(
                [sys.executable, helper, "approve"],
                input=invalid,
                env=env,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("invalid", json.loads(result.stdout)["error"])
            result = subprocess.run(
                [sys.executable, helper, "approve"],
                input=code,
                env=env,
                capture_output=True,
                text=True,
                check=True,
            )
            self.assertEqual(json.loads(result.stdout), {"approved": True})
            verify = subprocess.run(
                [
                    sys.executable,
                    "-c",
                    "from gateway.pairing import PairingStore; s=PairingStore(); assert s.is_approved('simplex', '3'); assert not s.is_approved('simplex', '4'); assert not s.is_approved('telegram', '3')",
                ],
                env=env,
                capture_output=True,
                text=True,
            )
            self.assertEqual(verify.returncode, 0, verify.stderr)


if __name__ == "__main__":
    unittest.main()

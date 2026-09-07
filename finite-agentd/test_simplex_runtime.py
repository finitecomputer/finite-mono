import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import AsyncMock, patch

import websockets

import simplex_runtime as runtime


class ControlTests(unittest.IsolatedAsyncioTestCase):
    async def test_correlates_response_and_ignores_broadcast(self):
        async def server(ws):
            request = json.loads(await ws.recv())
            await ws.send(json.dumps({"resp": {"type": "newChatItems", "chatItems": []}}))
            await ws.send(json.dumps({"corrId": "someone-else", "resp": {"type": "cmdOk"}}))
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
                json.dumps({"corrId": request["corrId"], "resp": {"type": "chatCmdError"}})
            )

        async with websockets.serve(server, "127.0.0.1", 0) as service:
            port = service.sockets[0].getsockname()[1]
            with (
                patch.object(runtime, "WS_URL", f"ws://127.0.0.1:{port}"),
                self.assertRaises(RuntimeError),
            ):
                await runtime.command("/address")

    async def test_open_socket_without_command_response_times_out(self):
        async def server(ws):
            await ws.recv()
            await ws.wait_closed()

        async with websockets.serve(server, "127.0.0.1", 0) as service:
            port = service.sockets[0].getsockname()[1]
            with (
                patch.object(runtime, "WS_URL", f"ws://127.0.0.1:{port}"),
                self.assertRaises(TimeoutError),
            ):
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
            patch.object(runtime, "saved_address", return_value="https://smp.example/a#synthetic"),
            patch.object(
                runtime,
                "command",
                new=AsyncMock(side_effect=AssertionError("must not consume live events")),
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
        self.assertEqual(runtime.address_from({"connLinkContact": link}), link["connShortLink"])
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
            request_id = next(row["request_id"] for row in pending if row["user_id"] == "3")
            self.assertTrue(all("hash" not in row and "salt" not in row for row in pending))
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


class ResetTests(unittest.TestCase):
    def test_reset_is_isolated_and_recoverable(self):
        script = r"""
import asyncio, json, os
from pathlib import Path
from unittest.mock import AsyncMock, patch
import yaml
import simplex_runtime as r
from gateway.pairing import PairingStore
from hermes_state import SessionDB
home=Path(os.environ['HERMES_HOME']); home.mkdir(exist_ok=True)
(home/'config.yaml').write_text(yaml.safe_dump({'gateway': {'platforms': {'simplex': {'enabled': False, 'extra': {'finite_managed': True}}}}}))
(home/'.env').write_text('UNRELATED_CREDENTIAL=synthetic\n')
r.state_dir().mkdir()
for name in ['identity_chat.db','identity_agent.db','address.json']:
    (r.state_dir()/name).write_text('synthetic old identity')
s=PairingStore()
for platform in ['simplex','telegram']:
    code=s.generate_code(platform,'3','Owner'); s.approve_code(platform,code)
    s.generate_code(platform,'4','Pending')
legacy=home/'pairing'; legacy.mkdir(exist_ok=True)
(legacy/'simplex-approved.json').write_text(json.dumps({'8': {'user_name':'Legacy'}}))
sessions=home/'sessions'; sessions.mkdir(exist_ok=True)
entries={"_README": "Hermes metadata, not a session", "invalid": False}; db=SessionDB()
for platform in ['simplex','telegram']:
    sid=platform+'-synthetic'; key='agent:'+platform+':dm:3'
    db.create_session(sid, platform, session_key=key)
    entry={'session_id':sid,'platform':platform,'origin':None}
    entries[key]=entry
    db.save_gateway_routing_entry(key,json.dumps(entry),scope=str(sessions.resolve()))
    (sessions/(sid+'.jsonl')).write_text('synthetic transcript')
(sessions/'sessions.json').write_text(json.dumps(entries)); db.close()
r.prepare_reset()
with patch.object(r,'tcp_ready',new=AsyncMock(return_value=True)), patch.object(r.asyncio,'sleep',new=AsyncMock()):
    try: asyncio.run(r.finish_reset())
    except RuntimeError: pass
    else: raise AssertionError('deleted live state')
assert (r.state_dir()/'identity_chat.db').exists()
try: asyncio.run(r.create_address())
except RuntimeError: pass
else: raise AssertionError('bootstrapped during reset')
# Simulate a filesystem failure after SQLite deletion. The durable plan must
# retain transcript IDs even after the source records have disappeared.
original_unlink = Path.unlink
def fail_transcript(path, *args, **kwargs):
    if path.name == 'simplex-synthetic.jsonl':
        raise PermissionError('synthetic transient file failure')
    return original_unlink(path, *args, **kwargs)
with patch.object(r,'tcp_ready',new=AsyncMock(return_value=False)):
    with patch.object(Path, 'unlink', fail_transcript):
        try: asyncio.run(r.finish_reset())
        except PermissionError: pass
        else: raise AssertionError('reported success while transcript remained')
    assert r.reset_marker().exists()
    assert (sessions/'simplex-synthetic.jsonl').exists()
    before=r.reset_marker().read_text()
    r.prepare_reset()
    assert r.reset_marker().read_text()==before
    asyncio.run(r.finish_reset()); asyncio.run(r.finish_reset())
assert not r.state_dir().exists() and not r.reset_marker().exists()
s=PairingStore()
assert not s.list_approved('simplex') and not s.list_pending('simplex')
assert s.is_approved('telegram','3') and s.list_pending('telegram')
assert s.generate_code('simplex','3','New owner')
assert (home/'.env').read_text()=='UNRELATED_CREDENTIAL=synthetic\n'
db=SessionDB()
assert db.get_session('simplex-synthetic') is None
assert db.get_session('telegram-synthetic') is not None
assert list(db.load_gateway_routing_entries(scope=str(sessions.resolve())))==['agent:telegram:dm:3']
assert list(json.loads((sessions/'sessions.json').read_text()))==['_README','invalid','agent:telegram:dm:3']
assert not (sessions/'simplex-synthetic.jsonl').exists()
assert (sessions/'telegram-synthetic.jsonl').exists()
db.close()
"""
        with tempfile.TemporaryDirectory() as root:
            result = subprocess.run(
                [sys.executable, "-c", script],
                cwd=Path(__file__).parent,
                env={
                    **os.environ,
                    "HERMES_HOME": root + "/hermes",
                    "FINITECHAT_HOME": root,
                },
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()

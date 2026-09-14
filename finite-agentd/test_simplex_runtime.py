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


class RelayTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        import socket

        sockets = [socket.socket(), socket.socket()]
        for sock in sockets:
            sock.bind(("127.0.0.1", 0))
        ports = [sock.getsockname()[1] for sock in sockets]
        for sock in sockets:
            sock.close()
        for name, value in [
            ("CHAT_PORT", ports[0]),
            ("SETUP_PORT", ports[1]),
            ("WS_URL", f"ws://127.0.0.1:{ports[0]}"),
        ]:
            patcher = patch.object(runtime, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)

    def defaults(self):
        return [
            {
                "operator": {"operatorTag": "simplex", "enabled": True},
                "smpServers": [{"server": "smp://synthetic", "enabled": False}],
                "xftpServers": [{"serverId": 1, "server": "xftp://synthetic", "enabled": True}],
                "chatRelays": [],
            },
            {"smpServers": [], "xftpServers": [], "chatRelays": []},
        ]

    def test_preserves_existing_fields_and_adds_download_only_relays_once(self):
        before = self.defaults()
        original = json.dumps(before)
        after = runtime.relay_settings(before)
        self.assertEqual(json.dumps(before), original)
        self.assertEqual(after[0], before[0])
        self.assertEqual(len(after[1]["xftpServers"]), 6)
        self.assertTrue(all(not s["enabled"] for s in after[1]["xftpServers"]))
        self.assertEqual(runtime.relay_settings(after), after)

    def test_custom_policy_and_disabled_operator_are_unchanged(self):
        for field in ["smpServers", "xftpServers", "chatRelays"]:
            before = self.defaults()
            before[1][field] = [{"server": "custom://owner-choice"}]
            self.assertEqual(runtime.relay_settings(before), before)
        for enabled in [True, False]:
            before = self.defaults()
            before[1]["xftpServers"] = [{"server": runtime.FLUX_SERVERS[0], "enabled": enabled}]
            self.assertEqual(runtime.relay_settings(before), before)
        before = self.defaults()
        before[0]["operator"]["enabled"] = False
        self.assertEqual(runtime.relay_settings(before), before)

    async def test_failed_validation_does_not_write_settings_or_backup(self):
        with tempfile.TemporaryDirectory() as home:
            cmd = AsyncMock(
                side_effect=[
                    {"user": {"userId": 7}},
                    {"userServers": self.defaults()},
                    {"type": "userServersValidation", "serverErrors": ["synthetic"]},
                ]
            )
            with patch.object(runtime, "command", cmd), self.assertRaises(RuntimeError):
                await runtime.configure_relays(Path(home))
            self.assertEqual(len(cmd.call_args_list), 3)
            self.assertEqual(list(Path(home).iterdir()), [])

    async def test_setup_refuses_a_live_daemon(self):
        with (
            patch.object(runtime, "tcp_ready", AsyncMock(return_value=True)),
            patch.object(runtime, "start_child", AsyncMock()) as start,
        ):
            with self.assertRaises(RuntimeError):
                await runtime.prepare_relays(Path("/unused"))
            start.assert_not_called()

    async def test_unexpected_readback_restores_prior_settings(self):
        before = self.defaults()
        changed = runtime.relay_settings(before)
        for index, entry in enumerate(changed[1]["xftpServers"], 2):
            entry["serverId"] = index
        changed[0]["smpServers"][0]["enabled"] = True
        responses = [
            {"user": {"userId": 1}},
            {"userServers": before},
            {"type": "userServersValidation", "serverErrors": [], "serverWarnings": []},
            {"type": "cmdOk"},
            {"userServers": changed},
            {"type": "cmdOk"},
            {"userServers": before},
        ]
        with tempfile.TemporaryDirectory() as folder:
            home = Path(folder)
            with (
                patch.object(runtime, "command", AsyncMock(side_effect=responses)) as cmd,
                self.assertRaisesRegex(RuntimeError, "restored original"),
            ):
                await runtime.configure_relays(home)
            rollback = json.loads((home / "flux-relays-rollback.json").read_text())
            self.assertEqual(rollback[0], before[0])
            self.assertTrue(all(entry["deleted"] for entry in rollback[1]["xftpServers"]))
            self.assertIn(json.dumps(rollback), cmd.call_args_list[-2].args[0])
            self.assertFalse((home / "flux-relays-ready.json").exists())

    async def test_restart_recovers_receipt_after_cancellation_at_commit(self):
        before = self.defaults()
        applied = runtime.relay_settings(before)
        for index, entry in enumerate(applied[1]["xftpServers"], 2):
            entry["serverId"] = index
        with tempfile.TemporaryDirectory() as folder:
            home = Path(folder)
            interrupted = AsyncMock(
                side_effect=[
                    {"user": {"userId": 1}},
                    {"userServers": before},
                    {"type": "userServersValidation", "serverErrors": [], "serverWarnings": []},
                    runtime.asyncio.CancelledError,
                ]
            )
            with (
                patch.object(runtime, "command", interrupted),
                self.assertRaises(runtime.asyncio.CancelledError),
            ):
                await runtime.configure_relays(home)
            self.assertTrue((home / "flux-relays-before.json").exists())
            self.assertFalse((home / "flux-relays-ready.json").exists())
            # The released daemon committed before the connection disappeared.
            with patch.object(
                runtime,
                "command",
                AsyncMock(
                    side_effect=[
                        {"user": {"userId": 1}},
                        {"userServers": applied},
                    ]
                ),
            ) as cmd:
                await runtime.configure_relays(home)
                self.assertEqual(len(cmd.call_args_list), 2)  # recovery is read-only
            self.assertTrue((home / "flux-relays-ready.json").exists())
            rollback = json.loads((home / "flux-relays-rollback.json").read_text())
            self.assertEqual(rollback[0], before[0])
            self.assertTrue(all(s["deleted"] for s in rollback[1]["xftpServers"]))

    async def test_interrupted_setup_with_later_owner_edit_fails_closed(self):
        before = self.defaults()
        changed = runtime.relay_settings(before)
        for index, entry in enumerate(changed[1]["xftpServers"], 2):
            entry["serverId"] = index
        changed[1]["xftpServers"][0]["enabled"] = True
        with tempfile.TemporaryDirectory() as folder:
            home = Path(folder)
            (home / "flux-relays-before.json").write_text(json.dumps(before))
            with (
                patch.object(
                    runtime,
                    "command",
                    AsyncMock(
                        side_effect=[
                            {"user": {"userId": 1}},
                            {"userServers": changed},
                        ]
                    ),
                ) as cmd,
                self.assertRaisesRegex(RuntimeError, "operator review"),
            ):
                await runtime.configure_relays(home)
            self.assertEqual(len(cmd.call_args_list), 2)
            self.assertFalse((home / "flux-relays-ready.json").exists())

    async def test_slow_preparation_is_bounded_and_closes_maintenance_child(self):
        from types import SimpleNamespace

        with tempfile.TemporaryDirectory() as folder:
            home = Path(folder)
            for suffix in ["_chat.db", "_agent.db"]:
                (home / ("identity" + suffix)).touch()
            with (
                patch.object(runtime, "tcp_ready", AsyncMock(return_value=False)),
                patch.object(
                    runtime, "start_child", AsyncMock(return_value=SimpleNamespace(returncode=None))
                ),
                patch.object(runtime, "stop_child", AsyncMock()) as stop,
                patch.object(runtime, "SETUP_TIMEOUT", 0.01),
            ):
                with self.assertRaises(TimeoutError):
                    await runtime.prepare_relays(home)
                stop.assert_awaited_once()
                self.assertEqual(stop.call_args.kwargs, {"timeout": 2})

    async def test_supervisor_starts_normal_chat_after_failed_preparation(self):
        from types import SimpleNamespace

        with tempfile.TemporaryDirectory() as folder:
            home = Path(folder)
            stopped = runtime.asyncio.Event()
            child = SimpleNamespace(pid=12345, returncode=None)

            async def start(*args, **kwargs):
                stopped.set()
                return child

            with (
                patch.object(runtime, "state_dir", return_value=home),
                patch.object(runtime.asyncio, "Event", return_value=stopped),
                patch.object(runtime, "managed_enabled", return_value=True),
                patch.object(runtime, "saved_address", return_value="https://synthetic"),
                patch.object(runtime, "tcp_ready", AsyncMock(return_value=False)),
                patch.object(runtime, "prepare_relays", AsyncMock(side_effect=TimeoutError)),
                patch.object(runtime, "start_child", AsyncMock(side_effect=start)) as starts,
                patch.object(runtime, "stop_child", AsyncMock()),
            ):
                await runtime.supervise()
                starts.assert_awaited_once_with(home)
            self.assertFalse((home / "daemon.pid").exists())

    async def test_packaged_daemon_persistence_and_released_settings_rollback(self):
        import shutil

        if not shutil.which("simplex-chat"):
            self.skipTest("packaged simplex-chat is not on PATH")
        with tempfile.TemporaryDirectory() as folder:
            home = Path(folder)
            # Create untouched state with the released daemon first.
            child = await runtime.start_child(home)
            try:
                for _ in range(40):
                    await runtime.asyncio.sleep(0.25)
                    if await runtime.tcp_ready():
                        break
                original_user = (await runtime.command("/u"))["user"]["userId"]
            finally:
                await runtime.stop_child(child)
            await runtime.prepare_relays(home)
            before = json.loads((home / "flux-relays-before.json").read_text())
            after = json.loads((home / "flux-relays-after.json").read_text())
            self.assertEqual(before[0], after[0])
            self.assertEqual(len(after[1]["xftpServers"]), 6)
            self.assertTrue(
                all(
                    not s["enabled"] and not s["preset"] and not s["deleted"]
                    for s in after[1]["xftpServers"]
                )
            )
            with patch.object(
                runtime,
                "start_child",
                AsyncMock(side_effect=AssertionError("must skip completed setup")),
            ):
                await runtime.prepare_relays(home)
            self.assertEqual(json.loads((home / "flux-relays-before.json").read_text()), before)
            child = await runtime.start_child(home, maintenance=True)
            try:
                for _ in range(40):
                    await runtime.asyncio.sleep(0.25)
                    if await runtime.tcp_ready(runtime.SETUP_PORT):
                        break
                url = f"ws://127.0.0.1:{runtime.SETUP_PORT}"
                await runtime.command("/_start main=off snd_files=off", url=url)
                self.assertEqual(
                    (await runtime.command("/u", url=url))["user"]["userId"], original_user
                )
                self.assertEqual(
                    (await runtime.command("/_servers 1", url=url))["userServers"], after
                )
                rollback = json.loads((home / "flux-relays-rollback.json").read_text())
                await runtime.command("/_servers 1 " + json.dumps(rollback), url=url)
                self.assertEqual(
                    (await runtime.command("/_servers 1", url=url))["userServers"], before
                )
            finally:
                await runtime.stop_child(child)


if __name__ == "__main__":
    unittest.main()

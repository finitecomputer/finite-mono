"""Owner topic policy against the real pinned Hermes adapter and PairingStore."""

import asyncio
import copy
import hashlib
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import AsyncMock, patch

import hermes_cli
from gateway.config import PlatformConfig
from gateway.pairing import PairingStore
from gateway.platforms.base import MessageEvent

PLUGIN = Path(__file__).resolve().parents[2] / "integrations/hermes/finitechat"


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec is not None and spec.loader is not None
    value = importlib.util.module_from_spec(spec)
    sys.modules[name] = value
    spec.loader.exec_module(value)
    return value


topics = module("finite_topic_contract", PLUGIN / "simplex_topics.py")
upstream = module(
    "simplex_upstream_contract",
    Path(hermes_cli.__file__).parents[1] / "plugins/platforms/simplex/adapter.py",
)


class Adapter(topics.OwnerTopics, upstream.SimplexAdapter):
    pass


class TopicTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.env = patch.dict(
            os.environ,
            {"HERMES_HOME": self.temp.name + "/hermes", "FINITECHAT_HOME": self.temp.name},
        )
        self.env.start()
        self.addCleanup(self.env.stop)
        self.pairing = PairingStore()
        code = self.pairing.generate_code("simplex", "3", "Owner")
        assert code is not None
        self.pairing.approve_code("simplex", code)
        self.adapter = Adapter(PlatformConfig(enabled=True, extra={"finite_managed": True}))
        self.row = {"owner": "3", "title": "Gardening", "marker": "unique", "group_id": "7"}
        self.adapter.topics["topic"] = self.row
        self.group = {
            "groupInfo": {"membership": {"memberRole": "owner"}},
            "members": [
                {
                    "memberId": "opaque-member",
                    "memberContactId": 3,
                    "memberRole": "member",
                    "memberStatus": "complete",
                }
            ],
        }
        self.adapter.members = AsyncMock(side_effect=lambda _: copy.deepcopy(self.group))

    def save(self):
        topics.write_state(self.adapter.topic_path, self.adapter.topics)

    async def test_maps_exact_group_member_to_approved_contact_without_pairing_member_id(self):
        received = asyncio.Queue()

        async def handle(event):
            await received.put(event)

        self.adapter.set_message_handler(handle)
        source = self.adapter.build_source(
            chat_id="group:7", chat_type="group", user_id="opaque-member"
        )
        event = MessageEvent(
            source=source, text="Tomatoes", raw_message={"memberId": "opaque-member"}
        )
        await self.adapter.handle_message(event)
        got = await asyncio.wait_for(received.get(), 2)
        self.assertEqual(got.source.user_id, "3")
        self.assertEqual(got.source.chat_id, "group:7")
        self.assertEqual(got.raw_message["memberId"], "opaque-member")
        self.assertFalse(self.pairing.is_approved("simplex", "opaque-member"))
        await asyncio.sleep(0)

    async def test_real_gateway_authorization_and_distinct_sessions_keep_home(self):
        from gateway.config import GatewayConfig, HomeChannel
        from gateway.run import GatewayRunner
        from gateway.session import SessionStore

        source = self.adapter.build_source(
            chat_id="group:7", chat_type="group", user_id="opaque-member"
        )
        source.user_id = await self.adapter.check_group("7", sender=source.user_id)
        runner = object.__new__(GatewayRunner)
        runner.config = GatewayConfig()
        runner.adapters = {source.platform: self.adapter}
        runner.pairing_store = self.pairing
        self.assertTrue(runner._is_user_authorized_for_source(source))
        source.user_id = "unpaired-member"
        self.assertFalse(runner._is_user_authorized_for_source(source))
        source.user_id = "3"
        dm = self.adapter.build_source(chat_id="3", chat_type="dm", user_id="3")
        second = self.adapter.build_source(chat_id="group:8", chat_type="group", user_id="3")
        home = HomeChannel(platform=source.platform, chat_id="3", name="Owner")
        runner.config.platforms[source.platform] = PlatformConfig(home_channel=home)
        store = SessionStore(Path(self.temp.name) / "sessions", runner.config)
        entries = [store.get_or_create_session(s) for s in (dm, source, second)]
        self.assertEqual(len({e.session_id for e in entries}), 3)
        store.append_to_transcript(
            entries[1].session_id, {"role": "user", "content": "garden-only"}
        )
        self.assertFalse(store.load_transcript(entries[0].session_id))
        self.assertFalse(store.load_transcript(entries[2].session_id))
        retained_home = runner.config.platforms[source.platform].home_channel
        assert retained_home is not None
        self.assertEqual(retained_home.chat_id, "3")

    async def test_sender_collision_unknown_group_and_revocation_fail_closed(self):
        with self.assertRaises(ValueError):
            await self.adapter.check_group("7", sender="3")
        with self.assertRaises(ValueError):
            await self.adapter.check_group("8", sender="opaque-member")
        self.pairing.revoke("simplex", "3")
        with self.assertRaises(ValueError):
            await self.adapter.check_group("7", sender="opaque-member")

    async def test_new_member_blocks_both_text_and_media_and_stays_blocked_after_restart(self):
        self.save()
        self.group["members"].append({"memberContactId": 4})
        self.adapter._ws = AsyncMock()
        result = await self.adapter.send("group:7", "private answer")
        self.assertFalse(result.success)
        self.assertIn("membership changed", result.error)
        attachment = Path(self.temp.name) / "private.txt"
        attachment.write_text("private document")
        result = await self.adapter.send_document("group:7", str(attachment))
        self.assertFalse(result.success)
        self.adapter._ws.send.assert_not_called()
        restored = Adapter(self.adapter.config)
        with self.assertRaises(ValueError):
            restored.topic("7")

    async def test_owner_promotion_or_leave_disables_private_topic(self):
        for changed in ({"memberRole": "admin"}, {"memberStatus": "left"}):
            self.row.pop("blocked", None)
            self.group["members"][0].update(changed)
            with self.assertRaises(ValueError):
                await self.adapter.check_group("7")

    async def test_corrupt_registry_preserves_dm_and_never_overwrites_retained_state(self):
        self.save()
        self.adapter.topic_path.write_text("broken JSON")
        restored = Adapter(self.adapter.config)
        self.assertFalse(restored.group_allow_from)
        with self.assertRaises(ValueError):
            await restored.create_topic("garden", "3")
        self.assertEqual(restored.topic_path.read_text(), "broken JSON")
        received = asyncio.Event()

        async def handler(_event):
            received.set()

        restored.set_message_handler(handler)
        await restored.handle_message(
            MessageEvent(
                source=restored.build_source(chat_id="3", chat_type="dm", user_id="3"),
                text="DM still works",
            )
        )
        await asyncio.wait_for(received.wait(), 2)
        await asyncio.sleep(0)

    async def test_multiple_approved_contacts_are_not_an_owner_selection_rule(self):
        code = self.pairing.generate_code("simplex", "4", "Also approved")
        assert code is not None
        self.pairing.approve_code("simplex", code)
        with self.assertRaises(ValueError):
            await self.adapter.create_topic("garden", "3")

    async def test_unknown_membership_shape_or_transport_error_drops_message(self):
        handler = AsyncMock()
        self.adapter.set_message_handler(handler)
        self.adapter.members.side_effect = RuntimeError("timeout")
        event = MessageEvent(
            source=self.adapter.build_source(
                chat_id="group:7", chat_type="group", user_id="opaque-member"
            ),
            text="hello",
        )
        await self.adapter.admit(event)
        self.adapter.members.side_effect = None
        self.adapter.members.return_value = {}
        await self.adapter.admit(event)
        handler.assert_not_called()

    async def test_creation_timeout_reconciles_unique_marker_without_duplicate(self):
        self.adapter.topics = {}
        calls = []

        async def command(text):
            calls.append(text)
            if text == "/u":
                return {"user": {"userId": 1}}
            if text.startswith("/_group "):
                raise RuntimeError("response lost after creation")
            row = next(iter(self.adapter.topics.values()))
            if text == "/groups":
                return {"groups": [{"groupId": 7, "groupProfile": {"description": row["marker"]}}]}
            if text.startswith("/_group_profile "):
                return {"type": "groupUpdated"}
            self.fail(text)

        self.adapter.command = command
        with self.assertRaises(RuntimeError):
            await self.adapter.create_topic("Garden", "3")
        result = await self.adapter.create_topic("garden", "3")
        self.assertEqual(result["group_id"], "7")
        self.assertEqual(sum(c.startswith("/_group ") for c in calls), 1)
        self.assertEqual(len(self.adapter.topics), 1)
        self.assertEqual((await self.adapter.create_topic("Garden", "3"))["group_id"], "7")
        self.assertEqual(Adapter(self.adapter.config).group_allow_from, {"7"})
        self.assertEqual(self.adapter.topic_path.stat().st_mode & 0o777, 0o600)

    async def test_explicit_blocked_replacement_preserves_history_and_reconciles_retry(self):
        from gateway.config import GatewayConfig
        from gateway.session import SessionStore

        key = hashlib.sha256(b"3:gardening").hexdigest()
        blocked = {**self.row, "blocked": True}
        other = {"owner": "3", "title": "Cooking", "group_id": "9"}
        self.adapter.topics = {key: blocked, "other": other}
        self.save()
        before = self.adapter.topic_path.read_bytes()
        store = SessionStore(Path(self.temp.name) / "sessions", GatewayConfig())
        old_session = store.get_or_create_session(
            self.adapter.build_source(chat_id="group:7", chat_type="group", user_id="3")
        )
        store.append_to_transcript(
            old_session.session_id, {"role": "user", "content": "old garden"}
        )
        history = store.load_transcript(old_session.session_id)
        self.adapter.command = AsyncMock()
        for approval in (False, "true"):
            with self.assertRaisesRegex(ValueError, "fresh replacement"):
                await self.adapter.create_topic("Gardening", "3", replace_blocked=approval)
        self.adapter.command.assert_not_called()
        self.assertEqual(self.adapter.topic_path.read_bytes(), before)
        calls = []

        async def command(text):
            calls.append(text)
            if text == "/u":
                return {"user": {"userId": 1}}
            if text.startswith("/_group "):
                raise RuntimeError("lost response after replacement created")
            if text == "/groups":
                row = self.adapter.topics[key]
                return {"groups": [{"groupId": 8, "groupProfile": {"description": row["marker"]}}]}
            if text.startswith("/_group_profile "):
                return {"type": "groupUpdated"}
            self.fail(text)

        self.adapter.command = command
        with self.assertRaises(RuntimeError):
            await self.adapter.create_topic("Gardening", "3", replace_blocked=True)
        # Retain the interrupted replacement across reconstruction; retrying the
        # same explicit request must reconcile, not create yet another group.
        restored = Adapter(self.adapter.config)
        restored.command = command
        restored.members = self.adapter.members
        result = await restored.create_topic("Gardening", "3", replace_blocked=True)
        self.assertEqual((result["group_id"], result["replaced_group_id"]), ("8", "7"))
        self.assertEqual(
            (await restored.create_topic("Gardening", "3", replace_blocked=True))["group_id"], "8"
        )
        self.assertEqual(sum(c.startswith("/_group ") for c in calls), 1)
        self.assertEqual(restored.topics["retired:7"], blocked)
        self.assertEqual(restored.topics["other"], other)
        with self.assertRaises(ValueError):
            restored.topic("7")
        self.assertEqual(store.load_transcript(old_session.session_id), history)
        new_session = store.get_or_create_session(
            restored.build_source(chat_id="group:8", chat_type="group", user_id="3")
        )
        self.assertNotEqual(new_session.session_id, old_session.session_id)
        self.assertFalse(store.load_transcript(new_session.session_id))
        self.assertEqual(Adapter(self.adapter.config).topic("8")["owner"], "3")

    async def test_first_message_before_invitation_response_reaches_owner_session(self):
        self.adapter.topics = {}
        received = asyncio.Queue()
        self.adapter._text_batch_delay = 0

        async def handler(event):
            await received.put(event)

        self.adapter.set_message_handler(handler)
        self.adapter.members = AsyncMock(
            side_effect=[{**self.group, "members": []}, self.group, self.group]
        )

        async def command(text):
            if text == "/u":
                return {"user": {"userId": 1}}
            if text.startswith("/_group "):
                return {"type": "groupCreated", "groupInfo": {"groupId": 7}}
            if text.startswith("/_group_profile "):
                return {"type": "groupUpdated"}
            if text == "/_add #7 3 member":
                # Dispatch through the real upstream allowlist and text batcher
                # while create_topic is still waiting for the invite response.
                await self.adapter._handle_chat_item(
                    {
                        "chatInfo": {"type": "group", "groupInfo": {"groupId": 7}},
                        "chatItem": {
                            "chatDir": {
                                "type": "groupRcv",
                                "groupMember": self.group["members"][0],
                            },
                            "content": {
                                "type": "rcvMsgContent",
                                "msgContent": {"type": "text", "text": "first hello"},
                            },
                        },
                    }
                )
                first = await asyncio.wait_for(received.get(), 2)
                self.assertEqual(
                    (first.source.chat_id, first.source.user_id, first.text),
                    ("group:7", "3", "first hello"),
                )
                return {"type": "sentGroupInvitation"}
            self.fail(text)

        self.adapter.command = command
        await self.adapter.create_topic("Garden", "3")
        await asyncio.sleep(0)

    async def test_pinned_gateway_process_lock_excludes_second_writer_and_releases_on_exit(self):
        # Use the real upstream lock in separate processes, under our temporary
        # HERMES_HOME. The canonical gateway acquires it before starting adapters.
        holder = await asyncio.create_subprocess_exec(
            sys.executable,
            "-c",
            "from gateway.status import acquire_gateway_runtime_lock; "
            "import sys; print(acquire_gateway_runtime_lock(), flush=True); sys.stdin.read()",
            stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE,
        )
        assert holder.stdout is not None
        try:
            self.assertEqual(await asyncio.wait_for(holder.stdout.readline(), 10), b"True\n")
            probe = "from gateway.status import acquire_gateway_runtime_lock; print(acquire_gateway_runtime_lock())"
            result = await asyncio.to_thread(
                subprocess.run,
                [sys.executable, "-c", probe],
                capture_output=True,
                text=True,
                check=True,
                timeout=10,
            )
            self.assertEqual(result.stdout.strip(), "False")
        finally:
            holder.kill()
            await holder.wait()
        result = await asyncio.to_thread(
            subprocess.run,
            [sys.executable, "-c", probe],
            capture_output=True,
            text=True,
            check=True,
            timeout=10,
        )
        self.assertEqual(result.stdout.strip(), "True")

    async def test_unresolved_creation_never_blindly_repeats(self):
        self.adapter.topics = {}

        async def command(text):
            if text == "/u":
                return {"user": {"userId": 1}}
            if text == "/groups":
                return {"groups": []}
            raise RuntimeError("uncertain create")

        self.adapter.command = AsyncMock(side_effect=command)
        with self.assertRaises(RuntimeError):
            await self.adapter.create_topic("Garden", "3")
        with self.assertRaises(ValueError):
            await self.adapter.create_topic("Garden", "3")
        self.assertEqual(
            sum(c.args[0].startswith("/_group ") for c in self.adapter.command.call_args_list), 1
        )


class DiscoveryTests(unittest.TestCase):
    def test_unconfigured_simplex_stays_lazy_when_finite_chat_loads(self):
        script = r"""
import json, os, shutil, tempfile
from pathlib import Path
with tempfile.TemporaryDirectory() as home:
    os.environ["HERMES_HOME"] = home
    os.environ["FINITE_HOME"] = home + "/agent"
    os.environ["FINITECHAT_HOME"] = home + "/agent"
    config = {"plugins": {"enabled": ["finitechat"]}}
    simplex = json.loads(os.environ["TEST_SIMPLEX_CONFIG"])
    if simplex is not None:
        config["gateway"] = {"platforms": {"simplex": simplex}}
    Path(home, "config.yaml").write_text(json.dumps(config))
    shutil.copytree(os.environ["TEST_PLUGIN"], Path(home, "plugins/finitechat"))
    from hermes_cli.plugins import get_plugin_manager
    from gateway.platform_registry import platform_registry
    manager = get_plugin_manager()
    manager.discover_and_load()
    assert platform_registry.get("finitechat") is not None
    entry, deferred = platform_registry.snapshot_registration("simplex", scope=manager.scope_key)
    assert entry is None and deferred is not None, "Finite Chat eagerly loaded unused SimpleX"
"""
        env = dict(os.environ, TEST_PLUGIN=str(PLUGIN))
        env.setdefault(
            "HERMES_BUNDLED_PLUGINS", str(Path(hermes_cli.__file__).parents[1] / "plugins")
        )
        for simplex in (
            None,
            {"enabled": False, "extra": {"finite_managed": True}},
            {"enabled": True, "extra": {"finite_managed": False}},
        ):
            with self.subTest(simplex=simplex):
                env["TEST_SIMPLEX_CONFIG"] = json.dumps(simplex)
                result = subprocess.run(
                    [sys.executable, "-c", script],
                    env=env,
                    capture_output=True,
                    text=True,
                    timeout=30,
                )
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_rediscovered_tool_uses_connected_gateway_adapter(self):
        script = r"""
import asyncio, json, os, shutil, tempfile
from pathlib import Path
from unittest.mock import AsyncMock, patch
with tempfile.TemporaryDirectory() as home:
    os.environ["HERMES_HOME"] = home
    os.environ["FINITECHAT_HOME"] = home + "/agent"
    Path(home, "config.yaml").write_text(
        "plugins:\n  enabled: [finitechat]\n"
        "gateway:\n  platforms:\n    simplex:\n      enabled: true\n"
        "      extra:\n        finite_managed: true\n"
    )
    shutil.copytree(os.environ["TEST_PLUGIN"], Path(home, "plugins/finitechat"))
    from hermes_cli.plugins import get_plugin_manager
    from gateway.config import GatewayConfig, Platform, PlatformConfig
    from gateway.pairing import PairingStore
    from gateway.platform_registry import platform_registry
    from gateway.run import GatewayRunner
    from tools.registry import registry
    manager = get_plugin_manager()
    manager.discover_and_load()
    runner = GatewayRunner(config=GatewayConfig())
    pairing = PairingStore()
    code = pairing.generate_code("simplex", "3", "Owner")
    assert code is not None
    pairing.approve_code("simplex", code)
    async def check():
        entry = platform_registry.get("simplex")
        adapter = entry.adapter_factory(PlatformConfig(enabled=True, extra={"finite_managed": True}))
        # Keep real plugin discovery, gateway ownership, connect lifecycle and
        # tool dispatch. Only the external daemon and group operation are fake.
        upstream = type(adapter).__mro__[2]
        with patch.object(upstream, "connect", new=AsyncMock(return_value=True)):
            await adapter.connect()
        runner.adapters[Platform("simplex")] = adapter
        adapter.create_topic = AsyncMock(return_value={"status": "invited", "group_id": "7"})
        manager.discover_and_load(force=True)
        tool = registry.get_entry("simplex_create_topic")
        # A newly constructed, unconnected adapter must not replace the live
        # gateway connection either (e.g. a replacement being prepared).
        platform_registry.get("simplex").adapter_factory(
            PlatformConfig(enabled=True, extra={"finite_managed": True})
        )
        context = {"HERMES_SESSION_PLATFORM": "simplex", "HERMES_SESSION_CHAT_TYPE": "dm",
                   "HERMES_SESSION_USER_ID": "3", "HERMES_SESSION_CHAT_ID": "3"}
        async def invoke():
            return json.loads(await asyncio.to_thread(tool.handler, {"topic": "Release Canary"}))
        with patch.dict(os.environ, context):
            assert (await invoke())["status"] == "invited"
            adapter.create_topic.assert_awaited_once_with("Release Canary", "3", replace_blocked=False)
            # Never borrow the default gateway's connection for another home.
            with patch.object(adapter, "topic_home", Path(home) / "another-profile"):
                assert "error" in await invoke()
            with patch.dict(os.environ, {"HERMES_SESSION_USER_ID": "4"}):
                assert "error" in await invoke()
            with patch.object(upstream, "disconnect", new=AsyncMock()):
                await adapter.disconnect()
            assert "error" in await invoke()
            runner.adapters.clear()
            assert "error" in await invoke()
            adapter.create_topic.assert_awaited_once()
    asyncio.run(check())
"""
        env = dict(os.environ, TEST_PLUGIN=str(PLUGIN))
        env.setdefault(
            "HERMES_BUNDLED_PLUGINS", str(Path(hermes_cli.__file__).parents[1] / "plugins")
        )
        result = subprocess.run(
            [sys.executable, "-c", script], env=env, capture_output=True, text=True, timeout=30
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_real_plugin_discovery_tool_visibility_and_unmanaged_dm_compatibility(self):
        script = r"""
import os, shutil, tempfile
from pathlib import Path
with tempfile.TemporaryDirectory() as home:
    os.environ["HERMES_HOME"] = home
    os.environ["FINITECHAT_HOME"] = home + "/agent"
    Path(home, "config.yaml").write_text(
        "plugins:\n  enabled: [finitechat]\n"
        "gateway:\n  platforms:\n    simplex:\n      enabled: true\n"
        "      extra:\n        finite_managed: true\n"
    )
    shutil.copytree(os.environ["TEST_PLUGIN"], Path(home, "plugins/finitechat"))
    from hermes_cli.plugins import get_plugin_manager
    from gateway.config import PlatformConfig
    from gateway.platform_registry import platform_registry
    from tools.registry import registry
    from toolsets import resolve_toolset
    get_plugin_manager().discover_and_load()
    entry = platform_registry.get("simplex")
    assert entry.plugin_name == "finitechat"
    assert "simplex_create_topic" in resolve_toolset("hermes-simplex")
    from model_tools import get_tool_definitions
    from hermes_cli.tools_config import _get_platform_tools
    enabled = sorted(_get_platform_tools({}, "simplex"))
    definitions = get_tool_definitions(enabled_toolsets=enabled, quiet_mode=True, skip_tool_search_assembly=True)
    assert any(t["function"]["name"] == "simplex_create_topic" for t in definitions)
    tool = registry.get_entry("simplex_create_topic")
    assert "error" in tool.handler({"topic": "garden"})
    old = entry.adapter_factory(PlatformConfig(enabled=True))
    new = entry.adapter_factory(PlatformConfig(enabled=True, extra={"finite_managed": True}))
    assert not hasattr(old, "topics") and new.topics == {}
    assert not new.group_allow_from
"""
        env = dict(os.environ, TEST_PLUGIN=str(PLUGIN))
        env.setdefault(
            "HERMES_BUNDLED_PLUGINS", str(Path(hermes_cli.__file__).parents[1] / "plugins")
        )
        result = subprocess.run(
            [sys.executable, "-c", script], env=env, capture_output=True, text=True, timeout=30
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()

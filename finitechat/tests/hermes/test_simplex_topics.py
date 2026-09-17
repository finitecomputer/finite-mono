"""Owner topic policy against the real pinned Hermes adapter and PairingStore."""

import asyncio
import copy
import importlib.util
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
        with self.assertRaises(ValueError):
            await self.adapter.send("group:7", "private answer")
        with self.assertRaises(ValueError):
            await self.adapter._send_command('/_send #7 json [{"filePath":"private.png"}]')
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
    def test_real_plugin_discovery_tool_visibility_and_unmanaged_dm_compatibility(self):
        script = r"""
import os, shutil, tempfile
from pathlib import Path
with tempfile.TemporaryDirectory() as home:
    os.environ["HERMES_HOME"] = home
    os.environ["FINITECHAT_HOME"] = home + "/agent"
    Path(home, "config.yaml").write_text("plugins:\n  enabled: [finitechat]\n")
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

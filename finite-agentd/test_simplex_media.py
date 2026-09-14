"""Replay the packaged adapter's receive methods without opening a chat connection."""

import ast
import copy
import logging
import os
import unittest
from datetime import datetime, timezone
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch


class MediaTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        import tempfile

        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.voice = self.root / "voice.m4a"
        self.voice.write_bytes(b"synthetic audio")
        bundle = os.environ.get("HERMES_BUNDLED_PLUGINS")
        if not bundle:
            self.skipTest("requires the packaged Hermes bundled plugins")
        source = Path(bundle) / "platforms/simplex/adapter.py"
        tree = ast.parse(source.read_text())
        cls = next(
            n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == "SimplexAdapter"
        )
        methods = [
            n
            for n in cls.body
            if isinstance(n, ast.AsyncFunctionDef)
            and n.name in {"_handle_event", "_handle_chat_item"}
        ]
        helpers = [
            n
            for n in tree.body
            if isinstance(n, ast.FunctionDef) and n.name in {"_is_audio_ext", "_is_image_ext"}
        ]
        namespace = dict(
            Path=Path,
            os=os,
            List=list,
            datetime=datetime,
            timezone=timezone,
            logger=logging.getLogger("simplex-test"),
            _redact_id=lambda x: x,
            _CORR_PREFIX="hermes_",
            MessageEvent=lambda **kw: SimpleNamespace(**kw),
            MessageType=SimpleNamespace(
                TEXT="text", VOICE="voice", PHOTO="photo", DOCUMENT="document"
            ),
        )
        exec(
            compile(ast.Module(body=helpers + methods, type_ignores=[]), str(source), "exec"),
            namespace,
        )

        class Adapter:
            _handle_event = namespace["_handle_event"]
            _handle_chat_item = namespace["_handle_chat_item"]

            def __init__(self):
                self.commands = []
                self.delivered = []
                self._pending_file_transfers = {}
                self._pending_responses = {}
                self._pending_corr_ids = set()
                self.auto_accept = True
                self.group_allow_from = []

            async def _send_fire_and_forget(self, command):
                self.commands.append(command)

            async def handle_message(self, event):
                self.delivered.append(event)

            def build_source(self, **kw):
                return SimpleNamespace(**kw)

        self.adapter = Adapter()
        self.item = {
            "chatInfo": {
                "type": "direct",
                "contact": {"contactId": 3, "localDisplayName": "Synthetic"},
            },
            "chatItem": {
                "chatDir": {"type": "directRcv"},
                "meta": {},
                "content": {"type": "rcvMsgContent", "msgContent": {"type": "voice", "text": ""}},
                "file": {"fileId": 1, "fileName": "voice.m4a"},
            },
        }

    async def receive(self, path):
        await self.adapter._handle_event(
            {"resp": {"type": "newChatItems", "chatItems": [copy.deepcopy(self.item)]}}
        )
        self.assertEqual(self.adapter.delivered, [])
        complete = copy.deepcopy(self.item)
        complete["chatItem"]["file"]["fileSource"] = {"filePath": path}
        await self.adapter._handle_event(
            {"resp": {"type": "rcvFileComplete", "chatItem": complete}}
        )

    async def test_relative_completion_reaches_transcription_as_existing_absolute_file(self):
        with patch.dict(os.environ, {"SIMPLEX_FILES_FOLDER": str(self.root)}):
            await self.receive("voice.m4a")
        self.assertEqual(len(self.adapter.delivered), 1)
        event = self.adapter.delivered[0]
        self.assertEqual(event.message_type, "voice")
        self.assertEqual(event.media_urls, [str(self.voice.resolve())])
        self.assertTrue(Path(event.media_urls[0]).is_file())

    async def test_absolute_path_is_preserved(self):
        with patch.dict(os.environ, {"SIMPLEX_FILES_FOLDER": str(self.root)}):
            await self.receive(str(self.voice))
        self.assertEqual(self.adapter.delivered[0].media_urls, [str(self.voice)])

    async def test_unmanaged_relative_path_is_unchanged(self):
        with patch.dict(os.environ):
            os.environ.pop("SIMPLEX_FILES_FOLDER", None)
            await self.receive("voice.m4a")
        self.assertEqual(self.adapter.delivered[0].media_urls, ["voice.m4a"])

    async def test_relative_escape_does_not_reach_transcription(self):
        with patch.dict(os.environ, {"SIMPLEX_FILES_FOLDER": str(self.root)}):
            await self.receive("../outside.m4a")
        self.assertEqual(self.adapter.delivered, [])

"""Real Hermes v0.14 export to pinned v0.20 migration proof."""

from __future__ import annotations

import hashlib
import importlib.metadata
import importlib.util
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

MONO_ROOT = Path(__file__).resolve().parents[3]
MIGRATION_TOOL = MONO_ROOT / "scripts" / "legacy_hermes_migration.py"
LEGACY_SOURCE = Path(__file__).with_name("legacy_hermes_v014_source.py")
sys.path.insert(0, str(MIGRATION_TOOL.parent))
SPEC = importlib.util.spec_from_file_location("finite_legacy_hermes_migration", MIGRATION_TOOL)
assert SPEC is not None and SPEC.loader is not None
migration = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = migration
SPEC.loader.exec_module(migration)


class LegacyHermesRealImporterTest(unittest.TestCase):
    def test_real_v014_export_imports_into_v020_without_gateway_ownership(
        self,
    ) -> None:
        legacy_python = os.environ.get("LEGACY_HERMES_AGENT_PYTHON")
        if not legacy_python:
            self.skipTest("LEGACY_HERMES_AGENT_PYTHON is required for mixed-version proof")

        with tempfile.TemporaryDirectory() as raw_temp:
            root = Path(raw_temp)
            result = subprocess.run(
                [
                    legacy_python,
                    str(LEGACY_SOURCE),
                    "--root",
                    str(root),
                    "--migration-tool",
                    str(MIGRATION_TOOL),
                ],
                check=True,
                capture_output=True,
                text=True,
            )
            evidence = json.loads(result.stdout)
            self.assertEqual(evidence["hermes_version"], "0.14.0")
            self.assertEqual(evidence["sessions"]["sessions"], 1)
            self.assertEqual(evidence["sessions"]["messages"], 2)
            self.assertEqual(evidence["memory"]["facts"], 1)
            self.assertEqual(importlib.metadata.version("hermes-agent"), "0.20.0")

            bundle = root / "bundle"
            migration.create_manifest(
                bundle,
                migration.SourceMetadata(
                    host_id="box1",
                    machine_id="austin-finite",
                    owner_email="austin@finite.vip",
                    hermes_version="0.14.0",
                    image_reference="docker.io/library/fc-agent-runtime:main",
                    image_manifest_digest="sha256:" + "a" * 64,
                    container_image_id="sha256:" + "b" * 64,
                    source_inventory_sha256=evidence["source_inventory_sha256"],
                ),
            )

            target = root / "target"
            (target / "agent/identity").mkdir(parents=True)
            (target / "agent/hermes-home").mkdir(parents=True)
            (target / "workspace").mkdir()
            identity = target / "agent/identity/identity.json"
            identity.write_text('{"npub":"npub-target"}\n', encoding="utf-8")
            chat_client = target / "agent/client.sqlite3"
            chat_client.write_bytes(b"target-chat-store")
            sqlite3.connect(target / "agent/hermes-home/state.db").close()
            identity_sha256 = hashlib.sha256(identity.read_bytes()).hexdigest()
            chat_client_sha256 = hashlib.sha256(chat_client.read_bytes()).hexdigest()
            manifest_sha256 = hashlib.sha256((bundle / "manifest.json").read_bytes()).hexdigest()

            migration.install_bundle(
                bundle,
                target,
                expected_machine_id="austin-finite",
                expected_manifest_sha256=manifest_sha256,
                expected_identity_sha256=identity_sha256,
                expected_chat_client_sha256=chat_client_sha256,
            )

            from hermes_state import SessionDB
            from plugins.memory.holographic.store import MemoryStore

            database = SessionDB(db_path=target / "agent/hermes-home/state.db")
            try:
                imported = database.export_session("legacy-telegram-session")
            finally:
                database.close()
            assert imported is not None
            self.assertEqual(len(imported["messages"]), 2)
            self.assertEqual(
                imported["messages"][0]["content"][1]["path"],
                "/data/workspace/legacy-box1/uploads/photo.jpg",
            )
            self.assertEqual(
                imported["messages"][0]["content"][2]["path"],
                "/home/node/.hermes/audio_cache/voice.ogg",
            )
            for field in (
                "chat_id",
                "chat_type",
                "thread_id",
                "session_key",
                "handoff_state",
                "handoff_platform",
                "handoff_error",
                "last_activity_at",
                "last_activity_description",
                "last_activity_provenance",
            ):
                self.assertIsNone(imported.get(field), field)
            self.assertEqual(hashlib.sha256(identity.read_bytes()).hexdigest(), identity_sha256)
            self.assertFalse((target / "agent/hermes-home/skills").exists())
            self.assertTrue(
                target.joinpath(
                    "migration/legacy-hermes-v1/review-only/skills/legacy-finite/SKILL.md"
                ).is_file()
            )
            target_memory = MemoryStore(db_path=target / "agent/hermes-home/memory_store.db")
            try:
                facts = target_memory.list_facts(limit=100)
            finally:
                target_memory.close()
            self.assertEqual(len(facts), 1)
            self.assertEqual(facts[0]["content"], "Austin hosts a recurring AI meetup")


if __name__ == "__main__":
    unittest.main()

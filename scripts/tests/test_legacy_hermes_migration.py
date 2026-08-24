from __future__ import annotations

import hashlib
import importlib.util
import io
import json
import sqlite3
import sys
import tempfile
import types
import unittest
from contextlib import closing, redirect_stderr, redirect_stdout
from pathlib import Path
from typing import ClassVar
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
MODULE = ROOT / "scripts" / "legacy_hermes_migration.py"
RUNBOOK = ROOT / "infra" / "runbooks" / "legacy-hermes-box1-to-lat3.md"
sys.path.insert(0, str(MODULE.parent))
SPEC = importlib.util.spec_from_file_location("legacy_hermes_migration", MODULE)
assert SPEC is not None and SPEC.loader is not None
migration = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = migration
SPEC.loader.exec_module(migration)


class FakeSessionDB:
    calls: ClassVar[list[dict]] = []

    def __init__(self, db_path: Path):
        self.db_path = Path(db_path)

    def import_sessions(self, sessions: list[dict]) -> dict:
        self.calls.extend(sessions)
        return {
            "ok": True,
            "imported": len(sessions),
            "skipped": 0,
            "detached": 0,
            "errors": [],
        }

    def close(self) -> None:
        return None


class FakeMemoryStore:
    def __init__(self, db_path: Path):
        self.db_path = Path(db_path)

    def rebuild_all_vectors(self) -> int:
        return migration._memory_database_fact_count(self.db_path)

    def close(self) -> None:
        return None


class LegacyHermesMigrationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.hermes_version = mock.patch(
            "importlib.metadata.version", return_value="0.20.0"
        )
        self.hermes_version.start()
        self.addCleanup(self.hermes_version.stop)
        self.tempdir = tempfile.TemporaryDirectory()
        self.root = Path(self.tempdir.name)
        self.bundle = self.root / "bundle"
        self.payload = self.bundle / "payload"
        (self.payload / "hermes/memories").mkdir(parents=True)
        (self.payload / "hermes/skills/austin-skill").mkdir(parents=True)
        (self.payload / "hermes/cron").mkdir(parents=True)
        (self.payload / "hermes/scripts").mkdir(parents=True)
        (self.payload / "home/workspace").mkdir(parents=True)
        (self.payload / "home/dev/project").mkdir(parents=True)
        (self.payload / "home/uploads").mkdir(parents=True)
        (self.payload / "hermes/memories/MEMORY.md").write_text(
            "Austin memory\n", encoding="utf-8"
        )
        (self.payload / "hermes/skills/austin-skill/SKILL.md").write_text(
            "# Austin\n", encoding="utf-8"
        )
        (self.payload / "hermes/cron/jobs.json").write_text(
            json.dumps(
                {
                    "jobs": [
                        {
                            "id": "daily-austin",
                            "enabled": True,
                            "prompt": "Prepare the daily report",
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        (self.payload / "hermes/scripts/report.py").write_text(
            "print('report')\n", encoding="utf-8"
        )
        (self.payload / "home/workspace/notes.md").write_text(
            "workspace\n", encoding="utf-8"
        )
        (self.payload / "home/dev/project/README.md").write_text(
            "project\n", encoding="utf-8"
        )
        (self.payload / "home/dev/project/readme-link").symlink_to("README.md")
        (self.payload / "home/uploads/photo.txt").write_text(
            "upload\n", encoding="utf-8"
        )
        sessions = [
            {
                "id": "child",
                "parent_session_id": "parent",
                "cwd": "/home/node/dev/project",
                "git_repo_root": "/home/node/dev/project",
                "messages": [
                    {
                        "role": "user",
                        "content": [
                            {"type": "text", "text": "child"},
                            {
                                "type": "file",
                                "path": "/home/node/uploads/photo.txt",
                            },
                            {
                                "type": "audio",
                                "path": "/home/node/.hermes/audio_cache/voice.ogg",
                            },
                            {
                                "type": "file",
                                "path": "/home/node/.brain/private-note.md",
                            },
                        ],
                    },
                    {
                        "role": "assistant",
                        "content": (
                            "MEDIA: /home/node/uploads/photo.txt "
                            "MEDIA: /home/node/.hermes/image_cache/result.png"
                        ),
                    },
                ],
            },
            {
                "id": "parent",
                "parent_session_id": None,
                "cwd": "/home/node/workspace",
                "messages": [{"role": "user", "content": "parent"}],
            },
        ]
        with (self.payload / "sessions.jsonl").open("w", encoding="utf-8") as handle:
            for session in sessions:
                handle.write(json.dumps(session) + "\n")
        with closing(sqlite3.connect(self.payload / "memory_store.db")) as connection:
            connection.execute(
                "CREATE TABLE facts (fact_id INTEGER PRIMARY KEY, content TEXT)"
            )
            connection.execute("INSERT INTO facts VALUES (1, 'Austin fact')")
            connection.commit()
        source_home = self.root / "manifest-source-home"
        (source_home / "workspace").mkdir(parents=True)
        (source_home / "workspace/notes.md").write_text("workspace\n", encoding="utf-8")
        migration.inventory_source_volume(
            self.bundle / "source-volume-inventory.json", source_home
        )

    def tearDown(self) -> None:
        self.tempdir.cleanup()

    def metadata(self) -> migration.SourceMetadata:
        return migration.SourceMetadata(
            host_id="box1",
            machine_id="austin-finite",
            owner_email="austin@finite.vip",
            hermes_version="0.14.0",
            image_reference="docker.io/library/fc-agent-runtime:main",
            image_manifest_digest="sha256:" + "a" * 64,
            container_image_id="sha256:" + "b" * 64,
            source_inventory_sha256=hashlib.sha256(
                (self.bundle / "source-volume-inventory.json").read_bytes()
            ).hexdigest(),
        )

    def build_manifest(self) -> dict:
        return migration.create_manifest(self.bundle, self.metadata())

    def manifest_sha256(self) -> str:
        return hashlib.sha256((self.bundle / "manifest.json").read_bytes()).hexdigest()

    def target_root(self) -> Path:
        target = self.root / "target"
        (target / "agent/identity").mkdir(parents=True)
        (target / "agent/hermes-home").mkdir(parents=True)
        (target / "workspace").mkdir(parents=True)
        (target / "agent/identity/identity.json").write_text(
            '{"npub":"npub-target"}\n', encoding="utf-8"
        )
        (target / "agent/client.sqlite3").write_bytes(b"finite-chat-client")
        with closing(
            sqlite3.connect(target / "agent/hermes-home/state.db")
        ) as connection:
            connection.execute("CREATE TABLE target_seed (value TEXT)")
            connection.execute("INSERT INTO target_seed VALUES ('preserved')")
            connection.commit()
        return target

    def test_manifest_is_versioned_hashed_and_reproducibly_verified(self) -> None:
        manifest = self.build_manifest()

        self.assertEqual(manifest["schema"], "finite.legacy-hermes-migration.v1")
        self.assertEqual(manifest["source"]["machine_id"], "austin-finite")
        self.assertEqual(manifest["cron"]["count"], 1)
        self.assertEqual(manifest["cron"]["target_state"], "review-only-not-active")
        self.assertEqual(manifest["memory"]["fact_count"], 1)
        self.assertEqual(
            manifest["source_inventory"]["sha256"],
            self.metadata().source_inventory_sha256,
        )
        self.assertEqual(
            manifest["compatibility"]["session_paths"],
            {
                "cache_media_archive_only_count": 2,
                "archive_only_policy": "retained-in-source-recovery-set",
                "rewritable_count": 2,
                "unmapped_source_path_count": 1,
            },
        )
        self.assertGreater(len(manifest["files"]), 5)
        migration.verify_bundle(self.bundle)

        memory = self.payload / "hermes/memories/MEMORY.md"
        memory.write_text("Tamper memory\n", encoding="utf-8")
        with self.assertRaisesRegex(migration.MigrationError, "sha256 mismatch"):
            migration.verify_bundle(self.bundle)

    def test_manifest_rejects_an_unapproved_source_hermes_version(self) -> None:
        metadata = self.metadata()
        unsupported = migration.SourceMetadata(
            **{**metadata.__dict__, "hermes_version": "0.15.0"}
        )

        with self.assertRaisesRegex(
            migration.MigrationError,
            "source Hermes version must be 0.14.0",
        ):
            migration.create_manifest(self.bundle, unsupported)

    def test_runbook_mounts_the_reviewed_tool_by_hash_into_existing_image(self) -> None:
        runbook = RUNBOOK.read_text(encoding="utf-8")

        self.assertIn("MIGRATION_TOOL_ARCHIVE", runbook)
        self.assertIn("MIGRATION_TOOL_SHA256", runbook)
        self.assertIn("sha256sum --check", runbook)
        self.assertIn("archive --format=tar", runbook)
        self.assertIn("legacy_hermes_contract.py", runbook)
        self.assertIn("legacy_hermes_source.py", runbook)
        self.assertIn("legacy_hermes_target.py", runbook)
        self.assertIn("source-volume-inventory", runbook)
        self.assertIn("--source-volume-inventory-sha256", runbook)
        self.assertIn("readOnlyRootFilesystem == true", runbook)
        self.assertIn("zero unresolved entries", runbook)
        self.assertIn("dst=/opt/migration,options=rbind:ro", runbook)
        self.assertIn("TARGET_RUNTIME_IMAGE", runbook)
        self.assertNotIn("MIGRATION_IMAGE", runbook)
        self.assertNotIn("Publish and prove the migration image", runbook)
        self.assertNotIn("legacy-hermes-source-export", runbook)
        self.assertNotIn("legacy-hermes-source-memory", runbook)

    def test_source_volume_inventory_requires_a_disposition_for_every_file(
        self,
    ) -> None:
        source = self.root / "source-home"
        (source / ".hermes/memories").mkdir(parents=True)
        (source / ".hermes/audio_cache").mkdir(parents=True)
        (source / ".brain/finite-mono").mkdir(parents=True)
        (source / "workspace").mkdir()
        (source / "dev/reap-video/venv/bin").mkdir(parents=True)
        (source / "custom-data").mkdir()
        (source / ".hermes/memories/MEMORY.md").write_text("memory\n", encoding="utf-8")
        (source / ".hermes/state.db").write_bytes(b"legacy sessions")
        (source / ".hermes/audio_cache/voice.ogg").write_bytes(b"voice")
        (source / ".brain/finite-mono/README.md").write_text(
            "brain checkout\n", encoding="utf-8"
        )
        (source / "workspace/notes.md").write_text("notes\n", encoding="utf-8")
        (source / "dev/reap-video/venv/bin/python").write_text(
            "generated\n", encoding="utf-8"
        )
        (source / "custom-data/ledger.txt").write_text(
            "must not be missed\n", encoding="utf-8"
        )

        inventory_path = self.root / "source-volume-inventory.json"
        result = migration.inventory_source_volume(inventory_path, source)

        self.assertEqual(result["schema"], "finite.legacy-hermes-source-inventory.v1")
        self.assertEqual(result["status"], "review-required")
        self.assertEqual(result["classifications"]["bundle"]["regular_files"], 2)
        self.assertEqual(result["classifications"]["converted"]["regular_files"], 1)
        self.assertEqual(result["classifications"]["archive-only"]["regular_files"], 2)
        self.assertEqual(result["classifications"]["rebuild"]["regular_files"], 1)
        self.assertEqual(result["classifications"]["unresolved"]["regular_files"], 1)
        self.assertEqual(
            [entry["path"] for entry in result["unresolved_roots"]],
            ["custom-data"],
        )
        self.assertEqual(inventory_path.stat().st_mode & 0o777, 0o600)
        self.assertNotIn("must not be missed", inventory_path.read_text())

        stderr = io.StringIO()
        with redirect_stderr(stderr):
            exit_code = migration.main(
                [
                    "source-volume-inventory",
                    "--source-root",
                    str(source),
                    "--output",
                    str(self.root / "second-inventory.json"),
                ]
            )
        self.assertEqual(exit_code, 1)
        self.assertIn("unresolved source entries", stderr.getvalue())

        (source / "custom-data/ledger.txt").unlink()
        (source / "custom-data").rmdir()
        complete = migration.inventory_source_volume(
            self.root / "complete-inventory.json", source
        )
        self.assertEqual(complete["status"], "complete")
        self.assertEqual(complete["classifications"]["unresolved"]["entries"], 0)

    def test_manifest_rejects_an_unresolved_or_rewritten_source_inventory(
        self,
    ) -> None:
        inventory_path = self.bundle / "source-volume-inventory.json"
        inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
        inventory["status"] = "review-required"
        inventory["classifications"]["unresolved"]["entries"] = 1
        inventory["unresolved_roots"] = [
            {
                "path": "surprise-data",
                "entries": 1,
                "regular_files": 1,
                "bytes": 12,
                "symlinks": 0,
                "special_files": 0,
            }
        ]
        inventory_path.write_text(json.dumps(inventory), encoding="utf-8")

        with self.assertRaisesRegex(
            migration.MigrationError, "still has unresolved entries"
        ):
            self.build_manifest()

        inventory["status"] = "complete"
        inventory["classifications"]["unresolved"]["entries"] = 0
        inventory["unresolved_roots"] = []
        inventory_path.write_text(json.dumps(inventory), encoding="utf-8")
        self.build_manifest()
        inventory["directories"] += 1
        inventory_path.write_text(json.dumps(inventory), encoding="utf-8")
        with self.assertRaisesRegex(
            migration.MigrationError, "source volume inventory sha256 mismatch"
        ):
            migration.verify_bundle(self.bundle)

    def test_manifest_rejects_unknown_paths_and_symlinks(self) -> None:
        (self.payload / "hermes/.env").write_text("TOKEN=secret\n", encoding="utf-8")
        with self.assertRaisesRegex(migration.MigrationError, "not allowed"):
            self.build_manifest()
        (self.payload / "hermes/.env").unlink()

        (self.payload / "home/dev/escape").symlink_to("../../../../etc/passwd")
        with self.assertRaisesRegex(migration.MigrationError, "escapes"):
            self.build_manifest()

    def test_offline_install_preserves_identity_chat_and_orders_sessions(self) -> None:
        manifest = self.build_manifest()
        target = self.target_root()
        identity = target / "agent/identity/identity.json"
        client = target / "agent/client.sqlite3"
        identity_before = hashlib.sha256(identity.read_bytes()).hexdigest()
        client_before = hashlib.sha256(client.read_bytes()).hexdigest()
        FakeSessionDB.calls = []

        receipt = migration.install_bundle(
            self.bundle,
            target,
            expected_machine_id="austin-finite",
            expected_manifest_sha256=self.manifest_sha256(),
            expected_identity_sha256=identity_before,
            expected_chat_client_sha256=client_before,
            session_db_factory=FakeSessionDB,
            memory_store_factory=FakeMemoryStore,
        )

        self.assertEqual(
            hashlib.sha256(identity.read_bytes()).hexdigest(), identity_before
        )
        self.assertEqual(hashlib.sha256(client.read_bytes()).hexdigest(), client_before)
        self.assertTrue((target / "agent/hermes-home/memories/MEMORY.md").is_file())
        self.assertFalse((target / "agent/hermes-home/skills").exists())
        self.assertTrue(
            target.joinpath(
                "migration/legacy-hermes-v1/review-only/skills/austin-skill/SKILL.md"
            ).is_file()
        )
        self.assertTrue(
            target.joinpath(
                "migration/legacy-hermes-v1/review-only/cron/jobs.json"
            ).is_file()
        )
        self.assertFalse(target.joinpath("agent/hermes-home/cron/jobs.json").exists())
        self.assertTrue(
            target.joinpath(
                "migration/legacy-hermes-v1/review-only/scripts/report.py"
            ).is_file()
        )
        self.assertTrue(
            (target / "workspace/legacy-box1/dev/project/README.md").is_file()
        )
        self.assertEqual(
            (target / "workspace/legacy-box1/dev/project/readme-link").readlink(),
            Path("README.md"),
        )
        self.assertEqual(
            [row["id"] for row in FakeSessionDB.calls], ["parent", "child"]
        )
        self.assertEqual(
            FakeSessionDB.calls[1]["cwd"],
            "/data/workspace/legacy-box1/dev/project",
        )
        self.assertEqual(
            FakeSessionDB.calls[1]["messages"][0]["content"][1]["path"],
            "/data/workspace/legacy-box1/uploads/photo.txt",
        )
        self.assertEqual(
            FakeSessionDB.calls[1]["messages"][0]["content"][2]["path"],
            "/home/node/.hermes/audio_cache/voice.ogg",
        )
        self.assertEqual(
            FakeSessionDB.calls[1]["messages"][1]["content"],
            (
                "MEDIA: /data/workspace/legacy-box1/uploads/photo.txt "
                "MEDIA: /home/node/.hermes/image_cache/result.png"
            ),
        )
        self.assertEqual(receipt["sessions"]["imported"], 2)
        self.assertEqual(receipt["source_inventory"], manifest["source_inventory"])
        self.assertEqual(receipt["compatibility"], manifest["compatibility"])
        self.assertEqual(receipt["cron"]["count"], 1)
        self.assertEqual(receipt["cron"]["target_state"], "review-only-not-active")
        self.assertEqual(receipt["memory"]["imported_fact_count"], 1)
        self.assertEqual(
            migration._memory_database_fact_count(
                target / "agent/hermes-home/memory_store.db"
            ),
            1,
        )
        self.assertEqual(receipt["protected_state"]["identity_sha256"], identity_before)
        self.assertTrue((target / "migration/legacy-hermes-v1/receipt.json").is_file())

        with self.assertRaisesRegex(migration.MigrationError, "already installed"):
            migration.install_bundle(
                self.bundle,
                target,
                expected_machine_id="austin-finite",
                expected_manifest_sha256=self.manifest_sha256(),
                expected_identity_sha256=identity_before,
                expected_chat_client_sha256=client_before,
                session_db_factory=FakeSessionDB,
                memory_store_factory=FakeMemoryStore,
            )

    def test_install_fails_closed_on_target_identity_or_file_collision(self) -> None:
        self.build_manifest()
        target = self.target_root()
        actual_identity = hashlib.sha256(
            (target / "agent/identity/identity.json").read_bytes()
        ).hexdigest()
        actual_client = hashlib.sha256(
            (target / "agent/client.sqlite3").read_bytes()
        ).hexdigest()

        with self.assertRaisesRegex(migration.MigrationError, "manifest sha256"):
            migration.install_bundle(
                self.bundle,
                target,
                expected_machine_id="austin-finite",
                expected_manifest_sha256="0" * 64,
                expected_identity_sha256=actual_identity,
                expected_chat_client_sha256=actual_client,
                session_db_factory=FakeSessionDB,
                memory_store_factory=FakeMemoryStore,
            )

        with self.assertRaisesRegex(migration.MigrationError, "identity sha256"):
            migration.install_bundle(
                self.bundle,
                target,
                expected_machine_id="austin-finite",
                expected_manifest_sha256=self.manifest_sha256(),
                expected_identity_sha256="0" * 64,
                expected_chat_client_sha256=actual_client,
                session_db_factory=FakeSessionDB,
                memory_store_factory=FakeMemoryStore,
            )

        with self.assertRaisesRegex(migration.MigrationError, "Chat client sha256"):
            migration.install_bundle(
                self.bundle,
                target,
                expected_machine_id="austin-finite",
                expected_manifest_sha256=self.manifest_sha256(),
                expected_identity_sha256=actual_identity,
                expected_chat_client_sha256="0" * 64,
                session_db_factory=FakeSessionDB,
                memory_store_factory=FakeMemoryStore,
            )

        target_memory = target / "agent/hermes-home/memories/MEMORY.md"
        target_memory.parent.mkdir(parents=True)
        target_memory.write_text("different\n", encoding="utf-8")
        with self.assertRaisesRegex(migration.MigrationError, "destination collision"):
            migration.install_bundle(
                self.bundle,
                target,
                expected_machine_id="austin-finite",
                expected_manifest_sha256=self.manifest_sha256(),
                expected_identity_sha256=actual_identity,
                expected_chat_client_sha256=actual_client,
                session_db_factory=FakeSessionDB,
                memory_store_factory=FakeMemoryStore,
            )

    def test_install_rejects_an_unapproved_target_hermes_version(self) -> None:
        self.build_manifest()
        target = self.target_root()
        identity_sha256 = hashlib.sha256(
            (target / "agent/identity/identity.json").read_bytes()
        ).hexdigest()
        chat_client_sha256 = hashlib.sha256(
            (target / "agent/client.sqlite3").read_bytes()
        ).hexdigest()

        with (
            mock.patch("importlib.metadata.version", return_value="0.21.0"),
            self.assertRaisesRegex(
                migration.MigrationError,
                "target Hermes version must be 0.20.0",
            ),
        ):
            migration.install_bundle(
                self.bundle,
                target,
                expected_machine_id="austin-finite",
                expected_manifest_sha256=self.manifest_sha256(),
                expected_identity_sha256=identity_sha256,
                expected_chat_client_sha256=chat_client_sha256,
                session_db_factory=FakeSessionDB,
                memory_store_factory=FakeMemoryStore,
            )

    def test_receipt_failure_restores_target_database_and_copied_files(self) -> None:
        self.build_manifest()
        target = self.target_root()
        target_db = target / "agent/hermes-home/state.db"
        state_before = hashlib.sha256(target_db.read_bytes()).hexdigest()
        identity_sha256 = hashlib.sha256(
            (target / "agent/identity/identity.json").read_bytes()
        ).hexdigest()
        chat_client_sha256 = hashlib.sha256(
            (target / "agent/client.sqlite3").read_bytes()
        ).hexdigest()
        migration_target = sys.modules["legacy_hermes_target"]
        original_write = migration_target._write_private_json

        def fail_receipt(path: Path, value: dict) -> None:
            if path.name == "receipt.json":
                raise OSError("synthetic receipt failure")
            original_write(path, value)

        with (
            mock.patch.object(migration_target, "_write_private_json", fail_receipt),
            self.assertRaisesRegex(OSError, "synthetic receipt failure"),
        ):
            migration.install_bundle(
                self.bundle,
                target,
                expected_machine_id="austin-finite",
                expected_manifest_sha256=self.manifest_sha256(),
                expected_identity_sha256=identity_sha256,
                expected_chat_client_sha256=chat_client_sha256,
                session_db_factory=FakeSessionDB,
                memory_store_factory=FakeMemoryStore,
            )

        self.assertEqual(
            hashlib.sha256(target_db.read_bytes()).hexdigest(), state_before
        )
        self.assertFalse(target.joinpath("workspace/legacy-box1").exists())
        self.assertFalse(target.joinpath("migration/legacy-hermes-v1").exists())
        self.assertFalse(target.joinpath("agent/hermes-home/memory_store.db").exists())

    def test_install_refuses_to_replace_fresh_target_memory_facts(self) -> None:
        self.build_manifest()
        target = self.target_root()
        target_memory = target / "agent/hermes-home/memory_store.db"
        with closing(sqlite3.connect(target_memory)) as connection:
            connection.execute(
                "CREATE TABLE facts (fact_id INTEGER PRIMARY KEY, content TEXT)"
            )
            connection.execute("INSERT INTO facts VALUES (1, 'new target fact')")
            connection.commit()
        identity_sha256 = hashlib.sha256(
            (target / "agent/identity/identity.json").read_bytes()
        ).hexdigest()
        chat_client_sha256 = hashlib.sha256(
            (target / "agent/client.sqlite3").read_bytes()
        ).hexdigest()

        with self.assertRaisesRegex(migration.MigrationError, "already contains facts"):
            migration.install_bundle(
                self.bundle,
                target,
                expected_machine_id="austin-finite",
                expected_manifest_sha256=self.manifest_sha256(),
                expected_identity_sha256=identity_sha256,
                expected_chat_client_sha256=chat_client_sha256,
                session_db_factory=FakeSessionDB,
                memory_store_factory=FakeMemoryStore,
            )

    def test_source_export_uses_a_sqlite_snapshot_and_streams_every_session(
        self,
    ) -> None:
        source_db = self.root / "legacy-state.db"
        with closing(sqlite3.connect(source_db)) as connection:
            connection.execute("CREATE TABLE sessions (id TEXT PRIMARY KEY, body TEXT)")
            connection.executemany(
                "INSERT INTO sessions VALUES (?, ?)",
                [("one", "first"), ("two", "second")],
            )
            connection.commit()

        class LegacySessionDB:
            def __init__(self, db_path: Path):
                self.connection = sqlite3.connect(db_path)
                self.connection.row_factory = sqlite3.Row

            def search_sessions(self, limit: int, offset: int = 0) -> list[dict]:
                rows = [
                    dict(row)
                    for row in self.connection.execute("SELECT id FROM sessions")
                ]
                return rows[offset : offset + limit]

            def session_count(self) -> int:
                return self.connection.execute(
                    "SELECT COUNT(*) FROM sessions"
                ).fetchone()[0]

            def export_session(self, session_id: str) -> dict:
                body = self.connection.execute(
                    "SELECT body FROM sessions WHERE id = ?", (session_id,)
                ).fetchone()[0]
                return {
                    "id": session_id,
                    "messages": [{"role": "user", "content": body}],
                }

            def close(self) -> None:
                self.connection.close()

        old_module = sys.modules.get("hermes_state")
        sys.modules["hermes_state"] = types.SimpleNamespace(SessionDB=LegacySessionDB)
        output = self.root / "sessions.jsonl"
        source_before = hashlib.sha256(source_db.read_bytes()).hexdigest()
        try:
            with mock.patch("importlib.metadata.version", return_value="0.14.0"):
                result = migration.export_source_sessions(output, source_db)
        finally:
            if old_module is None:
                sys.modules.pop("hermes_state", None)
            else:
                sys.modules["hermes_state"] = old_module

        self.assertEqual(result["sessions"], 2)
        self.assertEqual(result["messages"], 2)
        self.assertEqual(
            hashlib.sha256(source_db.read_bytes()).hexdigest(), source_before
        )
        self.assertEqual(
            [json.loads(line)["id"] for line in output.read_text().splitlines()],
            ["one", "two"],
        )

    def test_source_export_rejects_an_unapproved_installed_version(self) -> None:
        source_db = self.root / "legacy-state.db"
        sqlite3.connect(source_db).close()

        with (
            mock.patch("importlib.metadata.version", return_value="0.13.0"),
            self.assertRaisesRegex(
                migration.MigrationError,
                "source Hermes version must be 0.14.0",
            ),
        ):
            migration.export_source_sessions(
                self.root / "sessions.jsonl",
                source_db,
            )

    def test_source_memory_snapshot_uses_sqlite_backup_and_preserves_facts(
        self,
    ) -> None:
        source_db = self.root / "source-memory.db"
        with closing(sqlite3.connect(source_db)) as connection:
            connection.execute(
                "CREATE TABLE facts (fact_id INTEGER PRIMARY KEY, content TEXT)"
            )
            connection.execute("INSERT INTO facts VALUES (1, 'remember Austin')")
            connection.commit()
        source_before = hashlib.sha256(source_db.read_bytes()).hexdigest()
        output = self.root / "memory_store.db"

        with mock.patch("importlib.metadata.version", return_value="0.14.0"):
            result = migration.snapshot_source_memory(output, source_db)

        self.assertEqual(result["facts"], 1)
        self.assertEqual(migration._memory_database_fact_count(output), 1)
        self.assertEqual(
            hashlib.sha256(source_db.read_bytes()).hexdigest(), source_before
        )
        self.assertFalse(Path(str(output) + "-wal").exists())
        self.assertFalse(Path(str(output) + "-shm").exists())

    def test_source_writer_check_fails_on_a_writable_fd_under_the_pvc(self) -> None:
        source_root = self.root / "source-pvc"
        source_root.mkdir()
        source_file = source_root / "state.db"
        source_file.touch()
        proc_root = self.root / "proc"
        fd_root = proc_root / "123/fd"
        fdinfo_root = proc_root / "123/fdinfo"
        fd_root.mkdir(parents=True)
        fdinfo_root.mkdir(parents=True)
        (fd_root / "4").symlink_to(source_file)
        (fdinfo_root / "4").write_text("flags:\t0100002\n", encoding="utf-8")

        stderr = io.StringIO()
        with redirect_stderr(stderr):
            result = migration.main(
                [
                    "source-writer-check",
                    "--source-root",
                    str(source_root),
                    "--proc-root",
                    str(proc_root),
                ]
            )

        self.assertEqual(result, 1)
        self.assertIn("writable file descriptor", stderr.getvalue())

        (fdinfo_root / "4").write_text("flags:\t0100000\n", encoding="utf-8")
        with redirect_stdout(io.StringIO()):
            self.assertEqual(
                migration.main(
                    [
                        "source-writer-check",
                        "--source-root",
                        str(source_root),
                        "--proc-root",
                        str(proc_root),
                    ]
                ),
                0,
            )

    def test_source_writer_check_fails_on_a_writable_map_under_the_pvc(self) -> None:
        source_root = self.root / "source-pvc"
        source_root.mkdir()
        source_file = source_root / "state.db"
        source_file.touch()
        proc_root = self.root / "proc"
        process_root = proc_root / "456"
        (process_root / "fd").mkdir(parents=True)
        (process_root / "maps").write_text(
            f"7f00-7f10 rw-s 00000000 00:00 1 {source_file}\n",
            encoding="utf-8",
        )

        with self.assertRaisesRegex(migration.MigrationError, "writable memory map"):
            migration.check_source_writers(source_root, proc_root)


if __name__ == "__main__":
    unittest.main()

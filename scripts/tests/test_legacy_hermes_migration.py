from __future__ import annotations

import hashlib
import importlib.util
import io
import json
import sqlite3
import sys
import tarfile
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
SOURCE_LAUNCHER = ROOT / "scripts" / "legacy-hermes-source"
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
        (source_home / "dev/published-site").mkdir(parents=True)
        (source_home / ".hermes").mkdir()
        (source_home / ".brain").mkdir()
        (source_home / ".finite").mkdir()
        (source_home / "custom-data").mkdir()
        (source_home / "workspace/notes.md").write_text("workspace\n", encoding="utf-8")
        (source_home / ".finite/device.key").write_text(
            "legacy identity\n", encoding="utf-8"
        )
        (source_home / "custom-data/ledger.txt").write_text(
            "unknown but durable\n", encoding="utf-8"
        )
        (source_home / "dev/published-site/index.html").write_text(
            "Austin site\n", encoding="utf-8"
        )
        (source_home / ".hermes/.env").write_text(
            "TELEGRAM_BOT_TOKEN=synthetic-telegram-secret\n"
            "TELEGRAM_ALLOWED_USERS=synthetic-user-id\n"
            "SIGNAL_HTTP_URL=http://signal.invalid\n"
            "SIGNAL_ACCOUNT=synthetic-signal-account\n"
            "GOOGLE_WORKSPACE_CLI_CREDENTIALS_FILE=/home/node/.hermes/google_token.json\n"
            "CUSTOM_SAAS_TOKEN=synthetic-custom-secret\n",
            encoding="utf-8",
        )
        (source_home / ".hermes/config.yaml").write_text(
            "platforms:\n"
            "  telegram:\n"
            "    enabled: true\n"
            "  custom_chat:\n"
            "    enabled: true\n",
            encoding="utf-8",
        )
        (source_home / ".hermes/google_token.json").write_text(
            '{"token":"synthetic-google-secret"}\n', encoding="utf-8"
        )
        (source_home / ".brain/agent.json").write_text(
            '{"identity":"synthetic-brain-identity"}\n', encoding="utf-8"
        )
        migration.inventory_source_volume(
            self.bundle / "source-volume-inventory.json", source_home
        )
        self.published_endpoints = self.root / "default-published-endpoints.json"
        self.published_endpoints.write_text(
            json.dumps(
                {
                    "machineId": "austin-finite",
                    "endpoints": [
                        {
                            "hostname": "austin-site.finite.computer",
                            "label": "Austin Site",
                            "target_port": 3000,
                            "status": "published",
                            "run_command": "python3 -m http.server 3000",
                            "run_cwd": "/home/node/dev/published-site",
                            "desired_process_state": "running",
                            "auth": {
                                "mode": "self",
                                "owner_email": "austin@finite.vip",
                            },
                            "created_at": "2026-07-01T00:00:00Z",
                            "updated_at": "2026-08-01T00:00:00Z",
                        }
                    ],
                }
            ),
            encoding="utf-8",
        )
        migration.inventory_source_sites(
            self.bundle / "sites.json",
            self.published_endpoints,
            self.bundle / "source-volume-inventory.json",
            expected_machine_id="austin-finite",
        )
        migration.inventory_source_integrations(
            self.bundle / "integrations.json",
            source_home,
            self.bundle / "source-volume-inventory.json",
        )
        with tarfile.open(self.payload / "source-home.tar", "w") as archive:
            archive.add(source_home, arcname=".", recursive=True)
        (self.payload / "source-home.tar").chmod(0o600)

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

    def test_sites_inventory_binds_authoritative_records_to_preserved_source(
        self,
    ) -> None:
        control_plane_export = self.root / "published-endpoints.json"
        control_plane_export.write_text(
            json.dumps(
                {
                    "machineId": "austin-finite",
                    "endpoints": [
                        {
                            "hostname": "austin-site.finite.computer",
                            "label": "Austin Site",
                            "target_port": 3000,
                            "status": "published",
                            "run_command": "python3 -m http.server 3000",
                            "run_cwd": "/home/node/dev/published-site",
                            "desired_process_state": "running",
                            "auth": {
                                "mode": "self",
                                "owner_email": "austin@finite.vip",
                            },
                            "created_at": "2026-07-01T00:00:00Z",
                            "updated_at": "2026-08-01T00:00:00Z",
                        },
                        {
                            "hostname": "reserved.finite.computer",
                            "label": "Reserved",
                            "status": "reserved",
                            "desired_process_state": "external",
                            "auth": {"mode": "self"},
                            "created_at": "2026-07-01T00:00:00Z",
                            "updated_at": "2026-07-01T00:00:00Z",
                        },
                    ],
                }
            ),
            encoding="utf-8",
        )
        output = self.bundle / "sites-roundtrip.json"

        result = migration.inventory_source_sites(
            output,
            control_plane_export,
            self.bundle / "source-volume-inventory.json",
            expected_machine_id="austin-finite",
        )

        self.assertEqual(result["schema"], "finite.legacy-hermes-sites.v1")
        self.assertEqual(result["machine_id"], "austin-finite")
        self.assertEqual(result["endpoint_count"], 2)
        self.assertEqual(result["source_paths_required"], 1)
        self.assertEqual(result["source_paths_present"], 1)
        self.assertEqual(result["status"], "complete")
        self.assertEqual(
            result["endpoints"][0]["source"],
            {
                "relative_path": "dev/published-site",
                "status": "present-in-source-snapshot",
            },
        )
        self.assertEqual(
            result["endpoints"][1]["source"],
            {"relative_path": None, "status": "not-required"},
        )
        self.assertEqual(output.stat().st_mode & 0o777, 0o600)

        export = json.loads(control_plane_export.read_text(encoding="utf-8"))
        export["endpoints"][0]["run_cwd"] = "/home/node/dev/missing-site"
        control_plane_export.write_text(json.dumps(export), encoding="utf-8")
        (self.bundle / "sites-missing.json").unlink(missing_ok=True)
        with self.assertRaisesRegex(
            migration.MigrationError, "published site source is missing"
        ):
            migration.inventory_source_sites(
                self.bundle / "sites-missing.json",
                control_plane_export,
                self.bundle / "source-volume-inventory.json",
                expected_machine_id="austin-finite",
            )

    def test_integrations_inventory_records_policy_without_secret_values(self) -> None:
        output = self.bundle / "integrations-roundtrip.json"

        result = migration.inventory_source_integrations(
            output,
            self.root / "manifest-source-home",
            self.bundle / "source-volume-inventory.json",
        )

        self.assertEqual(result["schema"], "finite.legacy-hermes-integrations.v1")
        self.assertEqual(result["status"], "complete")
        integrations = {item["name"]: item for item in result["integrations"]}
        self.assertEqual(
            integrations["telegram"]["migration_policy"],
            "controlled-transfer-after-rehearsal",
        )
        self.assertEqual(
            integrations["signal"]["migration_policy"],
            "controlled-transfer-after-rehearsal",
        )
        self.assertEqual(
            integrations["google-workspace"]["migration_policy"],
            "fresh-authorization-required",
        )
        self.assertEqual(
            integrations["finitebrain"]["migration_policy"],
            "fresh-authorization-required",
        )
        self.assertEqual(
            integrations["custom-chat"]["migration_policy"],
            "preserve-disabled-until-supported-setup",
        )
        self.assertEqual(
            integrations["other-environment-config"]["configured_keys"],
            ["CUSTOM_SAAS_TOKEN"],
        )
        rendered = json.dumps(result, sort_keys=True)
        for secret in (
            "synthetic-telegram-secret",
            "synthetic-user-id",
            "synthetic-signal-account",
            "synthetic-custom-secret",
            "synthetic-google-secret",
            "synthetic-brain-identity",
        ):
            self.assertNotIn(secret, rendered)
        self.assertEqual(output.stat().st_mode & 0o777, 0o600)

    def test_sites_and_integrations_are_available_through_the_operator_cli(
        self,
    ) -> None:
        stdout = io.StringIO()
        with redirect_stdout(stdout):
            sites_exit = migration.main(
                [
                    "source-sites-inventory",
                    "--published-endpoints",
                    str(self.published_endpoints),
                    "--source-volume-inventory",
                    str(self.bundle / "source-volume-inventory.json"),
                    "--expected-machine-id",
                    "austin-finite",
                    "--output",
                    str(self.bundle / "sites-cli.json"),
                ]
            )
        self.assertEqual(sites_exit, 0)
        self.assertEqual(json.loads(stdout.getvalue())["status"], "complete")

        stdout = io.StringIO()
        with redirect_stdout(stdout):
            integrations_exit = migration.main(
                [
                    "source-integrations-inventory",
                    "--source-root",
                    str(self.root / "manifest-source-home"),
                    "--source-volume-inventory",
                    str(self.bundle / "source-volume-inventory.json"),
                    "--output",
                    str(self.bundle / "integrations-cli.json"),
                ]
            )
        self.assertEqual(integrations_exit, 0)
        self.assertEqual(json.loads(stdout.getvalue())["status"], "complete")

    def test_manifest_is_versioned_hashed_and_reproducibly_verified(self) -> None:
        manifest = self.build_manifest()

        self.assertEqual(manifest["schema"], "finite.legacy-hermes-migration.v2")
        self.assertEqual(manifest["source"]["machine_id"], "austin-finite")
        self.assertEqual(manifest["cron"]["count"], 1)
        self.assertEqual(manifest["cron"]["target_state"], "review-only-not-active")
        self.assertEqual(manifest["memory"]["fact_count"], 1)
        self.assertEqual(manifest["sites"]["endpoint_count"], 1)
        self.assertEqual(manifest["sites"]["source_paths_present"], 1)
        self.assertEqual(manifest["integrations"]["integration_count"], 6)
        self.assertEqual(
            manifest["integrations"]["activation_policy"],
            "inventory-only-no-secret-values-or-activation",
        )
        self.assertEqual(
            manifest["source_inventory"]["sha256"],
            self.metadata().source_inventory_sha256,
        )
        self.assertEqual(
            manifest["compatibility"]["session_paths"],
            {
                "cache_media_preserved_count": 2,
                "preservation_policy": "retained-in-sealed-source-home",
                "rewritable_count": 2,
                "unmapped_source_path_count": 1,
            },
        )
        self.assertEqual(
            manifest["preserved_inert"]["default"],
            "every non-activated source entry remains in source-home.tar",
        )
        self.assertGreater(len(manifest["files"]), 5)
        migration.verify_bundle(self.bundle)

        memory = self.payload / "hermes/memories/MEMORY.md"
        memory.write_text("Tamper memory\n", encoding="utf-8")
        with self.assertRaisesRegex(migration.MigrationError, "sha256 mismatch"):
            migration.verify_bundle(self.bundle)

    def test_manifest_requires_untampered_sites_and_integrations_inventories(
        self,
    ) -> None:
        self.build_manifest()

        sites = self.bundle / "sites.json"
        sites.write_text(sites.read_text(encoding="utf-8") + " ", encoding="utf-8")
        with self.assertRaisesRegex(migration.MigrationError, "Sites summary mismatch"):
            migration.verify_bundle(self.bundle)

        sites.unlink()
        with self.assertRaisesRegex(
            migration.MigrationError, "Sites inventory is missing"
        ):
            self.build_manifest()

    def test_manifest_rechecks_site_source_evidence_against_the_volume(self) -> None:
        sites_path = self.bundle / "sites.json"
        sites = json.loads(sites_path.read_text(encoding="utf-8"))
        sites["endpoints"][0]["source"]["relative_path"] = "custom-data"
        sites_path.write_text(json.dumps(sites), encoding="utf-8")

        with self.assertRaisesRegex(
            migration.MigrationError,
            "Sites inventory source evidence does not match run_cwd",
        ):
            self.build_manifest()

    def test_manifest_rechecks_integration_evidence_against_the_volume(self) -> None:
        integrations_path = self.bundle / "integrations.json"
        integrations = json.loads(integrations_path.read_text(encoding="utf-8"))
        integrations["integrations"][0]["evidence_paths"] = [
            ".hermes/missing-credential.json"
        ]
        integrations_path.write_text(json.dumps(integrations), encoding="utf-8")

        with self.assertRaisesRegex(
            migration.MigrationError,
            "Integrations inventory evidence is missing from source snapshot",
        ):
            self.build_manifest()

    def test_manifest_binds_every_inventory_entry_to_the_source_home_snapshot(
        self,
    ) -> None:
        manifest = self.build_manifest()

        self.assertEqual(
            manifest["source_snapshot"]["target"],
            "migration/legacy-hermes-v2/preserved/source-home.tar",
        )
        self.assertEqual(manifest["source_snapshot"]["default_disposition"], "preserve")
        self.assertEqual(manifest["source_snapshot"]["status"], "complete")
        migration.verify_bundle(self.bundle)

        source_home = self.root / "manifest-source-home"
        archive_path = self.payload / "source-home.tar"
        archive_path.unlink()
        with tarfile.open(archive_path, "w") as archive:
            archive.add(source_home / "workspace", arcname="workspace", recursive=True)
        archive_path.chmod(0o600)

        with self.assertRaisesRegex(
            migration.MigrationError, "source-home snapshot does not match inventory"
        ):
            self.build_manifest()

    def test_manifest_requires_the_source_home_snapshot_to_remain_root_only(
        self,
    ) -> None:
        (self.payload / "source-home.tar").chmod(0o644)

        with self.assertRaisesRegex(
            migration.MigrationError, "source-home snapshot must be mode 0600"
        ):
            self.build_manifest()

    def test_manifest_requires_the_source_inventory_to_remain_root_only(self) -> None:
        (self.bundle / "source-volume-inventory.json").chmod(0o644)

        with self.assertRaisesRegex(
            migration.MigrationError, "source volume inventory must be mode 0600"
        ):
            self.build_manifest()

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
        self.assertIn("source-sites-inventory", runbook)
        self.assertIn("source-integrations-inventory", runbook)
        self.assertIn("--source-volume-inventory-sha256", runbook)
        self.assertIn("readOnlyRootFilesystem == true", runbook)
        self.assertIn("source-home.tar", runbook)
        self.assertIn("unknown safe paths default to `preserve`", runbook)
        self.assertIn("zero structurally blocked entries", runbook)
        self.assertNotIn("Austin explicitly chooses", runbook)
        self.assertNotIn("owner decisions", runbook)
        self.assertIn("dst=/opt/migration,ro", runbook)
        self.assertIn("TARGET_RUNTIME_IMAGE", runbook)
        self.assertNotIn("MIGRATION_IMAGE", runbook)
        self.assertNotIn("Publish and prove the migration image", runbook)
        self.assertNotIn("legacy-hermes-source-export", runbook)
        self.assertNotIn("legacy-hermes-source-memory", runbook)

        launcher = SOURCE_LAUNCHER.read_text(encoding="utf-8")
        self.assertIn("source-sites-inventory", launcher)
        self.assertIn("source-integrations-inventory", launcher)

    def test_source_volume_inventory_preserves_unknown_safe_data_automatically(
        self,
    ) -> None:
        source = self.root / "source-home"
        (source / ".hermes/memories").mkdir(parents=True)
        (source / ".hermes/audio_cache").mkdir(parents=True)
        (source / ".hermes/cron/outputs/job-1").mkdir(parents=True)
        (source / ".hermes/sessions").mkdir(parents=True)
        (source / ".codex").mkdir()
        (source / ".brain/finite-mono").mkdir(parents=True)
        (source / "workspace").mkdir()
        (source / "dev/reap-video/venv/bin").mkdir(parents=True)
        (source / "custom-data").mkdir()
        (source / ".hermes/memories/MEMORY.md").write_text("memory\n", encoding="utf-8")
        (source / ".hermes/state.db").write_bytes(b"legacy sessions")
        (source / ".hermes/audio_cache/voice.ogg").write_bytes(b"voice")
        (source / ".hermes/cron/outputs/job-1/result.txt").write_text(
            "generated cron output\n", encoding="utf-8"
        )
        (source / ".hermes/sessions/session.jsonl").write_text(
            "transcript log\n", encoding="utf-8"
        )
        (source / ".codex/cache.json").write_text("generated\n", encoding="utf-8")
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
        (source / "custom-data/ledger-link").symlink_to("ledger.txt")

        inventory_path = self.root / "source-volume-inventory.json"
        result = migration.inventory_source_volume(inventory_path, source)

        self.assertEqual(result["schema"], "finite.legacy-hermes-source-inventory.v2")
        self.assertEqual(result["status"], "complete")
        self.assertEqual(result["classifications"]["activate"]["regular_files"], 2)
        self.assertEqual(result["classifications"]["converted"]["regular_files"], 1)
        self.assertEqual(result["classifications"]["preserve"]["regular_files"], 3)
        self.assertEqual(result["classifications"]["preserve"]["symlinks"], 1)
        self.assertEqual(result["classifications"]["quarantine"]["regular_files"], 3)
        self.assertEqual(result["classifications"]["rebuild"]["regular_files"], 1)
        self.assertEqual(result["classifications"]["blocked"]["entries"], 0)
        preserved_paths = {
            entry["path"]
            for entry in result["entries"]
            if entry["disposition"] == "preserve"
        }
        self.assertIn("custom-data/ledger.txt", preserved_paths)
        self.assertIn("custom-data/ledger-link", preserved_paths)
        self.assertEqual(inventory_path.stat().st_mode & 0o777, 0o600)
        self.assertNotIn("must not be missed", inventory_path.read_text())

        stderr = io.StringIO()
        stdout = io.StringIO()
        with redirect_stderr(stderr), redirect_stdout(stdout):
            exit_code = migration.main(
                [
                    "source-volume-inventory",
                    "--source-root",
                    str(source),
                    "--output",
                    str(self.root / "second-inventory.json"),
                ]
            )
        self.assertEqual(exit_code, 0)
        self.assertEqual(stderr.getvalue(), "")
        self.assertEqual(json.loads(stdout.getvalue())["status"], "complete")

    def test_source_volume_inventory_blocks_symlinks_that_escape_the_source(
        self,
    ) -> None:
        source = self.root / "source-home"
        source.mkdir()
        outside = self.root / "outside-secret"
        outside.write_text("do not follow\n", encoding="utf-8")
        (source / "escape").symlink_to(outside)

        result = migration.inventory_source_volume(
            self.root / "blocked-inventory.json", source
        )

        self.assertEqual(result["status"], "blocked")
        self.assertEqual(result["classifications"]["blocked"]["symlinks"], 1)
        self.assertEqual(result["blocked_roots"][0]["path"], "escape")

        stderr = io.StringIO()
        with redirect_stderr(stderr):
            exit_code = migration.main(
                [
                    "source-volume-inventory",
                    "--source-root",
                    str(source),
                    "--output",
                    str(self.root / "second-blocked-inventory.json"),
                ]
            )
        self.assertEqual(exit_code, 1)
        self.assertIn("structurally blocked entries", stderr.getvalue())

    def test_source_volume_inventory_rebuilds_generated_external_symlinks(
        self,
    ) -> None:
        source = self.root / "source-home"
        generated_links = {
            ".config/pulse/austin-finite-0-runtime": "/tmp/pulse-runtime",
            ".hermes/venv/bin/python": "/nix/store/legacy-python/bin/python",
            ".local/uv-tools/ruff/bin/python": "/nix/store/legacy-ruff/bin/python",
            "dev/reap-video/venv/bin/python": "/nix/store/legacy-python/bin/python",
        }
        for relative, target in generated_links.items():
            candidate = source / relative
            candidate.parent.mkdir(parents=True, exist_ok=True)
            candidate.symlink_to(target)

        result = migration.inventory_source_volume(
            self.root / "generated-links-inventory.json", source
        )

        self.assertEqual(result["status"], "complete")
        self.assertEqual(result["classifications"]["blocked"]["entries"], 0)
        self.assertEqual(result["classifications"]["rebuild"]["symlinks"], 4)
        dispositions = {
            entry["path"]: entry["disposition"] for entry in result["entries"]
        }
        self.assertEqual(
            {path: dispositions[path] for path in generated_links},
            {path: "rebuild" for path in generated_links},
        )

    def test_source_volume_inventory_fails_cleanly_on_unreadable_data(self) -> None:
        source = self.root / "source-home"
        source.mkdir()
        source_file = source / "private.bin"
        source_file.write_bytes(b"private")
        source_module = sys.modules["legacy_hermes_source"]

        with (
            mock.patch.object(
                source_module,
                "_sha256",
                side_effect=PermissionError("synthetic permission failure"),
            ),
            self.assertRaisesRegex(
                migration.MigrationError, "could not hash source entry: private.bin"
            ),
        ):
            migration.inventory_source_volume(
                self.root / "unreadable-inventory.json", source
            )

    def test_manifest_rejects_a_structurally_blocked_or_rewritten_source_inventory(
        self,
    ) -> None:
        inventory_path = self.bundle / "source-volume-inventory.json"
        inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
        inventory["status"] = "blocked"
        inventory["classifications"]["blocked"]["entries"] = 1
        inventory["classifications"]["blocked"]["special_files"] = 1
        inventory["blocked_roots"] = [
            {
                "path": "surprise-data",
                "entries": 1,
                "directories": 0,
                "regular_files": 0,
                "bytes": 12,
                "symlinks": 0,
                "special_files": 1,
            }
        ]
        inventory_path.write_text(json.dumps(inventory), encoding="utf-8")

        with self.assertRaisesRegex(
            migration.MigrationError, "structurally blocked entries"
        ):
            self.build_manifest()

        inventory["status"] = "complete"
        inventory["classifications"]["blocked"]["entries"] = 0
        inventory["classifications"]["blocked"]["special_files"] = 0
        inventory["blocked_roots"] = []
        inventory_path.write_text(json.dumps(inventory), encoding="utf-8")
        self.build_manifest()
        inventory["directories"] += 1
        inventory_path.write_text(json.dumps(inventory), encoding="utf-8")
        with self.assertRaisesRegex(
            migration.MigrationError, "directory count mismatch"
        ):
            migration.verify_bundle(self.bundle)

    def test_manifest_rejects_inventory_summaries_that_disagree_with_entries(
        self,
    ) -> None:
        inventory_path = self.bundle / "source-volume-inventory.json"
        inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
        inventory["classifications"]["preserve"]["bytes"] += 1
        inventory_path.write_text(json.dumps(inventory), encoding="utf-8")

        with self.assertRaisesRegex(
            migration.MigrationError, "classification summary does not match entries"
        ):
            self.build_manifest()

    def test_manifest_rejects_malformed_inventory_entry_metadata(self) -> None:
        inventory_path = self.bundle / "source-volume-inventory.json"
        inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
        inventory["entries"][0]["path"] = None
        inventory_path.write_text(json.dumps(inventory), encoding="utf-8")

        with self.assertRaisesRegex(
            migration.MigrationError,
            "source volume inventory entry metadata is invalid",
        ):
            self.build_manifest()

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
                "migration/legacy-hermes-v2/review-only/skills/austin-skill/SKILL.md"
            ).is_file()
        )
        self.assertTrue(
            target.joinpath(
                "migration/legacy-hermes-v2/review-only/cron/jobs.json"
            ).is_file()
        )
        self.assertFalse(target.joinpath("agent/hermes-home/cron/jobs.json").exists())
        self.assertTrue(
            target.joinpath(
                "migration/legacy-hermes-v2/review-only/scripts/report.py"
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
        self.assertEqual(receipt["sites"], manifest["sites"])
        self.assertEqual(receipt["integrations"], manifest["integrations"])
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
        self.assertTrue((target / "migration/legacy-hermes-v2/receipt.json").is_file())

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

    def test_offline_install_preserves_complete_source_home_without_activating_it(
        self,
    ) -> None:
        manifest = self.build_manifest()
        target = self.target_root()
        identity = target / "agent/identity/identity.json"
        client = target / "agent/client.sqlite3"

        receipt = migration.install_bundle(
            self.bundle,
            target,
            expected_machine_id="austin-finite",
            expected_manifest_sha256=self.manifest_sha256(),
            expected_identity_sha256=hashlib.sha256(identity.read_bytes()).hexdigest(),
            expected_chat_client_sha256=hashlib.sha256(client.read_bytes()).hexdigest(),
            session_db_factory=FakeSessionDB,
            memory_store_factory=FakeMemoryStore,
        )

        preserved = target / "migration/legacy-hermes-v2/preserved/source-home.tar"
        self.assertTrue(preserved.is_file())
        self.assertEqual(preserved.stat().st_mode & 0o777, 0o600)
        with tarfile.open(preserved, "r:") as archive:
            paths = {
                member.name.removeprefix("./")
                for member in archive
                if member.name not in ("", ".")
            }
        self.assertIn("custom-data/ledger.txt", paths)
        self.assertIn(".finite/device.key", paths)
        self.assertFalse((target / ".finite").exists())
        self.assertEqual(receipt["source_snapshot"], manifest["source_snapshot"])

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
        self.assertFalse(target.joinpath("migration/legacy-hermes-v2").exists())
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

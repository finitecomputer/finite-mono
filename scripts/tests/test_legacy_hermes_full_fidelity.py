"""Unit and contract tests for the opt-in full-fidelity importer mode.

The default-mode suite (test_legacy_hermes_migration.py) is unchanged; these
tests build a census-shaped box1 home (legacy_box1_fixture.py) and install it
with fake Hermes databases. Real-library proof against the pinned target
Hermes, including a live gateway, is finitechat/tests/hermes/
test_legacy_hermes_full_fidelity.py.
"""

from __future__ import annotations

import functools
import hashlib
import importlib.util
import io
import json
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from datetime import datetime, timedelta, timezone
from pathlib import Path, PurePosixPath
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
MODULE = ROOT / "scripts" / "legacy_hermes_migration.py"
sys.path.insert(0, str(MODULE.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
SPEC = importlib.util.spec_from_file_location("legacy_hermes_migration_ff", MODULE)
assert SPEC is not None and SPEC.loader is not None
migration = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = migration
SPEC.loader.exec_module(migration)

import legacy_box1_fixture as fx  # noqa: E402
import legacy_hermes_fidelity as fidelity  # noqa: E402
import yaml  # noqa: E402


class FakeSessionDB:
    imported: list[dict] = []

    def __init__(self, db_path: Path):
        self.db_path = db_path

    def import_sessions(self, sessions: list[dict]) -> dict:
        FakeSessionDB.imported.extend(sessions)
        return {"ok": True, "imported": len(sessions), "skipped": 0, "detached": 0}

    def close(self) -> None:
        return None


class FakeMemoryStore:
    def __init__(self, db_path: Path):
        self.db_path = db_path

    def rebuild_all_vectors(self) -> int:
        return 0

    def close(self) -> None:
        return None


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class FullFidelityTests(unittest.TestCase):
    def setUp(self) -> None:
        version = mock.patch("importlib.metadata.version", return_value="0.21.0")
        version.start()
        self.addCleanup(version.stop)
        self.tempdir = tempfile.TemporaryDirectory()
        self.addCleanup(self.tempdir.cleanup)
        self.root = Path(self.tempdir.name)
        self.now = datetime(2026, 11, 13, 18, 0, tzinfo=timezone.utc)
        evidence = fx.build_bundle(self.root, migration, now=self.now)
        self.bundle = evidence["bundle"]
        self.metadata = fx.source_metadata(migration, evidence["inventory_sha256"])
        FakeSessionDB.imported = []
        self.hooks = fidelity.FidelityHooks(
            is_default_soul=lambda text: text == fx.DEFAULT_SOUL,
            bundled_skill_names=lambda: {
                "apple-notes": PurePosixPath("apple/apple-notes")
            },
            telegram_preserves_pending_updates=lambda: True,
            now=lambda: self.now,
        )

    def manifest(self, mode: str = "full-fidelity") -> dict:
        return migration.create_manifest(self.bundle, self.metadata, mode=mode)

    def install(self, target: Path, *, mode: str = "full-fidelity", hooks=None) -> dict:
        return migration.install_bundle(
            self.bundle,
            target,
            expected_machine_id="frankie",
            expected_manifest_sha256=sha256(self.bundle / "manifest.json"),
            expected_identity_sha256=sha256(target / "agent/identity/identity.json"),
            expected_chat_client_sha256=sha256(target / "agent/client.sqlite3"),
            session_db_factory=FakeSessionDB,
            memory_store_factory=FakeMemoryStore,
            mode=mode,
            fidelity_hooks=hooks or self.hooks,
        )

    def installed(self) -> tuple[Path, dict]:
        self.manifest()
        target = fx.build_target(self.root)
        return target, self.install(target)

    def tree(self, root: Path) -> dict[str, str]:
        return {
            path.relative_to(root).as_posix(): (
                "link:" + str(path.readlink())
                if path.is_symlink()
                else ("dir" if path.is_dir() else sha256(path))
            )
            for path in sorted(root.rglob("*"))
        }

    # -- manifest contract ---------------------------------------------------

    def test_default_manifest_carries_no_fidelity_plan(self) -> None:
        manifest = self.manifest(mode="default")
        self.assertNotIn("mode", manifest)
        self.assertNotIn("fidelity", manifest)

    def test_plan_is_bound_into_the_manifest_without_secret_values(self) -> None:
        manifest = self.manifest()
        plan = manifest["fidelity"]
        self.assertEqual(manifest["mode"], "full-fidelity")
        text = (self.bundle / "manifest.json").read_text()
        for secret in fx.SECRET_VALUES:
            self.assertNotIn(secret, text)
        self.assertTrue(plan["telegram"]["activate"])
        self.assertEqual(plan["telegram"]["token_sources"], ["env:TELEGRAM_BOT_TOKEN"])
        self.assertEqual(plan["telegram"]["pairing"]["approved_count"], 2)
        self.assertEqual(plan["telegram"]["pairing"]["pending_count"], 1)
        self.assertEqual(
            [skill["path"] for skill in plan["skills"]["user"]],
            [fx.USER_SKILL, fx.CLASH_SKILL],
        )
        self.assertEqual(
            {
                item["path"]: item["reason"]
                for item in plan["skills"]["managed_not_activated"]
            },
            {
                fx.BUNDLED_SKILL: "hermes-bundled-unmodified",
                "devops/overlay-helper": "box1-image-managed-overlay",
            },
        )
        self.assertEqual(
            plan["home"]["placed"],
            ["Downloads", "lessons", "make_post.py", "poster_final.jpg", "recipes"],
        )
        self.assertEqual(
            {item["name"]: item["reason"] for item in plan["home"]["withheld"]},
            {
                "AGENTS.md": "box1-platform-seeded-review-only",
                "__pycache__": "generated-bytecode",
                "finitebrain": "brain-working-tree-reauthorize-and-sync",
                "node_modules": "generated-dependency-tree",
            },
        )
        self.assertIn(
            "tts.elevenlabs.api_key", plan["config"]["dropped_secret_shaped_keys"]
        )
        self.assertIn("timezone", plan["config"]["adopted_keys"])
        self.assertIn("display.personality", plan["config"]["adopted_keys"])
        self.assertNotIn("display.platforms", plan["config"]["adopted_keys"])
        for withheld in (
            "model",
            "terminal",
            "delegation",
            "toolsets",
            "memory.provider",
        ):
            self.assertIn(withheld, plan["config"]["withheld_top_level_keys"])
        keys = {
            item["name"]: item["target_state"] for item in plan["omitted"]["tool_keys"]
        }
        self.assertEqual(
            keys["FIRECRAWL_API_KEY"], "target-provides-own-key-not-copied"
        )
        self.assertEqual(keys["FC_GITEA_TOKEN"], "not-provided-by-target-not-copied")
        self.assertEqual(
            keys["OPENROUTER_API_KEY"], "not-provided-by-target-not-copied"
        )
        self.assertEqual(keys["FINITE_PRIVATE_API_KEY"], "target-managed-not-copied")
        self.assertEqual(
            [channel["name"] for channel in plan["omitted"]["channels"]], ["signal"]
        )
        self.assertEqual(
            plan["omitted"]["google"]["target_state"],
            "out-of-scope-fresh-authorization-required",
        )

    def test_verify_rejects_a_rewritten_plan_or_a_dropped_mode(self) -> None:
        self.manifest()
        manifest_path = self.bundle / "manifest.json"
        original = json.loads(manifest_path.read_text())
        tampered = json.loads(json.dumps(original))
        tampered["fidelity"]["skills"]["user"].append({"path": "x", "name": "x"})
        manifest_path.write_text(json.dumps(tampered))
        with self.assertRaisesRegex(
            migration.MigrationError, "full-fidelity plan mismatch"
        ):
            migration.verify_bundle(self.bundle)
        dropped = dict(original)
        del dropped["mode"]
        manifest_path.write_text(json.dumps(dropped))
        with self.assertRaisesRegex(
            migration.MigrationError, "plan present without its mode"
        ):
            migration.verify_bundle(self.bundle)
        renamed = dict(original, mode="everything")
        manifest_path.write_text(json.dumps(renamed))
        with self.assertRaisesRegex(
            migration.MigrationError, "unsupported migration mode"
        ):
            migration.verify_bundle(self.bundle)

    def test_install_mode_must_match_the_approved_manifest(self) -> None:
        self.manifest()
        target = fx.build_target(self.root)
        with self.assertRaisesRegex(
            migration.MigrationError, "does not match the approved"
        ):
            self.install(target, mode="default")
        self.manifest(mode="default")
        with self.assertRaisesRegex(
            migration.MigrationError, "does not match the approved"
        ):
            self.install(target, mode="full-fidelity")
        self.assertFalse((target / "migration").exists())

    def test_cli_seals_and_installs_full_fidelity(self) -> None:
        stdout = io.StringIO()
        with redirect_stdout(stdout), redirect_stderr(io.StringIO()):
            code = migration.main(
                [
                    "manifest",
                    "--bundle",
                    str(self.bundle),
                    "--source-host-id",
                    "box1",
                    "--source-machine-id",
                    "frankie",
                    "--source-owner-email",
                    "owner@example.invalid",
                    "--source-hermes-version",
                    "0.14.0",
                    "--source-image-reference",
                    self.metadata.image_reference,
                    "--source-image-manifest-digest",
                    self.metadata.image_manifest_digest,
                    "--source-container-image-id",
                    self.metadata.container_image_id,
                    "--source-volume-inventory-sha256",
                    self.metadata.source_inventory_sha256,
                    "--mode",
                    "full-fidelity",
                ]
            )
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(stdout.getvalue())["mode"], "full-fidelity")
        target = fx.build_target(self.root)
        install = functools.partial(
            migration.install_bundle,
            session_db_factory=FakeSessionDB,
            memory_store_factory=FakeMemoryStore,
        )
        with (
            mock.patch.object(fidelity, "default_hooks", return_value=self.hooks),
            mock.patch.object(migration, "install_bundle", install),
            redirect_stdout(io.StringIO()),
        ):
            code = migration.main(
                [
                    "install",
                    "--bundle",
                    str(self.bundle),
                    "--target-root",
                    str(target),
                    "--expected-source-machine-id",
                    "frankie",
                    "--expected-manifest-sha256",
                    sha256(self.bundle / "manifest.json"),
                    "--expected-target-identity-sha256",
                    sha256(target / "agent/identity/identity.json"),
                    "--expected-target-chat-client-sha256",
                    sha256(target / "agent/client.sqlite3"),
                    "--mode",
                    "full-fidelity",
                ]
            )
        self.assertEqual(code, 0)

    # -- activations -----------------------------------------------------------

    def test_persona_settings_and_telegram_are_activated_like_agentd(self) -> None:
        target, receipt = self.installed()
        hermes = target / "agent/hermes-home"
        soul = (hermes / "SOUL.md").read_text()
        self.assertIn(fx.SOUL_MARKER, soul)
        self.assertIn("/data/workspace/legacy-box1/workspace", soul)
        self.assertIn("/data/workspace/legacy-box1/Downloads", soul)
        self.assertNotIn("/home/node", soul)
        config = yaml.safe_load((hermes / "config.yaml").read_text())
        self.assertEqual(oct((hermes / "config.yaml").stat().st_mode & 0o777), "0o600")
        telegram = config["gateway"]["platforms"]["telegram"]
        self.assertEqual(
            telegram,
            {
                "enabled": True,
                "token": fx.TOKEN,
                "reply_to_mode": "first",
                "gateway_restart_notification": False,
                "typing_indicator": True,
                "extra": {"dm_policy": "pairing", "preserve_pending_updates": True},
                "home_channel": {
                    "platform": "telegram",
                    "chat_id": str(fx.OWNER_ID),
                    "name": "Telegram",
                },
            },
        )
        # Target-owned settings stay the target's.
        self.assertEqual(config["model"]["api_key"], "${FINITE_PRIVATE_API_KEY}")
        self.assertEqual(config["terminal"]["cwd"], "/data/workspace")
        self.assertEqual(config["skills"]["external_dirs"], [fx.MANAGED_SKILLS])
        self.assertTrue(config["gateway"]["platforms"]["finitechat"]["enabled"])
        self.assertEqual(
            config["display"]["platforms"], {"finitechat": {"streaming": False}}
        )
        self.assertNotIn("finite_managed_skill_sources", config)
        self.assertNotIn("delegation", config)
        # User-facing settings come from the source.
        self.assertEqual(config["timezone"], fx.TIMEZONE)
        self.assertEqual(config["display"]["personality"], "kawaii")
        self.assertTrue(config["display"]["streaming"])
        self.assertEqual(config["agent"]["personality"], "kawaii")
        self.assertIn("kawaii", config["personalities"])
        self.assertEqual(config["session_reset"]["idle_minutes"], 1440)
        self.assertEqual(config["tts"]["provider"], "edge")
        self.assertNotIn("api_key", config["tts"]["elevenlabs"])
        self.assertTrue(config["onboarding"]["seen"])
        self.assertTrue(config["cron"]["wrap_response"])
        self.assertEqual(
            receipt["fidelity"]["installed"]["config"]["model"], "source-model-kept"
        )
        # The pre-import config and SOUL stay beside the receipt.
        backups = target / "migration/legacy-hermes-v2/preimport-state/full-fidelity"
        self.assertEqual(
            (backups / "agent__hermes-home__SOUL.md").read_text(), fx.DEFAULT_SOUL
        )
        self.assertTrue((backups / "agent__hermes-home__config.yaml").is_file())
        text = (target / "migration/legacy-hermes-v2/receipt.json").read_text()
        for secret in fx.SECRET_VALUES:
            self.assertNotIn(secret, text)

    def test_both_pairing_stores_merge_into_the_active_target_dir(self) -> None:
        target, receipt = self.installed()
        active = target / "agent/hermes-home/platforms/pairing"
        approved = json.loads((active / "telegram-approved.json").read_text())
        pending = json.loads((active / "telegram-pending.json").read_text())
        self.assertEqual(sorted(approved), [str(fx.OWNER_ID), str(fx.SECOND_ID)])
        self.assertEqual(approved[str(fx.OWNER_ID)]["user_name"], "Owner")
        self.assertEqual(
            [entry["user_id"] for entry in pending.values()], [str(fx.PENDING_ID)]
        )
        self.assertFalse((active / "_rate_limits.json").exists())
        self.assertFalse((active / "signal-approved.json").exists())
        self.assertEqual(
            oct((active / "telegram-approved.json").stat().st_mode & 0o777), "0o600"
        )
        self.assertEqual(
            receipt["fidelity"]["installed"]["pairing"],
            {
                "approved_count": 2,
                "pending_count": 1,
                "target_dir": "agent/hermes-home/platforms/pairing",
            },
        )

    def test_pairing_uses_the_legacy_dir_when_the_target_reads_it(self) -> None:
        self.manifest()
        target = fx.build_target(self.root)
        legacy = target / "agent/hermes-home/pairing"
        legacy.mkdir()
        (legacy / "_rate_limits.json").write_text("{}")
        self.install(target)
        self.assertTrue((legacy / "telegram-approved.json").is_file())
        self.assertFalse((target / "agent/hermes-home/platforms/pairing").exists())

    def test_user_skills_activate_and_baseline_clashes_stay_review_only(self) -> None:
        target, receipt = self.installed()
        skills = target / "agent/hermes-home/skills"
        skill = (skills / fx.USER_SKILL / "SKILL.md").read_text()
        self.assertIn("/data/workspace/legacy-box1/workspace/cellar.md", skill)
        self.assertTrue((skills / fx.USER_SKILL / "references/regions.md").is_file())
        self.assertFalse((skills / fx.CLASH_SKILL).exists())
        self.assertFalse((skills / fx.BUNDLED_SKILL).exists())
        self.assertFalse((skills / "devops/overlay-helper").exists())
        self.assertFalse((skills / ".archive").exists())
        review = target / "migration/legacy-hermes-v2/review-only/skills"
        self.assertTrue((review / fx.CLASH_SKILL / "SKILL.md").is_file())
        self.assertEqual(
            receipt["fidelity"]["installed"]["skills"]["review_only_clashes"],
            [
                {
                    "path": fx.CLASH_SKILL,
                    "name": "google-workspace-finite",
                    "reason": "target-managed-skills-baseline",
                }
            ],
        )

    def test_scripts_and_cron_activate_without_a_post_downtime_burst(self) -> None:
        target, receipt = self.installed()
        hermes = target / "agent/hermes-home"
        script = hermes / "scripts/check_inbox.py"
        self.assertIn(
            "/data/workspace/legacy-box1/workspace/notes.md", script.read_text()
        )
        self.assertEqual(oct(script.stat().st_mode & 0o777), "0o755")
        document = json.loads((hermes / "cron/jobs.json").read_text())
        jobs = {job["id"]: job for job in document["jobs"]}
        self.assertEqual(
            list(jobs),
            [job["id"] for job in receipt["fidelity"]["plan"]["cron"]["jobs"]],
        )
        for job_id in ("daily-briefing", "inbox-check", "openrouter-job"):
            next_run = datetime.fromisoformat(jobs[job_id]["next_run_at"])
            self.assertGreater(next_run, self.now, job_id)
            self.assertTrue(jobs[job_id]["enabled"], job_id)
        # 08:00 America/Chicago is 14:00 UTC; the next one after 18:00 UTC is tomorrow.
        self.assertEqual(
            datetime.fromisoformat(jobs["daily-briefing"]["next_run_at"]),
            datetime(2026, 11, 14, 14, 0, tzinfo=timezone.utc),
        )
        self.assertEqual(
            datetime.fromisoformat(jobs["inbox-check"]["next_run_at"]),
            self.now + timedelta(minutes=30),
        )
        self.assertNotIn("run_claim", jobs["daily-briefing"])
        self.assertEqual(
            jobs["inbox-check"]["script"],
            "/data/agent/hermes-home/scripts/check_inbox.py",
        )
        self.assertEqual(
            jobs["inbox-check"]["workdir"], "/data/workspace/legacy-box1/dev/project"
        )
        self.assertIn(
            "/data/workspace/legacy-box1/workspace/notes.md",
            jobs["daily-briefing"]["prompt"],
        )
        self.assertEqual(jobs["daily-briefing"]["origin"]["chat_id"], str(fx.OWNER_ID))
        self.assertEqual(
            jobs["future-reminder"]["next_run_at"],
            jobs["future-reminder"]["schedule"]["run_at"],
        )
        self.assertTrue(jobs["future-reminder"]["enabled"])
        self.assertFalse(jobs["missed-reminder"]["enabled"])
        self.assertEqual(
            jobs["missed-reminder"]["paused_reason"], fidelity.ONESHOT_MISSED_REASON
        )
        self.assertFalse(jobs["matrix-relay"]["enabled"])
        self.assertEqual(
            jobs["matrix-relay"]["paused_reason"], "delivery-platform-inactive: matrix"
        )
        self.assertFalse(jobs["paused-digest"]["enabled"])
        self.assertEqual(jobs["paused-digest"]["paused_reason"], "owner paused")
        self.assertIsNone(jobs["openrouter-job"]["provider"])
        self.assertIsNone(jobs["openrouter-job"]["base_url"])
        self.assertIsNone(jobs["openrouter-job"]["model"])
        outcomes = receipt["fidelity"]["installed"]["cron"]["outcomes"]
        self.assertEqual(
            outcomes["paused:oneshot-missed-during-cutover"], ["missed-reminder"]
        )
        self.assertEqual(
            outcomes["paused:delivery-platform-inactive"], ["matrix-relay"]
        )
        self.assertEqual(outcomes["kept-disabled"], ["paused-digest"])
        self.assertEqual(
            outcomes["recurring-next-run-recomputed+inference-route-normalized"],
            ["openrouter-job"],
        )

    def test_home_files_land_where_rewritten_paths_point(self) -> None:
        target, receipt = self.installed()
        legacy = target / "workspace/legacy-box1"
        self.assertEqual(
            (legacy / "poster_final.jpg").read_bytes()[:4], b"\xff\xd8\xff\xe0"
        )
        self.assertEqual(
            (legacy / "make_post.py").read_text(),
            (self.root / "source-home/make_post.py").read_text(),
        )
        self.assertEqual(oct((legacy / "make_post.py").stat().st_mode & 0o777), "0o755")
        self.assertEqual(
            (legacy / "Downloads/memo.txt").read_text(), "memo from Downloads\n"
        )
        self.assertEqual((legacy / "recipes/report.csv").read_text(), "a,b\n1,2\n")
        self.assertEqual(str((legacy / "lessons/latest").readlink()), "week1.md")
        self.assertEqual(
            str((legacy / "lessons/workspace-link").readlink()),
            "/data/workspace/legacy-box1/workspace/notes.md",
        )
        self.assertFalse((legacy / "lessons/hermes-cache-link").is_symlink())
        for withheld in (
            "AGENTS.md",
            "node_modules",
            "__pycache__",
            "finitebrain",
            ".finite",
            ".hermes",
        ):
            self.assertFalse((legacy / withheld).exists(), withheld)
        review = target / "migration/legacy-hermes-v2/review-only/agents"
        self.assertTrue((review / "home-AGENTS.md").is_file())
        self.assertTrue((review / "hermes-AGENTS.md").is_file())
        self.assertFalse((target / "workspace/AGENTS.md").exists())
        home = receipt["fidelity"]["installed"]["home"]
        self.assertEqual(home["skipped_symlink_count"], 1)
        memory = (target / "agent/hermes-home/memories/MEMORY.md").read_text()
        self.assertIn("/data/workspace/legacy-box1/workspace/trips.md", memory)
        self.assertIn(fx.MEMORY_MARKER, memory)
        session = FakeSessionDB.imported[0]
        self.assertEqual(session["cwd"], "/data/workspace/legacy-box1")
        rendered = json.dumps(session["messages"])
        for expected in (
            "/data/workspace/legacy-box1/poster_final.jpg",
            "/data/workspace/legacy-box1/Downloads/memo.txt",
            "MEDIA: /data/workspace/legacy-box1/uploads/photo.txt",
            "/data/workspace/legacy-box1/uploads/photo.txt",
            "/home/node/.hermes/audio_cache/voice.ogg",
        ):
            self.assertIn(expected, rendered)
        counts = receipt["fidelity"]["installed"]["session_path_rewrites"]
        self.assertEqual(counts, {"rewritten": 5, "unmapped_left": 1})

    def test_default_mode_bundle_installs_exactly_as_before(self) -> None:
        self.manifest(mode="default")
        target = fx.build_target(self.root)
        before = self.tree(target / "agent/hermes-home")
        receipt = self.install(target, mode="default")
        self.assertNotIn("fidelity", receipt)
        after = self.tree(target / "agent/hermes-home")
        self.assertEqual(
            {
                key: value
                for key, value in after.items()
                if not key.startswith(("memories", "state.db", "memory_store.db"))
            },
            {
                key: value
                for key, value in before.items()
                if not key.startswith(("state.db", "memory_store.db"))
            },
        )
        self.assertFalse((target / "workspace/legacy-box1/Downloads").exists())

    # -- fail closed -----------------------------------------------------------

    def assert_fails_without_mutation(
        self, target: Path, message: str, hooks=None
    ) -> None:
        before = self.tree(target)
        with self.assertRaisesRegex(migration.MigrationError, message):
            self.install(target, hooks=hooks)
        self.assertEqual(self.tree(target), before)

    def test_customized_target_soul_fails_closed(self) -> None:
        self.manifest()
        target = fx.build_target(self.root)
        (target / "agent/hermes-home/SOUL.md").write_text(
            "A persona someone wrote on the target\n"
        )
        self.assert_fails_without_mutation(target, "target SOUL.md was customized")

    def test_existing_target_telegram_connection_fails_closed(self) -> None:
        self.manifest()
        config = fx.build_target(self.root) / "agent/hermes-home/config.yaml"
        data = yaml.safe_load(config.read_text())
        data["gateway"]["platforms"]["telegram"] = {
            "enabled": True,
            "token": "1:" + "x" * 30,
        }
        config.write_text(yaml.safe_dump(data))
        self.assert_fails_without_mutation(
            config.parents[2], "already has a Telegram connection"
        )

    def test_non_empty_target_pairing_store_fails_closed(self) -> None:
        self.manifest()
        target = fx.build_target(self.root)
        store = target / "agent/hermes-home/platforms/pairing/telegram-approved.json"
        store.parent.mkdir(parents=True)
        store.write_text(json.dumps({"999": {"user_name": "x"}}))
        self.assert_fails_without_mutation(target, "pairing store is not empty")

    def test_target_with_scheduled_jobs_fails_closed(self) -> None:
        self.manifest()
        target = fx.build_target(self.root)
        jobs = target / "agent/hermes-home/cron/jobs.json"
        jobs.parent.mkdir(parents=True)
        jobs.write_text(json.dumps({"jobs": [{"id": "target-job"}]}))
        self.assert_fails_without_mutation(target, "already has scheduled jobs")

    def test_home_skill_and_script_collisions_fail_closed(self) -> None:
        self.manifest()
        for collision, message in (
            ("workspace/legacy-box1/Downloads", "legacy-box1/Downloads"),
            ("agent/hermes-home/skills/" + fx.USER_SKILL, "skills/" + fx.USER_SKILL),
            ("agent/hermes-home/scripts/check_inbox.py", "scripts/check_inbox.py"),
        ):
            with self.subTest(collision=collision):
                target = fx.build_target(Path(tempfile.mkdtemp(dir=self.root)))
                (target / collision).mkdir(parents=True)
                self.assert_fails_without_mutation(
                    target, "destination collision: .*" + message
                )

    def test_missing_target_config_or_pending_update_patch_fails_closed(self) -> None:
        self.manifest()
        target = fx.build_target(self.root)
        (target / "agent/hermes-home/config.yaml").unlink()
        self.assert_fails_without_mutation(target, "config.yaml is missing")
        target = fx.build_target(Path(tempfile.mkdtemp(dir=self.root)))
        hooks = fidelity.FidelityHooks(
            is_default_soul=self.hooks.is_default_soul,
            bundled_skill_names=self.hooks.bundled_skill_names,
            telegram_preserves_pending_updates=lambda: False,
            now=self.hooks.now,
        )
        self.assert_fails_without_mutation(
            target, "drops queued Telegram updates", hooks=hooks
        )

    def test_late_failure_restores_config_soul_and_removes_placed_files(self) -> None:
        self.manifest()
        target = fx.build_target(self.root)
        before = self.tree(target)
        with (
            mock.patch(
                "legacy_hermes_target._write_private_json", side_effect=OSError("disk")
            ),
            self.assertRaises(OSError),
        ):
            self.install(target)
        after = self.tree(target)
        # Like the default mode, only the empty parent of the receipt dir remains.
        self.assertEqual(after.pop("migration"), "dir")
        self.assertEqual(after, before)

    def test_source_telegram_shapes_that_agentd_cannot_represent_fail_closed(
        self,
    ) -> None:
        cases = (
            ({"TELEGRAM_BOT_TOKEN": "not-a-token"}, {}, "Bot API shape"),
            (
                {"TELEGRAM_BOT_TOKEN": fx.TOKEN},
                {"telegram": {"token": "1:" + "y" * 35}},
                "token sources disagree",
            ),
            (
                {"TELEGRAM_BOT_TOKEN": fx.TOKEN, "TELEGRAM_HOME_CHANNEL": "-1001234"},
                {},
                "not a numeric private chat id",
            ),
        )
        for env, config, message in cases:
            with (
                self.subTest(message=message),
                self.assertRaisesRegex(migration.MigrationError, message),
            ):
                fidelity.telegram_source(config, env)
        message = str(self._token_error())
        self.assertNotIn("not-a-token", message)

    def _token_error(self) -> Exception:
        try:
            fidelity.telegram_source({}, {"TELEGRAM_BOT_TOKEN": "not-a-token"})
        except migration.MigrationError as exc:
            return exc
        raise AssertionError("expected a token error")

    def test_config_yaml_token_and_reference_are_accepted(self) -> None:
        token, home, enabled, evidence = fidelity.telegram_source(
            {
                "telegram": {"token": "${LAZ_TOKEN}", "enabled": True},
                "platforms": {"telegram": {"enabled": True}},
            },
            {"LAZ_TOKEN": fx.TOKEN},
        )
        self.assertEqual((token, home, enabled), (fx.TOKEN, None, True))
        self.assertEqual(evidence["token_sources"], ["config:telegram.token"])
        _, _, enabled, _ = fidelity.telegram_source(
            {"platforms": {"telegram": {"enabled": False}}},
            {"TELEGRAM_BOT_TOKEN": fx.TOKEN},
        )
        self.assertFalse(enabled)

    def test_path_map_rewrites_only_placed_and_active_roots(self) -> None:
        path_map = fidelity.PathMap(["workspace", "Downloads", "poster.jpg"])
        cases = {
            "/home/node": "/data/workspace/legacy-box1",
            "/home/node/workspace/a b.md": "/data/workspace/legacy-box1/workspace/a b.md",
            "see ~/Downloads/x.pdf.": "see /data/workspace/legacy-box1/Downloads/x.pdf.",
            "/home/node/poster.jpg,": "/data/workspace/legacy-box1/poster.jpg,",
            "/home/node/.hermes/skills/x/SKILL.md": "/data/agent/hermes-home/skills/x/SKILL.md",
            "/home/node/.hermes/SOUL.md": "/data/agent/hermes-home/SOUL.md",
            "/home/node/.hermes/venv/bin/python": "/home/node/.hermes/venv/bin/python",
            "/home/node/.ssh/id_ed25519": "/home/node/.ssh/id_ed25519",
            "/home/nodejs/x": "/home/nodejs/x",
            "x/home/node/workspace": "x/home/node/workspace",
            "approx~/workspace": "approx~/workspace",
        }
        for source, expected in cases.items():
            with self.subTest(source=source):
                self.assertEqual(path_map.rewrite_text(source), expected)


if __name__ == "__main__":
    unittest.main()

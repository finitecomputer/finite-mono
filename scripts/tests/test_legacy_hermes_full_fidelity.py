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
import os
import sqlite3
import subprocess
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
    routes: list[dict] = []
    activity: list[tuple[str, float]] = []

    def __init__(self, db_path: Path):
        self.db_path = db_path

    def import_sessions(self, sessions: list[dict]) -> dict:
        FakeSessionDB.imported.extend(sessions)
        return {"ok": True, "imported": len(sessions), "skipped": 0, "detached": 0}

    def record_gateway_session_peer(self, session_id: str, **fields) -> None:
        FakeSessionDB.routes.append({"session_id": session_id, **fields})

    def touch_session_activity(self, session_id: str, ts: float) -> None:
        FakeSessionDB.activity.append((session_id, ts))

    def clear_session_activity_labels(self, session_id: str) -> None:
        return None

    def close(self) -> None:
        return None


class FakeMemoryStore:
    def __init__(self, db_path: Path):
        self.connection = sqlite3.connect(db_path)

    def list_facts(self, limit: int = 50) -> list[dict]:
        rows = self.connection.execute(
            "SELECT fact_id, content FROM facts LIMIT ?", (limit,)
        ).fetchall()
        return [{"fact_id": row[0], "content": row[1]} for row in rows]

    def update_fact(self, fact_id: int, content: str) -> bool:
        self.connection.execute(
            "UPDATE facts SET content = ? WHERE fact_id = ?", (content, fact_id)
        )
        self.connection.commit()
        return True

    def rebuild_all_vectors(self) -> int:
        return 0

    def close(self) -> None:
        self.connection.close()


QUALIFIED = {
    "qualified": True,
    "reasons": [],
    "fingerprint": "f" * 64,
    "capability": "finite.telegram.preserve-pending/2",
    "hermes_version": "0.21.0",
    "adapter_path": "/nix/store/x-hermes-agent/share/hermes-agent/plugins/platforms/telegram/adapter.py",
    "adapter_sha256": "a" * 64,
}


def run_hermes(hermes_home: Path, scratch: Path, code: str) -> str:
    """Run target Hermes code in a child with scratch HOME and HERMES_HOME."""
    completed = subprocess.run(
        [sys.executable, "-B", "-c", code],
        env={
            "HOME": str(scratch),
            "HERMES_HOME": str(hermes_home),
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "TZ": "UTC",
        },
        cwd=scratch,
        capture_output=True,
        text=True,
        timeout=120,
    )
    if completed.returncode != 0:
        raise AssertionError(completed.stderr[-3000:])
    return completed.stdout


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
        FakeSessionDB.routes = []
        FakeSessionDB.activity = []
        self.scratch_home = self.root / "scratch-home"
        self.scratch_home.mkdir()
        self.hooks = self.make_hooks()

    def make_hooks(self, **overrides) -> fidelity.FidelityHooks:
        values = dict(
            is_default_soul=lambda text: text == fx.DEFAULT_SOUL,
            bundled_skill_names=lambda: {
                "apple-notes": PurePosixPath("apple/apple-notes")
            },
            telegram_qualification=lambda: dict(QUALIFIED),
            now=lambda: self.now,
        )
        values.update(overrides)
        return fidelity.FidelityHooks(**values)

    def rebuild(self, now: datetime, **bundle_options) -> None:
        """Rebuild the source and bundle at another time (real-clock checks)."""
        self.now = now
        root = Path(tempfile.mkdtemp(dir=self.root))
        evidence = fx.build_bundle(root, migration, now=now, **bundle_options)
        self.bundle = evidence["bundle"]
        self.metadata = fx.source_metadata(migration, evidence["inventory_sha256"])
        self.root = root

    def manifest(self, mode: str = "full-fidelity") -> dict:
        return migration.create_manifest(self.bundle, self.metadata, mode=mode)

    def install(
        self,
        target: Path,
        *,
        mode: str = "full-fidelity",
        hooks=None,
        telegram: str = "live",
        fingerprint: str | None = QUALIFIED["fingerprint"],
    ) -> dict:
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
            telegram_mode=telegram,
            expected_telegram_fingerprint=fingerprint,
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
        self.assertEqual(plan["skills"]["layout"]["status"], "confirmed")
        self.assertEqual(
            [(item["path"], item["reason"]) for item in plan["skills"]["review_only"]],
            [("devops/edited-helper", "box1-managed-skill-modified-in-source")],
        )
        agents = plan["persona"]["agents_md"]
        self.assertTrue(agents["seed"]["staged"])
        self.assertEqual(
            {item["path"]: item["status"] for item in agents["files"]},
            {
                "AGENTS.md": "owner-additions-activated",
                ".hermes/AGENTS.md": "box1-seed-excluded",
            },
        )
        self.assertEqual(agents["target"], "/data/workspace/AGENTS.md")
        self.assertEqual(
            plan["routing"]["entries_by_platform"], {"matrix": 1, "telegram": 1}
        )
        hermes_home = {
            item["name"]: item["disposition"] for item in plan["omitted"]["hermes_home"]
        }
        for left_out in ("plugins", "hooks", "bin", "venv", "logs", "audio_cache"):
            self.assertEqual(hermes_home[left_out], "sealed-not-activated", left_out)
        self.assertEqual(hermes_home["SOUL.md"], "activated")
        self.assertEqual(
            {item["name"] for item in plan["omitted"]["hidden_home_entries"]},
            {".cache", ".codex", ".finite", ".npm"},
        )
        for name in ("telegram", "platforms", "gateway"):
            self.assertNotIn(name, plan["config"]["adopted_keys"])
        self.assertEqual(
            plan["home"]["placed"],
            ["Downloads", "lessons", "make_post.py", "poster_final.jpg", "recipes"],
        )
        self.assertEqual(
            {item["name"]: item["reason"] for item in plan["home"]["withheld"]},
            {
                "AGENTS.md": "box1-platform-seed-handled-by-persona-plan",
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
        out, err = io.StringIO(), io.StringIO()
        with (
            mock.patch.object(fidelity, "default_hooks", return_value=self.hooks),
            mock.patch.object(migration, "install_bundle", install),
            redirect_stdout(out),
            redirect_stderr(err),
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
                    "--telegram",
                    "live",
                    "--expected-telegram-runtime-fingerprint",
                    QUALIFIED["fingerprint"],
                ]
            )
        self.assertEqual(code, 0, err.getvalue())
        receipt = json.loads(out.getvalue())
        self.assertEqual(receipt["fidelity"]["installed"]["telegram"]["mode"], "live")
        for secret in fx.SECRET_VALUES:
            self.assertNotIn(secret, stdout.getvalue())
            self.assertNotIn(secret, out.getvalue())
            self.assertNotIn(secret, err.getvalue())

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
        self.assertEqual(config["tts"]["elevenlabs"]["voice_id"], "synthetic-voice")
        self.assertEqual(config["human_delay"]["max_ms"], 2500)
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
        # The daily slot is eight hours after `now` in the bot's timezone.
        self.assertEqual(
            datetime.fromisoformat(jobs["daily-briefing"]["next_run_at"]),
            self.now + timedelta(hours=8),
        )
        self.assertEqual(
            datetime.fromisoformat(jobs["inbox-check"]["next_run_at"]),
            self.now + timedelta(minutes=fx.INTERVAL_MINUTES),
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
            outcomes["kept-completed:repeat-exhausted"], ["exhausted-interval"]
        )
        self.assertEqual(outcomes["kept-completed"], ["completed-oneshot"])
        self.assertEqual(
            outcomes["recurring-error-state-rescheduled"], ["error-state-daily"]
        )
        self.assertEqual(
            outcomes["oneshot-scheduled"], ["claimed-oneshot", "future-reminder"]
        )
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
        # Activated user files point at the new home; the sealed snapshot keeps
        # the original bytes.
        self.assertEqual(
            (legacy / "make_post.py").read_text(),
            "OUT = '/data/workspace/legacy-box1/workspace/post.png'\nprint(OUT)\n",
        )
        self.assertEqual(
            (target / "workspace/legacy-box1/workspace/plan.md").read_text(),
            "Final memo: /data/workspace/legacy-box1/Downloads/memo.txt\n",
        )
        self.assertEqual(
            (legacy / "Downloads/cache.bin").read_bytes(),
            (self.root / "source-home/Downloads/cache.bin").read_bytes(),
        )
        installed = receipt["fidelity"]["installed"]
        self.assertIn(
            {"path": "workspace/legacy-box1/Downloads/cache.bin", "reason": "binary"},
            installed["unrewritable_references"],
        )
        self.assertGreaterEqual(
            installed["workspace_path_rewrites"]["files_rewritten"], 1
        )
        with sqlite3.connect(
            target / "agent/hermes-home/memory_store.db"
        ) as connection:
            fact = connection.execute("SELECT content FROM facts").fetchone()[0]
        self.assertIn("/data/workspace/legacy-box1/workspace/notes.md", fact)
        self.assertNotIn("/home/node", fact)
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
        # Only the owner's addition is activated, as project context.
        agents = (target / "workspace/AGENTS.md").read_text()
        self.assertIn(fx.AGENTS_OWNER_ADDITION, agents)
        self.assertNotIn("finitec publish", agents)
        home = receipt["fidelity"]["installed"]["home"]
        self.assertEqual(home["skipped_symlink_count"], 1)
        memory = (target / "agent/hermes-home/memories/MEMORY.md").read_text()
        self.assertIn("/data/workspace/legacy-box1/workspace/trips.md", memory)
        self.assertIn(fx.MEMORY_MARKER, memory)
        session = next(
            item
            for item in FakeSessionDB.imported
            if item["id"] == "legacy-telegram-session"
        )
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
        self.assertEqual(
            {key: counts[key] for key in ("rewritten", "unmapped_left")},
            {"rewritten": 6, "unmapped_left": 1},
        )

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
        hooks = self.make_hooks(
            telegram_qualification=lambda: {
                "qualified": False,
                "reasons": ["conflict-retry-drops-queue"],
            }
        )
        self.assert_fails_without_mutation(
            target, "drops queued Telegram updates.*conflict-retry", hooks=hooks
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

    # -- round 2: rehearsal, gate, routing, cron state, paths, persona -------

    def test_rehearsal_is_the_default_and_never_writes_the_token(self) -> None:
        self.manifest()
        target = fx.build_target(self.root)
        receipt = migration.install_bundle(
            self.bundle,
            target,
            expected_machine_id="frankie",
            expected_manifest_sha256=sha256(self.bundle / "manifest.json"),
            expected_identity_sha256=sha256(target / "agent/identity/identity.json"),
            expected_chat_client_sha256=sha256(target / "agent/client.sqlite3"),
            session_db_factory=FakeSessionDB,
            memory_store_factory=FakeMemoryStore,
            mode="full-fidelity",
            fidelity_hooks=self.hooks,
        )
        config_text = (target / "agent/hermes-home/config.yaml").read_text()
        self.assertNotIn(fx.TOKEN, config_text)
        telegram = yaml.safe_load(config_text)["gateway"]["platforms"]["telegram"]
        self.assertIs(telegram["enabled"], False)
        self.assertNotIn("token", telegram)
        self.assertIs(telegram["extra"]["finite_legacy_rehearsal"], True)
        self.assertEqual(telegram["extra"]["dm_policy"], "pairing")
        installed = receipt["fidelity"]["installed"]
        self.assertEqual(installed["telegram"]["mode"], "rehearsal")
        self.assertEqual(
            installed["config"]["telegram"],
            "rehearsal-disabled-no-token-with-home-channel",
        )
        # Everything else is the same as a live install: pairing and jobs.
        self.assertEqual(installed["pairing"]["approved_count"], 2)
        self.assertTrue((target / "agent/hermes-home/cron/jobs.json").is_file())
        for argv_default in (
            migration._parser()
            .parse_args(
                [
                    "install",
                    "--bundle",
                    "b",
                    "--target-root",
                    "t",
                    "--expected-source-machine-id",
                    "m",
                    "--expected-manifest-sha256",
                    "0" * 64,
                    "--expected-target-identity-sha256",
                    "0" * 64,
                    "--expected-target-chat-client-sha256",
                    "0" * 64,
                ]
            )
            .telegram,
        ):
            self.assertEqual(argv_default, "rehearsal")

    def test_live_telegram_needs_the_qualified_runtime_fingerprint(self) -> None:
        self.manifest()
        target = fx.build_target(self.root)
        before = self.tree(target)
        with self.assertRaisesRegex(
            migration.MigrationError, "needs --expected-telegram"
        ):
            self.install(target, telegram="live", fingerprint=None)
        with self.assertRaisesRegex(
            migration.MigrationError, "not the qualified Runtime"
        ):
            self.install(target, telegram="live", fingerprint="0" * 64)
        with self.assertRaisesRegex(
            migration.MigrationError, "not the qualified Runtime"
        ):
            self.install(target, telegram="rehearsal", fingerprint="0" * 64)
        self.assertEqual(self.tree(target), before)
        receipt = self.install(target)
        self.assertEqual(
            receipt["fidelity"]["installed"]["telegram"]["qualification"][
                "fingerprint"
            ],
            QUALIFIED["fingerprint"],
        )

    def test_active_telegram_chat_is_rebound_to_its_session(self) -> None:
        _, receipt = self.installed()
        self.assertEqual(len(FakeSessionDB.routes), 1)
        route = FakeSessionDB.routes[0]
        self.assertEqual(route["session_id"], fx.ACTIVE_SESSION_ID)
        self.assertEqual(route["session_key"], f"agent:main:telegram:dm:{fx.OWNER_ID}")
        self.assertEqual(
            (route["source"], route["chat_id"], route["chat_type"], route["user_id"]),
            ("telegram", str(fx.OWNER_ID), "dm", str(fx.OWNER_ID)),
        )
        origin = json.loads(route["origin_json"])
        self.assertEqual(origin["platform"], "telegram")
        for claim in (
            "active_turn_token",
            "resume_pending",
            "suspended",
            "model_override",
        ):
            self.assertNotIn(claim, json.dumps(route))
        # Activity is the source conversation's real last message, not install time.
        session = next(
            s for s in fx.sessions(self.now) if s["id"] == fx.ACTIVE_SESSION_ID
        )
        self.assertEqual(
            FakeSessionDB.activity,
            [(fx.ACTIVE_SESSION_ID, max(m["timestamp"] for m in session["messages"]))],
        )
        routing = receipt["fidelity"]["installed"]["routing"]
        self.assertEqual(routing["mapped_session_ids"], [fx.ACTIVE_SESSION_ID])
        self.assertEqual(routing["not_mapped"], {"platform-not-active:matrix": 1})

    def test_ambiguous_telegram_routing_fails_closed(self) -> None:
        home = self.root / "source-home/.hermes/sessions/sessions.json"
        index = json.loads(home.read_text())
        owner = dict(index[f"agent:main:telegram:dm:{fx.OWNER_ID}"])
        # An older key for the same chat that points at another session.
        index["agent:main:telegram:dm"] = {
            **owner,
            "session_key": "agent:main:telegram:dm",
            "session_id": "legacy-telegram-session",
        }
        self.rewrite_source_file(".hermes/sessions/sessions.json", json.dumps(index))
        self.manifest()
        target = fx.build_target(self.root)
        self.assert_fails_without_mutation(target, "ambiguous legacy Telegram routing")

    def rewrite_source_file(self, relative: str, text: str) -> None:
        """Change one source file and rebuild the sealed snapshot and inventory."""
        import tarfile

        home = self.root / "source-home"
        (home / relative).write_text(text)
        for name in ("source-volume-inventory.json", "integrations.json", "sites.json"):
            (self.bundle / name).unlink()
        inventory = self.bundle / "source-volume-inventory.json"
        migration.inventory_source_volume(inventory, home)
        migration.inventory_source_integrations(
            self.bundle / "integrations.json", home, inventory
        )
        endpoints = self.root / "published-endpoints.json"
        migration.inventory_source_sites(
            self.bundle / "sites.json",
            endpoints,
            inventory,
            expected_machine_id="frankie",
        )
        snapshot = self.bundle / "payload/source-home.tar"
        snapshot.unlink()
        with tarfile.open(snapshot, "w") as archive:
            archive.add(home, arcname=".", recursive=True)
        snapshot.chmod(0o600)
        self.metadata = fx.source_metadata(
            migration, hashlib.sha256(inventory.read_bytes()).hexdigest()
        )

    def test_box1_scheduler_state_cannot_cause_a_burst_in_target_hermes(self) -> None:
        # Real clock: the target's own get_due_jobs() decides what is due now.
        self.rebuild(datetime.now(timezone.utc).replace(microsecond=0))
        self.hooks = self.make_hooks(now=lambda: datetime.now(timezone.utc))
        target, receipt = self.installed()
        hermes = target / "agent/hermes-home"
        jobs = {
            job["id"]: job
            for job in json.loads((hermes / "cron/jobs.json").read_text())["jobs"]
        }
        for job in jobs.values():
            for key in fidelity.CRON_EXECUTION_FIELDS_REMOVED:
                self.assertNotIn(key, job, (job["id"], key))
            self.assertIsNone(job.get("last_status"), job["id"])
        self.assertFalse(jobs["exhausted-interval"]["enabled"])
        self.assertEqual(jobs["exhausted-interval"]["state"], "completed")
        self.assertEqual(jobs["completed-oneshot"]["state"], "completed")
        self.assertEqual(jobs["error-state-daily"]["state"], "scheduled")
        self.assertTrue(jobs["claimed-oneshot"]["enabled"])
        legacy = receipt["fidelity"]["installed"]["cron"]["legacy_execution"]
        self.assertEqual(legacy["errored-poll"]["last_status"], "error")
        self.assertTrue(legacy["errored-poll"]["had_last_error"])
        self.assertEqual(legacy["errored-poll"]["failure_streak"], 3)
        self.assertEqual(
            legacy["claimed-oneshot"]["cleared"],
            ["fire_claim", "manual_run_at", "run_claim"],
        )
        self.assertEqual(
            legacy["exhausted-interval"]["repeat"], {"times": 3, "completed": 3}
        )
        self.assertNotIn("synthetic old failure", json.dumps(receipt))
        due = run_hermes(
            hermes,
            self.scratch_home,
            "import json; from cron.jobs import get_due_jobs; "
            "print(json.dumps([j['id'] for j in get_due_jobs()]))",
        )
        self.assertEqual(json.loads(due.strip().splitlines()[-1]), [])
        self.assertFalse((hermes / "cron/persisted_error_recoveries.jsonl").exists())

    def test_empty_target_cron_store_is_replaced_and_restored_on_failure(self) -> None:
        self.manifest()
        target = fx.build_target(self.root)
        jobs = target / "agent/hermes-home/cron/jobs.json"
        jobs.parent.mkdir()
        empty = b'{"jobs": [], "updated_at": null}\n'
        jobs.write_bytes(empty)
        before = self.tree(target)
        with (
            mock.patch(
                "legacy_hermes_target._write_private_json", side_effect=OSError("disk")
            ),
            self.assertRaises(OSError),
        ):
            self.install(target)
        after = self.tree(target)
        self.assertEqual(after.pop("migration"), "dir")
        self.assertEqual(after, before)
        self.assertEqual(jobs.read_bytes(), empty)
        receipt = self.install(target)
        self.assertTrue(
            receipt["fidelity"]["installed"]["cron"]["replaced_empty_target_store"]
        )
        self.assertTrue(json.loads(jobs.read_text())["jobs"])
        backup = (
            target
            / "migration/legacy-hermes-v2/preimport-state/full-fidelity"
            / "agent__hermes-home__cron__jobs.json"
        )
        self.assertEqual(backup.read_bytes(), empty)

    def test_agents_md_activates_owner_additions_only(self) -> None:
        cases = (
            # (home AGENTS.md text, stage seed, expected home status)
            (fx.AGENTS_SEED, True, "box1-seed-excluded"),
            (
                "My intro\n" + fx.AGENTS_SEED + "\n" + fx.AGENTS_OWNER_ADDITION,
                True,
                "owner-additions-activated",
            ),
            (
                fx.AGENTS_SEED.replace("Traefik", "Caddy") + fx.AGENTS_OWNER_ADDITION,
                True,
                "unknown-variant-review-only",
            ),
            (
                fx.AGENTS_SEED + fx.AGENTS_OWNER_ADDITION,
                False,
                "seed-not-staged-review-only",
            ),
        )
        for text, staged, status in cases:
            with self.subTest(status=status, staged=staged):
                if not staged:
                    (self.bundle / "box1-seed/AGENTS.md").unlink()
                self.rewrite_source_file("AGENTS.md", text)
                plan = self.manifest()["fidelity"]["persona"]["agents_md"]
                files = {item["path"]: item["status"] for item in plan["files"]}
                self.assertEqual(files["AGENTS.md"], status)
                target = fx.build_target(Path(tempfile.mkdtemp(dir=self.root)))
                receipt = self.install(target)
                activated = target / "workspace/AGENTS.md"
                self.assertEqual(
                    activated.exists(), status == "owner-additions-activated"
                )
                if activated.exists():
                    body = activated.read_text()
                    self.assertIn("My intro", body)
                    self.assertIn(fx.AGENTS_OWNER_ADDITION, body)
                    self.assertNotIn("finitec publish", body)
                self.assertEqual(
                    receipt["fidelity"]["installed"]["agents_md"]["files"]["AGENTS.md"],
                    status,
                )

    def test_existing_target_agents_md_fails_closed(self) -> None:
        self.manifest()
        target = fx.build_target(self.root)
        (target / "workspace/AGENTS.md").write_text("target context\n")
        self.assert_fails_without_mutation(target, "workspace/AGENTS.md already exists")

    def test_every_box1_telegram_config_location_is_read(self) -> None:
        token, home, enabled, evidence = fidelity.telegram_source(
            {
                "gateway": {
                    "telegram": {"token": "${LAZ}", "home_channel": {"chat_id": 55}}
                },
                "platforms": {"telegram": True, "signal": {"enabled": True}},
                "telegram": {"enabled": True},
                "unauthorized_dm_behavior": "pair",
            },
            {"LAZ": fx.TOKEN},
        )
        self.assertEqual((token, home, enabled), (fx.TOKEN, "55", True))
        self.assertEqual(evidence["token_sources"], ["config:gateway.telegram.token"])
        self.assertEqual(
            evidence["home_channel_sources"], ["config:gateway.telegram.home_channel"]
        )
        failing = (
            (
                {"gateway": {"telegram": {"token": "${MISSING}"}}},
                {},
                "no token resolves",
            ),
            ({"telegram": {"enabled": True}}, {}, "no token resolves"),
            ({"platforms": {"telegram": "true"}}, {}, "no token resolves"),
            (
                {"gateway": {"telegram": {"token": "2:" + "z" * 35}}},
                {"TELEGRAM_BOT_TOKEN": fx.TOKEN},
                "token sources disagree",
            ),
            (
                {
                    "telegram": {"home_channel": 9},
                    "gateway": {"telegram": {"home_channel": 8}},
                },
                {"TELEGRAM_BOT_TOKEN": fx.TOKEN},
                "home channel sources disagree",
            ),
        )
        for config, env, message in failing:
            with self.subTest(message=message, config=config):
                with self.assertRaisesRegex(migration.MigrationError, message):
                    fidelity.telegram_source(config, env)
        plan = fidelity._plan_config(
            {
                "telegram": {"enabled": True},
                "platforms": {},
                "unauthorized_dm_behavior": "x",
            }
        )
        for key in ("telegram", "platforms", "unauthorized_dm_behavior"):
            self.assertIn(key, plan["withheld_top_level_keys"])

    def test_missing_skill_layout_is_visible_and_activates_nothing(self) -> None:
        import shutil

        skills = self.bundle / "payload/hermes/skills"
        (skills / ".bundled_manifest").unlink()
        shutil.rmtree(self.root / "source-home/.hermes/.runtime-managed")
        (self.root / "source-home/.hermes/skills/.bundled_manifest").unlink()
        self.rewrite_source_file("make_post.py", "print('x')\n")
        plan = self.manifest()["fidelity"]["skills"]
        self.assertEqual(
            plan["layout"]["status"], "unconfirmed-every-skill-review-only"
        )
        self.assertEqual(plan["user"], [])
        self.assertIn(
            (fx.USER_SKILL, "skill-layout-unconfirmed"),
            [(item["path"], item["reason"]) for item in plan["review_only"]],
        )
        target = fx.build_target(self.root)
        receipt = self.install(target)
        skills_receipt = receipt["fidelity"]["installed"]["skills"]
        self.assertEqual(skills_receipt["activated"], [])
        self.assertEqual(
            skills_receipt["layout"]["status"], "unconfirmed-every-skill-review-only"
        )

    def test_adopted_settings_that_target_validation_rejects_fail_closed(self) -> None:
        self.manifest()
        target = fx.build_target(self.root)

        def validate(config: dict) -> list[dict]:
            if config.get("timezone") == fx.TIMEZONE:
                return [{"severity": "error", "message": "synthetic rejection"}]
            return [{"severity": "warning", "message": "pre-existing"}]

        self.assert_fails_without_mutation(
            target,
            "fail target Hermes config validation: synthetic rejection",
            hooks=self.make_hooks(validate_config=validate),
        )
        # The real target validator accepts the adopted census settings.
        receipt = self.install(target)
        self.assertEqual(
            receipt["fidelity"]["installed"]["config"]["validation"]["validated"],
            "target-validate_config_structure",
        )

    def test_recent_pending_pairing_code_survives_and_is_approvable(self) -> None:
        self.rebuild(datetime.now(timezone.utc).replace(microsecond=0))
        self.hooks = self.make_hooks()
        target, _ = self.installed()
        output = run_hermes(
            target / "agent/hermes-home",
            self.scratch_home,
            "import json; from gateway.pairing import PairingStore; s = PairingStore(); "
            "pending = s.list_pending('telegram'); "
            f"approved = s.approve_code('telegram', '{fx.PENDING_CODE}'); "
            f"print(json.dumps([len(pending), approved, s.is_approved('telegram', '{fx.PENDING_ID}')]))",
        )
        pending, approved, is_approved = json.loads(output.strip().splitlines()[-1])
        self.assertEqual(pending, 1)
        self.assertEqual(approved["user_id"], str(fx.PENDING_ID))
        self.assertTrue(is_approved)

    def test_rollback_keeps_going_when_one_step_fails(self) -> None:
        self.manifest()
        target = fx.build_target(self.root)
        before = self.tree(target)
        real_replace = os.replace
        calls = {"count": 0}

        def flaky_replace(source, destination, *args, **kwargs):
            # Fail the first restore of a backed-up file during rollback.
            if "preimport-state/full-fidelity" in str(source) and not calls["count"]:
                calls["count"] += 1
                raise OSError("synthetic restore failure")
            return real_replace(source, destination, *args, **kwargs)

        with (
            mock.patch(
                "legacy_hermes_target._write_private_json", side_effect=OSError("disk")
            ),
            mock.patch("legacy_hermes_fidelity.os.replace", flaky_replace),
            self.assertRaises(OSError) as raised,
        ):
            self.install(target)
        self.assertEqual(calls["count"], 1)
        self.assertTrue(
            any("rollback incomplete" in note for note in raised.exception.__notes__)
        )
        after = self.tree(target)
        # Placed files and the other restores still happened.
        self.assertFalse((target / "workspace/legacy-box1/Downloads").exists())
        self.assertFalse((target / "agent/hermes-home/cron/jobs.json").exists())
        changed = {
            key for key in set(before) | set(after) if before.get(key) != after.get(key)
        }
        self.assertTrue(
            changed
            <= {
                "migration",
                "agent/hermes-home/SOUL.md",
                "agent/hermes-home/config.yaml",
            },
            changed,
        )

    def test_poller_check_reports_codes_and_never_the_token(self) -> None:
        import threading
        from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

        status = {"getUpdates": 409}

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args) -> None:
                return None

            def do_GET(self) -> None:
                method = self.path.split("?")[0].rsplit("/", 1)[-1]
                code = status.get(method, 200)
                body = {
                    "ok": code == 200,
                    "result": []
                    if method == "getUpdates"
                    else {"url": "", "pending_update_count": 2},
                }
                data = json.dumps(body).encode()
                self.send_response(code)
                self.send_header("content-length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        self.manifest()
        target = fx.build_target(self.root)
        self.install(target)
        config_path = target / "agent/hermes-home/config.yaml"
        config = yaml.safe_load(config_path.read_text())
        config["gateway"]["platforms"]["telegram"]["extra"]["base_url"] = (
            f"http://127.0.0.1:{server.server_address[1]}/bot"
        )
        config_path.write_text(yaml.safe_dump(config))
        for code, safe in ((409, False), (200, True)):
            status["getUpdates"] = code
            out = io.StringIO()
            with redirect_stdout(out), redirect_stderr(io.StringIO()):
                exit_code = migration.main(
                    ["telegram-poller-check", "--target-root", str(target)]
                )
            result = json.loads(out.getvalue())
            self.assertEqual(result["other_poller_detected"], code == 409)
            self.assertEqual(result["safe_to_start"], safe)
            self.assertEqual(result["pending_update_count"], 2)
            self.assertEqual(exit_code, 0 if safe else 3)
            self.assertNotIn(fx.TOKEN, out.getvalue())
            self.assertNotIn(fx.TOKEN.split(":")[1], out.getvalue())


class TelegramGateTests(unittest.TestCase):
    """The behavioral qualification against this interpreter's real adapter."""

    def copy_adapter(self, edit=None) -> Path:
        import shutil

        from hermes_cli.plugins import get_bundled_plugins_dir

        root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, root, True)
        source = get_bundled_plugins_dir() / "platforms/telegram"
        shutil.copytree(
            source,
            root / "platforms/telegram",
            ignore=shutil.ignore_patterns("__pycache__"),
        )
        adapter = root / "platforms/telegram/adapter.py"
        adapter.chmod(0o644)
        if edit is not None:
            text = adapter.read_text()
            edited = edit(text)
            self.assertNotEqual(edited, text, "the edit did not apply")
            adapter.write_text(edited)
        return root

    def test_patched_adapter_qualifies_with_a_stable_fingerprint(self) -> None:
        import legacy_hermes_telegram_gate as gate

        root = self.copy_adapter()
        first = gate.qualify_target_telegram(plugins_dir=root)
        self.assertTrue(first["qualified"], first)
        self.assertEqual(first["capability"], gate.REQUIRED_CAPABILITY)
        second = gate.qualify_target_telegram(plugins_dir=root)
        self.assertEqual(first["fingerprint"], second["fingerprint"])
        self.assertEqual(first["probes"]["control"]["conflict_drop_requests"], 1)

    def test_helper_kept_but_a_call_site_reverted_is_rejected(self) -> None:
        import legacy_hermes_telegram_gate as gate

        cases = {
            "conflict-retry-drops-queue": lambda text: text.replace(
                "drop_pending_updates=self._finite_drop_pending_updates(\n"
                '                        "conflict-retry", True\n'
                "                    ),",
                "drop_pending_updates=True,",
            ),
            "cold-start-drops-queue": lambda text: text.replace(
                "drop_pending_updates=self._finite_drop_pending_updates(\n"
                '                        "cold-start", not is_reconnect\n'
                "                    ),",
                "drop_pending_updates=not is_reconnect,",
            ),
        }
        for reason, edit in cases.items():
            with self.subTest(reason=reason):
                root = self.copy_adapter(edit)
                text = (root / "platforms/telegram/adapter.py").read_text()
                self.assertIn("def _preserve_pending_updates_on_start", text)
                self.assertIn(gate.REQUIRED_CAPABILITY, text)
                verdict = gate.qualify_target_telegram(plugins_dir=root)
                self.assertFalse(verdict["qualified"])
                self.assertIn(reason, verdict["reasons"])
                self.assertIn("patched-call-sites-not-exercised", verdict["reasons"])

    def test_a_missing_or_old_capability_marker_is_rejected(self) -> None:
        import legacy_hermes_telegram_gate as gate

        root = self.copy_adapter(
            lambda text: text.replace(
                '"finite.telegram.preserve-pending/2"',
                '"finite.telegram.preserve-pending/1"',
            )
        )
        verdict = gate.qualify_target_telegram(plugins_dir=root)
        self.assertFalse(verdict["qualified"])
        self.assertEqual(
            verdict["reasons"], ["capability-marker-missing-or-wrong-version"]
        )

    def test_a_marker_in_a_comment_is_not_a_capability(self) -> None:
        import legacy_hermes_telegram_gate as gate

        root = self.copy_adapter(
            lambda text: (
                "# "
                + gate.REQUIRED_CAPABILITY
                + " _preserve_pending_updates_on_start\n"
            )
        )
        verdict = gate.qualify_target_telegram(plugins_dir=root)
        self.assertFalse(verdict["qualified"])


if __name__ == "__main__":
    unittest.main()

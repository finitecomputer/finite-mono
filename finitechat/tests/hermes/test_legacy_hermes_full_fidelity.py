"""Full-fidelity import proven against the pinned target Hermes and its gateway.

A census-shaped synthetic box1 home (scripts/tests/legacy_box1_fixture.py) is
sealed and installed with the real target SessionDB and MemoryStore, after the
real Telegram qualification probe has run against this Hermes. The real
`hermes gateway run` then starts on the imported Hermes home against a
loopback fake Telegram Bot API and a stub model. The only harness edits after
import point the model and the Bot API at those loopback servers, disable the
Finite Chat transport (no finitechat binary here) and give terminal.cwd a real
directory; everything Telegram, pairing, persona, memory, routing and cron is
exactly what the importer wrote. HOME and HERMES_HOME are scratch directories.
Fixture times are relative to the real clock with hours of margin, so a run
can never cross a real cron slot.
"""

from __future__ import annotations

import contextlib
import hashlib
import importlib.util
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import time
import unittest
from collections.abc import Iterator
from datetime import datetime
from pathlib import Path
from unittest import mock

MONO_ROOT = Path(__file__).resolve().parents[3]
SCRIPTS = MONO_ROOT / "scripts"
sys.path.insert(0, str(SCRIPTS))
sys.path.insert(0, str(Path(__file__).resolve().parent))
SPEC = importlib.util.spec_from_file_location(
    "finite_legacy_hermes_migration_ff", SCRIPTS / "legacy_hermes_migration.py"
)
assert SPEC is not None and SPEC.loader is not None
migration = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = migration
SPEC.loader.exec_module(migration)
RECONCILER_SPEC = importlib.util.spec_from_file_location(
    "finite_reconcile_hermes_config",
    MONO_ROOT / "finitechat/containers/agent/reconcile_hermes_config.py",
)
assert RECONCILER_SPEC is not None and RECONCILER_SPEC.loader is not None
reconciler = importlib.util.module_from_spec(RECONCILER_SPEC)
RECONCILER_SPEC.loader.exec_module(reconciler)

FIXTURE_BUILDER_SPEC = importlib.util.spec_from_file_location(
    "finite_legacy_box1_fixture", SCRIPTS / "tests" / "legacy_box1_fixture.py"
)
assert FIXTURE_BUILDER_SPEC is not None and FIXTURE_BUILDER_SPEC.loader is not None
fx = importlib.util.module_from_spec(FIXTURE_BUILDER_SPEC)
FIXTURE_BUILDER_SPEC.loader.exec_module(fx)
GATE_SPEC = importlib.util.spec_from_file_location(
    "legacy_hermes_telegram_gate", SCRIPTS / "legacy_hermes_telegram_gate.py"
)
assert GATE_SPEC is not None and GATE_SPEC.loader is not None
gate = importlib.util.module_from_spec(GATE_SPEC)
GATE_SPEC.loader.exec_module(gate)
from legacy_telegram_fakes import FakeTelegramBotApi, StubModel  # noqa: E402

FIXTURE = Path(__file__).with_name("fixtures") / "legacy_v014"
STUB_REPLY = "Hello again from the same bot"
STRUCTURED_FACT = "The meetup notes are at /home/node/workspace/notes.md"
CRON_PROMPT_MARKERS = (
    "Summarize",
    "Run the check script",
    "Remind Owner",
    "Post the status",
    "Draft a note",
    "Check the feed",
    "Daily check",
    "Ping three times",
    "Already reminded",
)
TARGET_SETTINGS = {
    "FINITE_CONFIG_MODEL": "glm-5-3-flash",
    "FINITE_CONFIG_PROVIDER": "custom",
    "FINITE_CONFIG_BASE_URL": "https://finite-private.finite.containers.tinfoil.dev/v1",
    "FINITE_CONFIG_API_MODE": "chat_completions",
    "FINITE_CONFIG_CONTEXT_LENGTH": "393216",
    "FINITE_CONFIG_API_KEY_REFERENCE": "${FINITE_PRIVATE_API_KEY}",
    "FINITE_CONFIG_PLUGIN_NAME": "finitechat",
    "FINITE_CONFIG_TITLE_TIMEOUT_SECS": "2",
    "FINITE_CONFIG_AGENT_HOME": "/data/agent",
    "FINITE_CONFIG_FINITECHAT_BIN": "/usr/local/bin/finitechat",
    "FINITE_CONFIG_SERVICE_ADDR": "127.0.0.1:0",
    "FINITE_CONFIG_POLL_TIMEOUT_SECS": "1",
    "FINITE_CONFIG_POLL_LIMIT": "10",
    "FINITE_CONFIG_MANAGED_SKILLS_DIR": fx.MANAGED_SKILLS,
    "FINITE_CONFIG_WORKSPACE": "/data/workspace",
}
HERMES_PROBE = r"""
import json, os
from gateway.config import Platform, load_gateway_config
from gateway.pairing import PairingStore
from cron.jobs import get_due_jobs, load_jobs
token = os.environ.pop("EXPECTED_TOKEN")
config = load_gateway_config()
telegram = config.platforms[Platform.TELEGRAM]
home = config.get_home_channel(Platform.TELEGRAM)
store = PairingStore()
print("PROBE=" + json.dumps({
    "enabled": telegram.enabled,
    "token_matches": telegram.token == token,
    "home": home.chat_id if home else None,
    "extra": telegram.extra,
    "approved": sorted(item["user_id"] for item in store.list_approved("telegram")),
    "pending": [item["user_id"] for item in store.list_pending("telegram")],
    "due": [job["id"] for job in get_due_jobs()],
    "jobs": [job["id"] for job in load_jobs()],
}))
"""


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def legacy_memory_store(path: Path) -> int:
    with sqlite3.connect(path) as connection:
        connection.executescript((FIXTURE / "memory.sql").read_text())
        connection.execute("INSERT INTO facts(content) VALUES (?)", (STRUCTURED_FACT,))
        return connection.execute("SELECT count(*) FROM facts").fetchone()[0]


class LegacyHermesFullFidelityLiveTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tempdir = tempfile.TemporaryDirectory(prefix="legacy-ff-")
        self.addCleanup(self.tempdir.cleanup)
        self.root = Path(self.tempdir.name).resolve()
        self.scratch_home = self.root / "scratch-home"
        (self.scratch_home / "isolated-hermes").mkdir(parents=True)
        self.enterContext(
            mock.patch.dict(
                os.environ,
                {
                    "HOME": str(self.scratch_home),
                    "HERMES_HOME": str(self.scratch_home / "isolated-hermes"),
                },
            )
        )

    def hermes_env(self, hermes_home: Path) -> dict[str, str]:
        env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("HERMES_", "TELEGRAM_", "FINITE", "OPENAI", "OPENROUTER"))
        }
        env.update(HOME=str(self.scratch_home), HERMES_HOME=str(hermes_home), TZ="UTC")
        env["PATH"] = os.pathsep.join([str(Path(sys.executable).parent), "/usr/bin", "/bin"])
        return env

    def probe(self, hermes_home: Path) -> dict:
        completed = subprocess.run(
            [sys.executable, "-c", HERMES_PROBE],
            env={**self.hermes_env(hermes_home), "EXPECTED_TOKEN": fx.TOKEN},
            cwd=self.root,
            capture_output=True,
            text=True,
            timeout=120,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr[-4000:])
        line = next(item for item in completed.stdout.splitlines() if item.startswith("PROBE="))
        return json.loads(line.removeprefix("PROBE="))

    def build_target(self) -> Path:
        import yaml
        from hermes_cli.default_soul import DEFAULT_SOUL_MD
        from hermes_state import SessionDB
        from plugins.memory.holographic.store import MemoryStore

        seeded = reconciler.reconcile_config(None, TARGET_SETTINGS)
        target = fx.build_target(self.root, config_text=yaml.safe_dump(seeded, sort_keys=False))
        hermes = target / "agent/hermes-home"
        (hermes / "SOUL.md").write_text(DEFAULT_SOUL_MD, encoding="utf-8")
        SessionDB(db_path=hermes / "state.db").close()
        MemoryStore(db_path=hermes / "memory_store.db").close()
        return target

    def install(self, telegram_mode: str = "live") -> tuple[Path, Path, dict, dict]:
        """Seal, qualify this Hermes, and install, as runbook steps 6 to 8."""
        evidence = fx.build_bundle(self.root, migration, memory_store_writer=legacy_memory_store)
        bundle = evidence["bundle"]
        migration.create_manifest(
            bundle,
            fx.source_metadata(migration, evidence["inventory_sha256"]),
            mode="full-fidelity",
        )
        # The image qualification: the real adapter's cold-start and 409 paths.
        qualification = gate.qualify_target_telegram()
        self.assertTrue(qualification["qualified"], qualification)
        target = self.build_target()
        started = time.monotonic()
        receipt = migration.install_bundle(
            bundle,
            target,
            expected_machine_id="frankie",
            expected_manifest_sha256=sha256(bundle / "manifest.json"),
            expected_identity_sha256=sha256(target / "agent/identity/identity.json"),
            expected_chat_client_sha256=sha256(target / "agent/client.sqlite3"),
            mode="full-fidelity",
            telegram_mode=telegram_mode,
            expected_telegram_fingerprint=qualification["fingerprint"],
        )
        self.install_seconds = time.monotonic() - started
        for secret in fx.SECRET_VALUES:
            self.assertNotIn(secret, json.dumps(receipt))
        return target, target / "agent/hermes-home", receipt, qualification

    def point_at_fakes(
        self, target: Path, hermes: Path, telegram: FakeTelegramBotApi, model: StubModel
    ) -> None:
        import yaml

        config = yaml.safe_load((hermes / "config.yaml").read_text())
        config["model"].update(base_url=model.base_url, api_key="synthetic-loopback-key")
        config["gateway"]["platforms"]["finitechat"]["enabled"] = False
        config["plugins"]["enabled"] = []
        block = config["gateway"]["platforms"]["telegram"]
        block["extra"]["base_url"] = telegram.base_url
        if not block.get("token"):
            # Rehearsal copy: the offline check supplies the fake's token.
            block.update(enabled=True, token=telegram.token)
        config["terminal"]["cwd"] = str(target / "workspace")
        (hermes / "config.yaml").write_text(yaml.safe_dump(config, sort_keys=False))

    @contextlib.contextmanager
    def gateway(self, target: Path, hermes: Path, name: str) -> Iterator[None]:
        log = (self.root / f"{name}.log").open("w")
        self.current_log = self.root / f"{name}.log"
        process = subprocess.Popen(
            [
                sys.executable,
                "-c",
                "import sys; from hermes_cli.main import main; "
                "sys.argv = ['hermes', 'gateway', 'run', '--force']; sys.exit(main())",
            ],
            env=self.hermes_env(hermes),
            cwd=target / "workspace",
            stdout=log,
            stderr=subprocess.STDOUT,
        )
        try:
            yield
        finally:
            process.terminate()
            try:
                process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
            log.close()

    def gateway_log(self) -> str:
        path = getattr(self, "current_log", None)
        return path.read_text(errors="replace")[-6000:] if path and path.exists() else ""

    def resolves(self, target: Path, absolute: str) -> bool:
        self.assertTrue(absolute.startswith("/data/"), absolute)
        return (target / absolute.removeprefix("/data/")).exists()

    def test_moved_bot_answers_on_telegram_with_its_persona_memory_and_jobs(self) -> None:
        from hermes_state import SessionDB
        from plugins.memory.holographic.store import MemoryStore

        target, hermes, receipt, qualification = self.install()
        installed = receipt["fidelity"]["installed"]
        self.assertEqual(installed["config"]["telegram"], "connected-with-home-channel")
        self.assertEqual(
            installed["telegram"]["qualification"]["fingerprint"], qualification["fingerprint"]
        )
        self.assertEqual(installed["soul"], "activated")
        self.assertEqual(installed["routing"]["mapped_session_ids"], [fx.ACTIVE_SESSION_ID])

        # Target Hermes reads the imported Telegram, pairing and cron state.
        probe = self.probe(hermes)
        self.assertTrue(probe["enabled"])
        self.assertTrue(probe["token_matches"])
        self.assertEqual(probe["home"], str(fx.OWNER_ID))
        self.assertEqual(probe["extra"]["dm_policy"], "pairing")
        self.assertIs(probe["extra"]["preserve_pending_updates"], True)
        self.assertEqual(probe["approved"], sorted([str(fx.OWNER_ID), str(fx.SECOND_ID)]))
        self.assertEqual(probe["pending"], [str(fx.PENDING_ID)], "a recent code must survive")
        self.assertEqual(probe["due"], [], "no job may fire right after the downtime")
        self.assertEqual(
            probe["jobs"], [job["id"] for job in receipt["fidelity"]["plan"]["cron"]["jobs"]]
        )
        jobs_after_import = {
            job["id"]: job for job in json.loads((hermes / "cron/jobs.json").read_text())["jobs"]
        }

        # Imported sessions and memories are present through the target libraries.
        database = SessionDB(db_path=hermes / "state.db")
        try:
            session = database.export_session("legacy-telegram-session")
            active = database.find_latest_gateway_session_for_peer(
                source="telegram",
                user_id=str(fx.OWNER_ID),
                session_key=f"agent:main:telegram:dm:{fx.OWNER_ID}",
            )
        finally:
            database.close()
        assert session is not None
        rendered = json.dumps(session["messages"])
        self.assertIn("/data/workspace/legacy-box1/poster_final.jpg", rendered)
        self.assertIn("/data/workspace/legacy-box1/Downloads/memo.txt", rendered)
        self.assertEqual(
            session["messages"][0]["content"][1]["path"],
            "/data/workspace/legacy-box1/uploads/photo.txt",
        )
        self.assertIsNone(session.get("session_key"))
        assert active is not None
        self.assertEqual(active["id"], fx.ACTIVE_SESSION_ID)
        store = MemoryStore(db_path=hermes / "memory_store.db")
        try:
            facts = [fact["content"] for fact in store.list_facts(limit=10)]
        finally:
            store.close()
        self.assertIn(STRUCTURED_FACT.replace("/home/node", "/data/workspace/legacy-box1"), facts)
        self.assertFalse(any("/home/node" in fact for fact in facts))

        # Rewritten paths resolve in the target durable root.
        soul = (hermes / "SOUL.md").read_text()
        for absolute in (
            "/data/workspace/legacy-box1/workspace",
            "/data/workspace/legacy-box1/Downloads",
            "/data/workspace/legacy-box1/poster_final.jpg",
            "/data/workspace/legacy-box1/Downloads/memo.txt",
            "/data/workspace/legacy-box1/uploads/photo.txt",
            "/data/workspace/legacy-box1/workspace/notes.md",
            jobs_after_import["inbox-check"]["script"],
            jobs_after_import["inbox-check"]["workdir"],
        ):
            self.assertTrue(self.resolves(target, absolute), absolute)
        self.assertIn("/data/workspace/legacy-box1/workspace", soul)
        self.assertIn(
            "/data/workspace/legacy-box1/workspace/notes.md",
            jobs_after_import["daily-briefing"]["prompt"],
        )
        skill = (hermes / "skills" / fx.USER_SKILL / "SKILL.md").read_text()
        self.assertIn("/data/workspace/legacy-box1/workspace/cellar.md", skill)
        self.assertIn(
            "/data/workspace/legacy-box1/workspace/post.png",
            (target / "workspace/legacy-box1/make_post.py").read_text(),
        )

        with FakeTelegramBotApi(fx.TOKEN) as telegram, StubModel(STUB_REPLY) as model:
            self.point_at_fakes(target, hermes, telegram, model)
            during_move = "Are you still there? (sent during the move)"
            telegram.queue_message(fx.OWNER_ID, during_move, "Owner")
            started = time.time()
            with self.gateway(target, hermes, "gateway-cold-start"):
                owner = telegram.wait_for_message(fx.OWNER_ID, 180)
                self.first_reply_seconds = time.time() - started
                self.assertEqual(owner, [STUB_REPLY], self.gateway_log())
                # Continuity: the first turns continue the active conversation.
                history = model.conversation_for(during_move)
                assert history is not None, self.gateway_log()
                self.assertIn(fx.ACTIVE_USER_TEXT, history)
                self.assertIn(fx.ACTIVE_ASSISTANT_TEXT, history)
                follow_up = "Use the second version please"
                telegram.queue_message(fx.OWNER_ID, follow_up, "Owner")
                self.assertEqual(
                    telegram.wait_for_count(fx.OWNER_ID, 2, 120),
                    [STUB_REPLY, STUB_REPLY],
                    self.gateway_log(),
                )
                history = model.conversation_for(follow_up)
                assert history is not None
                self.assertIn("B-GREEN-8834", history)
                telegram.queue_message(fx.SECOND_ID, "Hi, it's me", "Second")
                self.assertEqual(
                    telegram.wait_for_message(fx.SECOND_ID, 120), [STUB_REPLY], self.gateway_log()
                )
                telegram.queue_message(202, "Who are you?", "Stranger")
                stranger = telegram.wait_for_message(202, 120)
                self.assertTrue(stranger and "pairing code" in stranger[0], stranger)
                # Network reconnect: getUpdates drops twice; a message queued
                # meanwhile is still answered.
                with telegram.condition:
                    telegram.network_failures_left = 2
                deadline = time.time() + 60
                while time.time() < deadline and telegram.network_failures_served < 2:
                    time.sleep(0.2)
                self.assertEqual(telegram.network_failures_served, 2, self.gateway_log())
                telegram.queue_message(fx.SECOND_ID, "Still there after the blip?", "Second")
                self.assertEqual(
                    telegram.wait_for_count(fx.SECOND_ID, 2, 180),
                    [STUB_REPLY, STUB_REPLY],
                    self.gateway_log(),
                )
                success = hermes / "cron/ticker_last_success"
                deadline = time.time() + 120
                while time.time() < deadline and not (
                    success.exists() and success.stat().st_mtime >= started
                ):
                    time.sleep(0.5)
                self.assertTrue(
                    success.exists() and success.stat().st_mtime >= started, self.gateway_log()
                )

            self.assertEqual(telegram.unauthorized_calls, 0)
            self.assertIn("getUpdates", telegram.calls)
            self.assertIn("getMe", telegram.calls)
            self.assertEqual(telegram.dropped_updates, 0, "a queued message was dropped")
            for chat in (fx.OWNER_ID, fx.SECOND_ID):
                self.assertFalse(any("pairing code" in text for text in telegram.messages_to(chat)))
            prompts = model.system_prompts()
            self.assertTrue(prompts)
            self.assertTrue(any(fx.SOUL_MARKER in prompt for prompt in prompts))
            self.assertTrue(
                any("/data/workspace/legacy-box1/workspace" in prompt for prompt in prompts)
            )
            self.assertTrue(any(fx.MEMORY_MARKER in prompt for prompt in prompts))
            self.assertTrue(any(fx.AGENTS_OWNER_ADDITION.splitlines()[1] in p for p in prompts))
            self.assertFalse(any("finitec publish" in prompt for prompt in prompts))
            for text in model.user_texts():
                for marker in CRON_PROMPT_MARKERS:
                    self.assertNotIn(marker, text)

        jobs_after_run = {
            job["id"]: job for job in json.loads((hermes / "cron/jobs.json").read_text())["jobs"]
        }
        self.assertEqual(sorted(jobs_after_run), sorted(jobs_after_import))
        for job_id, job in jobs_after_import.items():
            for field in ("next_run_at", "last_run_at", "enabled"):
                self.assertEqual(jobs_after_run[job_id].get(field), job.get(field), (job_id, field))
            if job.get("enabled") and job.get("next_run_at"):
                self.assertGreater(
                    datetime.fromisoformat(job["next_run_at"]).timestamp(), started, job_id
                )
        self.assertFalse(jobs_after_run["missed-reminder"]["enabled"])
        self.assertFalse((hermes / "cron/persisted_error_recoveries.jsonl").exists())
        self.assertEqual(
            sha256(target / "agent/client.sqlite3"),
            receipt["protected_state"]["chat_client_sha256"],
        )
        print(
            f"\nMEASURED install_seconds={self.install_seconds:.1f} "
            f"restart_to_first_reply_seconds={self.first_reply_seconds:.1f}",
            file=sys.stderr,
        )

    def test_queue_survives_conflicts_at_start_and_repeated_conflicts_later(self) -> None:
        target, hermes, _, _ = self.install()
        with FakeTelegramBotApi(fx.TOKEN) as telegram, StubModel(STUB_REPLY) as model:
            self.point_at_fakes(target, hermes, telegram, model)
            # The predecessor's long poll is still open when the target starts.
            telegram.conflicts_left = 1
            telegram.queue_message(fx.OWNER_ID, "Sent while the bot was moving", "Owner")
            with self.gateway(target, hermes, "gateway-conflicts"):
                self.assertEqual(
                    telegram.wait_for_message(fx.OWNER_ID, 240), [STUB_REPLY], self.gateway_log()
                )
                self.assertEqual(telegram.conflicts_served, 1)
                # Later, something else polls the token three times in a row.
                with telegram.condition:
                    telegram.conflicts_left = 3
                deadline = time.time() + 60
                while time.time() < deadline and telegram.conflicts_served < 2:
                    time.sleep(0.2)
                telegram.queue_message(fx.SECOND_ID, "Sent during the conflicts", "Second")
                self.assertEqual(
                    telegram.wait_for_message(fx.SECOND_ID, 400), [STUB_REPLY], self.gateway_log()
                )
            self.assertEqual(telegram.conflicts_served, 4, self.gateway_log())
            self.assertEqual(telegram.dropped_updates, 0, "a conflict retry flushed the queue")

    def test_rehearsal_copy_holds_no_token_and_passes_the_offline_telegram_check(
        self,
    ) -> None:
        _, hermes, receipt, _ = self.install(telegram_mode="rehearsal")
        self.assertNotIn(fx.TOKEN, (hermes / "config.yaml").read_text())
        self.assertEqual(receipt["fidelity"]["installed"]["telegram"]["mode"], "rehearsal")
        # The runbook's offline check runs on a copy of the rehearsal home.
        import shutil

        import legacy_rehearsal_telegram_check as rehearsal_check

        copy = self.root / "rehearsal-copy/hermes-home"
        shutil.copytree(hermes, copy, symlinks=True)
        summary = rehearsal_check.run_check(copy, self.root / "rehearsal-check")
        self.assertTrue(summary["ok"], summary)
        self.assertEqual(summary["approved_users_checked"], 2)
        self.assertEqual(summary["answered"], 2)
        self.assertNotIn(str(fx.OWNER_ID), json.dumps(summary))
        # The check refuses a home that holds a token (a live install).
        live_copy = self.root / "live-copy/hermes-home"
        shutil.copytree(hermes, live_copy, symlinks=True)
        import yaml

        config = yaml.safe_load((live_copy / "config.yaml").read_text())
        config["gateway"]["platforms"]["telegram"]["token"] = fx.TOKEN
        (live_copy / "config.yaml").write_text(yaml.safe_dump(config))
        with self.assertRaisesRegex(SystemExit, "holds a Telegram token"):
            rehearsal_check.run_check(live_copy, self.root / "live-check")


if __name__ == "__main__":
    unittest.main()

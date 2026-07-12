"""Focused contract tests for the image-side recover-known-good boot mode."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from collections.abc import Callable
from pathlib import Path
from typing import Any, cast

REPO_ROOT = Path(__file__).resolve().parents[2]
RECOVERY_BOOT = REPO_ROOT / "containers/agent/recover_chat_boot.py"
RECONCILER = REPO_ROOT / "containers/agent/reconcile_hermes_config.py"
GATEWAY = REPO_ROOT / "containers/agent/run_hermes_gateway.sh"
ACCOUNT_ID = "a" * 64
NPUB = "npub1finiteagent"
CLIENT_TABLES = (
    "client_device_states",
    "client_app_messages",
    "client_app_events",
    "client_app_outbox",
    "client_app_rooms",
    "client_app_state",
    "client_app_profiles",
)


def load_hermes_config(path: Path) -> dict[str, Any]:
    spec = importlib.util.spec_from_file_location(
        "reconcile_hermes_config_for_recovery_test", RECONCILER
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    loader = cast(Callable[[Path], dict[str, Any]], module.__dict__["_load"])
    return loader(path)


FAKE_FINITECHAT = r"""#!/usr/bin/env python3
import json
import os
import sys
from pathlib import Path

args = sys.argv[1:]
with Path(os.environ["FAKE_CALL_LOG"]).open("a", encoding="utf-8") as handle:
    handle.write(json.dumps(args) + "\n")

if args == ["auth", "status"]:
    print(json.dumps({"account_id": "a" * 64, "npub": "npub1finiteagent"}))
    raise SystemExit(0)

if "init" in args:
    print(json.dumps({"unexpected": "init"}))
    raise SystemExit(91)

if "install" in args:
    plugins_dir = Path(args[args.index("--plugins-dir") + 1])
    plugin_name = args[args.index("--plugin-name") + 1]
    plugin_dir = plugins_dir / plugin_name
    plugin_dir.mkdir(parents=True, exist_ok=True)
    (plugin_dir / "__init__.py").write_text("canonical-init\n", encoding="utf-8")
    (plugin_dir / "adapter.py").write_text("canonical-adapter\n", encoding="utf-8")
    (plugin_dir / "plugin.yaml").write_text("name: finitechat\n", encoding="utf-8")
    (plugin_dir / "finitechat.env").write_text("FINITECHAT_BIN=fake\n", encoding="utf-8")
    print(json.dumps({"plugin_name": plugin_name}))
    raise SystemExit(0)

if "home-channel" in args and "set" in args:
    home = Path(args[args.index("--home") + 1])
    room_id = args[args.index("--room-id") + 1]
    conversation_id = None
    if "--conversation-id" in args:
        conversation_id = args[args.index("--conversation-id") + 1]
    payload = {
        "room_id": room_id,
        "conversation_id": conversation_id,
        "set_at_ms": 1,
    }
    (home / "hermes-home-channel.json").write_text(
        json.dumps(payload) + "\n", encoding="utf-8"
    )
    print(json.dumps({"home_channel": payload}))
    raise SystemExit(0)

if "recover" in args:
    home = Path(args[args.index("--home") + 1])
    running_path = home / "hermes-running.json"
    running = json.loads(running_path.read_text(encoding="utf-8"))
    recovered = len(running.get("messages", []))
    running_path.write_text('{"messages": []}\n', encoding="utf-8")
    print(json.dumps({"recovered": recovered}))
    raise SystemExit(0)

print(json.dumps({"unsupported": args}))
raise SystemExit(92)
"""


class RecoveryFixture:
    def __init__(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.state_root = self.tmp / "data"
        self.agent_home = self.state_root / "agent"
        self.hermes_home = self.agent_home / "hermes-home"
        self.workspace = self.state_root / "workspace"
        self.identity_path = self.agent_home / "identity/identity.json"
        self.client_store = self.agent_home / "client.sqlite3"
        self.config_path = self.hermes_home / "config.yaml"
        self.call_log = self.tmp / "calls.jsonl"
        self.fake_bin = self.tmp / "finitechat"
        self.managed_skills = self.agent_home / "managed-skills/finite/current"
        self.operation_id = "recover-op-secret-value"

        for directory in (
            self.hermes_home / "plugins/finitechat/__pycache__",
            self.hermes_home / "plugins/user-plugin",
            self.hermes_home / "memory",
            self.hermes_home / "skills/user-skill",
            self.agent_home / "agentd",
            self.agent_home / "tools",
            self.agent_home / "connections",
            self.managed_skills,
            self.workspace,
            self.identity_path.parent,
        ):
            directory.mkdir(parents=True, exist_ok=True)

        self.fake_bin.write_text(FAKE_FINITECHAT, encoding="utf-8")
        self.fake_bin.chmod(0o755)
        self.identity_path.write_text('{"secret":"identity-preserve-sentinel"}\n', encoding="utf-8")
        (self.agent_home / "config.json").write_text(
            json.dumps(
                {
                    "server_url": "http://127.0.0.1:18787",
                    "device_id": "agent",
                    "account_id": ACCOUNT_ID,
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        self._create_client_store()
        self.config_path.write_text(
            json.dumps(self._hermes_config(), indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        self.config_path.chmod(0o600)

        (self.hermes_home / "plugins/finitechat/stale.py").write_text(
            "stale-plugin\n", encoding="utf-8"
        )
        (self.hermes_home / "plugins/finitechat/__pycache__/stale.pyc").write_bytes(b"stale-cache")
        (self.hermes_home / "plugins/user-plugin/plugin.yaml").write_text(
            "name: user-plugin\n", encoding="utf-8"
        )
        (self.hermes_home / "memory/user-memory.md").write_text(
            "memory-preserve\n", encoding="utf-8"
        )
        (self.hermes_home / "skills/user-skill/SKILL.md").write_text(
            "user-skill-preserve\n", encoding="utf-8"
        )
        (self.managed_skills / "SKILL.md").write_text("managed-skill-preserve\n", encoding="utf-8")
        (self.agent_home / "tools/user-tool.json").write_text(
            '{"tool":"preserve"}\n', encoding="utf-8"
        )
        (self.agent_home / "connections/user-connection.json").write_text(
            '{"connection":"preserve"}\n', encoding="utf-8"
        )
        (self.workspace / "user-work.txt").write_text("workspace-preserve\n", encoding="utf-8")
        (self.agent_home / "unknown-ready.json").write_text(
            "must-not-be-cleared\n", encoding="utf-8"
        )
        (self.agent_home / "hermes-home-channel.json").write_text(
            "corrupt-home-channel-metadata\n", encoding="utf-8"
        )
        (self.agent_home / "hermes-running.json").write_text(
            json.dumps(
                {
                    "messages": [
                        {
                            "room_id": "room-1",
                            "conversation_id": "conversation-1",
                            "message_id": "message-1",
                        }
                    ]
                }
            )
            + "\n",
            encoding="utf-8",
        )
        for name in (
            "hermes-service.json",
            "hermes-bridge-status.json",
            "hermes-gateway.pid",
            "finitechat-hermes.pid",
        ):
            (self.agent_home / name).write_text("transient\n", encoding="utf-8")
        for name in (
            "finitechat-ready.json",
            "status.json",
            "finitechat.pid",
            "health.pid",
            "hermes.pid",
        ):
            (self.agent_home / "agentd" / name).write_text("transient\n", encoding="utf-8")
        (self.hermes_home / ".config.yaml.interrupted").write_text("partial\n", encoding="utf-8")

        self.env = {
            **os.environ,
            "FAKE_CALL_LOG": str(self.call_log),
            "FINITE_AGENT_BOOT_INTENT_JSON": json.dumps(
                {
                    "schema_version": 1,
                    "kind": "recover_known_good",
                    "operation_id": self.operation_id,
                }
            ),
            "FINITE_AGENT_STATE_ROOT": str(self.state_root),
            "FINITECHAT_HOME": str(self.agent_home),
            "FINITE_HOME": str(self.agent_home),
            "HERMES_HOME": str(self.hermes_home),
            "FINITECHAT_WORKSPACE": str(self.workspace),
            "FINITECHAT_BIN": str(self.fake_bin),
            "FINITE_CONFIG_MODEL": "environment-must-not-overwrite",
            "FINITE_CONFIG_PROVIDER": "environment-must-not-overwrite",
            "FINITE_CONFIG_BASE_URL": "https://environment.invalid/v1",
            "FINITE_CONFIG_API_MODE": "chat_completions",
            "FINITE_CONFIG_API_KEY_REFERENCE": "",
            "FINITE_CONFIG_PLUGIN_NAME": "finitechat",
            "FINITE_CONFIG_TITLE_TIMEOUT_SECS": "2",
            "FINITE_CONFIG_AGENT_HOME": str(self.agent_home),
            "FINITE_CONFIG_FINITECHAT_BIN": str(self.fake_bin),
            "FINITE_CONFIG_SERVICE_ADDR": "127.0.0.1:0",
            "FINITE_CONFIG_POLL_TIMEOUT_SECS": "1",
            "FINITE_CONFIG_POLL_LIMIT": "10",
            "FINITE_CONFIG_HOME_CHANNEL": "",
            "FINITE_CONFIG_MANAGED_SKILLS_DIR": str(self.managed_skills),
            "FINITE_CONFIG_WORKSPACE": str(self.workspace),
        }

    def close(self) -> None:
        self._tmp.cleanup()

    def _create_client_store(self) -> None:
        connection = sqlite3.connect(self.client_store)
        try:
            for table in CLIENT_TABLES:
                connection.execute(f"CREATE TABLE {table} (id TEXT)")
            connection.commit()
        finally:
            connection.close()

    def _hermes_config(self) -> dict[str, Any]:
        return {
            "model": {
                "default": "user/model-v1",
                "provider": "user-provider",
                "api_key": "${USER_MODEL_API_KEY}",
            },
            "plugins": {"enabled": ["finite-platform", "finite", "user-plugin"]},
            "gateway": {
                "platforms": {
                    "finitechat": {
                        "enabled": False,
                        "home_channel": {
                            "platform": "finitechat",
                            "chat_id": "room-1",
                            "name": "User-selected room",
                        },
                        "extra": {"home": "/wrong", "user_extension": "preserve"},
                    },
                    "finite-platform": {"enabled": True, "custom": "preserve"},
                    "finite": "legacy-invalid-generated-value",
                    "telegram": {
                        "enabled": True,
                        "bot_token": "${TELEGRAM_BOT_TOKEN}",
                        "allowed_user_ids": [1234],
                    },
                    "user-platform": {"enabled": True, "opaque": {"keep": True}},
                }
            },
            "platforms": {"user-top-level-platform": {"preserve": True}},
            "skills": {"external_dirs": [str(self.hermes_home / "skills/user-skill")]},
            "tools": {"user-tool": {"enabled": True}},
            "connections": {"user-connection": {"enabled": True}},
            "user_extension": {"opaque": [1, 2, 3]},
        }

    def run(self, *, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(RECOVERY_BOOT), "--config", str(self.config_path)],
            env=env or self.env,
            capture_output=True,
            text=True,
            timeout=20,
            check=False,
        )

    def calls(self) -> list[list[str]]:
        if not self.call_log.exists():
            return []
        return [json.loads(line) for line in self.call_log.read_text(encoding="utf-8").splitlines()]

    def operation_marker(self) -> Path:
        digest = hashlib.sha256(self.operation_id.encode()).hexdigest()
        return self.agent_home / "recover-chat-operations" / f"{digest}.json"

    def preservation_snapshot(self) -> dict[str, tuple[int, bytes]]:
        paths = (
            self.identity_path,
            self.agent_home / "config.json",
            self.hermes_home / "plugins/user-plugin/plugin.yaml",
            self.hermes_home / "memory/user-memory.md",
            self.hermes_home / "skills/user-skill/SKILL.md",
            self.managed_skills / "SKILL.md",
            self.agent_home / "tools/user-tool.json",
            self.agent_home / "connections/user-connection.json",
            self.workspace / "user-work.txt",
            self.agent_home / "unknown-ready.json",
        )
        return {
            str(path): (path.stat().st_ino, path.read_bytes()) for path in paths if path.exists()
        }


class RecoverChatBootTest(unittest.TestCase):
    def make_fixture(self) -> RecoveryFixture:
        fixture = RecoveryFixture()
        self.addCleanup(fixture.close)
        return fixture

    def test_repair_is_allowlisted_and_preserves_user_state(self) -> None:
        fixture = self.make_fixture()
        preserved = fixture.preservation_snapshot()
        client_key = (fixture.client_store.stat().st_dev, fixture.client_store.stat().st_ino)
        before_config = load_hermes_config(fixture.config_path)

        result = fixture.run()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("FINITE_RECOVER_CHAT_COMPLETE", result.stdout)
        self.assertNotIn(fixture.operation_id, result.stdout + result.stderr)
        self.assertEqual(fixture.preservation_snapshot(), preserved)
        self.assertEqual(
            (fixture.client_store.stat().st_dev, fixture.client_store.stat().st_ino),
            client_key,
        )
        self.assertEqual(
            json.loads((fixture.agent_home / "hermes-running.json").read_text())["messages"],
            [],
        )

        for path in (
            *(
                fixture.agent_home / name
                for name in (
                    "hermes-service.json",
                    "hermes-bridge-status.json",
                    "hermes-gateway.pid",
                    "finitechat-hermes.pid",
                )
            ),
            *(
                fixture.agent_home / "agentd" / name
                for name in (
                    "finitechat-ready.json",
                    "status.json",
                    "finitechat.pid",
                    "health.pid",
                    "hermes.pid",
                )
            ),
            fixture.hermes_home / ".config.yaml.interrupted",
        ):
            self.assertFalse(path.exists(), path)
        self.assertTrue((fixture.agent_home / "unknown-ready.json").exists())
        plugin = fixture.hermes_home / "plugins/finitechat"
        self.assertFalse((plugin / "stale.py").exists())
        self.assertFalse((plugin / "__pycache__").exists())
        self.assertEqual(
            (plugin / "adapter.py").read_text(encoding="utf-8"),
            "canonical-adapter\n",
        )

        config = load_hermes_config(fixture.config_path)
        for key in ("model", "platforms", "tools", "connections", "user_extension"):
            self.assertEqual(config[key], before_config[key])
        self.assertEqual(
            config["gateway"]["platforms"]["telegram"],
            before_config["gateway"]["platforms"]["telegram"],
        )
        self.assertEqual(
            config["gateway"]["platforms"]["user-platform"],
            before_config["gateway"]["platforms"]["user-platform"],
        )
        self.assertEqual(config["plugins"]["enabled"], ["user-plugin", "finitechat"])
        self.assertFalse(config["gateway"]["platforms"]["finite-platform"]["enabled"])
        self.assertFalse(config["gateway"]["platforms"]["finite"]["enabled"])
        self.assertEqual(config["gateway"]["platforms"]["finite-platform"]["custom"], "preserve")
        self.assertTrue(config["gateway"]["platforms"]["finitechat"]["enabled"])
        self.assertEqual(
            config["gateway"]["platforms"]["finitechat"]["extra"]["user_extension"],
            "preserve",
        )
        self.assertIn(str(fixture.managed_skills), config["skills"]["external_dirs"])

        calls = fixture.calls()
        self.assertEqual(sum("install" in call for call in calls), 1)
        self.assertEqual(sum("home-channel" in call for call in calls), 1)
        self.assertEqual(sum("recover" in call for call in calls), 1)
        self.assertFalse(any("init" in call for call in calls))

        report_text = (fixture.agent_home / "startup-report.json").read_text(encoding="utf-8")
        report = json.loads(report_text)
        self.assertEqual(report["schema_version"], 1)
        self.assertEqual(report["boot_mode"], "recover_known_good")
        self.assertEqual(report["status"], "completed")
        self.assertEqual(report["identity"]["npub"], NPUB)
        self.assertNotIn(fixture.operation_id, report_text)
        self.assertNotIn("protected_tree", report_text)
        self.assertEqual(
            report["acceptance_scope"],
            {
                "runtime_spec_delivery": "not_proven",
                "provider_conformance": "not_proven",
                "phala_acceptance": "not_proven",
            },
        )
        self.assertTrue(report["preservation"]["identity_reused_in_place"])
        self.assertTrue(report["preservation"]["client_store_reused_in_place"])
        self.assertEqual(json.loads(fixture.operation_marker().read_text()), report)

    def test_same_operation_replay_is_a_true_noop(self) -> None:
        fixture = self.make_fixture()
        first = fixture.run()
        self.assertEqual(first.returncode, 0, first.stderr)
        report_path = fixture.agent_home / "startup-report.json"
        marker_path = fixture.operation_marker()
        before = {
            "calls": fixture.call_log.read_bytes(),
            "report": (report_path.stat().st_mtime_ns, report_path.read_bytes()),
            "marker": (marker_path.stat().st_mtime_ns, marker_path.read_bytes()),
        }

        second = fixture.run()

        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertIn("FINITE_RECOVER_CHAT_NOOP", second.stdout)
        self.assertNotIn(fixture.operation_id, second.stdout + second.stderr)
        self.assertEqual(fixture.call_log.read_bytes(), before["calls"])
        self.assertEqual(
            (report_path.stat().st_mtime_ns, report_path.read_bytes()), before["report"]
        )
        self.assertEqual(
            (marker_path.stat().st_mtime_ns, marker_path.read_bytes()), before["marker"]
        )

    def test_completed_replay_fails_closed_without_matching_startup_report(self) -> None:
        cases = (
            ("missing", lambda path: path.unlink(), "startup_report_missing_or_corrupt"),
            (
                "corrupt",
                lambda path: path.write_text("not-json\n", encoding="utf-8"),
                "startup_report_missing_or_corrupt",
            ),
            (
                "mismatched",
                lambda path: path.write_text(
                    json.dumps(
                        {
                            "schema_version": 1,
                            "report_kind": "finite_agent_startup",
                            "boot_mode": "recover_known_good",
                            "status": "completed",
                            "phase": "complete",
                            "operation_id_hash": "sha256:" + "0" * 64,
                        }
                    )
                    + "\n",
                    encoding="utf-8",
                ),
                "startup_report_terminal_mismatch",
            ),
        )
        for name, mutate, expected_code in cases:
            with self.subTest(name=name):
                fixture = RecoveryFixture()
                try:
                    first = fixture.run()
                    self.assertEqual(first.returncode, 0, first.stderr)
                    report_path = fixture.agent_home / "startup-report.json"
                    marker_path = fixture.operation_marker()
                    calls_before = fixture.call_log.read_bytes()
                    marker_before = marker_path.read_bytes()
                    mutate(report_path)
                    report_after_mutation = (
                        report_path.read_bytes() if report_path.exists() else None
                    )

                    replay = fixture.run()

                    self.assertEqual(replay.returncode, 65, replay.stderr)
                    self.assertIn(expected_code, replay.stderr)
                    self.assertNotIn(fixture.operation_id, replay.stdout + replay.stderr)
                    self.assertEqual(fixture.call_log.read_bytes(), calls_before)
                    self.assertEqual(marker_path.read_bytes(), marker_before)
                    self.assertEqual(
                        report_path.read_bytes() if report_path.exists() else None,
                        report_after_mutation,
                    )
                finally:
                    fixture.close()

    def test_interrupted_operation_resumes_idempotently(self) -> None:
        fixture = self.make_fixture()
        crash_env = {
            **fixture.env,
            "FINITE_RECOVER_CHAT_TESTING": "1",
            "FINITE_RECOVER_CHAT_TEST_FAILPOINT": "after_plugin_reinstall",
        }

        crashed = fixture.run(env=crash_env)

        self.assertEqual(crashed.returncode, 75, crashed.stderr)
        self.assertEqual(json.loads(fixture.operation_marker().read_text())["status"], "running")
        resumed = fixture.run()
        self.assertEqual(resumed.returncode, 0, resumed.stderr)
        report = json.loads(
            (fixture.agent_home / "startup-report.json").read_text(encoding="utf-8")
        )
        self.assertTrue(report["idempotency"]["resumed_after_interruption"])
        self.assertEqual(report["status"], "completed")
        self.assertEqual(sum("install" in call for call in fixture.calls()), 2)
        replay = fixture.run()
        self.assertEqual(replay.returncode, 0, replay.stderr)
        self.assertIn("FINITE_RECOVER_CHAT_NOOP", replay.stdout)

    def test_unknown_intent_version_refuses_without_running_commands(self) -> None:
        fixture = self.make_fixture()
        intent = json.loads(fixture.env["FINITE_AGENT_BOOT_INTENT_JSON"])
        intent["schema_version"] = 2
        env = {**fixture.env, "FINITE_AGENT_BOOT_INTENT_JSON": json.dumps(intent)}

        result = fixture.run(env=env)

        self.assertEqual(result.returncode, 65, result.stderr)
        report_text = (fixture.agent_home / "startup-report.json").read_text(encoding="utf-8")
        report = json.loads(report_text)
        self.assertEqual(report["error_code"], "boot_intent_version_unsupported")
        self.assertNotIn(fixture.operation_id, report_text)
        self.assertEqual(fixture.calls(), [])
        self.assertTrue((fixture.hermes_home / "plugins/finitechat/stale.py").exists())

    def test_missing_or_corrupt_durable_state_refuses_before_repair(self) -> None:
        cases = (
            (
                "missing_identity",
                "identity_missing_or_corrupt",
                lambda fixture: fixture.identity_path.unlink(),
            ),
            (
                "corrupt_agent_config",
                "agent_config_missing_or_corrupt",
                lambda fixture: (fixture.agent_home / "config.json").write_text(
                    "not-json\n", encoding="utf-8"
                ),
            ),
            (
                "corrupt_hermes_config",
                "generated_config_missing_or_corrupt",
                lambda fixture: fixture.config_path.write_text(
                    "model: [unterminated\n", encoding="utf-8"
                ),
            ),
            (
                "corrupt_client_store",
                "client_store_missing_or_corrupt",
                lambda fixture: fixture.client_store.write_bytes(b"not-sqlite"),
            ),
        )
        for name, code, corrupt in cases:
            with self.subTest(name=name):
                fixture = RecoveryFixture()
                try:
                    corrupt(fixture)
                    preserved = fixture.preservation_snapshot()
                    stale_plugin = (
                        fixture.hermes_home / "plugins/finitechat/stale.py"
                    ).read_bytes()

                    result = fixture.run()

                    self.assertEqual(result.returncode, 65, result.stderr)
                    report = json.loads(
                        (fixture.agent_home / "startup-report.json").read_text(encoding="utf-8")
                    )
                    self.assertEqual(report["status"], "refused")
                    self.assertEqual(report["error_code"], code)
                    self.assertEqual(fixture.preservation_snapshot(), preserved)
                    self.assertEqual(
                        (fixture.hermes_home / "plugins/finitechat/stale.py").read_bytes(),
                        stale_plugin,
                    )
                    calls = fixture.calls()
                    self.assertFalse(any("init" in call for call in calls))
                    self.assertFalse(any("install" in call for call in calls))
                    self.assertFalse(any("recover" in call for call in calls))
                finally:
                    fixture.close()

    def test_gateway_runs_recovery_before_fresh_initialization(self) -> None:
        fixture = self.make_fixture()
        fixture.config_path.unlink()
        gateway_env = {
            **fixture.env,
            "FINITE_RECOVER_CHAT_BOOT": str(RECOVERY_BOOT),
            "FINITE_HERMES_CONFIG_RECONCILER": str(RECONCILER),
            "FINITE_SERVER_URL": "http://127.0.0.1:18787",
            "FINITE_DEFAULT_INFERENCE_PROFILE": "openrouter",
        }

        result = subprocess.run(
            ["bash", str(GATEWAY), "--prepare-only"],
            env=gateway_env,
            capture_output=True,
            text=True,
            timeout=20,
            check=False,
        )

        self.assertEqual(result.returncode, 65, result.stderr)
        self.assertIn("generated_config_missing_or_corrupt", result.stderr)
        self.assertFalse(any("init" in call for call in fixture.calls()))
        self.assertFalse(any("install" in call for call in fixture.calls()))

    def test_canonical_and_standalone_images_package_recovery_boot(self) -> None:
        dockerfiles = (
            REPO_ROOT / "containers/agent/Dockerfile",
            REPO_ROOT.parent / "finitecomputer-v2/deploy/finite-computer/images/runtime.Dockerfile",
        )
        for dockerfile in dockerfiles:
            with self.subTest(dockerfile=dockerfile):
                contents = dockerfile.read_text(encoding="utf-8")
                self.assertIn(
                    "COPY finitechat/containers/agent/recover_chat_boot.py "
                    "/opt/recover_chat_boot.py",
                    contents,
                )
                self.assertGreaterEqual(contents.count("/opt/recover_chat_boot.py"), 2)


if __name__ == "__main__":
    unittest.main()

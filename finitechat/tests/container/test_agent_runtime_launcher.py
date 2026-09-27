"""Unit tests for the Agent Runtime gateway launcher and config reconciler.

These exercise `containers/agent/run_hermes_gateway.sh` and
`containers/agent/reconcile_hermes_config.py` directly on the host with stub
binaries. Both files ship in the one canonical Agent Runtime image
(`finitecomputer-v2/deploy/finite-computer/images/runtime.Dockerfile`); the
Docker-level proof of that image is `scripts/hermes-durable-home-docker-smoke.py`
run by `.github/workflows/hermes-runtime-smoke.yml` and `runtime-image.yml`.
"""

from __future__ import annotations

import importlib.util
import json
import os
import runpy
import subprocess
import sys
import tempfile
import unittest
from collections.abc import Mapping
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[2]
RECONCILER = REPO_ROOT / "containers/agent/reconcile_hermes_config.py"
AEON_BACKUP_NAME = "config.yaml.pre-aeon-vision-retirement"
# Obviously fake; production blocks held a literal worker key here.
FAKE_AEON_WORKER_KEY = "fake-retired-aeon-worker-key"


class AgentRuntimeLauncherConfigTest(unittest.TestCase):
    @staticmethod
    def _reconcile_config(
        existing: dict[str, Any] | None, settings: dict[str, str]
    ) -> dict[str, Any]:
        namespace = runpy.run_path(str(REPO_ROOT / "containers/agent/reconcile_hermes_config.py"))
        return namespace["reconcile_config"](existing, settings)

    @staticmethod
    def _reconciler_settings() -> dict[str, str]:
        return {
            "FINITE_CONFIG_MODEL": "glm-5-3-flash",
            "FINITE_CONFIG_PROVIDER": "custom",
            "FINITE_CONFIG_BASE_URL": "https://finite-private.finite.containers.tinfoil.dev/v1",
            "FINITE_CONFIG_CONTEXT_LENGTH": "393216",
            "FINITE_CONFIG_API_MODE": "chat_completions",
            "FINITE_CONFIG_API_KEY_REFERENCE": "${FINITE_PRIVATE_API_KEY}",
            "FINITE_CONFIG_TITLE_TIMEOUT_SECS": "2",
            "FINITE_CONFIG_WORKSPACE": "/workspace",
            "FINITE_CONFIG_PLUGIN_NAME": "finitechat",
            "FINITE_CONFIG_AGENT_HOME": "/data/agent",
            "FINITE_CONFIG_FINITECHAT_BIN": "/usr/local/bin/finitechat",
            "FINITE_CONFIG_SERVICE_ADDR": "127.0.0.1:4321",
            "FINITE_CONFIG_POLL_TIMEOUT_SECS": "30",
            "FINITE_CONFIG_POLL_LIMIT": "100",
        }

    @staticmethod
    def _retired_aeon_vision_blocks() -> list[dict[str, Any]]:
        """The two AEON shapes found in production, with and without extra_body."""
        blocks: list[dict[str, Any]] = []
        for model in (
            "nemotron-3-nano-omni-30b-a3b-reasoning-nvfp4-fast",
            "aeon-gemma-4-12b-k4-nvfp4-unified-fast",
        ):
            block: dict[str, Any] = {
                "provider": "custom",
                "base_url": "https://specialization.finite.vip/v1",
                "api_mode": "chat_completions",
                "model": model,
                "api_key": FAKE_AEON_WORKER_KEY,
                "timeout": 120,
                "download_timeout": 30,
            }
            blocks.append(block)
            blocks.append(
                {
                    **block,
                    "extra_body": {
                        "finite_specialization": {
                            "capabilities": {"audio": True, "image": True, "video": True},
                            "normalization_limits": {"max_image_pixels": 1048576},
                            "prompt_versions": {"image": "fake-prompt-v1"},
                        }
                    },
                }
            )
        return blocks

    @staticmethod
    def _load_config_text(text: str) -> dict[str, Any]:
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            import yaml

            return yaml.safe_load(text)

    @classmethod
    def _run_reconciler(cls, config_path: Path) -> None:
        result = subprocess.run(
            [sys.executable, str(RECONCILER), "--config", str(config_path)],
            env={**os.environ, **cls._reconciler_settings()},
            capture_output=True,
            text=True,
            timeout=15,
            check=False,
        )
        if result.returncode != 0:
            raise AssertionError(result.stderr)

    @staticmethod
    def _gateway_model(*, model: str, base_url: str) -> tuple[str, str]:
        launcher = REPO_ROOT / "containers/agent/run_hermes_gateway.sh"
        with tempfile.TemporaryDirectory() as raw_tmp:
            tmp = Path(raw_tmp)
            fake_bin = tmp / "bin"
            fake_bin.mkdir()
            finitechat = fake_bin / "finitechat"
            finitechat.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
            finitechat.chmod(0o755)
            capture = tmp / "capture.py"
            capture.write_text(
                "import os, pathlib\n"
                "pathlib.Path(os.environ['MODEL_CAPTURE']).write_text("
                "os.environ['FINITE_CONFIG_MODEL'] + '\\n' + "
                "os.environ['FINITE_CONFIG_BASE_URL'])\n",
                encoding="utf-8",
            )
            model_capture = tmp / "model.txt"
            agent_home = tmp / "agent"
            hermes_home = agent_home / "hermes-home"
            hermes_home.mkdir(parents=True)
            (agent_home / "config.json").write_text("{}\n", encoding="utf-8")
            env = {
                **os.environ,
                "FINITECHAT_BIN": str(finitechat),
                "FINITECHAT_HOME": str(agent_home),
                "HERMES_HOME": str(hermes_home),
                "FINITECHAT_WORKSPACE": str(tmp / "workspace"),
                "FINITE_DEFAULT_INFERENCE_PROFILE": "finite-private",
                "FINITECHAT_HERMES_MODEL": model,
                "FINITECHAT_HERMES_BASE_URL": base_url,
                "FINITE_PRIVATE_API_KEY": "fpk_live_test",
                "FINITE_HERMES_CONFIG_RECONCILER": str(capture),
                "MODEL_CAPTURE": str(model_capture),
            }

            result = subprocess.run(
                ["bash", str(launcher), "--prepare-only"],
                env=env,
                capture_output=True,
                text=True,
                timeout=15,
                check=False,
            )

            if result.returncode != 0:
                raise AssertionError(result.stderr)
            captured = model_capture.read_text(encoding="utf-8").splitlines()
            return captured[0], captured[1]

    def test_gateway_rewrites_historical_route_and_legacy_model(self) -> None:
        model, base_url = self._gateway_model(
            model="glm-5-2",
            base_url="https://kimi-k2-6.finite.containers.tinfoil.dev/v1",
        )
        self.assertEqual(model, "glm-5-3-flash")
        self.assertEqual(base_url, "https://finite-private.finite.containers.tinfoil.dev/v1")

    def test_gateway_rewrites_deepseek_label_on_the_historical_route(self) -> None:
        model, base_url = self._gateway_model(
            model="deepseek-v4-flash-0731",
            base_url="https://kimi-k2-6.finite.containers.tinfoil.dev/v1",
        )
        self.assertEqual(model, "glm-5-3-flash")
        self.assertEqual(base_url, "https://finite-private.finite.containers.tinfoil.dev/v1")

    def test_gateway_rewrites_dotted_glm_name_on_the_live_route(self) -> None:
        model, base_url = self._gateway_model(
            model="glm-5.3-flash",
            base_url="https://finite-private.finite.containers.tinfoil.dev/v1",
        )
        self.assertEqual(model, "glm-5-3-flash")
        self.assertEqual(base_url, "https://finite-private.finite.containers.tinfoil.dev/v1")

    def test_gateway_rewrites_historical_url_when_model_is_already_canonical(self) -> None:
        model, base_url = self._gateway_model(
            model="glm-5-3-flash",
            base_url="https://kimi-k2-6.finite.containers.tinfoil.dev/v1",
        )
        self.assertEqual(model, "glm-5-3-flash")
        self.assertEqual(base_url, "https://finite-private.finite.containers.tinfoil.dev/v1")

    def test_gateway_preserves_legacy_name_for_a_custom_endpoint(self) -> None:
        model, base_url = self._gateway_model(
            model="glm-5-2",
            base_url="https://inference.example.com/v1",
        )
        self.assertEqual(model, "glm-5-2")
        self.assertEqual(base_url, "https://inference.example.com/v1")

    def test_reconciler_seeds_current_finite_private_model_and_context(self) -> None:
        reconciled = self._reconcile_config(None, self._reconciler_settings())

        self.assertEqual(
            reconciled["model"],
            {
                "default": "glm-5-3-flash",
                "provider": "custom",
                "base_url": "https://finite-private.finite.containers.tinfoil.dev/v1",
                "context_length": 393216,
                "api_mode": "chat_completions",
                "api_key": "${FINITE_PRIVATE_API_KEY}",
                "supports_vision": True,
            },
        )

    def test_reconciler_restores_missing_managed_vision_default_without_replacing_preferences(
        self,
    ) -> None:
        existing = self._reconcile_config(None, self._reconciler_settings())
        # Absence means the product default, including deletion after adoption.
        existing["model"].pop("supports_vision", None)
        existing["model"]["temperature"] = 0.4
        existing["auxiliary"]["vision"] = {
            "provider": "custom",
            "base_url": "https://old-vision.example/v1",
            "model": "old-vision-model",
        }
        before = json.loads(json.dumps(existing))

        reconciled = self._reconcile_config(existing, self._reconciler_settings())

        expected = json.loads(json.dumps(before))
        expected["model"]["supports_vision"] = True
        self.assertEqual(reconciled, expected)
        self.assertEqual(existing, before)
        self.assertEqual(
            self._reconcile_config(reconciled, self._reconciler_settings()), reconciled
        )

    def test_reconciler_preserves_explicit_vision_and_routing_overrides(self) -> None:
        for capability in (False, True, "false", None):
            with self.subTest(capability=capability):
                existing = self._reconcile_config(None, self._reconciler_settings())
                existing["model"]["supports_vision"] = capability
                existing["agent"] = {"image_input_mode": "text"}
                self.assertEqual(
                    self._reconcile_config(existing, self._reconciler_settings()), existing
                )

    def test_reconciler_does_not_assume_vision_for_other_models_or_providers(self) -> None:
        for key, value in (
            ("default", "future-model"),
            ("provider", "openrouter"),
            ("base_url", "https://inference.example/v1"),
            ("api_key", "${USER_INFERENCE_KEY}"),
            ("api_mode", "anthropic_messages"),
        ):
            with self.subTest(key=key):
                existing = self._reconcile_config(None, self._reconciler_settings())
                existing["model"].pop("supports_vision", None)
                existing["model"][key] = value
                reconciled = self._reconcile_config(existing, self._reconciler_settings())
                self.assertEqual(reconciled, existing)

    def test_reconciler_preserves_provider_capability_overrides(self) -> None:
        for field in ("supports_vision", "vision"):
            for legacy in (False, True):
                with self.subTest(field=field, legacy=legacy):
                    existing = self._reconcile_config(None, self._reconciler_settings())
                    existing["model"].pop("supports_vision")
                    provider = {"models": {"glm-5-3-flash": {field: False}}}
                    if legacy:
                        existing["custom_providers"] = [{"name": "custom", **provider}]
                    else:
                        existing["providers"] = {"custom": provider}
                    self.assertEqual(
                        self._reconcile_config(existing, self._reconciler_settings()), existing
                    )

    def test_reconciler_does_not_seed_vision_for_another_model(self) -> None:
        settings = self._reconciler_settings()
        settings["FINITE_CONFIG_MODEL"] = "future-model"
        self.assertNotIn("supports_vision", self._reconcile_config(None, settings)["model"])

    def test_reconciler_migrates_only_the_legacy_finite_private_default(self) -> None:
        existing = {
            "model": {
                "default": "glm-5-2",
                "provider": "custom",
                "base_url": "https://kimi-k2-6.finite.containers.tinfoil.dev/v1",
                "api_mode": "chat_completions",
                "api_key": "${FINITE_PRIVATE_API_KEY}",
                "temperature": 0.4,
            }
        }

        reconciled = self._reconcile_config(existing, self._reconciler_settings())

        self.assertEqual(reconciled["model"]["default"], "glm-5-3-flash")
        self.assertEqual(
            reconciled["model"]["base_url"],
            "https://finite-private.finite.containers.tinfoil.dev/v1",
        )
        self.assertEqual(reconciled["model"]["context_length"], 393216)
        self.assertEqual(reconciled["model"]["temperature"], 0.4)
        self.assertIs(reconciled["model"]["supports_vision"], True)

    def test_reconciler_migrates_deepseek_image_owned_default(self) -> None:
        existing = {
            "model": {
                "default": "deepseek-v4-flash-0731",
                "provider": "custom",
                "base_url": "https://kimi-k2-6.finite.containers.tinfoil.dev/v1",
                "api_mode": "chat_completions",
                "api_key": "${FINITE_PRIVATE_API_KEY}",
            }
        }

        reconciled = self._reconcile_config(existing, self._reconciler_settings())

        self.assertEqual(reconciled["model"]["default"], "glm-5-3-flash")
        self.assertEqual(
            reconciled["model"]["base_url"],
            "https://finite-private.finite.containers.tinfoil.dev/v1",
        )

    def test_reconciler_preserves_user_selected_model(self) -> None:
        model = {
            "default": "openai/gpt-5",
            "provider": "openrouter",
            "api_key": "${OPENROUTER_API_KEY}",
        }

        reconciled = self._reconcile_config({"model": model.copy()}, self._reconciler_settings())

        self.assertEqual(reconciled["model"], model)

    def test_reconciler_rewrites_retired_url_but_keeps_a_non_alias_model(self) -> None:
        existing = {
            "model": {
                "default": "user-chosen-model",
                "provider": "custom",
                "base_url": "https://kimi-k2-6.finite.containers.tinfoil.dev/v1",
                "api_mode": "chat_completions",
                "api_key": "${FINITE_PRIVATE_API_KEY}",
            }
        }

        reconciled = self._reconcile_config(existing, self._reconciler_settings())

        self.assertEqual(reconciled["model"]["default"], "user-chosen-model")
        self.assertEqual(
            reconciled["model"]["base_url"],
            "https://finite-private.finite.containers.tinfoil.dev/v1",
        )

    def test_reconciler_preserves_near_match_with_custom_route(self) -> None:
        model = {
            "default": "glm-5-2",
            "provider": "custom",
            "base_url": "https://inference.example.com/v1",
            "api_mode": "chat_completions",
            "api_key": "${FINITE_PRIVATE_API_KEY}",
        }

        reconciled = self._reconcile_config({"model": model.copy()}, self._reconciler_settings())

        self.assertEqual(reconciled["model"], model)

    def test_reconciler_retires_deleted_aeon_vision_backend(self) -> None:
        settings = self._reconciler_settings()
        for block in self._retired_aeon_vision_blocks():
            for other_auxiliary in (True, False):
                with self.subTest(
                    model=block["model"],
                    extra_body="extra_body" in block,
                    other_auxiliary=other_auxiliary,
                ):
                    existing = self._reconcile_config(None, settings)
                    existing["model"].pop("supports_vision")
                    if not other_auxiliary:
                        existing.pop("auxiliary")
                    existing.setdefault("auxiliary", {})["vision"] = json.loads(json.dumps(block))
                    before = json.loads(json.dumps(existing))

                    reconciled = self._reconcile_config(existing, settings)

                    expected = json.loads(json.dumps(before))
                    expected["model"]["supports_vision"] = True
                    del expected["auxiliary"]["vision"]
                    if not other_auxiliary:
                        del expected["auxiliary"]
                    self.assertEqual(reconciled, expected)
                    self.assertNotIn(FAKE_AEON_WORKER_KEY, json.dumps(reconciled))
                    self.assertEqual(existing, before)
                    self.assertEqual(self._reconcile_config(reconciled, settings), reconciled)

    def test_reconciler_preserves_non_aeon_auxiliary_vision_blocks(self) -> None:
        for block in (
            {
                "provider": "openrouter",
                "base_url": "https://openrouter.ai/api/v1",
                "model": "anthropic/claude-opus-5.5",
                "api_key": "${OPENROUTER_API_KEY}",
                "timeout": 120,
                "download_timeout": 30,
            },
            {"provider": "xai", "model": "grok-4.5"},
            {"download_timeout": 30, "timeout": 120},
        ):
            with self.subTest(block=block):
                existing = self._reconcile_config(None, self._reconciler_settings())
                existing["model"].pop("supports_vision")
                existing["auxiliary"]["vision"] = dict(block)

                reconciled = self._reconcile_config(existing, self._reconciler_settings())

                self.assertEqual(json.dumps(reconciled["auxiliary"]["vision"]), json.dumps(block))
                self.assertIs(reconciled["model"]["supports_vision"], True)

    def test_reconciler_leaves_aeon_near_matches_user_owned(self) -> None:
        aeon = self._retired_aeon_vision_blocks()[1]
        near_matches: dict[str, dict[str, Any]] = {
            "unknown key": {**aeon, "temperature": 0.2},
            "unknown extra_body key": {
                **aeon,
                "extra_body": {**aeon["extra_body"], "user_option": True},
            },
            "unknown specialization key": {
                **aeon,
                "extra_body": {
                    "finite_specialization": {
                        **aeon["extra_body"]["finite_specialization"],
                        "user_option": True,
                    }
                },
            },
            "other model": {**aeon, "model": "user-vision-model"},
            "other provider": {**aeon, "provider": "openrouter"},
            "other host": {**aeon, "base_url": "https://specialization.finite.vip.example/v1"},
            "other port": {**aeon, "base_url": "https://specialization.finite.vip:8443/v1"},
            "plain http": {**aeon, "base_url": "http://specialization.finite.vip/v1"},
        }
        for label, block in near_matches.items():
            with self.subTest(label):
                existing = self._reconcile_config(None, self._reconciler_settings())
                existing["model"].pop("supports_vision")
                existing["auxiliary"]["vision"] = json.loads(json.dumps(block))

                reconciled = self._reconcile_config(existing, self._reconciler_settings())

                self.assertEqual(reconciled["auxiliary"]["vision"], block)

    def test_reconciler_never_retires_aeon_backend_on_first_seed(self) -> None:
        reconcile_config = runpy.run_path(str(RECONCILER))["reconcile_config"]

        def forbidden(config: dict[str, Any]) -> None:
            raise AssertionError("first seed must not run existing-config migrations")

        # run_path returns a copy; patch the globals the function resolves.
        reconcile_config.__globals__["_migrate_retired_aeon_vision_override"] = forbidden
        settings = self._reconciler_settings()
        seeded = reconcile_config(None, settings)
        self.assertNotIn("vision", seeded["auxiliary"])
        with self.assertRaisesRegex(AssertionError, "first seed"):
            reconcile_config(seeded, settings)

        with tempfile.TemporaryDirectory() as raw_tmp:
            config_path = Path(raw_tmp) / "config.yaml"
            self._run_reconciler(config_path)
            self.assertTrue(config_path.exists())
            self.assertFalse((config_path.parent / AEON_BACKUP_NAME).exists())

    def test_reconciler_keeps_one_rollback_copy_of_the_retired_aeon_config(self) -> None:
        nemotron, _, gemma, _ = self._retired_aeon_vision_blocks()
        with tempfile.TemporaryDirectory() as raw_tmp:
            config_path = Path(raw_tmp) / "config.yaml"
            backup = config_path.parent / AEON_BACKUP_NAME
            existing = self._reconcile_config(None, self._reconciler_settings())
            existing["model"].pop("supports_vision")
            existing["auxiliary"]["vision"] = nemotron
            original = (json.dumps(existing, indent=2) + "\n").encode()
            config_path.write_bytes(original)
            config_path.chmod(0o640)

            self._run_reconciler(config_path)

            retired = self._load_config_text(config_path.read_text(encoding="utf-8"))
            self.assertNotIn("vision", retired["auxiliary"])
            self.assertIs(retired["model"]["supports_vision"], True)
            self.assertEqual(config_path.stat().st_mode & 0o777, 0o640)
            self.assertEqual(backup.read_bytes(), original)
            self.assertEqual(backup.stat().st_mode & 0o777, 0o600)
            self.assertEqual(
                sorted(path.name for path in config_path.parent.iterdir()),
                sorted(["config.yaml", AEON_BACKUP_NAME]),
            )

            # Second boot: nothing left to retire, so neither file is rewritten.
            config_stat = config_path.stat()
            retired_bytes = config_path.read_bytes()
            backup_stat = backup.stat()
            self._run_reconciler(config_path)
            self.assertEqual(config_path.read_bytes(), retired_bytes)
            self.assertEqual(
                (config_path.stat().st_ino, config_path.stat().st_mtime_ns),
                (config_stat.st_ino, config_stat.st_mtime_ns),
            )
            self.assertEqual(
                (backup.stat().st_ino, backup.stat().st_mtime_ns),
                (backup_stat.st_ino, backup_stat.st_mtime_ns),
            )

            # A reintroduced block (for example a whole-file restore) is retired
            # again, but the first rollback copy is never overwritten.
            existing["auxiliary"]["vision"] = gemma
            config_path.write_text(json.dumps(existing, indent=2) + "\n", encoding="utf-8")
            self._run_reconciler(config_path)
            again = self._load_config_text(config_path.read_text(encoding="utf-8"))
            self.assertNotIn("vision", again["auxiliary"])
            self.assertEqual(backup.read_bytes(), original)
            self.assertEqual(backup.stat().st_mode & 0o777, 0o600)

    def test_reconciler_seeds_finitechat_display_defaults_without_touching_other_platforms(
        self,
    ) -> None:
        existing = {
            "display": {
                "streaming": True,
                "platforms": {
                    "telegram": {
                        "streaming": True,
                        "tool_progress_grouping": "accumulate",
                    }
                },
            }
        }

        reconciled = self._reconcile_config(existing, self._reconciler_settings())

        self.assertTrue(reconciled["display"]["streaming"])
        self.assertEqual(
            reconciled["display"]["platforms"]["telegram"],
            existing["display"]["platforms"]["telegram"],
        )
        self.assertEqual(
            reconciled["display"]["platforms"]["finitechat"],
            {
                "streaming": False,
                "tool_progress_grouping": "separate",
            },
        )

    def test_reconciler_repairs_incompatible_finitechat_display_overrides(self) -> None:
        finitechat_display = {
            "streaming": True,
            "tool_progress_grouping": "accumulate",
            "interim_assistant_messages": True,
            "custom_user_setting": "preserved",
        }
        existing = {"display": {"platforms": {"finitechat": finitechat_display.copy()}}}

        reconciled = self._reconcile_config(existing, self._reconciler_settings())

        self.assertEqual(
            reconciled["display"]["platforms"]["finitechat"],
            {
                "streaming": False,
                "tool_progress_grouping": "separate",
                "interim_assistant_messages": True,
                "custom_user_setting": "preserved",
            },
        )

    def test_gateway_launcher_does_not_persist_raw_finite_private_key(self) -> None:
        script = (REPO_ROOT / "containers/agent/run_hermes_gateway.sh").read_text(encoding="utf-8")

        self.assertIn("api_key_reference='${FINITE_PRIVATE_API_KEY}'", script)
        self.assertIn("api_key_reference='${FINITECHAT_HERMES_API_KEY}'", script)
        self.assertNotIn('FINITE_CONFIG_API_KEY_REFERENCE="$api_key"', script)

    def test_gateway_launcher_waits_for_welcome_instead_of_inventing_a_room(self) -> None:
        script = (REPO_ROOT / "containers/agent/run_hermes_gateway.sh").read_text(encoding="utf-8")

        self.assertIn("Room admission is Welcome-first", script)
        self.assertNotIn('hermes --agent-home "$agent_home" invite', script)
        self.assertNotIn("home-channel show", script)
        self.assertNotIn("home-channel set", script)
        self.assertNotIn("invite_room_id", script)
        self.assertIn('FINITE_CONFIG_HOME_CHANNEL="${FINITECHAT_HOME_CHANNEL:-}"', script)
        self.assertNotIn("gateway_home_channel_yaml", script)

    def test_gateway_launcher_has_agentd_prepare_and_supervised_modes(self) -> None:
        script = (REPO_ROOT / "containers/agent/run_hermes_gateway.sh").read_text(encoding="utf-8")

        prepared = script.index('if [[ "${1:-}" == "--prepare-only" ]]')
        health = script.index("python /opt/health_server.py &", prepared)
        gateway = script.index("exec hermes gateway run --replace", health)
        self.assertLess(prepared, health)
        self.assertLess(health, gateway)
        self.assertIn('"${FINITE_AGENTD_SUPERVISED:-0}" != "1"', script)

    def test_gateway_launcher_seeds_managed_skills_only_for_fresh_agents(self) -> None:
        script = (REPO_ROOT / "containers/agent/run_hermes_gateway.sh").read_text(encoding="utf-8")

        fresh_agent_branch = script.index('if [[ ! -f "${agent_home}/config.json" ]]')
        seed = script.index('cp -a "${bundled_skills_dir}/."', fresh_agent_branch)
        init = script.index('"$finitechat_bin" hermes --agent-home "$agent_home" init', seed)
        branch_end = script.index("\nfi\n", init)
        self.assertLess(fresh_agent_branch, seed)
        self.assertLess(seed, init)
        self.assertLess(init, branch_end)
        self.assertIn("managed-skills/finite/current", script)
        self.assertIn('FINITE_CONFIG_MANAGED_SKILLS_DIR="$managed_skills_config_dir"', script)
        self.assertNotIn("HERMES_BUNDLED_SKILLS", script)

    def test_gateway_launcher_seed_is_durable_and_does_not_touch_existing_agents(self) -> None:
        launcher = REPO_ROOT / "containers/agent/run_hermes_gateway.sh"

        with tempfile.TemporaryDirectory() as raw_tmp:
            tmp = Path(raw_tmp)
            fake_bin = tmp / "bin"
            fake_bin.mkdir()
            finitechat = fake_bin / "finitechat"
            finitechat.write_text(
                """#!/usr/bin/env bash
set -euo pipefail
printf '%s\\n' "$*" >>"${FAKE_CALL_LOG}"
case " $* " in
  *" init "*) printf '{}\\n' >"${FINITECHAT_HOME}/config.json" ;;
  *" invite "*) printf '{"room_id":"room-1","url":"finite://join?test=1"}\\n' ;;
  *" home-channel show "*) printf '{"home_channel":{"room_id":"room-1"}}\\n' ;;
  *) printf '{}\\n' ;;
esac
""",
                encoding="utf-8",
            )
            finitechat.chmod(0o755)
            hermes = fake_bin / "hermes"
            hermes.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
            hermes.chmod(0o755)
            python = fake_bin / "python"
            python.write_text(
                f"""#!/usr/bin/env bash
if [[ "${{1:-}}" == "/opt/health_server.py" ]]; then
  exit 0
fi
exec {sys.executable!s} "$@"
""",
                encoding="utf-8",
            )
            python.chmod(0o755)

            bundle = tmp / "bundle"
            bundled_skill = bundle / "software-development/finitebrain/SKILL.md"
            bundled_skill.parent.mkdir(parents=True)
            bundled_skill.write_text("baseline-v1\n", encoding="utf-8")
            agent_home = tmp / "fresh-agent"
            user_skill = agent_home / "hermes-home/skills/user-skill/SKILL.md"
            user_skill.parent.mkdir(parents=True)
            user_skill.write_text("user-owned\n", encoding="utf-8")
            call_log = tmp / "calls.log"
            env = {
                **os.environ,
                "PATH": f"{fake_bin}:{os.environ['PATH']}",
                "FAKE_CALL_LOG": str(call_log),
                "FINITECHAT_BIN": str(finitechat),
                "FINITECHAT_HOME": str(agent_home),
                "HERMES_HOME": str(agent_home / "hermes-home"),
                "FINITECHAT_WORKSPACE": str(tmp / "workspace"),
                "FINITE_SERVER_URL": "http://127.0.0.1:9",
                "FINITE_DEFAULT_INFERENCE_PROFILE": "openrouter",
                "FINITE_BUNDLED_SKILLS_DIR": str(bundle),
                "FINITE_REQUIRE_BUNDLED_SKILLS": "1",
                "FINITE_HERMES_CONFIG_RECONCILER": str(
                    REPO_ROOT / "containers/agent/reconcile_hermes_config.py"
                ),
            }

            first = subprocess.run(
                ["bash", str(launcher)],
                env=env,
                capture_output=True,
                text=True,
                timeout=15,
                check=False,
            )
            self.assertEqual(first.returncode, 0, first.stderr)
            installed_skill = (
                agent_home
                / "managed-skills/finite/current/software-development/finitebrain/SKILL.md"
            )
            self.assertEqual(installed_skill.read_text(encoding="utf-8"), "baseline-v1\n")
            config_path = agent_home / "hermes-home/config.yaml"
            config = config_path.read_text(encoding="utf-8")
            self.assertIn(str(agent_home / "managed-skills/finite/current"), config)
            self.assertEqual(config_path.stat().st_mode & 0o777, 0o600)
            self.assertEqual(user_skill.read_text(encoding="utf-8"), "user-owned\n")

            # Simulate Hermes/user-owned edits before the container restarts.
            # JSON is valid YAML and keeps this focused test independent of
            # whether the host Python has the runtime's PyYAML dependency.
            try:
                config_data = json.loads(config)
            except json.JSONDecodeError:
                import yaml

                config_data = yaml.safe_load(config)
            expected_model = {
                "default": "openai/gpt-5",
                "provider": "openrouter",
                "api_key": "${OPENROUTER_API_KEY}",
            }
            expected_platforms = {
                "telegram": {
                    "enabled": True,
                    "bot_token": "${TELEGRAM_BOT_TOKEN}",
                    "allowed_user_ids": [1234],
                }
            }
            config_data["model"] = expected_model
            config_data["platforms"] = expected_platforms
            config_data["plugins"]["enabled"].append("user-plugin")
            config_data["skills"]["external_dirs"].append("/data/user-skills")
            config_path.write_text(json.dumps(config_data, indent=2) + "\n", encoding="utf-8")

            bundled_skill.write_text("baseline-v2\n", encoding="utf-8")
            env["FINITECHAT_HERMES_MODEL"] = "environment-must-not-overwrite-durable-config"
            second = subprocess.run(
                ["bash", str(launcher)],
                env=env,
                capture_output=True,
                text=True,
                timeout=15,
                check=False,
            )
            self.assertEqual(second.returncode, 0, second.stderr)
            self.assertEqual(installed_skill.read_text(encoding="utf-8"), "baseline-v1\n")
            self.assertEqual(call_log.read_text(encoding="utf-8").count(" init "), 1)
            restarted_config = json.loads(config_path.read_text(encoding="utf-8"))
            self.assertEqual(restarted_config["model"], expected_model)
            self.assertEqual(restarted_config["platforms"], expected_platforms)
            self.assertIn("user-plugin", restarted_config["plugins"]["enabled"])
            self.assertIn("/data/user-skills", restarted_config["skills"]["external_dirs"])

            existing_home = tmp / "existing-agent"
            existing_home.mkdir()
            (existing_home / "config.json").write_text("{}\n", encoding="utf-8")
            existing_hermes_home = existing_home / "hermes-home"
            existing_hermes_home.mkdir()
            (existing_hermes_home / "config.yaml").write_text(
                json.dumps({"model": expected_model}) + "\n",
                encoding="utf-8",
            )
            existing_env = {
                **env,
                "FINITECHAT_HOME": str(existing_home),
                "HERMES_HOME": str(existing_hermes_home),
                "FINITE_DEFAULT_INFERENCE_PROFILE": "finite-private",
            }
            existing_env.pop("FINITE_PRIVATE_API_KEY", None)
            existing_env.pop("FINITECHAT_HERMES_API_KEY", None)
            existing = subprocess.run(
                ["bash", str(launcher)],
                env=existing_env,
                capture_output=True,
                text=True,
                timeout=15,
                check=False,
            )
            self.assertEqual(existing.returncode, 0, existing.stderr)
            self.assertFalse((existing_home / "managed-skills").exists())
            existing_config = (existing_hermes_home / "config.yaml").read_text(encoding="utf-8")
            self.assertNotIn("external_dirs", existing_config)
            try:
                existing_config_data = json.loads(existing_config)
            except json.JSONDecodeError:
                import yaml

                existing_config_data = yaml.safe_load(existing_config)
            self.assertEqual(existing_config_data["model"], expected_model)

    def _gateway_chat_authz_env(
        self,
        *,
        supervised: bool = True,
        owner_npubs: str | None = None,
        allowed_users: list[str] | None = None,
        seed_writes_mirror: bool = False,
        seed_fails: bool = False,
    ) -> dict[str, str]:
        """Run the launcher with stub binaries and capture the chat-authz env
        the gateway process would inherit. The runner always injects
        FINITECHAT_ALLOW_ALL_USERS=true for old-image compatibility.

        Under agentd supervision (the production topology) agentd has already
        run `finitechat hermes admission seed` before starting this script, so
        the launcher only reads the allowed-users mirror. Standalone mode
        (FINITE_AGENTD_SUPERVISED unset) makes the launcher run the seed step
        itself; the stub finitechat emulates it by consuming the env seed and
        publishing the mirror."""
        launcher = REPO_ROOT / "containers/agent/run_hermes_gateway.sh"
        with tempfile.TemporaryDirectory() as raw_tmp:
            tmp = Path(raw_tmp)
            fake_bin = tmp / "bin"
            fake_bin.mkdir()
            seed_stub = tmp / "seed-stub.env"
            seed_stub.write_text(
                f"SEED_WRITES={1 if seed_writes_mirror else 0}\n"
                f"SEED_FAILS={1 if seed_fails else 0}\n",
                encoding="utf-8",
            )
            finitechat = fake_bin / "finitechat"
            finitechat.write_text(
                "#!/usr/bin/env bash\n"
                f". {seed_stub}\n"
                "# Emulate `hermes admission seed`: consume the env seed and\n"
                "# publish the store's allowed-users mirror.\n"
                'if [[ "${1:-}" == "hermes" && "${4:-}" == "admission" ]]; then\n'
                '  if [[ "$SEED_FAILS" == "1" ]]; then exit 1; fi\n'
                '  if [[ "$SEED_WRITES" == "1" ]]; then\n'
                '    seed="${FINITECHAT_WELCOME_ALLOWLIST:-${FINITECHAT_OWNER_NPUBS:-}}"\n'
                '    if [[ -n "$seed" ]]; then\n'
                '      printf "%s\\n" ${seed//,/ } > "${FINITECHAT_HOME}/allowed-users"\n'
                "    fi\n"
                "  fi\n"
                "fi\n"
                "exit 0\n",
                encoding="utf-8",
            )
            finitechat.chmod(0o755)
            env_capture = tmp / "gateway.env"
            hermes = fake_bin / "hermes"
            hermes.write_text(
                "#!/usr/bin/env bash\n"
                "for key in FINITECHAT_ALLOW_ALL_USERS FINITE_ALLOW_ALL_USERS"
                " GATEWAY_ALLOW_ALL_USERS FINITECHAT_ALLOWED_USERS"
                " FINITECHAT_WELCOME_ALLOWLIST FINITECHAT_OWNER_NPUBS; do\n"
                '  if [[ -v $key ]]; then printf \'%s=%s\\n\' "$key" "${!key}"'
                f" >>{env_capture}; fi\n"
                "done\n",
                encoding="utf-8",
            )
            hermes.chmod(0o755)
            python = fake_bin / "python"
            python.write_text(
                f'#!/usr/bin/env bash\nexec {sys.executable!s} "$@"\n',
                encoding="utf-8",
            )
            python.chmod(0o755)
            agent_home = tmp / "agent"
            hermes_home = agent_home / "hermes-home"
            hermes_home.mkdir(parents=True)
            (agent_home / "config.json").write_text("{}\n", encoding="utf-8")
            if allowed_users is not None:
                # The sidecar-maintained mirror of the store's Welcome
                # allowlist: one 64-hex account id per line.
                (agent_home / "allowed-users").write_text(
                    "".join(f"{entry}\n" for entry in allowed_users),
                    encoding="utf-8",
                )
            env = {
                **os.environ,
                "PATH": f"{fake_bin}:{os.environ['PATH']}",
                "FINITECHAT_BIN": str(finitechat),
                "FINITECHAT_HOME": str(agent_home),
                "HERMES_HOME": str(hermes_home),
                "FINITECHAT_WORKSPACE": str(tmp / "workspace"),
                "FINITE_DEFAULT_INFERENCE_PROFILE": "openrouter",
                "FINITE_AGENTD_SUPERVISED": "1" if supervised else "0",
                "FINITECHAT_ALLOW_ALL_USERS": "true",
                "FINITE_ALLOW_ALL_USERS": "true",
                "GATEWAY_ALLOW_ALL_USERS": "true",
                "FINITE_HERMES_CONFIG_RECONCILER": str(
                    REPO_ROOT / "containers/agent/reconcile_hermes_config.py"
                ),
            }
            if owner_npubs is not None:
                env["FINITECHAT_OWNER_NPUBS"] = owner_npubs
            else:
                env.pop("FINITECHAT_OWNER_NPUBS", None)

            result = subprocess.run(
                ["bash", str(launcher)],
                env=env,
                capture_output=True,
                text=True,
                timeout=15,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            captured: dict[str, str] = {}
            if env_capture.exists():
                for line in env_capture.read_text(encoding="utf-8").splitlines():
                    key, _, value = line.partition("=")
                    captured[key] = value
            return captured

    def test_gateway_launcher_mirror_scopes_gateway_admission(self) -> None:
        """Under agentd the seed already ran and the store mirrored its
        allowlist: the gateway locks to exactly those entries."""
        owner = "a" * 64
        guest = "b" * 64
        captured = self._gateway_chat_authz_env(allowed_users=[owner, guest])

        self.assertEqual(captured.get("FINITECHAT_ALLOWED_USERS"), f"{owner},{guest}")
        # The mirror is a gateway concern only; the sidecar's store is the
        # source of truth and needs no env re-seed.
        self.assertNotIn("FINITECHAT_WELCOME_ALLOWLIST", captured)
        self.assertNotIn("FINITECHAT_ALLOW_ALL_USERS", captured)
        self.assertNotIn("FINITE_ALLOW_ALL_USERS", captured)
        self.assertNotIn("GATEWAY_ALLOW_ALL_USERS", captured)

    def test_gateway_launcher_without_mirror_keeps_legacy_allow_all(self) -> None:
        """No mirrored admission state means the legacy allow-all delegation
        stands. The launcher itself must not key on the birth-time
        FINITECHAT_OWNER_NPUBS: seeding belongs to the seed step, not the
        gateway launcher."""
        owner = "a" * 64
        captured = self._gateway_chat_authz_env(owner_npubs=owner)

        self.assertEqual(captured.get("FINITECHAT_ALLOW_ALL_USERS"), "true")
        self.assertNotIn("FINITECHAT_ALLOWED_USERS", captured)
        self.assertNotIn("FINITECHAT_WELCOME_ALLOWLIST", captured)
        self.assertNotIn("FINITE_ALLOW_ALL_USERS", captured)
        self.assertNotIn("GATEWAY_ALLOW_ALL_USERS", captured)

    def test_gateway_launcher_empty_mirror_keeps_legacy_allow_all(self) -> None:
        """An empty mirror must fail open to the same legacy behavior as a
        missing one: the sidecar writes no file until admission is locked."""
        captured = self._gateway_chat_authz_env(allowed_users=[])

        self.assertEqual(captured.get("FINITECHAT_ALLOW_ALL_USERS"), "true")
        self.assertNotIn("FINITECHAT_ALLOWED_USERS", captured)

    def test_gateway_launcher_standalone_mode_seeds_admission(self) -> None:
        """Without agentd the launcher runs the seed step itself; the seed
        consumed the birth env and published the mirror, and the gateway
        locks to it."""
        owner = "a" * 64
        captured = self._gateway_chat_authz_env(
            supervised=False,
            owner_npubs=owner,
            seed_writes_mirror=True,
        )

        self.assertEqual(captured.get("FINITECHAT_ALLOWED_USERS"), owner)
        self.assertNotIn("FINITECHAT_ALLOW_ALL_USERS", captured)
        self.assertNotIn("GATEWAY_ALLOW_ALL_USERS", captured)

    def test_gateway_launcher_standalone_seed_failure_falls_back_to_allow_all(self) -> None:
        """A failing seed step must not wedge the gateway: the legacy
        allow-all delegation stands and the launcher still boots."""
        owner = "a" * 64
        captured = self._gateway_chat_authz_env(
            supervised=False,
            owner_npubs=owner,
            seed_fails=True,
        )

        self.assertEqual(captured.get("FINITECHAT_ALLOW_ALL_USERS"), "true")
        self.assertNotIn("FINITECHAT_ALLOWED_USERS", captured)

    def test_gateway_launcher_fails_closed_without_replacing_invalid_config(self) -> None:
        reconciler = REPO_ROOT / "containers/agent/reconcile_hermes_config.py"
        with tempfile.TemporaryDirectory() as raw_tmp:
            config_path = Path(raw_tmp) / "config.yaml"
            invalid = "model: [unterminated\n"
            config_path.write_text(invalid, encoding="utf-8")
            env = {
                **os.environ,
                "FINITE_CONFIG_PLUGIN_NAME": "finitechat",
            }

            result = subprocess.run(
                [sys.executable, str(reconciler), "--config", str(config_path)],
                env=env,
                capture_output=True,
                text=True,
                timeout=15,
                check=False,
            )

            self.assertEqual(result.returncode, 64)
            self.assertIn("unsafe Hermes config", result.stderr)
            self.assertEqual(config_path.read_text(encoding="utf-8"), invalid)


FAKE_FP_KEY = "fp-FAKE-finite-private-key"
FP_PRODUCT_URL = "https://finite-private.finite.containers.tinfoil.dev/v1"
NEUTRAL_CODEX_HOME = "/dev/null/finite-codex-home-disabled"
RECORDED_ENV = (
    "OPENAI_API_KEY",
    "CODEX_HOME",
    "HERMES_HOME",
    "FINITE_CONFIG_FP_MODEL",
    "FINITE_CONFIG_FP_BASE_URL",
    "FINITE_CONFIG_FP_CONTEXT_LENGTH",
    "FINITE_CONFIG_FP_KEY_PRESENT",
    "FINITE_CONFIG_FP_FALLBACK_MODE",
)


def _helper_module_available() -> bool:
    try:
        return importlib.util.find_spec("hermes_cli.finite_inference_helper") is not None
    except ImportError:
        return False


class AgentRuntimeLauncherInferenceTest(unittest.TestCase):
    """Finite Private settings, the §5.6 launch rule, and the pending-disconnect step.

    The launcher runs for real against the real reconciler; `python -m
    hermes_cli.finite_inference_helper`, `timeout`, `hermes`, and `finitechat`
    are stubs that record what they saw into one ordered event log.
    """

    def _launch(
        self,
        tmp: Path,
        *,
        env: Mapping[str, str | None] | None = None,
        args: tuple[str, ...] = (),
        intent: dict[str, Any] | None = None,
        config: dict[str, Any] | None = None,
        helper_status: int = 0,
        timeout_status: int | None = None,
        real_helper: bool = False,
    ) -> tuple[subprocess.CompletedProcess[str], list[str], dict[str, dict[str, str]]]:
        fake_bin = tmp / "bin"
        fake_bin.mkdir()
        events = tmp / "events.log"
        records = tmp / "records"
        records.mkdir()
        record_env = (
            f"for key in {' '.join(RECORDED_ENV)}; do\n"
            '  if [[ -v $key ]]; then printf \'%s=%s\\n\' "$key" "${!key}"; fi\n'
            f'done >"{records}/$1.env"\n'
        )
        stubs = {
            "finitechat": "exit 0\n",
            "hermes": (
                f"record() {{\n{record_env}}}\n"
                "record hermes\n"
                f'printf "hermes %s\\n" "$*" >>"{events}"\n'
            ),
            "timeout": (
                f'printf "timeout %s\\n" "$*" >>"{events}"\n'
                'if [[ -n "${FAKE_TIMEOUT_STATUS:-}" ]]; then exit "$FAKE_TIMEOUT_STATUS"; fi\n'
                'while [[ "${1:-}" == -* ]]; do shift 2; done\n'
                "shift\n"
                'exec "$@"\n'
            ),
            "python": (
                f"record() {{\n{record_env}}}\n"
                'if [[ "${1:-}" == "-m" && "${2:-}" == "hermes_cli.finite_inference_helper" ]]; then\n'
                "  record helper\n"
                f'  printf "helper %s\\n" "${{*:3}}" >>"{events}"\n'
                f'  cp "$HERMES_HOME/config.yaml" "{records}/helper-config.yaml"\n'
                '  if [[ "${FAKE_REAL_HELPER:-0}" != "1" ]]; then\n'
                '    printf \'{"applied": "skipped", "reason": "stub"}\\n\'\n'
                '    exit "${FAKE_HELPER_STATUS:-0}"\n'
                "  fi\n"
                'elif [[ "${1:-}" == "$FINITE_HERMES_CONFIG_RECONCILER" ]]; then\n'
                "  record reconciler\n"
                f'  printf "reconciler\\n" >>"{events}"\n'
                'elif [[ "${1:-}" == "$FINITE_RECOVER_CHAT_BOOT" ]]; then\n'
                f'  printf "recover\\n" >>"{events}"\n'
                "  exit 0\n"
                "fi\n"
                f'exec {sys.executable!s} "$@"\n'
            ),
        }
        for name, body in stubs.items():
            stub = fake_bin / name
            stub.write_text(f"#!/usr/bin/env bash\n{body}", encoding="utf-8")
            stub.chmod(0o755)

        agent_home = tmp / "agent"
        hermes_home = agent_home / "hermes-home"
        hermes_home.mkdir(parents=True)
        (agent_home / "config.json").write_text("{}\n", encoding="utf-8")
        if config is not None:
            (hermes_home / "config.yaml").write_text(json.dumps(config) + "\n", encoding="utf-8")
        intent_path = agent_home / "agentd/inference-intent.json"
        if intent is not None:
            intent_path.parent.mkdir()
            intent_path.write_text(json.dumps(intent), encoding="utf-8")

        launch_env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("FINITE", "HERMES", "OPENAI", "OPENROUTER", "CODEX"))
        }
        launch_env.update(
            {
                "PATH": f"{fake_bin}:{os.environ['PATH']}",
                "HOME": str(tmp),
                "FINITECHAT_BIN": str(fake_bin / "finitechat"),
                "FINITECHAT_HOME": str(agent_home),
                "HERMES_HOME": str(hermes_home),
                "FINITECHAT_WORKSPACE": str(tmp / "workspace"),
                "FINITE_AGENTD_SUPERVISED": "1",
                "FINITE_HERMES_CONFIG_RECONCILER": str(RECONCILER),
                "FINITE_RECOVER_CHAT_BOOT": str(tmp / "recover_chat_boot.py"),
                "FINITE_DEFAULT_INFERENCE_PROFILE": "finite-private",
                "FINITE_PRIVATE_API_KEY": FAKE_FP_KEY,
                "FINITE_PRIVATE_MODEL": "glm-5-3-flash",
                "FINITE_PRIVATE_BASE_URL": FP_PRODUCT_URL,
                "FINITE_PRIVATE_CONTEXT_LENGTH": "393216",
                "FINITE_AGENTD_INTENT_PATH": str(intent_path),
                "FAKE_HELPER_STATUS": str(helper_status),
                "FAKE_REAL_HELPER": "1" if real_helper else "0",
            }
        )
        if timeout_status is not None:
            launch_env["FAKE_TIMEOUT_STATUS"] = str(timeout_status)
        for key, value in (env or {}).items():
            if value is None:
                launch_env.pop(key, None)
            else:
                launch_env[key] = value

        result = subprocess.run(
            ["bash", str(REPO_ROOT / "containers/agent/run_hermes_gateway.sh"), *args],
            env=launch_env,
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
        logged = events.read_text(encoding="utf-8").splitlines() if events.exists() else []
        recorded: dict[str, dict[str, str]] = {}
        for path in records.glob("*.env"):
            recorded[path.stem] = dict(
                line.split("=", 1) for line in path.read_text(encoding="utf-8").splitlines()
            )
        return result, logged, recorded

    @staticmethod
    def _intent(phase: str, route: str = "openrouter") -> dict[str, Any]:
        return {
            "v": 1,
            "id": "op_" + "0" * 32,
            "kind": "disconnect",
            "route": route,
            "model": None,
            "phase": phase,
            "state": "running",
            "error_code": None,
            "attempts": 0,
            "created_at_ms": 0,
            "updated_at_ms": 0,
        }

    # T-C15
    def test_launch_rule_unsets_only_the_finite_private_alias(self) -> None:
        with tempfile.TemporaryDirectory() as raw_tmp:
            result, _, recorded = self._launch(
                Path(raw_tmp),
                env={"OPENAI_API_KEY": FAKE_FP_KEY, "CODEX_HOME": "/root/.codex"},
                intent=self._intent("cleanup"),
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            for process in ("reconciler", "helper", "hermes"):
                with self.subTest(process):
                    self.assertNotIn("OPENAI_API_KEY", recorded[process])
                    self.assertEqual(recorded[process]["CODEX_HOME"], NEUTRAL_CODEX_HOME)

    def test_launch_rule_keeps_a_users_own_openai_key(self) -> None:
        cases = (
            ("different_value", {"OPENAI_API_KEY": "sk-user-FAKE-openai"}),
            (
                "no_finite_private_key",
                {
                    "OPENAI_API_KEY": "sk-user-FAKE-openai",
                    "FINITE_PRIVATE_API_KEY": None,
                    "FINITE_DEFAULT_INFERENCE_PROFILE": "openrouter",
                },
            ),
            (
                "empty_finite_private_key",
                {
                    "OPENAI_API_KEY": "sk-user-FAKE-openai",
                    "FINITE_PRIVATE_API_KEY": "",
                    "FINITE_DEFAULT_INFERENCE_PROFILE": "openrouter",
                },
            ),
        )
        for name, env in cases:
            with self.subTest(name), tempfile.TemporaryDirectory() as raw_tmp:
                result, _, recorded = self._launch(
                    Path(raw_tmp), env=env, intent=self._intent("cleanup")
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                for process in ("reconciler", "helper", "hermes"):
                    self.assertEqual(recorded[process]["OPENAI_API_KEY"], "sk-user-FAKE-openai")
                    self.assertEqual(recorded[process]["CODEX_HOME"], NEUTRAL_CODEX_HOME)

    def test_launch_rule_runs_before_the_reconciler_and_exec(self) -> None:
        script = (REPO_ROOT / "containers/agent/run_hermes_gateway.sh").read_text(encoding="utf-8")
        unset = script.index("unset OPENAI_API_KEY")
        codex_home = script.index(f"export CODEX_HOME={NEUTRAL_CODEX_HOME}")
        recover = script.index("\n    run_recover_chat_boot\n")
        reconcile = script.index("\n    run_config_reconciler\n")
        self.assertLess(unset, recover)
        self.assertLess(codex_home, recover)
        self.assertLess(recover, reconcile)
        self.assertIn(
            '[[ -n "${FINITE_PRIVATE_API_KEY:-}" && "${OPENAI_API_KEY:-}" == "$FINITE_PRIVATE_API_KEY" ]]',
            script,
        )

    # T-C12
    def _finite_private_settings(self, env: Mapping[str, str | None]) -> dict[str, str]:
        with tempfile.TemporaryDirectory() as raw_tmp:
            result, _, recorded = self._launch(Path(raw_tmp), env=env, args=("--prepare-only",))
            self.assertEqual(result.returncode, 0, result.stderr)
            return {
                key: value
                for key, value in recorded["reconciler"].items()
                if key.startswith("FINITE_CONFIG_FP_")
            }

    def test_launcher_passes_finite_private_settings_for_every_profile(self) -> None:
        expected = {
            "FINITE_CONFIG_FP_MODEL": "glm-5-3-flash",
            "FINITE_CONFIG_FP_BASE_URL": FP_PRODUCT_URL,
            "FINITE_CONFIG_FP_CONTEXT_LENGTH": "393216",
            "FINITE_CONFIG_FP_KEY_PRESENT": "1",
            "FINITE_CONFIG_FP_FALLBACK_MODE": "seed",
        }
        for profile in ("finite-private", "openrouter", None):
            with self.subTest(profile=profile):
                settings = self._finite_private_settings(
                    {
                        "FINITE_DEFAULT_INFERENCE_PROFILE": profile,
                        # The primary's overrides never leak into the backup route.
                        "FINITECHAT_HERMES_MODEL": "anthropic/claude-sonnet-4.6",
                        "FINITECHAT_HERMES_BASE_URL": "https://openrouter.ai/api/v1",
                    }
                )
                self.assertEqual(settings, expected)

    def test_launcher_finite_private_settings_rewrites_and_absence(self) -> None:
        cases = (
            (
                "historical_route_and_legacy_model",
                {
                    "FINITE_PRIVATE_MODEL": "glm-5-2",
                    "FINITE_PRIVATE_BASE_URL": "https://kimi-k2-6.finite.containers.tinfoil.dev/v1",
                },
                {
                    "FINITE_CONFIG_FP_MODEL": "glm-5-3-flash",
                    "FINITE_CONFIG_FP_BASE_URL": FP_PRODUCT_URL,
                },
            ),
            (
                "legacy_name_on_another_endpoint",
                {
                    "FINITE_PRIVATE_MODEL": "glm-5-2",
                    "FINITE_PRIVATE_BASE_URL": "http://127.0.0.1:8787/v1",
                },
                {
                    "FINITE_CONFIG_FP_MODEL": "glm-5-2",
                    "FINITE_CONFIG_FP_BASE_URL": "http://127.0.0.1:8787/v1",
                },
            ),
            (
                "no_runner_settings",
                {
                    "FINITE_DEFAULT_INFERENCE_PROFILE": "openrouter",
                    "FINITE_PRIVATE_MODEL": None,
                    "FINITE_PRIVATE_BASE_URL": None,
                    "FINITE_PRIVATE_CONTEXT_LENGTH": None,
                    "FINITE_PRIVATE_API_KEY": None,
                },
                {
                    "FINITE_CONFIG_FP_MODEL": "",
                    "FINITE_CONFIG_FP_BASE_URL": "",
                    "FINITE_CONFIG_FP_CONTEXT_LENGTH": "",
                    "FINITE_CONFIG_FP_KEY_PRESENT": "0",
                },
            ),
            (
                "kill_switch",
                {"FINITE_PRIVATE_FALLBACK_MODE": "remove"},
                {"FINITE_CONFIG_FP_FALLBACK_MODE": "remove"},
            ),
        )
        for name, env, expected in cases:
            with self.subTest(name):
                settings = self._finite_private_settings(env)
                self.assertEqual({key: settings[key] for key in expected}, expected)

    # T-C16
    def test_pending_disconnect_step_runs_after_the_reconciler_and_before_exec(self) -> None:
        with tempfile.TemporaryDirectory() as raw_tmp:
            tmp = Path(raw_tmp)
            result, events, recorded = self._launch(tmp, intent=self._intent("cleanup"))

            self.assertEqual(result.returncode, 0, result.stderr)
            intent_path = tmp / "agent/agentd/inference-intent.json"
            self.assertEqual(
                events,
                [
                    "reconciler",
                    "timeout -k 5 20 python -m hermes_cli.finite_inference_helper "
                    f"apply-pending-disconnect --intent {intent_path}",
                    f"helper apply-pending-disconnect --intent {intent_path}",
                    "hermes gateway run --replace",
                ],
            )
            # The helper decides on config.yaml as the reconciler left it on disk.
            helper_config = AgentRuntimeLauncherConfigTest._load_config_text(
                (tmp / "records/helper-config.yaml").read_text(encoding="utf-8")
            )
            self.assertIn("finite-private", helper_config["providers"])
            self.assertEqual(recorded["helper"]["HERMES_HOME"], str(tmp / "agent/hermes-home"))
            self.assertEqual(recorded["helper"]["FINITE_CONFIG_FP_BASE_URL"], FP_PRODUCT_URL)
            # Its log line goes to the launcher's log, not its stdout.
            self.assertIn('"applied": "skipped"', result.stderr)
            self.assertNotIn("applied", result.stdout)

    def test_pending_disconnect_step_needs_an_existing_intent_file(self) -> None:
        for name, env in (
            ("unset", {"FINITE_AGENTD_INTENT_PATH": None}),
            ("empty", {"FINITE_AGENTD_INTENT_PATH": ""}),
            ("missing_file", {}),
        ):
            with self.subTest(name), tempfile.TemporaryDirectory() as raw_tmp:
                result, events, _ = self._launch(Path(raw_tmp), env=env)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(events, ["reconciler", "hermes gateway run --replace"])

        with tempfile.TemporaryDirectory() as raw_tmp:
            tmp = Path(raw_tmp)
            (tmp / "intent-dir").mkdir()
            result, events, _ = self._launch(
                tmp, env={"FINITE_AGENTD_INTENT_PATH": str(tmp / "intent-dir")}
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(events, ["reconciler", "hermes gateway run --replace"])

    def test_pending_disconnect_step_failure_never_blocks_the_gateway(self) -> None:
        # (name, helper exit, stubbed `timeout` exit, status the launcher logs)
        cases = (
            ("helper_error", 2, None, 2),
            ("helper_crash", 1, None, 1),
            ("deadline", 0, 124, 124),
            ("killed_after_deadline", 0, 137, 137),
            ("helper_missing", 0, 127, 127),
        )
        for name, helper_status, timeout_status, status in cases:
            with self.subTest(name), tempfile.TemporaryDirectory() as raw_tmp:
                result, events, _ = self._launch(
                    Path(raw_tmp),
                    intent=self._intent("cleanup"),
                    helper_status=helper_status,
                    timeout_status=timeout_status,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(events[-1], "hermes gateway run --replace")
                self.assertIn(
                    f"pending-disconnect step failed (status {status}); "
                    "starting Hermes on the existing route",
                    result.stderr,
                )

    def test_pending_disconnect_step_is_skipped_on_recover_boot_and_prepare(self) -> None:
        with tempfile.TemporaryDirectory() as raw_tmp:
            result, events, _ = self._launch(
                Path(raw_tmp),
                env={"FINITE_AGENT_BOOT_INTENT_JSON": '{"kind": "recover_known_good"}'},
                intent=self._intent("cleanup"),
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(events, ["recover", "hermes gateway run --replace"])

        with tempfile.TemporaryDirectory() as raw_tmp:
            result, events, _ = self._launch(
                Path(raw_tmp), args=("--prepare-only",), intent=self._intent("cleanup")
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(events, ["reconciler"])

    @unittest.skipUnless(
        _helper_module_available(),
        "hermes_cli.finite_inference_helper ships in slice P1; rebuild the Hermes env after it lands",
    )
    def test_real_helper_clears_only_after_cleanup_and_route_switch(self) -> None:
        """F1 through the real launcher and the real helper (§3.7, §8.2)."""
        openrouter = {
            "default": "anthropic/claude-sonnet-4.6",
            "provider": "openrouter",
            "base_url": "https://openrouter.ai/api/v1",
            "api_mode": "chat_completions",
        }
        finite_private = {
            "default": "glm-5-3-flash",
            "provider": "custom",
            "base_url": FP_PRODUCT_URL,
            "api_key": "${FINITE_PRIVATE_API_KEY}",
            "api_mode": "chat_completions",
        }
        cases = (
            ("accepted", openrouter, "skipped"),
            ("route_switched", finite_private, "skipped"),
            ("cleanup", openrouter, "skipped"),
            ("cleanup", {"default": "openai/gpt-5", "provider": "openrouter"}, "skipped"),
            ("cleanup", finite_private, "yes"),
            ("verifying", {"default": "glm-5-3-flash", "provider": "finite-private"}, "yes"),
        )
        for phase, model, applied in cases:
            with self.subTest(phase=phase, model=model), tempfile.TemporaryDirectory() as raw_tmp:
                result, events, _ = self._launch(
                    Path(raw_tmp),
                    intent=self._intent(phase),
                    config={"model": model},
                    real_helper=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(events[-1], "hermes gateway run --replace")
                outcomes = [
                    json.loads(line)["applied"]
                    for line in result.stderr.splitlines()
                    if line.startswith("{") and '"applied"' in line
                ]
                self.assertEqual(outcomes, [applied])


if __name__ == "__main__":
    unittest.main()

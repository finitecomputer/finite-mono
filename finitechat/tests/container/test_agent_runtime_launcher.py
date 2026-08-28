"""Unit tests for the Agent Runtime gateway launcher and config reconciler.

These exercise `containers/agent/run_hermes_gateway.sh` and
`containers/agent/reconcile_hermes_config.py` directly on the host with stub
binaries. Both files ship in the one canonical Agent Runtime image
(`finitecomputer-v2/deploy/finite-computer/images/runtime.Dockerfile`); the
Docker-level proof of that image is `scripts/hermes-durable-home-docker-smoke.py`
run by `.github/workflows/hermes-runtime-smoke.yml` and `runtime-image.yml`.
"""

from __future__ import annotations

import json
import os
import runpy
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[2]


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
            },
        )

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
        self.assertNotIn('hermes --home "$agent_home" invite', script)
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
        init = script.index('"$finitechat_bin" hermes --home "$agent_home" init', seed)
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
        self, owner_npubs: str | None, allowed_users: list[str] | None = None
    ) -> dict[str, str]:
        """Run the launcher with stub binaries and capture the chat-authz env
        the gateway process would inherit. The runner always injects
        FINITECHAT_ALLOW_ALL_USERS=true for old-image compatibility."""
        launcher = REPO_ROOT / "containers/agent/run_hermes_gateway.sh"
        with tempfile.TemporaryDirectory() as raw_tmp:
            tmp = Path(raw_tmp)
            fake_bin = tmp / "bin"
            fake_bin.mkdir()
            finitechat = fake_bin / "finitechat"
            finitechat.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
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
                "FINITE_AGENTD_SUPERVISED": "1",
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

    def test_gateway_launcher_owner_npubs_scope_chat_admission(self) -> None:
        owner = "a" * 64
        captured = self._gateway_chat_authz_env(owner)

        self.assertEqual(captured.get("FINITECHAT_ALLOWED_USERS"), owner)
        self.assertEqual(captured.get("FINITECHAT_WELCOME_ALLOWLIST"), owner)
        self.assertEqual(captured.get("FINITECHAT_OWNER_NPUBS"), owner)
        # The runner-injected allow-all flags must be actively unset so the
        # allowlist is the only admission path.
        self.assertNotIn("FINITECHAT_ALLOW_ALL_USERS", captured)
        self.assertNotIn("FINITE_ALLOW_ALL_USERS", captured)
        self.assertNotIn("GATEWAY_ALLOW_ALL_USERS", captured)

    def test_gateway_launcher_without_owner_npubs_keeps_legacy_allow_all(self) -> None:
        captured = self._gateway_chat_authz_env(None)

        self.assertEqual(captured.get("FINITECHAT_ALLOW_ALL_USERS"), "true")
        self.assertNotIn("FINITECHAT_ALLOWED_USERS", captured)
        self.assertNotIn("FINITECHAT_WELCOME_ALLOWLIST", captured)
        self.assertNotIn("FINITE_ALLOW_ALL_USERS", captured)
        self.assertNotIn("GATEWAY_ALLOW_ALL_USERS", captured)

    def test_gateway_launcher_allowed_users_mirror_scopes_chat_admission(self) -> None:
        """No birth env, but the sidecar has locked admission and mirrored the
        store's allowlist: the gateway locks to the same entries."""
        owner = "a" * 64
        guest = "b" * 64
        captured = self._gateway_chat_authz_env(None, allowed_users=[owner, guest])

        self.assertEqual(captured.get("FINITECHAT_ALLOWED_USERS"), f"{owner},{guest}")
        # The mirror is a gateway concern only; the sidecar's store is the
        # source of truth and needs no env re-seed.
        self.assertNotIn("FINITECHAT_WELCOME_ALLOWLIST", captured)
        self.assertNotIn("FINITECHAT_ALLOW_ALL_USERS", captured)
        self.assertNotIn("FINITE_ALLOW_ALL_USERS", captured)
        self.assertNotIn("GATEWAY_ALLOW_ALL_USERS", captured)

    def test_gateway_launcher_owner_npubs_take_precedence_over_mirror(self) -> None:
        owner = "a" * 64
        stale = "b" * 64
        captured = self._gateway_chat_authz_env(owner, allowed_users=[stale])

        self.assertEqual(captured.get("FINITECHAT_ALLOWED_USERS"), owner)
        self.assertEqual(captured.get("FINITECHAT_WELCOME_ALLOWLIST"), owner)

    def test_gateway_launcher_empty_mirror_keeps_legacy_allow_all(self) -> None:
        """An empty mirror must fail open to the same legacy behavior as a
        missing one: the sidecar writes no file until admission is locked."""
        captured = self._gateway_chat_authz_env(None, allowed_users=[])

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


if __name__ == "__main__":
    unittest.main()

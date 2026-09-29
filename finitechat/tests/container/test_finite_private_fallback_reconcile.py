"""The reconciler's image-owned Finite Private provider and fallback entries.

Every agent boots through `containers/agent/reconcile_hermes_config.py`, so
these tests start from each config shape an existing agent can have today and
check that only the Finite-owned leaves change. They run under the pinned
Hermes Python, which provides `hermes_cli.fallback_config`, PyYAML, and
python-dotenv as in the runtime image. All keys are fake.
"""

from __future__ import annotations

import copy
import io
import os
import runpy
import subprocess
import sys
import tempfile
import unittest
from collections.abc import Callable
from contextlib import redirect_stderr
from pathlib import Path
from typing import Any
from unittest import mock

import yaml
from hermes_cli.fallback_config import get_fallback_chain

REPO_ROOT = Path(__file__).resolve().parents[2]
RECONCILER = REPO_ROOT / "containers/agent/reconcile_hermes_config.py"
reconcile_config: Callable[..., dict[str, Any]] = runpy.run_path(str(RECONCILER))[
    "reconcile_config"
]

FP_MODEL = "glm-5-3-flash"
FP_URL = "https://finite-private.finite.containers.tinfoil.dev/v1"
FP_CONTEXT = 393216
FAKE_OPENROUTER_KEY = "sk-or-FAKE-not-a-key"

CANONICAL_PROVIDER = {
    "name": "Finite Private",
    "base_url": FP_URL,
    "key_env": "FINITE_PRIVATE_API_KEY",
    "api_mode": "chat_completions",
    "models": {FP_MODEL: {"context_length": FP_CONTEXT, "supports_vision": True}},
    "discover_models": False,
}
CANONICAL_ENTRY = {
    "provider": "finite-private",
    "model": FP_MODEL,
    "base_url": FP_URL,
    "key_env": "FINITE_PRIVATE_API_KEY",
    "api_mode": "chat_completions",
}
# The block agentd v1 apply and the first seed write for Finite Private.
FP_BARE_MODEL = {
    "default": FP_MODEL,
    "provider": "custom",
    "base_url": FP_URL,
    "api_key": "${FINITE_PRIVATE_API_KEY}",
    "api_mode": "chat_completions",
    "context_length": FP_CONTEXT,
    "supports_vision": True,
}
# The block agentd v1 apply writes for OpenRouter.
OPENROUTER_MODEL = {
    "default": "anthropic/claude-sonnet-4.6",
    "provider": "openrouter",
    "base_url": "https://openrouter.ai/api/v1",
    "api_mode": "chat_completions",
}
# What `/model ... --global` leaves behind (G2).
GLOBAL_FP_MODEL = {"default": FP_MODEL, "provider": "finite-private"}
GLOBAL_OPENROUTER_MODEL = {"default": "openai/gpt-5", "provider": "openrouter"}
GLOBAL_CODEX_MODEL = {"default": "gpt-5.5", "provider": "openai-codex"}
# Today's hand-written Finite Private backup: a bare custom entry.
BARE_CUSTOM_CHAIN = [
    {
        "provider": "custom",
        "model": FP_MODEL,
        "base_url": FP_URL,
        "api_key": "${FINITE_PRIVATE_API_KEY}",
    }
]
USER_ENTRY = {"provider": "openrouter", "model": "openai/gpt-5-mini"}

EXISTING_MODELS: dict[str, Any] = {
    "seeded_finite_private": FP_BARE_MODEL,
    "openrouter_v1_apply": OPENROUTER_MODEL,
    "openrouter_legacy_config_key": {**OPENROUTER_MODEL, "api_key": FAKE_OPENROUTER_KEY},
    "global_finite_private": GLOBAL_FP_MODEL,
    "global_openrouter": GLOBAL_OPENROUTER_MODEL,
    "global_codex": GLOBAL_CODEX_MODEL,
    "hand_edited_custom": {
        "default": "llama-4",
        "provider": "custom",
        "base_url": "https://inference.example.invalid/v1",
        "api_key": "${USER_INFERENCE_KEY}",
        "temperature": 0.4,
    },
    "non_mapping_model": "anthropic/claude-sonnet-4.6",
}


def base_settings() -> dict[str, str]:
    return {
        "FINITE_CONFIG_MODEL": FP_MODEL,
        "FINITE_CONFIG_PROVIDER": "custom",
        "FINITE_CONFIG_BASE_URL": FP_URL,
        "FINITE_CONFIG_CONTEXT_LENGTH": str(FP_CONTEXT),
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


def fp_settings(*, key_present: bool = True, **overrides: str) -> dict[str, str]:
    settings = {
        **base_settings(),
        "FINITE_CONFIG_FP_MODEL": FP_MODEL,
        "FINITE_CONFIG_FP_BASE_URL": FP_URL,
        "FINITE_CONFIG_FP_CONTEXT_LENGTH": str(FP_CONTEXT),
        "FINITE_CONFIG_FP_KEY_PRESENT": "1" if key_present else "0",
        "FINITE_CONFIG_FP_FALLBACK_MODE": "seed",
    }
    settings.update(overrides)
    return settings


class FinitePrivateFallbackReconcileTest(unittest.TestCase):
    reconcile = staticmethod(reconcile_config)

    def _existing(self, model: Any, **extra: Any) -> dict[str, Any]:
        """A config Hermes already owns, as the previous image left it."""
        existing = self.reconcile(None, base_settings())
        existing["model"] = copy.deepcopy(model)
        existing.update(copy.deepcopy(extra))
        return existing

    def _assert_only_finite_private_leaves(
        self,
        existing: dict[str, Any],
        reconciled: dict[str, Any],
        *,
        chain: Any,
    ) -> None:
        expected = copy.deepcopy(existing)
        expected.setdefault("providers", {})["finite-private"] = CANONICAL_PROVIDER
        if chain is not None:
            expected["fallback_providers"] = chain
        self.assertEqual(reconciled, expected)

    # T-C1
    def test_first_seed_with_key_adds_provider_and_chain(self) -> None:
        reconciled = self.reconcile(None, fp_settings())

        self.assertEqual(reconciled["model"], FP_BARE_MODEL)
        self.assertEqual(reconciled["providers"], {"finite-private": CANONICAL_PROVIDER})
        self.assertEqual(reconciled["fallback_providers"], [CANONICAL_ENTRY])
        self.assertNotIn("fallback_model", reconciled)
        self.assertEqual(get_fallback_chain(reconciled), [CANONICAL_ENTRY])

    # T-C2
    def test_first_seed_without_key_adds_provider_only(self) -> None:
        reconciled = self.reconcile(None, fp_settings(key_present=False))

        self.assertEqual(reconciled["providers"], {"finite-private": CANONICAL_PROVIDER})
        self.assertNotIn("fallback_providers", reconciled)

    # T-C3
    def test_dotenv_key_follows_hermes_precedence(self) -> None:
        cases = (
            (
                "empty_last_assignment",
                "FINITE_PRIVATE_API_KEY=fp-FAKE\nFINITE_PRIVATE_API_KEY=\n",
                True,
                False,
            ),
            ("whitespace", "FINITE_PRIVATE_API_KEY='  '\n", True, False),
            (
                "unexpanded_reference",
                "FINITE_PRIVATE_API_KEY='${FINITE_PRIVATE_API_KEY}'\n",
                True,
                False,
            ),
            ("dotenv_only_key", "FINITE_PRIVATE_API_KEY=fp-FAKE\n", False, True),
            ("bare_name_keeps_process_env", "FINITE_PRIVATE_API_KEY\n", True, True),
            ("other_lines_only", f"OPENROUTER_API_KEY={FAKE_OPENROUTER_KEY}\n", True, True),
            ("bare_name_without_process_key", "FINITE_PRIVATE_API_KEY\n", False, False),
        )
        for name, dotenv, process_key, seeded in cases:
            with self.subTest(name), tempfile.TemporaryDirectory() as raw_home:
                hermes_home = Path(raw_home)
                (hermes_home / ".env").write_text(dotenv, encoding="utf-8")
                before = (hermes_home / ".env").read_bytes()

                reconciled = self.reconcile(
                    None, fp_settings(key_present=process_key), hermes_home=hermes_home
                )

                self.assertEqual("fallback_providers" in reconciled, seeded)
                self.assertEqual((hermes_home / ".env").read_bytes(), before)

    def test_dotenv_interpolation_matches_hermes(self) -> None:
        with tempfile.TemporaryDirectory() as raw_home:
            hermes_home = Path(raw_home)
            (hermes_home / ".env").write_text(
                "FINITE_PRIVATE_API_KEY=${FINITE_TEST_UNSET_REFERENCE}\n", encoding="utf-8"
            )
            with mock.patch.dict(os.environ, {}, clear=False):
                os.environ.pop("FINITE_TEST_UNSET_REFERENCE", None)
                reconciled = self.reconcile(None, fp_settings(), hermes_home=hermes_home)
            self.assertNotIn("fallback_providers", reconciled)

            with mock.patch.dict(os.environ, {"FINITE_TEST_UNSET_REFERENCE": "fp-FAKE"}):
                reconciled = self.reconcile(
                    None, fp_settings(key_present=False), hermes_home=hermes_home
                )
            self.assertEqual(reconciled["fallback_providers"], [CANONICAL_ENTRY])

    # T-C4, T-C5 and the other shapes an existing agent can have today.
    def test_existing_configs_gain_only_the_finite_private_leaves(self) -> None:
        for name, model in EXISTING_MODELS.items():
            with self.subTest(name):
                existing = self._existing(model)
                before = copy.deepcopy(existing)

                reconciled = self.reconcile(existing, fp_settings())

                self.assertEqual(existing, before)
                self.assertEqual(reconciled["model"], model)
                self._assert_only_finite_private_leaves(
                    existing, reconciled, chain=[CANONICAL_ENTRY]
                )
                self.assertEqual(self.reconcile(reconciled, fp_settings()), reconciled)

    def test_existing_configs_without_key_gain_no_chain(self) -> None:
        for name, model in EXISTING_MODELS.items():
            with self.subTest(name):
                existing = self._existing(model)
                reconciled = self.reconcile(existing, fp_settings(key_present=False))
                self._assert_only_finite_private_leaves(existing, reconciled, chain=None)

    def test_config_without_model_gains_the_leaves(self) -> None:
        existing = self._existing(None)
        del existing["model"]
        reconciled = self.reconcile(existing, fp_settings())
        self._assert_only_finite_private_leaves(existing, reconciled, chain=[CANONICAL_ENTRY])

    def test_user_providers_are_kept_and_the_owned_entry_is_replaced(self) -> None:
        existing = self._existing(
            OPENROUTER_MODEL,
            providers={
                "custom": {"models": {FP_MODEL: {"supports_vision": False}}},
                "finite-private": {
                    "base_url": "https://stale.example.invalid/v1",
                    "api_key": "fp-FAKE-inline",
                },
            },
        )
        reconciled = self.reconcile(existing, fp_settings())

        self.assertEqual(
            reconciled["providers"],
            {
                "custom": {"models": {FP_MODEL: {"supports_vision": False}}},
                "finite-private": CANONICAL_PROVIDER,
            },
        )

    def test_owned_entry_is_rewritten_with_discovery_off_except_on_recovery(self) -> None:
        # Without `discover_models: false`, patched Hermes would not validate
        # `/model` against the declared models.
        earlier_build = {
            key: value for key, value in CANONICAL_PROVIDER.items() if key != "discover_models"
        }
        discovery_on = {**CANONICAL_PROVIDER, "discover_models": True}
        for name, entry in (("earlier_build", earlier_build), ("discovery_on", discovery_on)):
            with self.subTest(name):
                existing = self._existing(FP_BARE_MODEL, providers={"finite-private": entry})

                reconciled = self.reconcile(existing, fp_settings())
                recovered = self.reconcile(existing, fp_settings(), recover_known_good=True)

                self.assertEqual(reconciled["providers"], {"finite-private": CANONICAL_PROVIDER})
                self.assertEqual(recovered["providers"], {"finite-private": entry})

    def test_null_providers_is_treated_as_absent(self) -> None:
        existing = self._existing(FP_BARE_MODEL, providers=None)
        reconciled = self.reconcile(existing, fp_settings())
        self.assertEqual(reconciled["providers"], {"finite-private": CANONICAL_PROVIDER})

    # T-C6
    def test_user_chain_is_untouched_and_owned_entries_refresh_in_place(self) -> None:
        stale_owned = {
            "provider": "finite-private",
            "model": "glm-5-2",
            "base_url": "https://kimi-k2-6.finite.containers.tinfoil.dev/v1",
            "api_key": "fp-FAKE-inline",
        }
        cases = (
            ("user_only", [USER_ENTRY], [USER_ENTRY]),
            ("owned_second", [USER_ENTRY, stale_owned], [USER_ENTRY, CANONICAL_ENTRY]),
            (
                "owned_between",
                [USER_ENTRY, stale_owned, "not-a-mapping", BARE_CUSTOM_CHAIN[0]],
                [USER_ENTRY, CANONICAL_ENTRY, "not-a-mapping", BARE_CUSTOM_CHAIN[0]],
            ),
            (
                "owned_twice",
                [stale_owned, USER_ENTRY, stale_owned],
                [CANONICAL_ENTRY, USER_ENTRY, CANONICAL_ENTRY],
            ),
            ("canonical", [CANONICAL_ENTRY], [CANONICAL_ENTRY]),
        )
        for name, chain, expected in cases:
            for key_present in (True, False):
                with self.subTest(name, key_present=key_present):
                    existing = self._existing(GLOBAL_CODEX_MODEL, fallback_providers=chain)
                    reconciled = self.reconcile(existing, fp_settings(key_present=key_present))
                    self._assert_only_finite_private_leaves(existing, reconciled, chain=expected)

    # T-C7
    def test_recover_known_good_adds_nothing(self) -> None:
        for name, model in EXISTING_MODELS.items():
            with self.subTest(name):
                existing = self._existing(model)
                reconciled = self.reconcile(existing, fp_settings(), recover_known_good=True)
                self.assertNotIn("providers", reconciled)
                self.assertNotIn("fallback_providers", reconciled)
                self.assertEqual(reconciled["model"], model)

        legacy_named = self._existing({"default": "glm-5-2", "provider": "finite-private"})
        reconciled = self.reconcile(legacy_named, fp_settings(), recover_known_good=True)
        self.assertEqual(reconciled["model"], {"default": "glm-5-2", "provider": "finite-private"})

    # T-C9
    def test_named_primary_legacy_model_is_migrated_without_new_fields(self) -> None:
        for provider in ("finite-private", "custom:finite-private"):
            for legacy in ("glm-5-2", "deepseek-v4-flash-0731", "glm-5.3-flash"):
                with self.subTest(provider=provider, legacy=legacy):
                    existing = self._existing({"default": legacy, "provider": provider})
                    reconciled = self.reconcile(existing, fp_settings())
                    self.assertEqual(
                        reconciled["model"], {"default": FP_MODEL, "provider": provider}
                    )

        user_model = self._existing({"default": "glm-6", "provider": "finite-private"})
        self.assertEqual(
            self.reconcile(user_model, fp_settings())["model"],
            {"default": "glm-6", "provider": "finite-private"},
        )
        other_route = self._existing({"default": "glm-5-2", "provider": "openrouter"})
        self.assertEqual(
            self.reconcile(other_route, fp_settings())["model"],
            {"default": "glm-5-2", "provider": "openrouter"},
        )

    # T-C10
    def test_codex_model_with_bare_custom_chain_is_preserved(self) -> None:
        for key_present in (True, False):
            with self.subTest(key_present=key_present):
                existing = self._existing(GLOBAL_CODEX_MODEL, fallback_providers=BARE_CUSTOM_CHAIN)
                reconciled = self.reconcile(existing, fp_settings(key_present=key_present))
                self.assertEqual(reconciled["model"], GLOBAL_CODEX_MODEL)
                self.assertEqual(reconciled["fallback_providers"], BARE_CUSTOM_CHAIN)
                self._assert_only_finite_private_leaves(existing, reconciled, chain=None)

    # T-C13
    def test_remove_mode_removes_only_owned_chain_entries(self) -> None:
        remove = fp_settings(FINITE_CONFIG_FP_FALLBACK_MODE="remove")
        cases = (
            (
                "owned_and_user",
                [USER_ENTRY, CANONICAL_ENTRY, BARE_CUSTOM_CHAIN[0]],
                [USER_ENTRY, BARE_CUSTOM_CHAIN[0]],
            ),
            ("owned_only", [CANONICAL_ENTRY], None),
            ("user_only", [USER_ENTRY], [USER_ENTRY]),
            ("explicitly_off", [], []),
        )
        for name, chain, expected in cases:
            with self.subTest(name):
                existing = self._existing(
                    FP_BARE_MODEL,
                    fallback_providers=chain,
                    fallback_model={"provider": "finite-private", "model": FP_MODEL},
                )
                reconciled = self.reconcile(existing, remove)

                expected_config = copy.deepcopy(existing)
                if expected is None:
                    del expected_config["fallback_providers"]
                else:
                    expected_config["fallback_providers"] = expected
                self.assertEqual(reconciled, expected_config)

        self.assertEqual(
            self.reconcile(self._existing(FP_BARE_MODEL), remove), self._existing(FP_BARE_MODEL)
        )
        self.assertNotIn("fallback_providers", self.reconcile(None, remove))

    # T-C14
    def test_no_seed_when_the_user_configured_either_fallback_key(self) -> None:
        cases = (
            (
                "fallback_model_dict",
                {"fallback_model": {"provider": "openrouter", "model": "openai/gpt-5"}},
            ),
            (
                "fallback_model_list",
                {"fallback_model": [{"provider": "openrouter", "model": "openai/gpt-5"}]},
            ),
            # Hermes reads no entry from these, so only the key itself blocks the seed.
            ("fallback_model_empty", {"fallback_model": {}}),
            ("fallback_model_null", {"fallback_model": None}),
            ("fallback_providers_off", {"fallback_providers": []}),
            (
                "fallback_providers_mapping",
                {"fallback_providers": {"provider": "openrouter", "model": "x"}},
            ),
            ("fallback_providers_string", {"fallback_providers": "finite-private"}),
            ("fallback_providers_null", {"fallback_providers": None}),
        )
        for name, extra in cases:
            with self.subTest(name):
                existing = self._existing(OPENROUTER_MODEL, **extra)
                reconciled = self.reconcile(existing, fp_settings())
                self._assert_only_finite_private_leaves(existing, reconciled, chain=None)

    def test_no_seed_without_the_hermes_chain_reader(self) -> None:
        stderr = io.StringIO()
        with (
            mock.patch.dict(sys.modules, {"hermes_cli.fallback_config": None}),
            redirect_stderr(stderr),
        ):
            reconciled = self.reconcile(None, fp_settings())

        self.assertEqual(reconciled["providers"], {"finite-private": CANONICAL_PROVIDER})
        self.assertNotIn("fallback_providers", reconciled)
        self.assertIn("FINITE_AGENT_START_WARNING", stderr.getvalue())

    def test_no_seed_when_hermes_would_read_a_different_chain(self) -> None:
        # Hermes normalizes a trailing slash, so the entry would not read back canonical.
        settings = fp_settings(FINITE_CONFIG_FP_BASE_URL=f"{FP_URL}/")
        with redirect_stderr(io.StringIO()):
            reconciled = self.reconcile(None, settings)
        self.assertIn("finite-private", reconciled["providers"])
        self.assertNotIn("fallback_providers", reconciled)

    def test_missing_or_invalid_settings_leave_config_as_today(self) -> None:
        cases = (
            ("no_fp_settings", base_settings()),
            ("empty_model", fp_settings(FINITE_CONFIG_FP_MODEL="")),
            ("empty_url", fp_settings(FINITE_CONFIG_FP_BASE_URL="")),
            ("not_a_url", fp_settings(FINITE_CONFIG_FP_BASE_URL="finite-private")),
            ("wrong_scheme", fp_settings(FINITE_CONFIG_FP_BASE_URL="ftp://finite.example/v1")),
            ("no_host", fp_settings(FINITE_CONFIG_FP_BASE_URL="https:///v1")),
            ("bad_context", fp_settings(FINITE_CONFIG_FP_CONTEXT_LENGTH="lots")),
            ("zero_context", fp_settings(FINITE_CONFIG_FP_CONTEXT_LENGTH="0")),
        )
        for name, settings in cases:
            for model in (FP_BARE_MODEL, {"default": "glm-5-2", "provider": "finite-private"}):
                with self.subTest(name, model=model), redirect_stderr(io.StringIO()):
                    existing = self._existing(model)
                    self.assertEqual(self.reconcile(existing, settings), existing)

    def test_http_route_and_other_models_get_matching_declarations(self) -> None:
        settings = fp_settings(
            FINITE_CONFIG_FP_MODEL="glm-6",
            FINITE_CONFIG_FP_BASE_URL="http://127.0.0.1:8787/v1",
            FINITE_CONFIG_FP_CONTEXT_LENGTH="",
        )
        reconciled = self.reconcile(self._existing(OPENROUTER_MODEL), settings)

        self.assertEqual(
            reconciled["providers"]["finite-private"],
            {
                "name": "Finite Private",
                "base_url": "http://127.0.0.1:8787/v1",
                "key_env": "FINITE_PRIVATE_API_KEY",
                "api_mode": "chat_completions",
                "models": {"glm-6": {}},
                "discover_models": False,
            },
        )
        self.assertEqual(
            reconciled["fallback_providers"],
            [{**CANONICAL_ENTRY, "model": "glm-6", "base_url": "http://127.0.0.1:8787/v1"}],
        )

    def test_unexpected_failure_keeps_the_config_and_boot(self) -> None:
        existing = self._existing(OPENROUTER_MODEL)
        stderr = io.StringIO()
        with (
            mock.patch("hermes_cli.fallback_config.get_fallback_chain", side_effect=RuntimeError),
            redirect_stderr(stderr),
        ):
            reconciled = self.reconcile(existing, fp_settings())

        self.assertEqual(reconciled, existing)
        self.assertIn("FINITE_AGENT_START_WARNING", stderr.getvalue())


class FinitePrivateFallbackReconcilerProcessTest(unittest.TestCase):
    """The reconciler as the launcher runs it: a process writing config.yaml."""

    @staticmethod
    def _run(hermes_home: Path, settings: dict[str, str]) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(RECONCILER), "--config", str(hermes_home / "config.yaml")],
            env={"PATH": os.environ.get("PATH", ""), "HOME": str(hermes_home), **settings},
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )

    # T-C8
    def test_non_mapping_providers_warns_and_boots(self) -> None:
        with tempfile.TemporaryDirectory() as raw_home:
            hermes_home = Path(raw_home)
            config_path = hermes_home / "config.yaml"
            config_path.write_text(
                yaml.safe_dump({"model": OPENROUTER_MODEL, "providers": ["finite-private"]}),
                encoding="utf-8",
            )

            result = self._run(hermes_home, fp_settings())

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("FINITE_AGENT_START_WARNING", result.stderr)
            self.assertIn("providers is not a mapping", result.stderr)
            config = yaml.safe_load(config_path.read_text(encoding="utf-8"))
            self.assertEqual(config["providers"], ["finite-private"])
            self.assertNotIn("fallback_providers", config)
            self.assertEqual(config["model"], OPENROUTER_MODEL)

    # T-C11
    def test_second_run_makes_no_write(self) -> None:
        for name, model in (("first_seed", None), *EXISTING_MODELS.items()):
            with self.subTest(name), tempfile.TemporaryDirectory() as raw_home:
                hermes_home = Path(raw_home)
                config_path = hermes_home / "config.yaml"
                if model is not None:
                    config_path.write_text(yaml.safe_dump({"model": model}), encoding="utf-8")

                first = self._run(hermes_home, fp_settings())
                self.assertEqual(first.returncode, 0, first.stderr)
                written = config_path.read_bytes()
                written_stat = config_path.stat()
                # The owned leaves are distinct objects, so YAML needs no anchors.
                self.assertNotIn(b"&id", written)
                config = yaml.safe_load(written)
                self.assertEqual(config["providers"]["finite-private"], CANONICAL_PROVIDER)
                self.assertEqual(config["fallback_providers"], [CANONICAL_ENTRY])

                second = self._run(hermes_home, fp_settings())

                self.assertEqual(second.returncode, 0, second.stderr)
                self.assertEqual(config_path.read_bytes(), written)
                self.assertEqual(config_path.stat().st_mtime_ns, written_stat.st_mtime_ns)
                self.assertEqual(config_path.stat().st_ino, written_stat.st_ino)

    def test_dotenv_beside_the_config_decides_the_seed(self) -> None:
        with tempfile.TemporaryDirectory() as raw_home:
            hermes_home = Path(raw_home)
            (hermes_home / ".env").write_text("FINITE_PRIVATE_API_KEY=\n", encoding="utf-8")

            result = self._run(hermes_home, fp_settings())

            self.assertEqual(result.returncode, 0, result.stderr)
            config = yaml.safe_load((hermes_home / "config.yaml").read_text(encoding="utf-8"))
            self.assertIn("finite-private", config["providers"])
            self.assertNotIn("fallback_providers", config)


if __name__ == "__main__":
    unittest.main()

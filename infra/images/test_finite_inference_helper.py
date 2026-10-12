"""Exercise the Finite inference helper against the pinned Hermes package.

Every test uses scratch HOME, HERMES_HOME, and CODEX_HOME directories and fake
credentials. No test starts a gateway or reaches the network.
"""

import codecs
import hashlib
import importlib.util
import json
import os
import runpy
import shutil
import subprocess
import sys
import tempfile
import unittest
from contextlib import ExitStack
from pathlib import Path
from unittest.mock import patch

REPO = Path(__file__).resolve().parents[2]
HELPER = REPO / "finite-agentd/integrations/hermes/finite_inference_helper.py"
RECONCILER = REPO / "finitechat/containers/agent/reconcile_hermes_config.py"
FIXTURES = REPO / "finite-agentd/tests/fixtures"

FP_MODEL = "glm-5-3-flash"
FP_BASE_URL = "https://finite-private.finite.containers.tinfoil.dev/v1"
# Variables that could leak real credentials or routing into a helper run.
SCRUBBED = (
    "OPENROUTER_API_KEY",
    "OPENAI_API_KEY",
    "FINITE_PRIVATE_API_KEY",
    "FINITE_CONFIG_FP_MODEL",
    "FINITE_CONFIG_FP_BASE_URL",
    "FINITE_CONFIG_FP_CONTEXT_LENGTH",
    "HERMES_CODEX_BASE_URL",
    "OP_SERVICE_ACCOUNT_TOKEN",
)
# SQLite's read sidecars and Hermes' auth lock may appear or change beside the
# stores; nothing else may.
READ_SIDECARS = {"auth.lock", "state.db-shm", "state.db-wal"}

_scratch = tempfile.TemporaryDirectory(prefix="finite-inference-helper-")
_environment = patch.dict(
    os.environ,
    {
        "HOME": os.path.join(_scratch.name, "home"),
        "HERMES_HOME": os.path.join(_scratch.name, "import-home"),
        "CODEX_HOME": os.path.join(_scratch.name, "import-codex"),
    },
)


def setUpModule():
    _environment.start()
    for name in SCRUBBED:
        os.environ.pop(name, None)
    global helper, auth, gateway_config, gateway_session
    spec = importlib.util.spec_from_file_location("finite_inference_helper", HELPER)
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    from gateway import config as gateway_config
    from gateway import session as gateway_session
    from hermes_cli import auth


def tearDownModule():
    _environment.stop()
    _scratch.cleanup()


def fake_jwt(tag):
    body = codecs.encode(json.dumps({"sub": tag}).encode(), "base64").decode().strip().rstrip("=")
    return f"eyJhbGciOiJub25lIn0.{body}.fake-{tag}"


def snapshot(root):
    return {
        path.relative_to(root).as_posix(): (path.read_bytes(), path.stat().st_mtime_ns)
        for path in sorted(Path(root).rglob("*"))
        if path.is_file()
    }


class HelperCase(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="case-", dir=_scratch.name))
        self.hermes = self.root / "hermes"
        self.codex = self.root / "codex"
        for directory in (self.root / "home", self.hermes, self.codex):
            directory.mkdir()
        self.env = {
            "HOME": str(self.root / "home"),
            "HERMES_HOME": str(self.hermes),
            "CODEX_HOME": str(self.codex),
            "FINITE_CONFIG_FP_MODEL": FP_MODEL,
            "FINITE_CONFIG_FP_BASE_URL": FP_BASE_URL,
            "FINITE_CONFIG_FP_CONTEXT_LENGTH": "393216",
        }
        stack = ExitStack()
        stack.enter_context(patch.dict(os.environ, self.env))
        self.addCleanup(stack.close)

    def run_helper(self, *args, env=None):
        environment = {"PATH": os.environ.get("PATH", ""), **self.env, **(env or {})}
        completed = subprocess.run(
            [sys.executable, str(HELPER), *args],
            env=environment,
            capture_output=True,
            text=True,
            timeout=300,
        )
        return completed

    def run_json(self, *args, env=None):
        completed = self.run_helper(*args, env=env)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        lines = completed.stdout.splitlines()
        self.assertEqual(len(lines), 1, completed.stdout)
        return json.loads(lines[0])

    def write_config(self, config):
        import yaml

        (self.hermes / "config.yaml").write_text(yaml.safe_dump(config), encoding="utf-8")

    def write_auth(self, store):
        (self.hermes / "auth.json").write_text(json.dumps(store), encoding="utf-8")

    def read_auth(self):
        return json.loads((self.hermes / "auth.json").read_text(encoding="utf-8"))

    def seed_overrides(self, providers):
        """Create one conversation per provider (None = no override)."""
        config = gateway_config.load_gateway_config()
        store = gateway_session.SessionStore(config.sessions_dir, config)
        keys = []
        for index, provider in enumerate(providers):
            source = gateway_session.SessionSource(
                platform=gateway_config.Platform.LOCAL, chat_id=f"room-{index}", user_id="user"
            )
            entry = store.get_or_create_session(source)
            if provider:
                store.set_model_override(
                    entry.session_key,
                    {"provider": provider, "model": "model", "base_url": "https://x.invalid/v1"},
                )
            keys.append(entry.session_key)
        return store, keys

    def persisted_overrides(self):
        config = gateway_config.load_gateway_config()
        store = gateway_session.SessionStore(config.sessions_dir, config)
        return {
            entry.session_key: (entry.model_override or {}).get("provider")
            for entry in store.list_sessions()
        }

    def facts(self):
        return helper.inference_facts()


def fp_model_block():
    return {
        "default": FP_MODEL,
        "provider": "custom",
        "base_url": FP_BASE_URL,
        "api_key": "${FINITE_PRIVATE_API_KEY}",
        "api_mode": "chat_completions",
        "context_length": 393216,
        "supports_vision": True,
    }


def canonical_provider():
    return {
        "name": "Finite Private",
        "base_url": FP_BASE_URL,
        "key_env": "FINITE_PRIVATE_API_KEY",
        "api_mode": "chat_completions",
        "models": {FP_MODEL: {"context_length": 393216, "supports_vision": True}},
        "discover_models": False,
    }


def canonical_entry():
    return {
        "provider": "finite-private",
        "model": FP_MODEL,
        "base_url": FP_BASE_URL,
        "key_env": "FINITE_PRIVATE_API_KEY",
        "api_mode": "chat_completions",
    }


def two_route_auth():
    return {
        "version": 1,
        "active_provider": "openai-codex",
        "providers": {
            "openai-codex": {
                "tokens": {"access_token": fake_jwt("single"), "refresh_token": "fake-rt"}
            },
            "nous": {"access_token": "fake-nous"},
        },
        "credential_pool": {
            "openai-codex": [
                {"id": "c1", "source": "device_code", "access_token": fake_jwt("pool")}
            ],
            "openrouter": [{"id": "o1", "source": "manual", "access_token": "sk-or-v1-fake-pool"}],
        },
    }


class SavedRouteTests(unittest.TestCase):
    """the Python classifier agrees with every shared fixture."""

    def test_shared_fixtures(self):
        fixtures = sorted((FIXTURES / "saved-route").glob("*.json"))
        self.assertGreaterEqual(len(fixtures), 18)
        for path in fixtures:
            case = json.loads(path.read_text())
            with self.subTest(fixture=path.name):
                self.assertEqual(
                    helper.classify_saved_route(case["model"], case["fp_base_url"]),
                    case["expected"],
                )


class InferenceFactsTests(HelperCase):
    """Read-only facts, unavailable sources, and independent section failures."""

    def test_output_contract(self):
        self.write_config({"model": fp_model_block()})
        facts = self.run_json("inference-facts")
        self.assertEqual(
            set(facts),
            {
                "v",
                "saved_route",
                "fallback",
                "finite_private",
                "openrouter",
                "codex",
                "session_overrides",
                "alias_present",
                "codex_home_neutral",
            },
        )
        self.assertEqual(facts["v"], 1)
        self.assertEqual(
            set(facts["fallback"]), {"fallback_providers", "fallback_model", "effective"}
        )
        self.assertEqual(set(facts["finite_private"]), {"provider_entry", "fp_key"})
        self.assertEqual(
            set(facts["openrouter"]),
            {"hermes_key", "hermes_key_fingerprint", "dotenv_key", "manual_pool_entries"},
        )
        self.assertEqual(
            set(facts["codex"]), {"state", "quota_reset_at", "reported_quota_reset_at"}
        )
        self.assertEqual(set(facts["session_overrides"]), {"openrouter", "openai_codex"})
        self.assertEqual(facts["saved_route"], "finite_private")
        self.assertEqual(facts["codex_home_neutral"], "no")
        launch_facts = self.run_json(
            "inference-facts",
            env={"OPENAI_API_KEY": "sk-personal-fake", "CODEX_HOME": helper.NEUTRAL_CODEX_HOME},
        )
        self.assertEqual(
            (launch_facts["alias_present"], launch_facts["codex_home_neutral"]), ("yes", "yes")
        )
        self.write_auth(two_route_auth())
        self.assertEqual(self.facts()["openrouter"]["manual_pool_entries"], "present")

    def openrouter(self, dotenv=None, env=None):
        if dotenv is not None:
            (self.hermes / ".env").write_text(dotenv, encoding="utf-8")
        with patch.dict(os.environ, env or {}):
            return self.facts()["openrouter"]

    def test_openrouter_key_resolution(self):
        def fingerprint(value):
            return hashlib.sha256(value.encode()).hexdigest()

        facts = self.openrouter()
        self.assertEqual(
            (facts["hermes_key"], facts["dotenv_key"], facts["hermes_key_fingerprint"]),
            ("absent", "absent", None),
        )

        facts = self.openrouter(env={"OPENROUTER_API_KEY": "sk-or-v1-process"})
        self.assertEqual((facts["hermes_key"], facts["dotenv_key"]), ("present", "absent"))
        self.assertEqual(facts["hermes_key_fingerprint"], fingerprint("sk-or-v1-process"))

        facts = self.openrouter(
            "OPENROUTER_API_KEY=sk-or-v1-first\nOPENROUTER_API_KEY=sk-or-v1-last\n",
            env={"OPENROUTER_API_KEY": "sk-or-v1-process"},
        )
        self.assertEqual((facts["hermes_key"], facts["dotenv_key"]), ("present", "present"))
        self.assertEqual(facts["hermes_key_fingerprint"], fingerprint("sk-or-v1-last"))

        facts = self.openrouter(
            "OPENROUTER_API_KEY=${MY_ROUTER_KEY}\n", env={"MY_ROUTER_KEY": "sk-or-v1-interp"}
        )
        self.assertEqual(facts["hermes_key_fingerprint"], fingerprint("sk-or-v1-interp"))

        for dotenv in (
            "OPENROUTER_API_KEY=${UNSET_ROUTER_KEY}\n",
            "OPENROUTER_API_KEY='${LITERAL}'\n",
            "OPENROUTER_API_KEY=\n",
        ):
            with self.subTest(dotenv=dotenv):
                facts = self.openrouter(dotenv, env={"OPENROUTER_API_KEY": "sk-or-v1-process"})
                self.assertEqual((facts["hermes_key"], facts["dotenv_key"]), ("absent", "present"))
                self.assertIsNone(facts["hermes_key_fingerprint"])

        (self.hermes / ".op.env").write_text("OP_SERVICE_ACCOUNT_TOKEN=fake\n")
        facts = self.openrouter("OPENROUTER_API_KEY=sk-or-v1-last\n")
        self.assertEqual(
            (facts["hermes_key"], facts["dotenv_key"], facts["hermes_key_fingerprint"]),
            ("unknown", "present", None),
        )
        self.assertEqual(self.facts()["finite_private"]["fp_key"], "unknown")

    def test_finite_private_key(self):
        self.assertEqual(self.facts()["finite_private"]["fp_key"], "absent")
        with patch.dict(os.environ, {"FINITE_PRIVATE_API_KEY": "fp-fake-process"}):
            self.assertEqual(self.facts()["finite_private"]["fp_key"], "present")
            (self.hermes / ".env").write_text("FINITE_PRIVATE_API_KEY=\n")
            self.assertEqual(self.facts()["finite_private"]["fp_key"], "absent")
        (self.hermes / ".env").write_text("FINITE_PRIVATE_API_KEY=fp-fake-dotenv\n")
        self.assertEqual(self.facts()["finite_private"]["fp_key"], "present")

    def test_saved_route_and_fallback_facts(self):
        self.assertEqual(self.facts()["saved_route"], "other")
        self.assertEqual(
            self.facts()["fallback"],
            {"fallback_providers": "absent", "fallback_model": "absent", "effective": []},
        )
        for model, route in (
            ({"default": FP_MODEL, "provider": "finite-private"}, "finite_private"),
            ({"default": "anthropic/claude-sonnet-4.6", "provider": "openrouter"}, "openrouter"),
            ({"default": "gpt-5.5", "provider": "openai-codex"}, "openai_codex"),
            ({"default": "m", "provider": "custom", "base_url": "https://fp.invalid/v1"}, "other"),
        ):
            with self.subTest(model=model):
                self.write_config({"model": model})
                self.assertEqual(self.facts()["saved_route"], route)
        with patch.dict(os.environ, {"FINITE_CONFIG_FP_BASE_URL": "https://fp.invalid/v1/"}):
            self.assertEqual(self.facts()["saved_route"], "finite_private")

        self.write_config({"model": fp_model_block(), "fallback_providers": [canonical_entry()]})
        fallback = self.facts()["fallback"]
        self.assertEqual(fallback["fallback_providers"], "present")
        self.assertEqual(
            fallback["effective"],
            [{"provider": "finite-private", "model": FP_MODEL, "owned_canonical": "yes"}],
        )

        extra = dict(canonical_entry(), api_key="${FINITE_PRIVATE_API_KEY}")
        self.write_config(
            {
                "fallback_providers": [extra, {"provider": "openrouter", "model": "x/y"}],
                "fallback_model": {"provider": "nous", "model": "z"},
            }
        )
        fallback = self.facts()["fallback"]
        self.assertEqual(
            (fallback["fallback_providers"], fallback["fallback_model"]), ("present", "present")
        )
        self.assertEqual(
            [entry["owned_canonical"] for entry in fallback["effective"]], ["no", "no", "no"]
        )
        self.assertEqual(
            [entry["provider"] for entry in fallback["effective"]],
            ["finite-private", "openrouter", "nous"],
        )

        self.write_config({"fallback_providers": [canonical_entry()]})
        with patch.dict(os.environ, {"FINITE_CONFIG_FP_MODEL": "other-model"}):
            self.assertEqual(self.facts()["fallback"]["effective"][0]["owned_canonical"], "no")
        with patch.dict(os.environ, {"FINITE_CONFIG_FP_MODEL": ""}):
            self.assertEqual(self.facts()["fallback"]["effective"][0]["owned_canonical"], "no")

        self.write_config({"fallback_providers": []})
        self.assertEqual(
            self.facts()["fallback"],
            {"fallback_providers": "present", "fallback_model": "absent", "effective": []},
        )

    def test_canonical_copies_equal_what_the_reconciler_writes(self):
        """The reconciler owns both entries; the helper keeps copies to
        recognize them. A key on one side only makes status report the image's
        own entries as modified."""
        reconcile = runpy.run_path(str(RECONCILER))["_reconcile_finite_private_route"]
        for model_id, base_url, context_length in (
            (FP_MODEL, FP_BASE_URL, "393216"),
            (FP_MODEL, "https://kimi-k2-6.finite.containers.tinfoil.dev/v1", ""),
            ("glm-6", "http://127.0.0.1:8787/v1", ""),
        ):
            settings = {
                "FINITE_CONFIG_FP_MODEL": model_id,
                "FINITE_CONFIG_FP_BASE_URL": base_url,
                "FINITE_CONFIG_FP_CONTEXT_LENGTH": context_length,
            }
            with self.subTest(**settings), patch.dict(os.environ, settings):
                config = {"model": fp_model_block()}
                reconcile(config, {**settings, "FINITE_CONFIG_FP_KEY_PRESENT": "1"}, None)
                self.assertEqual(
                    config["providers"]["finite-private"],
                    helper._canonical_provider(*helper._fp_settings()),
                )
                self.assertEqual(
                    config["fallback_providers"], [helper._canonical_entry(model_id, base_url)]
                )
                self.assertEqual(helper._provider_entry_fact(config), "canonical")
                config["providers"]["finite-private"]["discover_models"] = True
                self.assertEqual(helper._provider_entry_fact(config), "modified")
                self.assertEqual(
                    [
                        entry["owned_canonical"]
                        for entry in helper._fallback_facts(config)["effective"]
                    ],
                    ["yes"],
                )

    def test_session_override_facts(self):
        self.assertEqual(
            self.facts()["session_overrides"], {"openrouter": "absent", "openai_codex": "absent"}
        )
        self.seed_overrides(["codex", None])
        self.assertEqual(
            self.facts()["session_overrides"], {"openrouter": "absent", "openai_codex": "present"}
        )
        self.seed_overrides(["codex", "openrouter"])
        self.assertEqual(
            self.facts()["session_overrides"], {"openrouter": "present", "openai_codex": "present"}
        )

    def test_one_failing_section_does_not_fail_the_others(self):
        (self.hermes / "auth.json").write_text("{not json")
        (self.hermes / ".env").write_text("OPENROUTER_API_KEY=sk-or-v1-fake\n")
        self.write_config({"model": {"default": "x", "provider": "openrouter"}})
        facts = self.run_json("inference-facts")
        self.assertEqual(
            facts["codex"],
            {"state": "unknown", "quota_reset_at": None, "reported_quota_reset_at": None},
        )
        self.assertEqual(facts["openrouter"]["manual_pool_entries"], "unknown")
        self.assertEqual(facts["openrouter"]["hermes_key"], "present")
        self.assertEqual(facts["saved_route"], "openrouter")
        self.assertFalse((self.hermes / "auth.json.corrupt").exists())

        (self.hermes / "auth.json").unlink()
        (self.hermes / "config.yaml").write_text("model: [unclosed\n")
        facts = self.facts()
        self.assertEqual(facts["saved_route"], "unknown")
        self.assertEqual(
            facts["fallback"],
            {"fallback_providers": "unknown", "fallback_model": "unknown", "effective": None},
        )
        self.assertEqual(facts["finite_private"]["provider_entry"], "unknown")
        self.assertEqual(
            facts["session_overrides"], {"openrouter": "unknown", "openai_codex": "unknown"}
        )
        self.assertEqual(facts["codex"]["state"], "not_signed_in")
        self.assertEqual(facts["openrouter"]["hermes_key"], "present")
        self.assertEqual(
            sorted(path.name for path in self.hermes.glob("config.yaml*")), ["config.yaml"]
        )

    def test_status_read_never_writes(self):
        """The helper never rewrites .env, config.yaml, auth.json, or the session store."""
        self.write_config({"model": fp_model_block(), "fallback_providers": [canonical_entry()]})
        self.write_auth(
            json.loads((FIXTURES / "codex-auth/valid-singleton-quota-pool.json").read_text())[
                "auth"
            ]
        )
        store, _ = self.seed_overrides(["openrouter", "openai-codex", None])
        store.close_all_db_handles()
        for name, dotenv in (
            ("crlf", b"OPENROUTER_API_KEY=sk-or-v1-crlf\r\n  FINITE_PRIVATE_API_KEY=fp-fake\r\n"),
            ("bom", codecs.BOM_UTF8 + b"OPENROUTER_API_KEY=sk-or-v1-bom\nOTHER=\x00value\n"),
        ):
            with self.subTest(dotenv=name):
                (self.hermes / ".env").write_bytes(dotenv)
                control = self.root / f"control-{name}.env"
                control.write_bytes(dotenv)
                from hermes_cli.env_loader import _sanitize_env_file_if_needed

                _sanitize_env_file_if_needed(control)
                self.assertNotEqual(
                    control.read_bytes(), dotenv, "Hermes' own loader would rewrite this file"
                )

                before = snapshot(self.hermes)
                facts = self.run_json("inference-facts")
                after = snapshot(self.hermes)
                self.assertEqual(facts["openrouter"]["hermes_key"], "present")
                self.assertEqual(
                    facts["session_overrides"], {"openrouter": "present", "openai_codex": "present"}
                )
                self.assertEqual(facts["codex"]["state"], "signed_in")
                for path, (content, mtime) in before.items():
                    if path not in READ_SIDECARS:
                        self.assertEqual(after.get(path), (content, mtime), path)
                self.assertLessEqual(set(after) - set(before), READ_SIDECARS)
                self.assertEqual((self.hermes / ".env").read_bytes(), dotenv)


class CodexClassifierTests(HelperCase):
    """the classifier agrees with the pinned resolver on every fixture."""

    def resolve(self):
        from hermes_cli.runtime_provider import resolve_runtime_provider

        with (
            patch.object(auth, "_recover_codex_tokens_from_cli", return_value=None),
            patch.object(auth, "_probe_codex_quota_restored", return_value=False),
        ):
            try:
                resolve_runtime_provider(requested="openai-codex")
            except auth.AuthError as error:
                return "rate_limited" if auth.is_rate_limited_auth_error(error) else str(error.code)
        return "resolved"

    def test_classifier_agrees_with_resolver(self):
        fixtures = sorted((FIXTURES / "codex-auth").glob("*.json"))
        self.assertEqual(len(fixtures), 10)
        for path in fixtures:
            case = json.loads(path.read_text())
            with self.subTest(fixture=path.name):
                self.write_auth(case["auth"])
                expected = case["expected"]
                self.assertEqual(
                    self.facts()["codex"],
                    {
                        key: expected[key]
                        for key in ("state", "quota_reset_at", "reported_quota_reset_at")
                    },
                )
                self.assertEqual(self.resolve(), expected["resolver"])

    def test_cooldown_passage_without_a_write(self):
        case = json.loads((FIXTURES / "codex-auth/stripped-singleton-quota-pool.json").read_text())
        self.write_auth(case["auth"])
        before = (
            (self.hermes / "auth.json").read_bytes(),
            (self.hermes / "auth.json").stat().st_mtime_ns,
        )
        self.assertEqual(self.facts()["codex"]["state"], "quota_limited")
        reset_at = case["expected"]["quota_reset_at"]
        with patch("time.time", return_value=reset_at + 1):
            self.assertEqual(
                self.facts()["codex"],
                {"state": "signed_in", "quota_reset_at": None, "reported_quota_reset_at": None},
            )
        self.assertEqual(
            (
                (self.hermes / "auth.json").read_bytes(),
                (self.hermes / "auth.json").stat().st_mtime_ns,
            ),
            before,
        )


class ClearTests(HelperCase):
    """Credential clearing preserves unrelated providers and session state."""

    def test_clear_auth_scope(self):
        self.write_auth(two_route_auth())
        self.assertEqual(
            self.run_json("clear-auth", "--provider", "openai-codex"), {"cleared": "yes"}
        )
        store = self.read_auth()
        self.assertEqual(set(store["providers"]), {"nous"})
        self.assertEqual(set(store["credential_pool"]), {"openrouter"})
        self.assertIsNone(store["active_provider"])
        self.assertEqual(
            self.run_json("clear-auth", "--provider", "openai-codex"), {"cleared": "no"}
        )
        self.assertEqual(
            self.run_json("clear-auth", "--provider", "openrouter"), {"cleared": "yes"}
        )
        store = self.read_auth()
        self.assertEqual((set(store["providers"]), store["credential_pool"]), ({"nous"}, {}))

    def test_clear_session_overrides(self):
        _, keys = self.seed_overrides(["openai-codex", "codex", "openrouter", None, "OpenAI_Codex"])
        self.assertEqual(
            self.run_json(
                "clear-session-overrides", "--provider", "openai-codex", "codex", "openai_codex"
            ),
            {"cleared": 3},
        )
        self.assertEqual(
            self.persisted_overrides(),
            dict(zip(keys, [None, None, "openrouter", None, None], strict=True)),
        )
        self.assertEqual(
            self.run_json("clear-session-overrides", "--provider", "openai-codex"), {"cleared": 0}
        )

    def test_live_stores_can_restore_cleared_state(self):
        live, keys = self.seed_overrides(["openrouter", None])
        self.assertEqual(
            self.run_json("clear-session-overrides", "--provider", "openrouter"), {"cleared": 1}
        )
        self.assertIsNone(self.persisted_overrides()[keys[0]])
        live.set_model_override(keys[1], {"provider": "nous", "model": "m"})
        self.assertEqual(self.persisted_overrides()[keys[0]], "openrouter")

        from agent.credential_pool import load_pool

        auth._save_codex_tokens({"access_token": fake_jwt("live"), "refresh_token": "fake-rt"})
        pool = load_pool("openai-codex")
        self.assertEqual(
            self.run_json("clear-auth", "--provider", "openai-codex"), {"cleared": "yes"}
        )
        pool.mark_exhausted_and_rotate(status_code=429)
        self.assertIn("openai-codex", self.read_auth()["credential_pool"])


SAVED_ROUTE_MODELS = {
    "openrouter": {"default": "anthropic/claude-sonnet-4.6", "provider": "openrouter"},
    "openai_codex": {"default": "gpt-5.5", "provider": "codex"},
}


class PendingDisconnectTests(HelperCase):
    """F1, The launcher's pending-disconnect step."""

    PHASES = (
        "accepted",
        "login_cancelled",
        "route_switched",
        "credential_removed",
        "cleanup",
        "verifying",
    )

    def intent(self, **fields):
        record = {
            "v": 1,
            "id": "op_" + "0" * 32,
            "kind": "disconnect",
            "route": "openrouter",
            "model": None,
            "phase": "cleanup",
            "state": "running",
            "error_code": None,
            "attempts": 0,
            "created_at_ms": 0,
            "updated_at_ms": 0,
        }
        record.update(fields)
        path = self.root / "inference-intent.json"
        path.write_text(json.dumps(record))
        return path

    def prepare(self, saved_model):
        self.write_config({"model": saved_model})
        self.write_auth(two_route_auth())
        (self.hermes / ".env").write_text("OPENROUTER_API_KEY=sk-or-v1-fake-dotenv\n")
        store, keys = self.seed_overrides(["openrouter", "openai-codex", "codex", None])
        store.close_all_db_handles()
        return keys

    def stores(self):
        """The credential stores and the session store, byte for byte."""
        names = ("auth.json", ".env", "state.db", "sessions/sessions.json")
        return {name: (self.hermes / name).read_bytes() for name in names}

    def assert_skipped(self, intent, reason):
        before = self.stores()
        result = helper.apply_pending_disconnect(str(intent))
        self.assertEqual(result, {"applied": "skipped", "reason": reason})
        self.assertEqual(self.stores(), before)

    def test_matrix(self):
        for route, (auth_provider, _) in helper.ROUTE_PROVIDERS.items():
            for phase in self.PHASES:
                for saved_is_route in (True, False):
                    with self.subTest(route=route, phase=phase, saved_is_route=saved_is_route):
                        self.setUp()
                        saved = SAVED_ROUTE_MODELS[route] if saved_is_route else fp_model_block()
                        keys = self.prepare(saved)
                        before = self.stores()
                        result = helper.apply_pending_disconnect(
                            str(self.intent(route=route, phase=phase))
                        )
                        if phase in ("cleanup", "verifying") and not saved_is_route:
                            self.assertEqual(result, {"applied": "yes", "reason": "cleared"})
                            store = self.read_auth()
                            self.assertNotIn(auth_provider, store["providers"])
                            self.assertNotIn(auth_provider, store["credential_pool"])
                            other = "openrouter" if route == "openai_codex" else "openai-codex"
                            self.assertIn(other, store["credential_pool"])
                            remaining = [None, None, None, None]
                            if route == "openai_codex":
                                remaining[0] = "openrouter"
                            else:
                                remaining[1:3] = ["openai-codex", "codex"]
                            self.assertEqual(
                                self.persisted_overrides(), dict(zip(keys, remaining, strict=True))
                            )
                        else:
                            self.assertEqual(result["applied"], "skipped")
                            self.assertEqual(self.stores(), before)

    def test_shared_intent_fixtures(self):
        """agentd's reader runs the same files. With the saved default
        switched away, a lenient reader would clear on any refused record."""
        self.prepare(fp_model_block())
        fixtures = sorted((FIXTURES / "intent").glob("*.json"))
        self.assertGreaterEqual(len(fixtures), 32)
        intent = self.root / "inference-intent.json"
        for path in fixtures:
            case = json.loads(path.read_text(encoding="utf-8"))
            with self.subTest(fixture=path.name):
                intent.write_text(json.dumps(case["record"]))
                if case["valid"]:
                    self.assertEqual(helper._read_intent(intent), case["record"])
                else:
                    self.assert_skipped(intent, "intent_unparsable")

    def test_encodings_agentd_refuses(self):
        """Records that JSON fixtures cannot state. Each verdict was checked
        against serde_json with agentd's `IntentRecord`."""
        self.prepare(fp_model_block())
        text = self.intent().read_text()
        record = json.loads(text)
        refused = {
            "negative zero": text.replace('"attempts": 0', '"attempts": -0'),
            "float version": text.replace('"v": 1', '"v": 1.0'),
            "exponent": text.replace('"attempts": 0', '"attempts": 1e0'),
            "boolean version": text.replace('"v": 1', '"v": true'),
            "NaN": text.replace('"attempts": 0', '"attempts": NaN'),
            "beyond 64 bits": text.replace(
                '"updated_at_ms": 0', '"updated_at_ms": 18446744073709551616'
            ),
            "lone surrogate": text.replace('"model": null', '"model": "\\ud800"'),
            "lone trailing surrogate": text.replace(
                '"error_code": null', '"error_code": "\\udc00"'
            ),
            "duplicate field": text[:-1] + ', "attempts": 1}',
            "duplicate optional field": text[:-1] + ', "model": "x"}',
            "trailing data": text + "{}",
            "BOM": "﻿" + text,
            # serde reads a positional array too; the launcher refuses it.
            "positional array": json.dumps(list(record.values())),
        }
        encoded = {name: value.encode("utf-8") for name, value in refused.items()}
        encoded["UTF-16"] = text.encode("utf-16")
        encoded["not UTF-8"] = text.replace('"model": null', '"model": "caf\xe9"').encode("latin-1")
        intent = self.root / "raw-intent.json"
        for name, raw in encoded.items():
            with self.subTest(record=name):
                intent.write_bytes(raw)
                self.assert_skipped(intent, "intent_unparsable")
        accepted = {
            "surrogate pair": text.replace('"model": null', '"model": "\\ud83d\\ude00"'),
            "escaped NUL": text.replace('"error_code": null', '"error_code": "a\\u0000b"'),
            "largest values": text.replace('"attempts": 0', '"attempts": 4294967295').replace(
                '"updated_at_ms": 0', '"updated_at_ms": 18446744073709551615'
            ),
            "trailing newlines": text + "\n\n",
        }
        for name, value in accepted.items():
            with self.subTest(record=name):
                intent.write_text(value)
                self.assertIsNotNone(helper._read_intent(intent))

    def test_ambiguous_saved_default_changes_nothing(self):
        """only a named provider shows that the
        default moved away from the route."""
        self.prepare(fp_model_block())
        intent = self.intent()
        config = self.hermes / "config.yaml"
        config.unlink()
        self.assert_skipped(intent, "config_missing")
        config.mkdir()
        self.assert_skipped(intent, "config_unreadable")
        config.rmdir()
        for text in ("", "null\n", "[]\n", "just text\n", "model: [unclosed\n"):
            with self.subTest(config=text):
                config.write_text(text)
                self.assert_skipped(intent, "config_unreadable")
        for model in (
            "absent",
            None,
            "anthropic/claude-sonnet-4.6",
            ["openrouter"],
            {},
            {"default": "anthropic/claude-sonnet-4.6"},
            {"provider": None, "default": "x"},
            {"provider": "", "default": "x"},
            {"provider": "  ", "default": "x"},
            {"provider": 7, "default": "x"},
            {"provider": ["openrouter"], "default": "x"},
        ):
            with self.subTest(model=model):
                self.write_config({} if model == "absent" else {"model": model})
                self.assert_skipped(intent, "model_unclassifiable")

    def test_provider_name_normalization_never_clears_the_saved_route(self):
        self.prepare(fp_model_block())
        for route, providers in (
            ("openrouter", ("OpenRouter", "  OPENROUTER  ")),
            ("openai_codex", ("OpenAI-Codex", " CODEX ", "OPENAI_CODEX")),
        ):
            for provider in providers:
                with self.subTest(provider=provider):
                    self.write_config({"model": {"provider": provider, "default": "m"}})
                    self.assert_skipped(self.intent(route=route), "route_still_saved")

    def test_only_a_disconnect_of_an_external_route_clears(self):
        """A select also reaches verifying, and the saved default can differ
        from its route while the executor restarts the gateway."""
        self.prepare(fp_model_block())
        self.assertIn("openrouter", self.read_auth()["credential_pool"])
        self.assertIn("openrouter", self.persisted_overrides().values())
        for fields, reason in (
            ({"kind": "select", "phase": "verifying"}, "not_a_disconnect"),
            ({"route": "finite_private"}, "route_not_disconnectable"),
        ):
            with self.subTest(reason):
                self.assert_skipped(self.intent(**fields), reason)

    def test_process_contract(self):
        self.prepare(fp_model_block())
        completed = self.run_helper(
            "apply-pending-disconnect", "--intent", str(self.intent(phase="credential_removed"))
        )
        self.assertEqual(completed.returncode, 0)
        self.assertEqual(
            json.loads(completed.stdout), {"applied": "skipped", "reason": "phase_before_cleanup"}
        )

        (self.hermes / "auth.json").unlink()
        (self.hermes / "auth.json").mkdir()
        completed = self.run_helper("apply-pending-disconnect", "--intent", str(self.intent()))
        self.assertEqual(completed.returncode, 0)
        result = json.loads(completed.stdout)
        self.assertEqual(result, {"applied": "error", "reason": "clear_auth_failed"})
        self.assertIn("openrouter", self.persisted_overrides().values())

        shutil.rmtree(self.hermes / "auth.json")
        completed = self.run_helper("apply-pending-disconnect", "--intent", str(self.intent()))
        self.assertEqual(
            (completed.returncode, json.loads(completed.stdout)),
            (0, {"applied": "yes", "reason": "cleared"}),
        )
        for reason in ("cleared", "clear_auth_failed", "phase_before_cleanup"):
            self.assertTrue(reason.isascii() and len(reason) <= 64)


class PackagedModuleTests(HelperCase):
    """The packaged env carries this helper as ``hermes_cli.finite_inference_helper``."""

    def test_runs_as_packaged_module(self):
        spec = importlib.util.find_spec("hermes_cli.finite_inference_helper")
        self.assertIsNotNone(spec, "the Hermes env does not package the helper")
        self.assertEqual(
            Path(spec.origin).read_bytes(), HELPER.read_bytes(), "stale packaged helper"
        )
        self.write_config({"model": fp_model_block()})
        completed = subprocess.run(
            [sys.executable, "-m", "hermes_cli.finite_inference_helper", "inference-facts"],
            env={"PATH": os.environ.get("PATH", ""), **self.env},
            cwd=self.root,
            capture_output=True,
            text=True,
            timeout=300,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        facts = json.loads(completed.stdout)
        self.assertEqual((facts["v"], facts["saved_route"]), (1, "finite_private"))


class SecretTests(HelperCase):
    """no key or token reaches stdout or stderr."""

    def test_no_secret_in_output(self):
        secrets = {
            "OPENROUTER_API_KEY": "sk-or-v1-FAKESECRETROUTER0001",
            "FINITE_PRIVATE_API_KEY": "fp-FAKESECRETPRIVATE0002",
            "OPENAI_API_KEY": "sk-FAKESECRETALIAS0003",
        }
        access, refresh, pool = (
            fake_jwt("SECRETACCESS0004"),
            "FAKESECRETREFRESH0005",
            "sk-or-v1-FAKESECRETPOOL0006",
        )
        (self.hermes / ".env").write_text(f"OPENROUTER_API_KEY={secrets['OPENROUTER_API_KEY']}\n")
        store = two_route_auth()
        store["providers"]["openai-codex"]["tokens"] = {
            "access_token": access,
            "refresh_token": refresh,
        }
        store["credential_pool"]["openrouter"][0]["access_token"] = pool
        self.write_auth(store)
        self.write_config(
            {
                "model": {
                    "provider": "openrouter",
                    "default": "x",
                    "api_key": secrets["OPENROUTER_API_KEY"],
                }
            }
        )
        self.seed_overrides(["openrouter"])
        needles = [*secrets.values(), access, refresh, pool, "FAKESECRET"]

        runs = [
            ("inference-facts",),
            ("clear-session-overrides", "--provider", "openrouter"),
            ("clear-auth", "--provider", "openai-codex"),
            ("apply-pending-disconnect", "--intent", str(self.root / "missing.json")),
        ]
        (self.root / "bad-config").mkdir()
        outputs = [self.run_helper(*args, env=secrets) for args in runs]
        # Failure paths: a store whose content is a secret, and a config Hermes cannot parse.
        (self.hermes / "auth.json").write_text(f"{secrets['OPENROUTER_API_KEY']} {access}")
        (self.hermes / "config.yaml").write_text(f"model: [{secrets['OPENROUTER_API_KEY']}\n")
        outputs += [self.run_helper(*args, env=secrets) for args in runs[:3]]
        intent = self.root / "intent.json"
        intent.write_text(
            json.dumps(
                {
                    "v": 1,
                    "kind": "disconnect",
                    "route": "openrouter",
                    "phase": secrets["OPENAI_API_KEY"],
                }
            )
        )
        outputs.append(
            self.run_helper("apply-pending-disconnect", "--intent", str(intent), env=secrets)
        )
        for completed in outputs:
            for needle in needles:
                self.assertNotIn(needle, completed.stdout)
                self.assertNotIn(needle, completed.stderr)


if __name__ == "__main__":
    unittest.main()

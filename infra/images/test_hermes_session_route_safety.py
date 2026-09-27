"""Exercise the packaged session-route safety patch without inference.

Every case runs in a fresh interpreter (this file with ``--case``) with a
scratch HERMES_HOME, a neutral CODEX_HOME, proxies that black-hole the
network, and fake credentials only. The session store is a stub on an
uninitialized GatewayRunner, so no gateway starts.
"""

import base64
import itertools
import json
import os
import subprocess
import sys
import tempfile
import unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

FP = "https://private.example.invalid/v1"
OPENROUTER = "https://openrouter.ai/api/v1"
CODEX = "https://chatgpt.com/backend-api/codex"
USERFB = "http://127.0.0.1:9/v1"
BARE = "https://user.example/v1"
TENANT = "https://tenant.example.invalid/TenantA/v1"
NEUTRAL_CODEX_HOME = "/dev/null/finite-codex-home-disabled"
FUTURE = 4102444800
PAST = 1700000000


def _b64(value):
    return base64.urlsafe_b64encode(json.dumps(value).encode()).rstrip(b"=").decode()


CODEX_TOKEN = (
    _b64({"alg": "none"})
    + "."
    + _b64({"exp": FUTURE, "https://api.openai.com/auth": {"chatgpt_account_id": "acct-FAKE"}})
    + ".sig"
)
KEY_ROUTES = {"fp-FAKE": "fp", "sk-or-FAKE": "or", CODEX_TOKEN: "codex"}
ROUTES = {
    "fp": {"provider": "finite-private", "model": "glm-5-3-flash", "base_url": FP},
    "or": {
        "provider": "openrouter",
        "model": "anthropic/claude-sonnet-4.6",
        "base_url": OPENROUTER,
    },
    "codex": {"provider": "openai-codex", "model": "gpt-5.5", "base_url": CODEX},
    "keyless": {"provider": "userfb", "model": "local-model", "base_url": USERFB},
    "barecustom": {"provider": "custom", "model": "x", "base_url": BARE},
}
FP_DEFAULT = {
    "default": "glm",
    "provider": "custom",
    "base_url": FP,
    "api_key": "${FINITE_PRIVATE_API_KEY}",
    "api_mode": "chat_completions",
}
OR_DEFAULT = {
    "default": "anthropic/claude-sonnet-4.6",
    "provider": "openrouter",
    "base_url": OPENROUTER,
    "api_mode": "chat_completions",
}
FP_ENTRY = {
    "provider": "finite-private",
    "model": "glm",
    "base_url": FP,
    "key_env": "FINITE_PRIVATE_API_KEY",
    "api_mode": "chat_completions",
}
FALLBACKS = {
    "off": [],
    "custom": [{"provider": "userfb", "model": "local-model"}],
    "fp": [FP_ENTRY],
}
PROVIDERS = {
    "finite-private": {
        "name": "Finite Private",
        "base_url": FP,
        "key_env": "FINITE_PRIVATE_API_KEY",
        "api_mode": "chat_completions",
        "models": {"glm": {}, "glm-5-3-flash": {}},
    },
    "userfb": {"name": "userfb", "base_url": USERFB, "models": {"local-model": {}}},
    "tenant": {"name": "tenant", "base_url": TENANT, "key_env": "TENANT_KEY", "models": {"m": {}}},
}
OPENROUTER_KEY_MISSING = (
    "No OpenRouter API key is configured for this agent, and no fallback model is available."
)


def config(default, fallback="fp"):
    return {"model": default, "providers": PROVIDERS, "fallback_providers": FALLBACKS[fallback]}


def codex_auth(state):
    """``ok``: signed in; ``revoked``: nothing stored; ``transient``: quota cooldown only."""
    store = {"version": 1, "providers": {}, "credential_pool": {}}
    if state == "ok":
        store["providers"]["openai-codex"] = {
            "tokens": {"access_token": CODEX_TOKEN, "refresh_token": "codex-FAKE-refresh"},
            "auth_mode": "chatgpt",
        }
    elif state == "transient":
        store["providers"]["openai-codex"] = {"tokens": {}, "auth_mode": "chatgpt"}
        store["credential_pool"]["openai-codex"] = [
            {
                "id": "p1",
                "label": "x",
                "auth_type": "oauth",
                "priority": 0,
                "source": "manual:device_code",
                "access_token": CODEX_TOKEN,
                "refresh_token": "codex-FAKE-refresh",
                "last_status": "exhausted",
                "last_status_at": PAST,
                "last_error_code": 429,
                "last_error_reason": "usage_limit_reached",
                "last_error_reset_at": FUTURE,
                "base_url": CODEX,
            }
        ]
    return store


def route_of_url(url):
    url = (url or "").rstrip("/").lower()
    for name, spec in ROUTES.items():
        if url == spec["base_url"].rstrip("/").lower():
            return name
    return "?:" + url


def route_of_key(key):
    if not key or key == "no-key-required" or key.startswith("${"):
        return "none"
    return KEY_ROUTES.get(key, "?")


def run_case(spec, *, home):
    """Run one case in a fresh interpreter; returns its JSON result."""
    environment = {
        "HOME": str(home / "home"),
        "PATH": "/usr/bin:/bin",
        "HERMES_HOME": str(home / "hermes"),
        "CODEX_HOME": NEUTRAL_CODEX_HOME,
        "HTTP_PROXY": "http://127.0.0.1:9",
        "HTTPS_PROXY": "http://127.0.0.1:9",
        "PYTHONDONTWRITEBYTECODE": "1",
        **spec.pop("process_env", {}),
    }
    completed = subprocess.run(
        [sys.executable, __file__, "--case", json.dumps(spec)],
        env=environment,
        capture_output=True,
        text=True,
        timeout=600,
    )
    if completed.returncode != 0 or not completed.stdout.strip():
        raise AssertionError(f"case runner failed: {completed.stderr[-2000:]}")
    return json.loads(completed.stdout.strip().splitlines()[-1])


# --- case runner (fresh interpreter) ----------------------------------------------


def _case_main(spec):
    import yaml

    home = Path(os.environ["HERMES_HOME"])
    home.mkdir(parents=True, exist_ok=True)
    cfg = spec.get("config") or {}
    (home / "config.yaml").write_text(yaml.safe_dump(cfg))
    (home / ".env").write_text(spec.get("dotenv", ""))
    if spec.get("auth") is not None:
        (home / "auth.json").write_text(json.dumps(spec["auth"]))
    os.environ.update(spec.get("env") or {})

    from hermes_cli.env_loader import load_hermes_dotenv

    load_hermes_dotenv(hermes_home=home)
    kind = spec.get("kind", "session")
    if kind == "openrouter":
        from agent.secret_scope import get_secret
        from hermes_cli.runtime_provider import resolve_runtime_provider

        runtime = resolve_runtime_provider(requested="openrouter")
        return {"api_key": runtime.get("api_key"), "image_key": get_secret("OPENAI_API_KEY")}
    if kind == "codex":
        from hermes_cli import auth
        from hermes_cli.runtime_provider import resolve_runtime_provider

        try:
            runtime = resolve_runtime_provider(requested="openai-codex")
        except auth.AuthError as error:
            return {"error": str(error.code)}
        return {"base_url": runtime.get("base_url"), "api_key": runtime.get("api_key")}
    if kind == "fallback_entry":
        return _fallback_entry_case(cfg)
    if kind == "model_switch":
        return _model_switch_case(cfg)
    return _session_case(spec, cfg)


def _session_case(spec, cfg):
    from unittest.mock import patch

    import gateway.run as gateway_run
    from gateway.session import sanitize_model_override
    from hermes_cli import auth

    class Store:
        def __init__(self, saved):
            self.saved = saved

        def get_model_override(self, key):
            return self.saved

        def set_model_override(self, key, value):
            self.saved = value

    runner = object.__new__(gateway_run.GatewayRunner)
    override = spec.get("override")
    runner.session_store = Store(sanitize_model_override(override) if override else None)

    def resolve():
        try:
            model, runtime = runner._resolve_session_agent_runtime(session_key="k", user_config=cfg)
        except Exception as error:
            result = {"error": str(error)}
        else:
            result = {
                "model": model,
                "provider": runtime.get("provider"),
                "base_url": runtime.get("base_url"),
                "api_key": runtime.get("api_key"),
            }
        # Whether the persisted override survived; None when there is none.
        result["kept"] = runner.session_store.saved is not None if override else None
        return result

    results = []
    with (
        patch.object(
            gateway_run,
            "_resolve_runtime_agent_kwargs_for_provider",
            return_value=spec["provider_runtime"],
        )
        if spec.get("provider_runtime")
        else _nothing()
    ):
        for step in spec.get("steps", ["resolve"]):
            if step == "resolve":
                results.append(resolve())
            elif step == "save_codex":
                auth._save_codex_tokens(
                    {"access_token": CODEX_TOKEN, "refresh_token": "codex-FAKE-refresh"}
                )
            elif step == "clear_codex":
                auth.clear_provider_auth("openai-codex")
    return results if len(results) > 1 else results[0]


class _nothing:
    def __enter__(self):
        return None

    def __exit__(self, *exc):
        return False


def _fallback_entry_case(cfg):
    from agent.backend_identity import BackendIdentity, should_skip_candidate
    from hermes_cli.fallback_config import get_fallback_chain, resolve_entry_api_key
    from hermes_cli.runtime_provider import resolve_runtime_provider

    entry = get_fallback_chain(cfg)[0]
    key = resolve_entry_api_key(entry)
    runtime = resolve_runtime_provider(
        requested=entry["provider"], explicit_base_url=entry.get("base_url"), explicit_api_key=key
    )
    candidate = BackendIdentity.build(
        provider=entry["provider"], model=entry["model"], base_url=entry.get("base_url")
    )

    def skipped_against(provider, model, base_url):
        primary = BackendIdentity.build(provider=provider, model=model, base_url=base_url)
        return should_skip_candidate(candidate, primary)

    return {
        "base_url": runtime.get("base_url"),
        "api_key": runtime.get("api_key"),
        "skip_vs_fp": skipped_against("custom", entry["model"], FP),
        "skip_vs_codex": skipped_against("openai-codex", "gpt-5.5", CODEX),
        "skip_vs_openrouter": skipped_against(
            "openrouter", "anthropic/claude-sonnet-4.6", OPENROUTER
        ),
    }


def _model_switch_case(cfg):
    import threading
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

    from hermes_cli.config import (
        get_custom_provider_context_length,
        get_custom_provider_model_capability,
    )
    from hermes_cli.model_switch import switch_model

    requests = []

    class NotFound(BaseHTTPRequestHandler):
        def do_GET(self):
            requests.append(self.path)
            self.send_response(404)
            self.end_headers()

        def log_message(self, *args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), NotFound)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base_url = f"http://127.0.0.1:{server.server_port}/v1"
    providers = json.loads(json.dumps(cfg["providers"]).replace("{FP_URL}", base_url))
    cfg = dict(cfg, providers=providers)
    result = switch_model(
        "glm-5-3-flash",
        current_provider="custom",
        current_model="glm-5-3-flash",
        current_base_url=base_url,
        current_api_key="fp-FAKE",
        explicit_provider="finite-private",
        user_providers=providers,
    )
    server.shutdown()
    return {
        "success": result.success,
        "provider": result.target_provider,
        "same_endpoint": result.base_url.rstrip("/") == base_url,
        "api_key": result.api_key,
        "models_requests": [path for path in requests if path.endswith("/models")],
        "context_length": get_custom_provider_context_length("glm-5-3-flash", base_url, config=cfg),
        "supports_vision": get_custom_provider_model_capability(
            "glm-5-3-flash", base_url, "supports_vision", config=cfg
        ),
    }


# --- tests ----------------------------------------------------------------------------


class RouteSafetyCase(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.scratch = tempfile.TemporaryDirectory(prefix="hermes-route-safety-")

    @classmethod
    def tearDownClass(cls):
        cls.scratch.cleanup()

    def run_cases(self, specs):
        homes = [Path(tempfile.mkdtemp(dir=self.scratch.name)) for _ in specs]
        workers = min(8, os.cpu_count() or 2)
        with ThreadPoolExecutor(max_workers=workers) as pool:
            return list(
                pool.map(
                    lambda pair: run_case(pair[0], home=pair[1]), zip(specs, homes, strict=True)
                )
            )

    def run_one(self, spec):
        return self.run_cases([spec])[0]

    def assert_runtime(self, result, model, base_url, api_key):
        self.assertNotIn("error", result, result)
        self.assertEqual(
            (result["model"], result["base_url"], result["api_key"]), (model, base_url, api_key)
        )
        self.assertIsNot(result["kept"], False, "the persisted override must never be cleared")

    def assert_error(self, result, message):
        self.assertEqual(result.get("error"), message, result)
        self.assertIsNot(result["kept"], False)


def keyless_openrouter_env():
    return {"FINITE_PRIVATE_API_KEY": "fp-FAKE", "OPENAI_API_KEY": "personal-FAKE"}


class OpenRouterBorrowingTests(RouteSafetyCase):
    def test_openrouter_never_returns_openai_api_key(self):
        """T-H1: neither the FP alias nor a personal key; plugins still see a personal key."""
        results = self.run_cases(
            [
                {
                    "kind": "openrouter",
                    "config": config(OR_DEFAULT),
                    "env": {"FINITE_PRIVATE_API_KEY": "fp-FAKE", "OPENAI_API_KEY": value},
                }
                for value in ("fp-FAKE", "personal-FAKE")
            ]
        )
        for result, value in zip(results, ("fp-FAKE", "personal-FAKE"), strict=True):
            with self.subTest(alias=value):
                self.assertEqual(result["api_key"], "")
                self.assertEqual(result["image_key"], value)

    def test_keyless_openrouter_goes_through_fallback_resolution(self):
        """T-H10 (F4): saved default and session override by fallback fp / off / custom."""
        specs = []
        for fallback in ("fp", "off", "custom"):
            specs.append({"config": config(OR_DEFAULT, fallback), "env": keyless_openrouter_env()})
            specs.append(
                {
                    "config": config(FP_DEFAULT, fallback),
                    "env": keyless_openrouter_env(),
                    "override": {"provider": "openrouter", "model": "x/y", "base_url": OPENROUTER},
                }
            )
        default_fp, override_fp, default_off, override_off, default_custom, override_custom = (
            self.run_cases(specs)
        )
        self.assert_runtime(default_fp, "glm", FP, "fp-FAKE")
        self.assert_runtime(override_fp, "glm", FP, "fp-FAKE")
        self.assert_error(default_off, OPENROUTER_KEY_MISSING)
        self.assert_error(override_off, "credentials for provider 'openrouter' are not configured")
        self.assert_runtime(default_custom, "local-model", USERFB, "no-key-required")
        self.assert_runtime(override_custom, "local-model", USERFB, "no-key-required")


class EndpointIdentityTests(RouteSafetyCase):
    def tenant(self, base_url, fallback="fp", env=None):
        return {
            "config": config(FP_DEFAULT, fallback),
            "env": {
                "FINITE_PRIVATE_API_KEY": "fp-FAKE",
                "TENANT_KEY": "tenant-A-FAKE",
                **(env or {}),
            },
            "override": {"provider": "tenant", "model": "m", "base_url": base_url},
        }

    def test_endpoint_identity(self):
        """T-H11 (F5): path case is exact; scheme/host case and a trailing slash are not."""
        upper = "HTTPS://Tenant.Example.INVALID/TenantA/v1/"
        path_fp, path_off, host, port, unparsable, same, mirror = self.run_cases(
            [
                self.tenant("https://tenant.example.invalid/tenanta/v1"),
                self.tenant("https://tenant.example.invalid/tenanta/v1", "off"),
                self.tenant(upper),
                self.tenant("https://tenant.example.invalid:8443/TenantA/v1"),
                self.tenant("not a url"),
                self.tenant(TENANT),
                {
                    "config": config(FP_DEFAULT),
                    "env": {
                        "FINITE_PRIVATE_API_KEY": "fp-FAKE",
                        "OPENROUTER_API_KEY": "sk-or-FAKE",
                        "OPENROUTER_BASE_URL": "https://mirror.example.invalid/v1",
                    },
                    "override": {"provider": "openrouter", "model": "m", "base_url": OPENROUTER},
                },
            ]
        )
        self.assert_runtime(path_fp, "glm", FP, "fp-FAKE")
        self.assert_error(
            path_off, "credentials for provider 'tenant' are bound to a different endpoint"
        )
        self.assert_runtime(host, "m", upper, "tenant-A-FAKE")
        self.assert_runtime(port, "glm", FP, "fp-FAKE")
        self.assert_runtime(unparsable, "glm", FP, "fp-FAKE")
        self.assert_runtime(same, "m", TENANT, "tenant-A-FAKE")
        self.assert_runtime(mirror, "glm", FP, "fp-FAKE")

    def test_uncertain_binding_without_fallback_is_refused(self):
        result = self.run_one(self.tenant("not a url", "off"))
        self.assert_error(
            result, "credentials for provider 'tenant' have an uncertain endpoint binding"
        )


class SessionBoundaryTests(RouteSafetyCase):
    def test_endpoint_without_provider_and_blank_resolved_endpoint(self):
        """T-H6 (X8)."""
        stub = {"provider": "custom", "api_key": "fp-FAKE", "base_url": None}
        specs = []
        for fallback in ("fp", "off"):
            base = {
                "config": config(FP_DEFAULT, fallback),
                "env": {"FINITE_PRIVATE_API_KEY": "fp-FAKE"},
            }
            specs.append(
                dict(base, override={"model": "other", "base_url": "https://other.invalid/v1"})
            )
            specs.append(
                dict(
                    base,
                    override={
                        "provider": "custom",
                        "model": "other",
                        "base_url": "https://other.invalid/v1",
                    },
                    provider_runtime=stub,
                )
            )
        providerless_fp, blank_fp, providerless_off, blank_off = self.run_cases(specs)
        self.assert_runtime(providerless_fp, "glm", FP, "fp-FAKE")
        self.assert_runtime(blank_fp, "glm", FP, "fp-FAKE")
        self.assert_error(
            providerless_off, "session model override names an endpoint but no provider"
        )
        self.assert_error(
            blank_off, "credentials for provider 'custom' have no verified endpoint binding"
        )

    def test_model_only_override_inherits_the_default(self):
        """T-H8."""
        for result in self.run_cases(
            [
                {
                    "config": config(FP_DEFAULT, fallback),
                    "env": {"FINITE_PRIVATE_API_KEY": "fp-FAKE"},
                    "override": {"model": "glm-other"},
                }
                for fallback in ("fp", "off")
            ]
        ):
            self.assert_runtime(result, "glm-other", FP, "fp-FAKE")

    def test_keyless_named_custom_override(self):
        """T-H4."""
        result = self.run_one(
            {
                "config": config(FP_DEFAULT, "off"),
                "env": {"FINITE_PRIVATE_API_KEY": "fp-FAKE", "OPENAI_API_KEY": "fp-FAKE"},
                "override": {"provider": "userfb", "model": "local-model", "base_url": USERFB},
            }
        )
        self.assertNotIn("error", result, result)
        self.assertEqual((result["model"], result["base_url"]), ("local-model", USERFB))
        self.assertEqual(route_of_key(result["api_key"]), "none")

    def test_codex_failure_recovery_removal(self):
        """T-H3 (X8): one conversation across a sign-in and a removal."""
        turns = self.run_one(
            {
                "config": config(FP_DEFAULT),
                "env": {"FINITE_PRIVATE_API_KEY": "fp-FAKE", "OPENAI_API_KEY": "fp-FAKE"},
                "auth": codex_auth("revoked"),
                "override": ROUTES["codex"],
                "steps": ["resolve", "save_codex", "resolve", "clear_codex", "resolve"],
            }
        )
        self.assert_runtime(turns[0], "glm", FP, "fp-FAKE")
        self.assert_runtime(turns[1], "gpt-5.5", CODEX, CODEX_TOKEN)
        # The captured token lasts until restart, which disconnect performs.
        self.assert_runtime(turns[2], "gpt-5.5", CODEX, CODEX_TOKEN)

    def test_codex_never_uses_openai_api_key(self):
        """T-H7: at the resolver, the saved default, and a session override."""
        env = {"FINITE_PRIVATE_API_KEY": "fp-FAKE", "OPENAI_API_KEY": "fp-FAKE"}
        codex_default = {"default": "gpt-5.5", "provider": "openai-codex"}
        specs = [{"kind": "codex", "config": config(FP_DEFAULT), "env": env}]
        for fallback in ("fp", "off"):
            specs.append({"config": config(codex_default, fallback), "env": env})
            specs.append(
                {"config": config(FP_DEFAULT, fallback), "env": env, "override": ROUTES["codex"]}
            )
        for spec in specs:
            spec["auth"] = codex_auth("revoked")
        resolver, default_fp, override_fp, default_off, override_off = self.run_cases(specs)
        self.assertEqual(resolver, {"error": "codex_auth_missing"})
        self.assert_runtime(default_fp, "glm", FP, "fp-FAKE")
        self.assert_runtime(override_fp, "glm", FP, "fp-FAKE")
        for result in (default_off, override_off):
            self.assertIn("error", result, result)
            self.assertIsNot(result["kept"], False)


class NormalServingTests(RouteSafetyCase):
    def test_saved_defaults_resolve_as_before(self):
        """A plain FP agent, OpenRouter with its own key, and a user's own endpoint."""
        own = {
            "default": "local-model",
            "provider": "custom",
            "base_url": "https://llm.example.invalid/v1",
            "api_key": "user-own-FAKE",
            "api_mode": "chat_completions",
        }
        env = {"FINITE_PRIVATE_API_KEY": "fp-FAKE", "OPENAI_API_KEY": "fp-FAKE"}
        fp, openrouter, custom = self.run_cases(
            [
                {"config": config(FP_DEFAULT), "env": env},
                {
                    "config": config(OR_DEFAULT),
                    "env": env,
                    "dotenv": "OPENROUTER_API_KEY=sk-or-FAKE\n",
                },
                {"config": config(own, "off"), "env": env},
            ]
        )
        self.assertEqual((fp["model"], fp["base_url"], fp["api_key"]), ("glm", FP, "fp-FAKE"))
        self.assertEqual(
            (
                openrouter["model"],
                openrouter["provider"],
                openrouter["base_url"],
                openrouter["api_key"],
            ),
            ("anthropic/claude-sonnet-4.6", "openrouter", OPENROUTER, "sk-or-FAKE"),
        )
        self.assertEqual(
            (custom["model"], custom["base_url"], custom["api_key"]),
            ("local-model", "https://llm.example.invalid/v1", "user-own-FAKE"),
        )

    def test_canonical_fp_fallback_entry(self):
        """T-H5 (E1): the FP entry resolves its own key and skips itself against the FP primary."""
        result = self.run_one(
            {
                "kind": "fallback_entry",
                "config": config(FP_DEFAULT),
                "env": {"FINITE_PRIVATE_API_KEY": "fp-FAKE", "OPENAI_API_KEY": "personal-FAKE"},
            }
        )
        self.assertEqual(
            result,
            {
                "base_url": FP,
                "api_key": "fp-FAKE",
                "skip_vs_fp": True,
                "skip_vs_codex": False,
                "skip_vs_openrouter": False,
            },
        )

    def test_finite_private_model_switch_with_404_models(self):
        """T-H9: the switch succeeds on FP's own endpoint; the models map supplies capabilities."""
        providers = json.loads(json.dumps(PROVIDERS))
        providers["finite-private"]["base_url"] = "{FP_URL}"
        providers["finite-private"]["models"] = {
            "glm-5-3-flash": {"context_length": 393216, "supports_vision": True}
        }
        result = self.run_one(
            {
                "kind": "model_switch",
                "config": {"model": FP_DEFAULT, "providers": providers},
                "env": {"FINITE_PRIVATE_API_KEY": "fp-FAKE"},
                "process_env": {"NO_PROXY": "127.0.0.1"},
            }
        )
        self.assertTrue(result["success"], result)
        self.assertEqual(
            (result["provider"], result["same_endpoint"], result["api_key"]),
            ("finite-private", True, "fp-FAKE"),
        )
        self.assertTrue(result["models_requests"], "validation must have met the 404 /models")
        self.assertEqual((result["context_length"], result["supports_vision"]), (393216, True))


def matrix_specs():
    """T-H2: saved default, override, fallback, credentials, alias (120 cases)."""
    defaults = {
        "fp": dict(FP_DEFAULT, default="glm-5-3-flash"),
        "or": OR_DEFAULT,
        "codex": {"default": "gpt-5.5", "provider": "openai-codex"},
    }
    cases = []
    for default, override, fallback, creds, alias in itertools.product(
        ("fp", "or", "codex"),
        ("fp", "or", "codex", "keyless", "barecustom"),
        ("off", "custom", "fp"),
        ("ok", "revoked", "transient"),
        (0, 1),
    ):
        if default == override or (creds == "transient" and override != "codex"):
            continue
        if creds != "ok" and override in ("keyless", "barecustom"):
            continue
        env = {}
        if not (override == "fp" and creds == "revoked"):
            env["FINITE_PRIVATE_API_KEY"] = "fp-FAKE"
        if alias:
            env["OPENAI_API_KEY"] = env.get("FINITE_PRIVATE_API_KEY", "fp-FAKE")
        with_openrouter_key = (override == "or" and creds == "ok") or (
            default == "or" and not (override == "or" and creds != "ok")
        )
        fallback_entries = (
            [dict(FP_ENTRY, model="glm-5-3-flash")] if fallback == "fp" else FALLBACKS[fallback]
        )
        spec = {
            "label": f"default={default} override={override} fallback={fallback} creds={creds} alias={alias}",
            "config": {
                "model": defaults[default],
                "providers": PROVIDERS,
                "fallback_providers": fallback_entries,
            },
            "env": env,
            "dotenv": "OPENROUTER_API_KEY=sk-or-FAKE\n" if with_openrouter_key else "",
            "override": ROUTES[override],
        }
        if "codex" in (default, override):
            spec["auth"] = codex_auth(creds if override == "codex" else "ok")
        cases.append(spec)
    return cases


class MatrixTests(RouteSafetyCase):
    def test_no_route_ever_carries_another_routes_key(self):
        """T-H2: 0 of 120 mixed, 120 of 120 persisted overrides kept."""
        specs = matrix_specs()
        self.assertEqual(len(specs), 120)
        labels = [spec.pop("label") for spec in specs]
        mixed, dropped = [], []
        for label, result in zip(labels, self.run_cases(specs), strict=True):
            if not result["kept"]:
                dropped.append(label)
            if "error" not in result:
                endpoint, key = route_of_url(result["base_url"]), route_of_key(result["api_key"])
                if key not in ("none", endpoint):
                    mixed.append(f"{label}: {endpoint} endpoint with the {key} key")
        self.assertEqual(mixed, [], f"{len(mixed)} of {len(specs)} cases mixed routes")
        self.assertEqual(dropped, [], f"{len(dropped)} of {len(specs)} overrides dropped")


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--case":
        sys.stdout.write(json.dumps(_case_main(json.loads(sys.argv[2]))) + "\n")
    else:
        unittest.main()

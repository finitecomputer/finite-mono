"""Finite inference helper, sealed into Hermes' Python package.

agentd runs ``python -m hermes_cli.finite_inference_helper <subcommand>`` with
the environment a Hermes process gets from it. The launcher runs
``apply-pending-disconnect`` before ``exec hermes``, while no gateway exists.
Every subcommand prints JSON on stdout. Output and diagnostics never carry a
key, token, or upstream text: only enums, counts, and SHA-256 fingerprints.
"""

from __future__ import annotations

import argparse
import codecs
import contextlib
import hashlib
import io
import json
import logging
import os
import sys
import urllib.parse
from pathlib import Path
from typing import Any

import yaml
from dotenv import dotenv_values

FINITE_PRIVATE_PRODUCT_BASE_URL = "https://finite-private.finite.containers.tinfoil.dev/v1"
HISTORICAL_FINITE_PRIVATE_BASE_URL = "https://kimi-k2-6.finite.containers.tinfoil.dev/v1"
FINITE_PRIVATE_KEY_ENV = "FINITE_PRIVATE_API_KEY"
NEUTRAL_CODEX_HOME = "/dev/null/finite-codex-home-disabled"

CODEX_PROVIDERS = ("openai-codex", "codex", "openai_codex")
# Route → (auth.json provider id, session override provider names).
ROUTE_PROVIDERS = {
    "openrouter": ("openrouter", ("openrouter",)),
    "openai_codex": ("openai-codex", CODEX_PROVIDERS),
}
# The version-1 intent record as agentd's reader (`intent.rs`) accepts it.
INTENT_FIELDS = frozenset(
    (
        "v",
        "id",
        "kind",
        "route",
        "model",
        "phase",
        "state",
        "error_code",
        "attempts",
        "created_at_ms",
        "updated_at_ms",
    )
)
INTENT_OPTIONAL_FIELDS = frozenset(("model", "error_code"))
INTENT_ROUTES = ("finite_private", "openrouter", "openai_codex")
INTENT_STATES = ("running", "failed")
_SELECT_PHASES = ("accepted", "config_written", "restarting", "verifying")
INTENT_KIND_PHASES = {
    "select": _SELECT_PHASES,
    "activate": _SELECT_PHASES,
    "disconnect": (
        "accepted",
        "login_cancelled",
        "route_switched",
        "credential_removed",
        "cleanup",
        "verifying",
    ),
}
# F1: clears run only once the disconnect has reached cleanup.
CLEAR_PHASES = ("cleanup", "verifying")


def _hermes_home() -> Path:
    from hermes_constants import get_hermes_home

    return get_hermes_home()


def _endpoint_identity(url: Any) -> tuple | None:
    """Scheme and host compare case-insensitively, a trailing path `/` is
    ignored, and everything else compares exactly (§5.5)."""
    if not isinstance(url, str) or not url:
        return None
    try:
        parts = urllib.parse.urlsplit(url)
        port = parts.port
    except ValueError:
        return None
    if not parts.scheme or not parts.hostname:
        return None
    return (
        parts.scheme.lower(),
        parts.hostname.lower(),
        port,
        parts.username,
        parts.password,
        parts.path.rstrip("/"),
        parts.query,
        parts.fragment,
    )


def classify_saved_route(model: Any, fp_base_url: str | None = None) -> str:
    """Port of agentd's §3.3 classifier over raw ``config.yaml`` ``model``."""
    if not isinstance(model, dict):
        return "other"
    provider = model.get("provider")
    provider = provider.strip().lower() if isinstance(provider, str) else None
    if provider == "openrouter":
        return "openrouter"
    if provider in CODEX_PROVIDERS:
        return "openai_codex"
    if provider in ("finite-private", "custom:finite-private"):
        return "finite_private"
    if provider == "custom":
        identity = _endpoint_identity(model.get("base_url"))
        if identity is not None and identity in {
            _endpoint_identity(url)
            for url in (
                fp_base_url,
                FINITE_PRIVATE_PRODUCT_BASE_URL,
                HISTORICAL_FINITE_PRIVATE_BASE_URL,
            )
        }:
            return "finite_private"
    return "other"


def _override_provider(override: Any) -> str:
    if not isinstance(override, dict):
        return ""
    return str(override.get("provider") or "").strip().lower()


def _read_config(home: Path) -> dict:
    """Raw ``config.yaml``; raises when it exists but cannot be parsed."""
    path = home / "config.yaml"
    if not path.exists():
        return {}
    data = yaml.safe_load(path.read_text(encoding="utf-8"))
    return data if isinstance(data, dict) else {}


def _read_auth_store(home: Path) -> dict:
    """Raw ``auth.json``. Parsed here first so Hermes' loader never meets a
    corrupt store, which it would copy aside (a write)."""
    path = home / "auth.json"
    if not path.exists():
        return {}
    data = json.loads(path.read_text(encoding="utf-8-sig"))
    if not isinstance(data, dict):
        raise ValueError("auth store root is not an object")
    return data


def _read_dotenv(home: Path) -> dict[str, str | None]:
    """``$HERMES_HOME/.env`` parsed the way Hermes loads it (last wins,
    ``${VAR}`` interpolation over this environment), never sanitized."""
    path = home / ".env"
    if not path.exists():
        return {}
    try:
        return dict(dotenv_values(path, encoding="utf-8-sig"))
    except UnicodeDecodeError:
        raw = path.read_bytes()
        if raw.startswith(codecs.BOM_UTF8):
            raw = raw[len(codecs.BOM_UTF8) :]
        return dict(dotenv_values(stream=io.StringIO(raw.decode("latin-1"))))


def _effective(dotenv: dict[str, str | None], name: str) -> str | None:
    value = dotenv.get(name)
    return value if value is not None else os.environ.get(name)


def _key_present(value: str | None) -> bool:
    return isinstance(value, str) and bool(value.strip()) and not value.strip().startswith("${")


def _fact(default: Any, compute) -> Any:
    try:
        return compute()
    except Exception:
        return default


def _fp_settings() -> tuple[str, str, int | None]:
    context = os.environ.get("FINITE_CONFIG_FP_CONTEXT_LENGTH", "").strip()
    return (
        os.environ.get("FINITE_CONFIG_FP_MODEL", ""),
        os.environ.get("FINITE_CONFIG_FP_BASE_URL", ""),
        int(context) if context.isascii() and context.isdigit() else None,
    )


def _canonical_provider(model_id: str, base_url: str, context_length: int | None) -> dict:
    """The reconciler's `_finite_private_provider`, key for key."""
    capabilities: dict[str, Any] = {}
    if context_length is not None:
        capabilities["context_length"] = context_length
    if model_id == "glm-5-3-flash" and base_url in (
        FINITE_PRIVATE_PRODUCT_BASE_URL,
        HISTORICAL_FINITE_PRIVATE_BASE_URL,
    ):
        capabilities["supports_vision"] = True
    return {
        "name": "Finite Private",
        "base_url": base_url,
        "key_env": FINITE_PRIVATE_KEY_ENV,
        "api_mode": "chat_completions",
        "models": {model_id: capabilities},
        "discover_models": False,
    }


def _canonical_entry(model_id: str, base_url: str) -> dict:
    return {
        "provider": "finite-private",
        "model": model_id,
        "base_url": base_url,
        "key_env": FINITE_PRIVATE_KEY_ENV,
        "api_mode": "chat_completions",
    }


def _fallback_facts(config: dict) -> dict:
    from hermes_cli.fallback_config import get_fallback_chain

    model_id, base_url, _ = _fp_settings()
    canonical = _canonical_entry(model_id, base_url) if model_id and base_url else None
    return {
        "fallback_providers": "present" if "fallback_providers" in config else "absent",
        "fallback_model": "present" if "fallback_model" in config else "absent",
        "effective": [
            {
                "provider": str(entry["provider"])[:128],
                "model": str(entry["model"])[:256],
                "owned_canonical": "yes" if entry == canonical else "no",
            }
            for entry in get_fallback_chain(config)
        ],
    }


def _provider_entry_fact(config: dict) -> str:
    providers = config.get("providers")
    if not isinstance(providers, dict) or "finite-private" not in providers:
        return "absent"
    model_id, base_url, context_length = _fp_settings()
    if (
        model_id
        and base_url
        and providers["finite-private"] == _canonical_provider(model_id, base_url, context_length)
    ):
        return "canonical"
    return "modified"


def _secret_fact(home: Path, dotenv: dict[str, str | None], name: str) -> str | None:
    """The effective value, or None when an external secret source could
    supply it (not evaluated here, so the fact is unknown)."""
    if (home / ".op.env").exists():
        raise LookupError("external secret source")
    return _effective(dotenv, name)


def _openrouter_facts(home: Path) -> dict:
    facts: dict[str, Any] = {
        "hermes_key": "unknown",
        "hermes_key_fingerprint": None,
        "dotenv_key": "unknown",
        "manual_pool_entries": "unknown",
    }
    with contextlib.suppress(Exception):
        dotenv = _read_dotenv(home)
        facts["dotenv_key"] = "present" if "OPENROUTER_API_KEY" in dotenv else "absent"
        value = _secret_fact(home, dotenv, "OPENROUTER_API_KEY")
        if _key_present(value):
            facts["hermes_key"] = "present"
            facts["hermes_key_fingerprint"] = hashlib.sha256(value.encode("utf-8")).hexdigest()
        else:
            facts["hermes_key"] = "absent"
    with contextlib.suppress(Exception):
        pool = _read_auth_store(home).get("credential_pool")
        entries = pool.get("openrouter") if isinstance(pool, dict) else None
        manual = isinstance(entries, list) and any(
            isinstance(entry, dict) and not str(entry.get("source") or "").startswith("env:")
            for entry in entries
        )
        facts["manual_pool_entries"] = "present" if manual else "none"
    return facts


def _codex_facts(home: Path) -> dict:
    """§8.6: Hermes' own read-only predicates, in resolver order."""
    from hermes_cli import auth

    store = _read_auth_store(home)
    providers = store.get("providers")
    pool = store.get("credential_pool")
    stored = bool(isinstance(providers, dict) and providers.get("openai-codex")) or bool(
        isinstance(pool, dict) and pool.get("openai-codex")
    )
    facts: dict[str, Any] = {
        "state": "not_signed_in",
        "quota_reset_at": None,
        "reported_quota_reset_at": None,
    }
    if not stored:
        return facts
    try:
        auth._read_codex_tokens()
    except auth.AuthError:
        pass
    else:
        facts["state"] = "signed_in"
        limit = auth._codex_pool_rate_limit_status()
        if limit:
            facts["reported_quota_reset_at"] = limit.get("reset_at")
        return facts
    if auth._pool_codex_access_token():
        facts["state"] = "signed_in"
        return facts
    limit = auth._codex_pool_rate_limit_status()
    if limit:
        facts["state"] = "quota_limited"
        facts["quota_reset_at"] = limit.get("reset_at")
        return facts
    facts["state"] = "sign_in_required"
    return facts


def _persisted_overrides(home: Path) -> set[str]:
    """Providers named by persisted conversation overrides, read the way
    ``SessionStore`` loads them (routing table first, then legacy
    ``sessions.json``) but read-only: the store's own load may prune and save."""
    from gateway.config import load_gateway_config
    from gateway.session import sanitize_model_override
    from hermes_state import SessionDB

    sessions_dir = Path(load_gateway_config().sessions_dir)
    entries: dict[str, str] = {}
    if (home / "state.db").exists():
        db = SessionDB(read_only=True)
        try:
            entries.update(db.load_gateway_routing_entries(scope=str(sessions_dir.resolve())))
        finally:
            db.close()
    legacy = sessions_dir / "sessions.json"
    if legacy.exists():
        for key, entry in json.loads(legacy.read_text(encoding="utf-8")).items():
            if not key.startswith("_") and key not in entries and isinstance(entry, dict):
                entries[key] = json.dumps(entry)
    providers = set()
    for entry_json in entries.values():
        entry = json.loads(entry_json)
        if isinstance(entry, dict):
            providers.add(_override_provider(sanitize_model_override(entry.get("model_override"))))
    return providers


def inference_facts() -> dict:
    home = _hermes_home()
    try:
        config = _read_config(home)
    except Exception:
        config = None

    def from_config(compute):
        if config is None:
            raise ValueError("config.yaml is unreadable")
        return compute(config)

    def fp_key() -> str:
        value = _secret_fact(home, _read_dotenv(home), FINITE_PRIVATE_KEY_ENV)
        return "present" if _key_present(value) else "absent"

    def session_overrides() -> dict:
        # Hermes' config loaders copy an unparsable config.yaml aside (a
        # write), so the sessions location is read only from a parsable one.
        providers = from_config(lambda _: _persisted_overrides(home))
        return {
            "openrouter": "present" if "openrouter" in providers else "absent",
            "openai_codex": "present" if providers & set(CODEX_PROVIDERS) else "absent",
        }

    fp_base_url = os.environ.get("FINITE_CONFIG_FP_BASE_URL")
    return {
        "v": 1,
        "saved_route": _fact(
            "unknown",
            lambda: from_config(lambda c: classify_saved_route(c.get("model"), fp_base_url)),
        ),
        "fallback": _fact(
            {"fallback_providers": "unknown", "fallback_model": "unknown", "effective": None},
            lambda: from_config(_fallback_facts),
        ),
        "finite_private": {
            "provider_entry": _fact("unknown", lambda: from_config(_provider_entry_fact)),
            "fp_key": _fact("unknown", fp_key),
        },
        "openrouter": _openrouter_facts(home),
        "codex": _fact(
            {"state": "unknown", "quota_reset_at": None, "reported_quota_reset_at": None},
            lambda: _codex_facts(home),
        ),
        "session_overrides": _fact(
            {"openrouter": "unknown", "openai_codex": "unknown"}, session_overrides
        ),
        "alias_present": "yes" if os.environ.get("OPENAI_API_KEY") else "no",
        "codex_home_neutral": "yes" if os.environ.get("CODEX_HOME") == NEUTRAL_CODEX_HOME else "no",
    }


def clear_auth(provider: str) -> dict:
    from hermes_cli.auth import clear_provider_auth

    return {"cleared": "yes" if clear_provider_auth(provider) else "no"}


def clear_session_overrides(providers: list[str]) -> dict:
    """Only with no gateway running: a live store rewrites cleared overrides."""
    from gateway.config import load_gateway_config
    from gateway.session import SessionStore

    names = {provider.strip().lower() for provider in providers}
    config = load_gateway_config()
    store = SessionStore(config.sessions_dir, config)
    cleared = 0
    for entry in store.list_sessions():
        if _override_provider(entry.model_override) in names:
            store.set_model_override(entry.session_key, None)
            cleared += 1
    return {"cleared": cleared}


def _unique_fields(pairs: list[tuple[str, Any]]) -> dict:
    record = dict(pairs)
    if len(record) != len(pairs):
        raise ValueError("duplicate field")
    return record


def _unsigned_literal(literal: str) -> int:
    # Every number in the record is unsigned, and serde reads `-0` as a float.
    if literal.startswith("-"):
        raise ValueError("negative number")
    return int(literal)


def _no_constant(_literal: str) -> None:
    raise ValueError("not a JSON number")


def _unsigned(value: Any, bits: int) -> bool:
    return type(value) is int and 0 <= value < 1 << bits


def _optional_text(value: Any) -> bool:
    if value is None:
        return True
    if type(value) is not str:
        return False
    try:
        value.encode("utf-8")  # rejects a lone surrogate escape, as serde does
    except UnicodeEncodeError:
        return False
    return True


def _is_operation_id(value: Any) -> bool:
    return (
        type(value) is str
        and len(value) == 35
        and value.startswith("op_")
        and all(char in "0123456789abcdef" for char in value[3:])
    )


def _read_intent(path: Path) -> dict | None:
    """The record, only when agentd's reader would accept it. The shared
    fixtures in ``finite-agentd/tests/fixtures/intent`` state the rules. One
    difference, on the safe side: serde also reads a positional array."""
    try:
        record = json.loads(
            path.read_bytes().decode("utf-8"),
            object_pairs_hook=_unique_fields,
            parse_int=_unsigned_literal,
            parse_constant=_no_constant,
        )
    except Exception:
        return None
    if not isinstance(record, dict) or not (
        INTENT_FIELDS - INTENT_OPTIONAL_FIELDS <= record.keys() <= INTENT_FIELDS
    ):
        return None
    kind = record["kind"]
    if not (
        _unsigned(record["v"], 32)
        and record["v"] == 1
        and _is_operation_id(record["id"])
        and type(kind) is str
        and kind in INTENT_KIND_PHASES
        and record["phase"] in INTENT_KIND_PHASES[kind]
        and record["route"] in INTENT_ROUTES
        and record["state"] in INTENT_STATES
        and _optional_text(record.get("model"))
        and _optional_text(record.get("error_code"))
        and _unsigned(record["attempts"], 32)
        and _unsigned(record["created_at_ms"], 64)
        and _unsigned(record["updated_at_ms"], 64)
    ):
        return None
    return record


def apply_pending_disconnect(intent_path: str) -> dict:
    """Launcher step (F1). Clears only for a valid disconnect record in
    cleanup, and only when ``config.yaml`` names a provider for the saved
    default that is not the record's route. Anything else changes nothing
    (R24): a skip is safe, because agentd re-runs the step."""

    def result(applied: str, reason: str) -> dict:
        return {"applied": applied, "reason": reason}

    record = _read_intent(Path(intent_path))
    if record is None:
        return result("skipped", "intent_unparsable")
    if record["kind"] != "disconnect":
        return result("skipped", "not_a_disconnect")
    if record["phase"] not in CLEAR_PHASES:
        return result("skipped", "phase_before_cleanup")
    if record["route"] not in ROUTE_PROVIDERS:
        return result("skipped", "route_not_disconnectable")
    try:
        config = yaml.safe_load((_hermes_home() / "config.yaml").read_text(encoding="utf-8"))
    except FileNotFoundError:
        return result("skipped", "config_missing")
    except Exception:
        return result("skipped", "config_unreadable")
    if not isinstance(config, dict):
        return result("skipped", "config_unreadable")
    # A missing, blank, or non-mapping default is no evidence that the route
    # was switched away. Hermes itself reads a blank provider as none.
    model = config.get("model")
    provider = model.get("provider") if isinstance(model, dict) else None
    if not isinstance(provider, str) or not provider.strip():
        return result("skipped", "model_unclassifiable")
    fp_base_url = os.environ.get("FINITE_CONFIG_FP_BASE_URL")
    if classify_saved_route(model, fp_base_url) == record["route"]:
        return result("skipped", "route_still_saved")
    auth_provider, override_providers = ROUTE_PROVIDERS[record["route"]]
    try:
        clear_auth(auth_provider)
    except Exception:
        return result("error", "clear_auth_failed")
    try:
        clear_session_overrides(list(override_providers))
    except Exception:
        return result("error", "clear_session_overrides_failed")
    return result("yes", "cleared")


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="finite_inference_helper")
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("inference-facts")
    command = commands.add_parser("clear-auth")
    command.add_argument("--provider", required=True, choices=("openrouter", "openai-codex"))
    command = commands.add_parser("clear-session-overrides")
    command.add_argument("--provider", required=True, nargs="+")
    command = commands.add_parser("apply-pending-disconnect")
    command.add_argument("--intent", required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    # Upstream logs and prints diagnostics (paths, parser errors) that Finite
    # cannot vouch for; discard them so only the JSON line leaves the helper.
    logging.disable(logging.CRITICAL)
    exit_code = 0
    with (
        open(os.devnull, "w") as sink,
        contextlib.redirect_stdout(sink),
        contextlib.redirect_stderr(sink),
    ):
        if args.command == "inference-facts":
            output = inference_facts()
        elif args.command == "apply-pending-disconnect":
            try:
                output = apply_pending_disconnect(args.intent)
            except Exception:
                output = {"applied": "error", "reason": "unexpected_failure"}
        else:
            try:
                if args.command == "clear-auth":
                    output = clear_auth(args.provider)
                else:
                    output = clear_session_overrides(args.provider)
            except Exception:
                output, exit_code = None, 2
    if output is not None:
        print(json.dumps(output, separators=(",", ":")))
    return exit_code


if __name__ == "__main__":
    sys.exit(main())

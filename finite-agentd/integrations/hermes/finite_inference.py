"""Finite's named inference routes; secrets remain in the agent's durable home."""

from agent.secret_scope import get_secret
from dotenv import dotenv_values
from hermes_cli.config import get_hermes_home

OPENROUTER_URL = "https://openrouter.ai/api/v1"


def is_finite_runtime():
    return bool(get_secret("FINITE_PRIVATE_API_KEY"))


def openrouter_runtime(model_config):
    model_config = model_config if isinstance(model_config, dict) else {}
    # Read on every resolution: saving/rotating a key must not restart chat or
    # reuse a key cached by an earlier session. A present empty value revokes it.
    values = dotenv_values(get_hermes_home() / ".env", interpolate=False)
    if "OPENROUTER_API_KEY" in values:
        key = values["OPENROUTER_API_KEY"]
    else:
        key = get_secret("OPENROUTER_API_KEY")
        if not key and model_config.get("provider") == "openrouter":
            key = model_config.get("api_key")
    key = str(key or "").strip()
    if not key or "${" in key or key == get_secret("FINITE_PRIVATE_API_KEY"):
        raise ValueError(
            "Save an OpenRouter key in Connections before choosing an OpenRouter model."
        )
    return {
        "provider": "openrouter",
        "requested_provider": "openrouter",
        "api_key": key,
        "base_url": OPENROUTER_URL,
        "api_mode": "chat_completions",
    }


def private_route():
    values = tuple(
        str(get_secret(name) or "").strip()
        for name in ("FINITE_PRIVATE_MODEL", "FINITE_PRIVATE_BASE_URL", "FINITE_PRIVATE_API_KEY")
    )
    if not all(values):
        raise ValueError("Finite Private is not configured on this agent.")
    return values


def session_runtime(override, model_config):
    """Refresh only managed routes, never overlay another route's credential."""
    endpoint = str(override.get("base_url") or "").rstrip("/")
    if override.get("provider") == "openrouter":
        runtime = openrouter_runtime(model_config)
        if endpoint and endpoint != runtime["base_url"]:
            raise ValueError("The saved OpenRouter endpoint changed. Select the model again.")
        return runtime
    private_endpoint = str(get_secret("FINITE_PRIVATE_BASE_URL") or "").rstrip("/")
    if override.get("provider") == "custom" and endpoint and endpoint == private_endpoint:
        _, base_url, api_key = private_route()
        return {
            "provider": "custom",
            "requested_provider": "custom",
            "base_url": base_url,
            "api_key": api_key,
            "api_mode": "chat_completions",
        }
    return None

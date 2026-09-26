#!/usr/bin/env python3
"""Seed Hermes config once, then reconcile only Finite-owned invariants.

Retired AEON vision override: the deleted AEON specialization writer left an
`auxiliary.vision` backend pointing at the torn-down worker, which outranks
native image routing in the pinned Hermes. Startup removes only that exact
block (`_migrate_retired_aeon_vision_override`). Before the first rewrite that
removes it, the replaced `config.yaml` bytes are kept once, mode 0600, beside it
as `config.yaml.pre-aeon-vision-retirement`; a later boot never overwrites that
copy. Rollback: stop the Runtime's writers, select the previous artifact, and
restore that copy over `config.yaml` only if no owner edits since would be
lost. The current image would retire the block again on its next start.
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import stat
import sys
import tempfile
import urllib.parse
from pathlib import Path
from typing import Any


class ConfigError(RuntimeError):
    """The durable Hermes config cannot be safely reconciled."""


LEGACY_PLUGIN_NAMES = ("finite-platform", "finite")
LEGACY_FINITE_PRIVATE_MODELS = ("glm-5-2", "deepseek-v4-flash-0731", "glm-5.3-flash")
FINITE_PRIVATE_PROVIDER = "custom"
FINITE_PRIVATE_BASE_URL = "https://finite-private.finite.containers.tinfoil.dev/v1"
HISTORICAL_FINITE_PRIVATE_BASE_URL = "https://kimi-k2-6.finite.containers.tinfoil.dev/v1"
FINITE_PRIVATE_BASE_URLS = (FINITE_PRIVATE_BASE_URL, HISTORICAL_FINITE_PRIVATE_BASE_URL)
FINITE_PRIVATE_API_MODES = ("chat_completions", None)
FINITE_PRIVATE_KEY_REFERENCES = (
    "${FINITE_PRIVATE_API_KEY}",
    "${FINITECHAT_HERMES_API_KEY}",
)
# The exact auxiliary vision block written by the AEON specialization writer
# that aa5d05ee deleted from finite-agentd. Anything else is user-owned.
RETIRED_AEON_VISION_NETLOC = "specialization.finite.vip"
RETIRED_AEON_VISION_MODELS = (
    "nemotron-3-nano-omni-30b-a3b-reasoning-nvfp4-fast",
    "aeon-gemma-4-12b-k4-nvfp4-unified-fast",
)
RETIRED_AEON_VISION_KEYS = frozenset(
    {
        "provider",
        "base_url",
        "model",
        "api_key",
        "api_mode",
        "timeout",
        "download_timeout",
        "extra_body",
    }
)
RETIRED_AEON_SPECIALIZATION_KEYS = frozenset(
    {"capabilities", "normalization_limits", "prompt_versions"}
)
AEON_VISION_RETIREMENT_BACKUP_SUFFIX = ".pre-aeon-vision-retirement"


def _mapping(parent: dict[str, Any], key: str) -> dict[str, Any]:
    value = parent.get(key)
    if value is None:
        value = {}
        parent[key] = value
    if not isinstance(value, dict):
        raise ConfigError(f"{key} must be an object")
    return value


def _string_list(parent: dict[str, Any], key: str) -> list[str]:
    value = parent.get(key)
    if value is None:
        value = []
        parent[key] = value
    if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
        raise ConfigError(f"{key} must be a list of strings")
    return value


def _integer(settings: dict[str, str], key: str, *, minimum: int = 0) -> int:
    try:
        value = int(settings[key])
    except (KeyError, ValueError) as exc:
        raise ConfigError(f"{key} must be an integer") from exc
    if value < minimum:
        raise ConfigError(f"{key} must be at least {minimum}")
    return value


def _is_image_owned_finite_private_shape(model: dict[str, Any]) -> bool:
    return (
        model.get("provider") == FINITE_PRIVATE_PROVIDER
        and model.get("base_url") in FINITE_PRIVATE_BASE_URLS
        and model.get("api_mode") in FINITE_PRIVATE_API_MODES
        and model.get("api_key") in FINITE_PRIVATE_KEY_REFERENCES
    )


def _migrate_historical_finite_private_route(config: dict[str, Any]) -> None:
    """Rewrite only the retired kimi-k2-6 Finite Private URL.

    Matching the complete Finite Private provider/key shape prevents an image
    restart from moving a deliberate custom-provider selection.
    """

    model = config.get("model")
    if not isinstance(model, dict) or not _is_image_owned_finite_private_shape(model):
        return
    if model.get("base_url") == HISTORICAL_FINITE_PRIVATE_BASE_URL:
        model["base_url"] = FINITE_PRIVATE_BASE_URL


def _migrate_legacy_finite_private_model(config: dict[str, Any], settings: dict[str, str]) -> None:
    """Move only the exact image-owned Finite Private default to the current model.

    Model configuration is otherwise durable and user-owned. Matching the
    complete Finite Private route/provider/key shape prevents an image restart
    from overwriting a deliberate custom-provider or model selection.
    """

    if settings.get("FINITE_CONFIG_PROVIDER") != FINITE_PRIVATE_PROVIDER:
        return
    model = config.get("model")
    if not isinstance(model, dict) or not _is_image_owned_finite_private_shape(model):
        return
    if model.get("default") not in LEGACY_FINITE_PRIVATE_MODELS:
        return
    model["default"] = settings["FINITE_CONFIG_MODEL"]
    model["context_length"] = _integer(
        settings,
        "FINITE_CONFIG_CONTEXT_LENGTH",
        minimum=1,
    )


def _is_retired_aeon_vision_override(config: dict[str, Any]) -> bool:
    """Match only the backend the deleted AEON writer installed; fail closed otherwise."""
    auxiliary = config.get("auxiliary")
    vision = auxiliary.get("vision") if isinstance(auxiliary, dict) else None
    if not isinstance(vision, dict) or not RETIRED_AEON_VISION_KEYS.issuperset(vision):
        return False
    base_url = vision.get("base_url")
    if not isinstance(base_url, str):
        return False
    try:
        url = urllib.parse.urlsplit(base_url)
    except ValueError:
        return False
    if (
        vision.get("provider") != "custom"
        or url.scheme != "https"
        or url.netloc != RETIRED_AEON_VISION_NETLOC
        or vision.get("model") not in RETIRED_AEON_VISION_MODELS
    ):
        return False
    if "extra_body" not in vision:
        return True
    extra_body = vision["extra_body"]
    if not isinstance(extra_body, dict) or set(extra_body) != {"finite_specialization"}:
        return False
    specialization = extra_body["finite_specialization"]
    return isinstance(specialization, dict) and RETIRED_AEON_SPECIALIZATION_KEYS.issuperset(
        specialization
    )


def _migrate_retired_aeon_vision_override(config: dict[str, Any]) -> None:
    """Remove only the auxiliary vision backend of the deleted AEON worker.

    That explicit backend outranks native image routing in the pinned Hermes,
    and its worker no longer exists. Removing the block also removes the stored
    worker key. Any unexpected key or value leaves the block user-owned.
    """

    if not _is_retired_aeon_vision_override(config):
        return
    auxiliary = config["auxiliary"]
    del auxiliary["vision"]
    if not auxiliary:
        del config["auxiliary"]


def _has_provider_vision_override(config: dict[str, Any], model: dict[str, Any]) -> bool:
    """Do not shadow Hermes's supported per-provider capability declarations."""
    providers = config.get("providers")
    entries = [providers.get("custom")] if isinstance(providers, dict) else []
    custom_providers = config.get("custom_providers")
    if isinstance(custom_providers, list):
        entries.extend(
            entry
            for entry in custom_providers
            if isinstance(entry, dict) and str(entry.get("name", "")).strip().lower() == "custom"
        )
    for entry in entries:
        models = entry.get("models") if isinstance(entry, dict) else None
        capability = models.get(model["default"]) if isinstance(models, dict) else None
        if isinstance(capability, dict) and {"supports_vision", "vision"}.intersection(capability):
            return True
    return False


def reconcile_config(
    existing: dict[str, Any] | None,
    settings: dict[str, str],
    *,
    recover_known_good: bool = False,
) -> dict[str, Any]:
    """Return first-boot defaults plus the narrow Finite-owned config merge.

    Model/provider configuration and non-Finite platforms are seeded only when
    no config exists. Once Hermes owns the file, this function deliberately
    leaves those sections semantically unchanged except for narrowly matched
    migrations (including removal of the deleted AEON auxiliary vision backend)
    and the missing capability default of the known Finite Private profile.
    Deleting that declaration restores the product default on boot; an
    explicit capability or routing override remains user-owned.
    """

    first_seed = existing is None
    config: dict[str, Any] = {} if first_seed else copy.deepcopy(existing)

    if first_seed:
        model: dict[str, Any] = {
            "default": settings["FINITE_CONFIG_MODEL"],
            "provider": settings["FINITE_CONFIG_PROVIDER"],
            "base_url": settings["FINITE_CONFIG_BASE_URL"],
            "api_mode": settings["FINITE_CONFIG_API_MODE"],
        }
        context_length = settings.get("FINITE_CONFIG_CONTEXT_LENGTH", "")
        if context_length:
            model["context_length"] = _integer(
                settings,
                "FINITE_CONFIG_CONTEXT_LENGTH",
                minimum=1,
            )
        api_key_reference = settings.get("FINITE_CONFIG_API_KEY_REFERENCE", "")
        if api_key_reference:
            model["api_key"] = api_key_reference
        config.update(
            {
                "model": model,
                "auxiliary": {
                    "title_generation": {
                        "timeout": _integer(
                            settings,
                            "FINITE_CONFIG_TITLE_TIMEOUT_SECS",
                        )
                    }
                },
                "terminal": {
                    "backend": "local",
                    "cwd": settings["FINITE_CONFIG_WORKSPACE"],
                    "persistent_shell": True,
                },
                "approvals": {"mode": "off"},
                "display": {"streaming": False},
                "security": {"redact_secrets": True},
                "_config_version": 10,
            }
        )

    if not first_seed:
        _migrate_historical_finite_private_route(config)
        _migrate_legacy_finite_private_model(config, settings)
        _migrate_retired_aeon_vision_override(config)

    # Hermes cannot discover capabilities for our generic custom provider.
    # Declare only the known Finite Private GLM model, including existing
    # configs after the migrations above. Explicit user overrides still win.
    current_model = config.get("model")
    if (
        isinstance(current_model, dict)
        and _is_image_owned_finite_private_shape(current_model)
        and current_model.get("default") == "glm-5-3-flash"
        and not _has_provider_vision_override(config, current_model)
    ):
        current_model.setdefault("supports_vision", True)

    # Outside the migrations and capability default above, these are the only settings Finite
    # repairs after first boot. They keep the encrypted transport and managed
    # skill catalog reachable without turning the runtime launcher into a
    # second Hermes configuration store.
    plugins = _mapping(config, "plugins")
    enabled_plugins = _string_list(plugins, "enabled")
    plugin_name = settings["FINITE_CONFIG_PLUGIN_NAME"]
    if plugin_name not in enabled_plugins:
        enabled_plugins.append(plugin_name)
    if recover_known_good:
        enabled_plugins[:] = [name for name in enabled_plugins if name not in LEGACY_PLUGIN_NAMES]

    gateway = _mapping(config, "gateway")
    platforms = _mapping(gateway, "platforms")
    if recover_known_good:
        for legacy_name in LEGACY_PLUGIN_NAMES:
            legacy = platforms.get(legacy_name)
            if legacy is not None and not isinstance(legacy, dict):
                platforms[legacy_name] = {"enabled": False}
            elif isinstance(legacy, dict) and legacy.get("enabled") is not False:
                legacy["enabled"] = False
    finitechat = _mapping(platforms, "finitechat")
    finitechat["enabled"] = True
    extra = _mapping(finitechat, "extra")
    extra.update(
        {
            "home": settings["FINITE_CONFIG_AGENT_HOME"],
            "finitechat_bin": settings["FINITE_CONFIG_FINITECHAT_BIN"],
            "inbound_stream": True,
            "service_addr": settings["FINITE_CONFIG_SERVICE_ADDR"],
            "poll_timeout_secs": _integer(
                settings,
                "FINITE_CONFIG_POLL_TIMEOUT_SECS",
            ),
            "poll_limit": _integer(settings, "FINITE_CONFIG_POLL_LIMIT", minimum=1),
        }
    )

    display = _mapping(config, "display")
    display_platforms = _mapping(display, "platforms")
    finitechat_display = _mapping(display_platforms, "finitechat")
    # Finite Chat is append-only. Hermes' edit-based token streaming and
    # accumulated progress bubbles cannot be represented faithfully, so keep
    # these adapter capabilities authoritative even when an older resident
    # config contains incompatible values. Interim assistant commentary stays
    # enabled by Hermes' normal default and is delivered as separate messages.
    finitechat_display["streaming"] = False
    finitechat_display["tool_progress_grouping"] = "separate"

    home_channel = settings.get("FINITE_CONFIG_HOME_CHANNEL", "")
    if home_channel:
        finitechat["home_channel"] = {
            "platform": "finitechat",
            "chat_id": home_channel,
            "name": "Finite Chat Home",
        }

    managed_skills_dir = settings.get("FINITE_CONFIG_MANAGED_SKILLS_DIR", "")
    if managed_skills_dir:
        skills = _mapping(config, "skills")
        external_dirs = _string_list(skills, "external_dirs")
        if managed_skills_dir not in external_dirs:
            external_dirs.append(managed_skills_dir)

    return config


def _load(path: Path) -> dict[str, Any]:
    text = path.read_text(encoding="utf-8")
    try:
        try:
            import yaml
        except ImportError:
            # JSON is valid YAML and keeps focused launcher tests independent
            # of the Hermes virtualenv. The canonical image healthcheck
            # requires PyYAML, so production preserves ordinary YAML configs.
            value = json.loads(text)
        else:
            value = yaml.safe_load(text)
    except Exception as exc:
        raise ConfigError("config could not be parsed") from exc
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise ConfigError("config root must be an object")
    if not all(isinstance(key, str) for key in value):
        raise ConfigError("config keys must be strings")
    return value


def _dump(value: dict[str, Any]) -> str:
    try:
        import yaml
    except ImportError:
        return json.dumps(value, indent=2, ensure_ascii=False) + "\n"
    return yaml.safe_dump(value, sort_keys=False, allow_unicode=True)


def _atomic_write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    previous_mode = stat.S_IMODE(path.stat().st_mode) if path.exists() else 0o600
    fd, raw_temp = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temp = Path(raw_temp)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            os.fchmod(handle.fileno(), previous_mode)
            handle.write(text)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temp, path)
        directory_fd = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    finally:
        temp.unlink(missing_ok=True)


def _preserve_pre_aeon_retirement_config(path: Path, existing: dict[str, Any] | None) -> None:
    """Keep the bytes a retiring write replaces, once, as the rollback copy.

    Call immediately before the reconciled write. An existing copy is never
    overwritten, so it always holds the first pre-retirement config. The name
    stays outside recovery boot's `.config.yaml.*` transient-file cleanup.
    """

    if existing is None or not _is_retired_aeon_vision_override(existing):
        return
    original = path.read_bytes()
    backup = path.with_name(path.name + AEON_VISION_RETIREMENT_BACKUP_SUFFIX)
    try:
        fd = os.open(backup, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        return
    try:
        with os.fdopen(fd, "wb") as handle:
            os.fchmod(handle.fileno(), 0o600)
            handle.write(original)
            handle.flush()
            os.fsync(handle.fileno())
    except BaseException:
        backup.unlink(missing_ok=True)
        raise
    directory_fd = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(directory_fd)
    finally:
        os.close(directory_fd)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", type=Path, required=True)
    args = parser.parse_args()
    path: Path = args.config

    try:
        existing = _load(path) if path.exists() else None
        reconciled = reconcile_config(existing, dict(os.environ))
        if existing is None or reconciled != existing:
            _preserve_pre_aeon_retirement_config(path, existing)
            _atomic_write(path, _dump(reconciled))
    except (ConfigError, KeyError, OSError, ValueError) as exc:
        print(f"FINITE_AGENT_START_ERROR unsafe Hermes config: {exc}", file=sys.stderr)
        raise SystemExit(64) from exc


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Seed Hermes config once, then reconcile only Finite-owned invariants."""

from __future__ import annotations

import argparse
import copy
import json
import os
import stat
import sys
import tempfile
from pathlib import Path
from typing import Any


class ConfigError(RuntimeError):
    """The durable Hermes config cannot be safely reconciled."""


LEGACY_PLUGIN_NAMES = ("finite-platform", "finite")


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


def reconcile_config(
    existing: dict[str, Any] | None,
    settings: dict[str, str],
    *,
    recover_known_good: bool = False,
) -> dict[str, Any]:
    """Return first-boot defaults plus the narrow Finite-owned config merge.

    Model/provider configuration and non-Finite platforms are seeded only when
    no config exists. Once Hermes owns the file, this function deliberately
    leaves those sections semantically unchanged.
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

    # These are the only settings Finite repairs after first boot. They keep
    # the encrypted transport and managed skill catalog reachable without
    # turning the runtime launcher into a second Hermes configuration store.
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


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", type=Path, required=True)
    args = parser.parse_args()
    path: Path = args.config

    try:
        existing = _load(path) if path.exists() else None
        reconciled = reconcile_config(existing, dict(os.environ))
        if existing is None or reconciled != existing:
            _atomic_write(path, _dump(reconciled))
    except (ConfigError, KeyError, OSError, ValueError) as exc:
        print(f"FINITE_AGENT_START_ERROR unsafe Hermes config: {exc}", file=sys.stderr)
        raise SystemExit(64) from exc


if __name__ == "__main__":
    main()

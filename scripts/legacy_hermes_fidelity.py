#!/usr/bin/env python3
"""Opt-in full-fidelity activation for legacy Hermes migrations.

The default mode activates only memories and three workspace roots. This mode
also activates the bot's persona, user-facing settings, Telegram connection and
pairing list, user-created skills, helper scripts, scheduled jobs and the loose
files below `/home/node`, so the person keeps talking to the same bot in the
same chat. The source plan is derived from the sealed bundle at manifest time
and bound into the approved manifest; the installer re-derives it, checks it
against the stopped target and records every activation and every omission in
the receipt. No secret value enters the manifest, the receipt or an error.
"""

from __future__ import annotations

import copy
import hashlib
import json
import os
import re
import shutil
import stat
import tarfile
from collections.abc import Callable, Iterable
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from pathlib import Path, PurePosixPath
from typing import Any

from legacy_hermes_contract import (
    SOURCE_SNAPSHOT_RELATIVE_PATH,
    MigrationError,
    _fsync_directory,
    _normalized_tar_member_path,
)

DEFAULT_MODE = "default"
FIDELITY_MODE = "full-fidelity"
MODES = (DEFAULT_MODE, FIDELITY_MODE)
FIDELITY_SCHEMA = "finite.legacy-hermes-full-fidelity.v1"

LEGACY_HOME = "/home/node"
TARGET_LEGACY_ROOT = "/data/workspace/legacy-box1"
TARGET_HERMES_HOME = "/data/agent/hermes-home"
TARGET_HERMES_RELATIVE = PurePosixPath("agent/hermes-home")
TARGET_LEGACY_RELATIVE = PurePosixPath("workspace/legacy-box1")
REVIEW_ONLY_RELATIVE = PurePosixPath("migration/legacy-hermes-v2/review-only")

# Home roots the default mode already copies from the payload.
PAYLOAD_HOME_ROOTS = ("workspace", "dev", "uploads")
# Top-level home entries that are never placed in active target paths.
WITHHELD_HOME_ENTRIES = {
    # box1 rewrites both AGENTS.md copies from its cluster render on every
    # boot (finitecomputer nix/agent-runtime/entrypoint.nix install_file), so
    # they hold box1 platform instructions (finitec, Traefik), not the bot's.
    "AGENTS.md": "box1-platform-seeded-review-only",
    "finitebrain": "brain-working-tree-reauthorize-and-sync",
    "node_modules": "generated-dependency-tree",
    "__pycache__": "generated-bytecode",
}
# Hermes-home names whose legacy paths map into the active target Hermes home.
HERMES_ACTIVE_NAMES = frozenset({"SOUL.md", "cron", "memories", "scripts", "skills"})
TEXT_REWRITE_LIMIT_BYTES = 4 * 1024 * 1024

# User-facing settings the target adopts from the source config.yaml. The
# target keeps model credentials and routing, managed skill sources, terminal,
# gateway, plugins, security, approvals and delegation.
CONFIG_REPLACE_KEYS = (
    "human_delay",
    "onboarding",
    "personalities",
    "personality",
    "session_reset",
    "stt",
    "timezone",
    "tts",
    "voice",
)
CONFIG_MERGE_PATHS = (
    ("agent", "personality"),
    ("cron", "wrap_response"),
    ("memory", "memory_enabled"),
    ("memory", "user_profile_enabled"),
    ("memory", "memory_char_limit"),
    ("memory", "user_char_limit"),
)
CONFIG_DISPLAY_TARGET_OWNED = frozenset({"platforms"})
_SECRET_KEY = re.compile(
    r"(api_?key|token|secret|password|passwd|credential|auth)", re.I
)

# The v2 target's Telegram block, exactly as finite-agentd's
# telegram_connect_offer and telegram_home_offer write it.
AGENTD_TELEGRAM_DEFAULTS = {
    "reply_to_mode": "first",
    "gateway_restart_notification": False,
    "typing_indicator": True,
}
TELEGRAM_PRESERVE_PENDING_KEY = "preserve_pending_updates"
_TELEGRAM_TOKEN = re.compile(r"^[0-9]+:[A-Za-z0-9_-]{30,}$")

# Tool credentials Core supplies to every new Runtime by reference
# (infra/nixos/modules/finite-saas-core.nix FC_CORE_RUNTIME_SECRET_REFERENCES_JSON).
TARGET_RUNTIME_SECRET_REFERENCES = frozenset(
    {
        "ELEVENLABS_API_KEY",
        "FAL_KEY",
        "FIRECRAWL_API_KEY",
        "FRED_API_KEY",
        "GOOGLE_PLACES_API_KEY",
        "PERPLEXITY_API_KEY",
        "XAI_API_KEY",
        "X_API_BEARER_TOKEN",
    }
)
TARGET_MANAGED_ENV = frozenset({"FINITE_PRIVATE_API_KEY"})
CHANNEL_ENV_PREFIXES = {
    "MATRIX_": "matrix",
    "SIGNAL_": "signal",
    "DISCORD_": "discord",
    "SLACK_": "slack",
    "WHATSAPP_": "whatsapp",
    "TELEGRAM_": "telegram",
}
CHANNEL_CONFIG_KEYS = ("discord", "whatsapp", "slack", "signal", "matrix", "telegram")
SUPPORTED_TARGET_CHANNELS = frozenset({"telegram"})
ONESHOT_MISSED_REASON = (
    "legacy-migration: one-shot time passed before cutover; not fired"
)
_ENV_NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
_PATH_TOKEN = re.compile(
    r"(?<![\w.~/-])(?:/home/node(?![\w.-])|~/)[^\s\"'`<>()\[\]{}|,;*?]*"
)


# --------------------------------------------------------------------------
# Path mapping


class PathMap:
    """Map legacy `/home/node` and `~` paths onto active target locations."""

    def __init__(self, placed_home_names: Iterable[str]):
        self.placed = frozenset(placed_home_names)
        self.rewritten = 0
        self.unmapped = 0

    def map_path(self, value: str) -> str | None:
        if value.startswith("~/"):
            rest = value[1:]
        elif value == LEGACY_HOME or value.startswith(LEGACY_HOME + "/"):
            rest = value[len(LEGACY_HOME) :]
        else:
            return None
        stripped = rest.lstrip("/")
        if not stripped:
            return TARGET_LEGACY_ROOT + rest
        head, _, tail = stripped.partition("/")
        tail = "/" + tail if _ else ""
        if head == ".hermes":
            if not tail.strip("/"):
                return TARGET_HERMES_HOME + tail
            hermes_head = tail.lstrip("/").split("/", 1)[0]
            if _strip_punctuation(hermes_head) in HERMES_ACTIVE_NAMES:
                return TARGET_HERMES_HOME + tail
            return None
        if _strip_punctuation(head) in self.placed:
            return f"{TARGET_LEGACY_ROOT}/{head}{tail}"
        return None

    def rewrite_text(self, text: str) -> str:
        def replace(match: re.Match[str]) -> str:
            token = match.group(0)
            mapped = self.map_path(token)
            if mapped is None:
                self.unmapped += 1
                return token
            self.rewritten += 1
            return mapped

        return _PATH_TOKEN.sub(replace, text)

    def rewrite_value(self, value: Any) -> Any:
        if isinstance(value, str):
            return self.rewrite_text(value)
        if isinstance(value, list):
            return [self.rewrite_value(item) for item in value]
        if isinstance(value, dict):
            return {key: self.rewrite_value(item) for key, item in value.items()}
        return value

    def counts(self) -> dict[str, int]:
        return {"rewritten": self.rewritten, "unmapped_left": self.unmapped}


def _strip_punctuation(value: str) -> str:
    return value.rstrip(".:!?")


# --------------------------------------------------------------------------
# Sealed source snapshot reader


class SourceSnapshot:
    """Read members of the verified `source-home.tar` without extracting it."""

    def __init__(self, archive_path: Path, inventory: dict[str, Any]):
        self.archive_path = archive_path
        self.archive = tarfile.open(archive_path, mode="r:")
        self.members: dict[str, tarfile.TarInfo] = {}
        for member in self.archive:
            path = _normalized_tar_member_path(member.name)
            if path is not None:
                self.members[path] = member
        self.dispositions = {
            entry["path"]: entry["disposition"] for entry in inventory["entries"]
        }

    def close(self) -> None:
        self.archive.close()

    def is_file(self, path: str) -> bool:
        member = self.members.get(path)
        return member is not None and member.isfile()

    def read_bytes(self, path: str) -> bytes:
        member = self.members.get(path)
        if member is None or not member.isfile():
            raise MigrationError(f"source snapshot has no regular file: {path}")
        handle = self.archive.extractfile(member)
        if handle is None:
            raise MigrationError(f"source snapshot file cannot be read: {path}")
        return handle.read()

    def read_text(self, path: str, label: str) -> str:
        try:
            return self.read_bytes(path).decode("utf-8")
        except UnicodeDecodeError as exc:
            raise MigrationError(f"{label} is not UTF-8") from exc

    def under(self, prefix: str) -> list[str]:
        return sorted(path for path in self.members if path.startswith(prefix + "/"))

    def top_level(self) -> list[str]:
        return sorted({path.split("/", 1)[0] for path in self.members})


# --------------------------------------------------------------------------
# Small parsers


def _yaml():
    try:
        import yaml
    except ImportError as exc:  # pragma: no cover - the Hermes env ships PyYAML
        raise MigrationError("full-fidelity mode requires PyYAML") from exc
    return yaml


def _load_yaml_mapping(text: str, label: str) -> dict[str, Any]:
    try:
        value = _yaml().safe_load(text)
    except Exception as exc:
        raise MigrationError(f"{label} could not be parsed") from exc
    if value is None:
        return {}
    if not isinstance(value, dict) or not all(isinstance(key, str) for key in value):
        raise MigrationError(f"{label} root must be a mapping with string keys")
    return value


def parse_env(text: str) -> dict[str, str]:
    """Parse a Hermes `.env` the way the integrations inventory reads names."""
    values: dict[str, str] = {}
    for line_number, line in enumerate(text.splitlines(), start=1):
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if stripped.startswith("export "):
            stripped = stripped.removeprefix("export ").lstrip()
        if "=" not in stripped:
            raise MigrationError(
                f"legacy Hermes .env line {line_number} is not an assignment"
            )
        name, value = stripped.split("=", 1)
        name = name.strip()
        if not _ENV_NAME.fullmatch(name):
            raise MigrationError(
                f"legacy Hermes .env line {line_number} has an invalid name"
            )
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
            value = value[1:-1]
        if value:
            values[name] = value
    return values


def _get(mapping: Any, *path: str) -> Any:
    for part in path:
        if not isinstance(mapping, dict):
            return None
        mapping = mapping.get(part)
    return mapping


def _resolve_env_reference(value: Any, env: dict[str, str]) -> str | None:
    if not isinstance(value, str) or not value.strip():
        return None
    match = re.fullmatch(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}", value.strip())
    if match:
        return env.get(match.group(1))
    return value.strip()


def _skill_name(skill_md: bytes, fallback: str) -> str:
    """Mirror Hermes tools.skills_sync._read_skill_name."""
    content = skill_md[:4000].decode("utf-8", errors="replace")
    in_frontmatter = False
    for line in content.split("\n"):
        stripped = line.strip()
        if stripped == "---":
            if in_frontmatter:
                break
            in_frontmatter = True
            continue
        if in_frontmatter and stripped.startswith("name:"):
            value = stripped.split(":", 1)[1].strip().strip("\"'")
            if value:
                return value
    return fallback


def _dir_md5(directory: Path) -> str:
    """Mirror Hermes tools.skills_sync._dir_hash (the bundled origin hash)."""
    hasher = hashlib.md5()
    for path in sorted(directory.rglob("*")):
        if path.is_file():
            hasher.update(str(path.relative_to(directory)).encode("utf-8"))
            hasher.update(path.read_bytes())
    return hasher.hexdigest()


def _scan_skill_dirs(root: Path) -> list[PurePosixPath]:
    """Top-most directories holding a SKILL.md, skipping hidden/support paths."""
    if not root.is_dir():
        return []
    found: list[PurePosixPath] = []
    for skill_md in sorted(root.rglob("SKILL.md")):
        relative = PurePosixPath(skill_md.parent.relative_to(root).as_posix())
        if relative == PurePosixPath("."):
            continue
        if any(
            part.startswith(".") or part in WITHHELD_SKILL_PARTS
            for part in relative.parts
        ):
            continue
        if any(relative.is_relative_to(parent) for parent in found):
            continue
        found.append(relative)
    return sorted(found)


WITHHELD_SKILL_PARTS = frozenset(
    {"node_modules", "venv", "site-packages", "__pycache__"}
)


def skill_names_in(root: Path) -> dict[str, PurePosixPath]:
    names: dict[str, PurePosixPath] = {}
    for relative in _scan_skill_dirs(root):
        skill_md = root / Path(relative.as_posix()) / "SKILL.md"
        try:
            names.setdefault(
                _skill_name(skill_md.read_bytes(), relative.name), relative
            )
        except OSError:
            continue
    return names


# --------------------------------------------------------------------------
# Manifest-time plan


def build_plan(
    bundle: Path, inventory: dict[str, Any], sites: dict[str, Any]
) -> dict[str, Any]:
    """Derive the deterministic, secret-free full-fidelity plan from a bundle."""
    payload = bundle / "payload"
    snapshot = SourceSnapshot(
        payload / Path(SOURCE_SNAPSHOT_RELATIVE_PATH.as_posix()), inventory
    )
    try:
        home = _plan_home(snapshot)
        env = _source_env(snapshot)
        config = _source_config(snapshot)
        plan = {
            "schema": FIDELITY_SCHEMA,
            "path_map": {
                "legacy_home": LEGACY_HOME,
                "home_target": TARGET_LEGACY_ROOT,
                "hermes_target": TARGET_HERMES_HOME,
                "placed_home_entries": sorted(
                    PAYLOAD_HOME_ROOTS + tuple(home["placed"])
                ),
                "active_hermes_names": sorted(HERMES_ACTIVE_NAMES),
            },
            "home": home,
            "persona": _plan_persona(snapshot),
            "config": _plan_config(config),
            "telegram": _plan_telegram(snapshot, config, env),
            "skills": _plan_skills(payload, snapshot),
            "scripts": _plan_scripts(payload),
            "cron": _plan_cron(payload),
            "omitted": _plan_omissions(snapshot, config, env, sites),
        }
        if plan["telegram"]["activate"]:
            plan["omitted"]["channels"] = [
                channel
                for channel in plan["omitted"]["channels"]
                if channel["name"] != "telegram"
            ]
    finally:
        snapshot.close()
    return plan


def _plan_home(snapshot: SourceSnapshot) -> dict[str, Any]:
    placed: list[str] = []
    withheld: list[dict[str, str]] = []
    for name in snapshot.top_level():
        if name.startswith(".") or name in PAYLOAD_HOME_ROOTS:
            continue
        reason = WITHHELD_HOME_ENTRIES.get(name)
        disposition = snapshot.dispositions.get(name)
        if reason is None and disposition not in ("preserve", "activate"):
            reason = f"source-disposition-{disposition}"
        member = snapshot.members.get(name)
        if reason is None and (
            member is None or not (member.isfile() or member.isdir() or member.issym())
        ):
            reason = "unsupported-entry"
        if reason is not None:
            withheld.append({"name": name, "reason": reason})
            continue
        placed.append(name)
    entries = 0
    skipped = 0
    for name in placed:
        if snapshot.members[name].isdir():
            for path in snapshot.under(name):
                if snapshot.dispositions.get(path) in ("preserve", "activate"):
                    entries += 1
                else:
                    skipped += 1
        else:
            entries += 1
    return {
        "placed": placed,
        "withheld": withheld,
        "placed_entry_count": entries,
        "skipped_generated_entry_count": skipped,
        "target_root": TARGET_LEGACY_ROOT,
        "collision_policy": "fail-closed",
    }


def _plan_persona(snapshot: SourceSnapshot) -> dict[str, Any]:
    soul = ".hermes/SOUL.md"
    agents = [
        path for path in ("AGENTS.md", ".hermes/AGENTS.md") if snapshot.is_file(path)
    ]
    return {
        "soul": {
            "present": snapshot.is_file(soul),
            "sha256": hashlib.sha256(snapshot.read_bytes(soul)).hexdigest()
            if snapshot.is_file(soul)
            else None,
            "target": f"{TARGET_HERMES_HOME}/SOUL.md",
        },
        "agents_md": {
            "paths": agents,
            "policy": WITHHELD_HOME_ENTRIES["AGENTS.md"],
            "target": (REVIEW_ONLY_RELATIVE / "agents").as_posix() if agents else None,
        },
    }


def _source_env(snapshot: SourceSnapshot) -> dict[str, str]:
    path = ".hermes/.env"
    if not snapshot.is_file(path):
        return {}
    return parse_env(snapshot.read_text(path, "legacy Hermes .env"))


def _source_config(snapshot: SourceSnapshot) -> dict[str, Any]:
    path = ".hermes/config.yaml"
    if not snapshot.is_file(path):
        return {}
    return _load_yaml_mapping(
        snapshot.read_text(path, "legacy Hermes config.yaml"),
        "legacy Hermes config.yaml",
    )


def _without_secret_keys(value: Any, prefix: str, dropped: list[str]) -> Any:
    if isinstance(value, dict):
        kept = {}
        for key, item in value.items():
            path = f"{prefix}.{key}"
            if isinstance(key, str) and _SECRET_KEY.search(key):
                dropped.append(path)
                continue
            kept[key] = _without_secret_keys(item, path, dropped)
        return kept
    if isinstance(value, list):
        return [_without_secret_keys(item, prefix, dropped) for item in value]
    return value


def adopted_settings(config: dict[str, Any]) -> tuple[dict[str, Any], list[str]]:
    """Return the allowlisted user-facing settings and the secret-shaped keys dropped."""
    dropped: list[str] = []
    adopted: dict[str, Any] = {}
    for key in CONFIG_REPLACE_KEYS:
        if key in config:
            adopted[key] = _without_secret_keys(config[key], key, dropped)
    display = config.get("display")
    if isinstance(display, dict):
        kept = {
            key: value
            for key, value in display.items()
            if key not in CONFIG_DISPLAY_TARGET_OWNED
        }
        if kept:
            adopted["display"] = _without_secret_keys(kept, "display", dropped)
    for path in CONFIG_MERGE_PATHS:
        value = _get(config, *path)
        if value is not None:
            adopted.setdefault(path[0], {})[path[1]] = _without_secret_keys(
                value, ".".join(path), dropped
            )
    return adopted, sorted(dropped)


def _flatten_keys(value: dict[str, Any], prefix: str = "") -> list[str]:
    keys = []
    for key, item in value.items():
        path = f"{prefix}.{key}" if prefix else key
        if isinstance(item, dict) and path.split(".")[0] in {
            "display",
            "agent",
            "cron",
            "memory",
        }:
            keys.extend(_flatten_keys(item, path))
        else:
            keys.append(path)
    return keys


def _plan_config(config: dict[str, Any]) -> dict[str, Any]:
    adopted, dropped = adopted_settings(config)
    adopted_keys = set(_flatten_keys(adopted))
    withheld = []
    for key, value in config.items():
        if key in CONFIG_REPLACE_KEYS or key in ("platforms", "telegram"):
            continue
        if key in ("display", "agent", "cron", "memory") and isinstance(value, dict):
            withheld.extend(
                f"{key}.{sub}" for sub in value if f"{key}.{sub}" not in adopted_keys
            )
        else:
            withheld.append(key)
    withheld.sort()
    model = config.get("model") if isinstance(config.get("model"), dict) else {}
    return {
        "adopted_keys": sorted(adopted_keys),
        "dropped_secret_shaped_keys": dropped,
        "withheld_top_level_keys": withheld,
        "source_model": model.get("default")
        if isinstance(model.get("default"), str)
        else None,
        "target_owned": [
            "model credentials and base_url",
            "managed skill sources",
            "terminal",
            "gateway",
            "plugins",
            "security and approvals",
            "delegation credentials",
        ],
    }


def telegram_source(
    config: dict[str, Any], env: dict[str, str]
) -> tuple[str | None, str | None, bool, dict[str, Any]]:
    """Return (token, home_chat_id, enabled, evidence-without-values)."""
    candidates = [
        ("env:TELEGRAM_BOT_TOKEN", env.get("TELEGRAM_BOT_TOKEN")),
        (
            "config:gateway.platforms.telegram.token",
            _get(config, "gateway", "platforms", "telegram", "token"),
        ),
        (
            "config:platforms.telegram.token",
            _get(config, "platforms", "telegram", "token"),
        ),
        ("config:telegram.token", _get(config, "telegram", "token")),
    ]
    tokens: dict[str, str] = {}
    for label, raw in candidates:
        resolved = _resolve_env_reference(raw, env)
        if resolved:
            tokens[label] = resolved
    if len(set(tokens.values())) > 1:
        raise MigrationError(
            "legacy Telegram token sources disagree: " + ", ".join(sorted(tokens))
        )
    token = next(iter(tokens.values()), None)
    if token is not None and (len(token) > 256 or not _TELEGRAM_TOKEN.fullmatch(token)):
        # Same shape finite-agentd's validate_telegram_token accepts.
        raise MigrationError(
            "legacy Telegram bot token does not have the Bot API shape"
        )
    homes = [
        ("env:TELEGRAM_HOME_CHANNEL", env.get("TELEGRAM_HOME_CHANNEL")),
        (
            "config:gateway.platforms.telegram.home_channel",
            _get(config, "gateway", "platforms", "telegram", "home_channel", "chat_id"),
        ),
        (
            "config:platforms.telegram.home_channel",
            _get(config, "platforms", "telegram", "home_channel", "chat_id"),
        ),
    ]
    home_values = {
        label: str(value).strip() for label, value in homes if value not in (None, "")
    }
    if len(set(home_values.values())) > 1:
        raise MigrationError(
            "legacy Telegram home channel sources disagree: "
            + ", ".join(sorted(home_values))
        )
    home = next(iter(home_values.values()), None)
    if home is not None and (not home.isdigit() or len(home) > 64):
        # finite-agentd's telegram_home_offer accepts only a numeric user chat.
        raise MigrationError(
            "legacy Telegram home channel is not a numeric private chat id; "
            "finite-agentd cannot represent it"
        )
    explicit = [
        _get(config, "gateway", "platforms", "telegram", "enabled"),
        _get(config, "platforms", "telegram", "enabled"),
        _get(config, "telegram", "enabled"),
    ]
    enabled = token is not None and not any(value is False for value in explicit)
    return (
        token,
        home,
        enabled,
        {
            "token_sources": sorted(tokens),
            "home_channel_sources": sorted(home_values),
            "source_explicitly_disabled": any(value is False for value in explicit),
        },
    )


def _read_pairing(
    snapshot: SourceSnapshot, directory: str, suffix: str
) -> dict[str, Any]:
    path = f".hermes/{directory}/telegram-{suffix}.json"
    if not snapshot.is_file(path):
        return {}
    try:
        value = json.loads(snapshot.read_bytes(path))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise MigrationError(f"legacy pairing store is not JSON: {path}") from exc
    if not isinstance(value, dict) or any(
        not isinstance(key, str) or not isinstance(item, dict)
        for key, item in value.items()
    ):
        raise MigrationError(f"legacy pairing store has an unsupported shape: {path}")
    return value


def merged_pairing(snapshot: SourceSnapshot) -> dict[str, dict[str, Any]]:
    """Union both legacy pairing dirs; `pairing/` wins, as target Hermes merges."""
    merged: dict[str, dict[str, Any]] = {}
    for suffix in ("approved", "pending"):
        combined = dict(_read_pairing(snapshot, "platforms/pairing", suffix))
        combined.update(_read_pairing(snapshot, "pairing", suffix))
        merged[suffix] = combined
    return merged


def _plan_telegram(
    snapshot: SourceSnapshot, config: dict[str, Any], env: dict[str, str]
) -> dict[str, Any]:
    token, home, enabled, evidence = telegram_source(config, env)
    pairing = merged_pairing(snapshot) if enabled else {"approved": {}, "pending": {}}
    sources = [
        directory
        for directory in ("pairing", "platforms/pairing")
        if any(
            snapshot.is_file(f".hermes/{directory}/telegram-{s}.json")
            for s in ("approved", "pending")
        )
    ]
    return {
        "activate": enabled,
        **evidence,
        "home_channel": "configured" if home else "absent",
        "target_path": "gateway.platforms.telegram",
        "shape": "finite-agentd telegram_connect_offer + telegram_home_offer",
        "dm_policy": "pairing",
        "preserve_pending_updates_on_start": enabled,
        "pairing": {
            "source_dirs": sources,
            "approved_count": len(pairing["approved"]),
            "pending_count": len(pairing["pending"]),
            "rate_limits": "not-carried",
        },
    }


def _plan_skills(payload: Path, snapshot: SourceSnapshot) -> dict[str, Any]:
    root = payload / "hermes/skills"
    manifest: dict[str, str] = {}
    manifest_path = root / ".bundled_manifest"
    if manifest_path.is_file():
        for line in manifest_path.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line:
                name, _, digest = line.partition(":")
                manifest[name.strip()] = digest.strip()
    managed_state = {
        path.removeprefix(".hermes/.runtime-managed/.hermes/skills/").removesuffix(
            ".sha256"
        )
        for path in snapshot.members
        if path.startswith(".hermes/.runtime-managed/.hermes/skills/")
        and path.endswith(".sha256")
    }
    user: list[dict[str, str]] = []
    managed: list[dict[str, str]] = []
    for relative in _scan_skill_dirs(root):
        directory = root / Path(relative.as_posix())
        name = _skill_name((directory / "SKILL.md").read_bytes(), relative.name)
        rel = relative.as_posix()
        if any(item == rel or item.startswith(rel + "/") for item in managed_state):
            managed.append(
                {"path": rel, "name": name, "reason": "box1-image-managed-overlay"}
            )
        elif name in manifest and (
            not manifest[name] or manifest[name] == _dir_md5(directory)
        ):
            reason = (
                "hermes-bundled-unmodified"
                if manifest[name]
                else "hermes-bundled-unverified"
            )
            managed.append({"path": rel, "name": name, "reason": reason})
        else:
            user.append({"path": rel, "name": name})
    hidden = (
        sorted(
            entry.name
            for entry in root.iterdir()
            if entry.name.startswith(".") and entry.is_dir()
        )
        if root.is_dir()
        else []
    )
    return {
        "user": user,
        "managed_not_activated": managed,
        "hidden_review_only": hidden,
        "target": f"{TARGET_HERMES_HOME}/skills",
        "clash_policy": "target-baseline-wins-source-copy-review-only",
    }


def _plan_scripts(payload: Path) -> dict[str, Any]:
    root = payload / "hermes/scripts"
    files = (
        sorted(
            path.relative_to(root).as_posix()
            for path in root.rglob("*")
            if path.is_file() or path.is_symlink()
        )
        if root.is_dir()
        else []
    )
    return {"files": files, "target": f"{TARGET_HERMES_HOME}/scripts"}


def load_jobs(payload: Path) -> list[dict[str, Any]]:
    path = payload / "hermes/cron/jobs.json"
    if not path.exists():
        return []
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise MigrationError(f"invalid legacy cron jobs.json: {exc}") from exc
    jobs = document.get("jobs") if isinstance(document, dict) else document
    if isinstance(jobs, dict):
        jobs = [{**job, "id": job.get("id") or key} for key, job in jobs.items()]
    if not isinstance(jobs, list) or any(not isinstance(job, dict) for job in jobs):
        raise MigrationError("legacy cron jobs.json must contain job objects")
    ids = [job.get("id") for job in jobs]
    if any(not isinstance(job_id, str) or not job_id for job_id in ids) or len(
        set(ids)
    ) != len(ids):
        raise MigrationError("legacy cron jobs need unique string ids")
    return jobs


def _plan_cron(payload: Path) -> dict[str, Any]:
    jobs = load_jobs(payload)
    summary = []
    for job in jobs:
        schedule = job.get("schedule") if isinstance(job.get("schedule"), dict) else {}
        summary.append(
            {
                "id": job["id"],
                "kind": schedule.get("kind"),
                "source_enabled": job.get("enabled", True) is not False,
                "deliver": sorted(_deliver_platforms(job)),
                "has_script": bool(job.get("script")),
            }
        )
    return {
        "jobs": summary,
        "target": f"{TARGET_HERMES_HOME}/cron/jobs.json",
        "next_run_policy": "recurring-recomputed-from-install-time",
        "overdue_oneshot_policy": "paused-with-reason",
    }


def _deliver_tokens(job: dict[str, Any]) -> list[str]:
    deliver = job.get("deliver", "local")
    if deliver is None:
        return ["local"]
    if isinstance(deliver, list):
        tokens = [str(item) for item in deliver]
    else:
        tokens = str(deliver).split(",")
    return [token.strip() for token in tokens if token.strip()] or ["local"]


def _deliver_platforms(job: dict[str, Any]) -> set[str]:
    platforms = set()
    for token in _deliver_tokens(job):
        if token == "local":
            continue
        if token == "origin":
            origin = job.get("origin")
            platform = origin.get("platform") if isinstance(origin, dict) else None
            platforms.add(str(platform).lower() if platform else "origin-missing")
        else:
            platforms.add(token.split(":", 1)[0].lower())
    return platforms


def _plan_omissions(
    snapshot: SourceSnapshot,
    config: dict[str, Any],
    env: dict[str, str],
    sites: dict[str, Any],
) -> dict[str, Any]:
    channels: dict[str, set[str]] = {}
    for name in env:
        for prefix, channel in CHANNEL_ENV_PREFIXES.items():
            if name.startswith(prefix):
                channels.setdefault(channel, set()).add(f"env:{name}")
    platforms = (
        config.get("platforms") if isinstance(config.get("platforms"), dict) else {}
    )
    gateway_platforms = _get(config, "gateway", "platforms")
    for source, mapping in (
        ("platforms", platforms),
        ("gateway.platforms", gateway_platforms),
    ):
        if isinstance(mapping, dict):
            for name, value in mapping.items():
                if isinstance(value, dict) and value.get("enabled") is not False:
                    channels.setdefault(str(name).lower(), set()).add(
                        f"config:{source}.{name}"
                    )
    for key in CHANNEL_CONFIG_KEYS:
        if isinstance(config.get(key), dict) and (
            _get(config, key, "token") or _get(config, key, "enabled") is True
        ):
            channels.setdefault(key, set()).add(f"config:{key}")
    for path in snapshot.members:
        match = re.fullmatch(
            r"\.hermes/(?:platforms/)?pairing/([a-z0-9_-]+)-approved\.json", path
        )
        if match:
            channels.setdefault(match.group(1), set()).add(f"pairing:{match.group(1)}")
    rendered_channels = [
        {
            "name": name,
            "evidence": sorted(evidence),
            "target_state": "configured-in-source-but-not-activated"
            if name in SUPPORTED_TARGET_CHANNELS
            else "not-supported-by-target-left-inactive",
        }
        for name, evidence in sorted(channels.items())
        if name != "finitechat"
    ]
    channel_env = tuple(CHANNEL_ENV_PREFIXES)
    tool_keys = []
    for name in sorted(env):
        if name.startswith(channel_env):
            continue
        if name in TARGET_RUNTIME_SECRET_REFERENCES:
            state = "target-provides-own-key-not-copied"
        elif name in TARGET_MANAGED_ENV:
            state = "target-managed-not-copied"
        else:
            state = "not-provided-by-target-not-copied"
        tool_keys.append({"name": name, "target_state": state})
    google = sorted(
        path
        for path in (".hermes/google_token.json", ".hermes/google_client_secret.json")
        if snapshot.is_file(path)
    )
    if any(path.startswith(".hermes/gws/") for path in snapshot.members):
        google.append(".hermes/gws")
    inactive_state = sorted(
        path
        for path in (
            ".hermes/channel_directory.json",
            ".hermes/gateway_state.json",
            ".hermes/kanban.db",
            ".hermes/auth.json",
            ".hermes/sessions",
            ".hermes/processes.json",
        )
        if path in snapshot.members
    )
    return {
        "channels": rendered_channels,
        "tool_keys": tool_keys,
        "google": {
            "evidence": google,
            "target_state": "out-of-scope-fresh-authorization-required"
            if google
            else "absent",
        },
        "sites": {
            "endpoint_count": sites["endpoint_count"],
            "target_state": "recorded-not-republished",
        },
        "other_inactive_state": inactive_state,
    }


# --------------------------------------------------------------------------
# Install-time application


@dataclass
class Transaction:
    """Record every created or replaced target path so failure can undo it."""

    target_root: Path
    backup_root: Path
    created_files: list[Path] = field(default_factory=list)
    created_directories: list[Path] = field(default_factory=list)
    replaced: list[tuple[Path, Path]] = field(default_factory=list)

    def ensure_parents(self, target: Path) -> None:
        cursor = self.target_root
        for component in target.relative_to(self.target_root).parts[:-1]:
            cursor /= component
            if cursor.is_symlink():
                raise MigrationError(f"target parent is a symlink: {cursor}")
            if cursor.exists():
                if not cursor.is_dir():
                    raise MigrationError(f"target parent is not a directory: {cursor}")
                continue
            cursor.mkdir(mode=0o700)
            self.created_directories.append(cursor)

    def make_directory(self, target: Path, mode: int) -> None:
        self.ensure_parents(target)
        if target.is_symlink() or (target.exists() and not target.is_dir()):
            raise MigrationError(f"destination collision: {self.relative(target)}")
        if target.exists():
            return
        target.mkdir(mode=0o700)
        self.created_directories.append(target)
        os.chmod(target, mode & 0o777 | 0o700)

    def write_new(self, target: Path, data: bytes, mode: int) -> None:
        self.ensure_parents(target)
        try:
            descriptor = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        except FileExistsError as exc:
            raise MigrationError(
                f"destination collision: {self.relative(target)}"
            ) from exc
        self.created_files.append(target)
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(data)
        os.chmod(target, mode & 0o777)

    def copy_stream(self, target: Path, source: Any, mode: int) -> None:
        self.ensure_parents(target)
        try:
            descriptor = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        except FileExistsError as exc:
            raise MigrationError(
                f"destination collision: {self.relative(target)}"
            ) from exc
        self.created_files.append(target)
        with os.fdopen(descriptor, "wb") as handle:
            shutil.copyfileobj(source, handle, 1024 * 1024)
        os.chmod(target, mode & 0o777)

    def symlink_new(self, target: Path, link: str) -> None:
        self.ensure_parents(target)
        if target.exists() or target.is_symlink():
            raise MigrationError(f"destination collision: {self.relative(target)}")
        target.symlink_to(link)
        self.created_files.append(target)

    def replace(self, target: Path, data: bytes, mode: int) -> None:
        """Atomically replace an existing target file, keeping the original."""
        self.ensure_parents(target)
        if target.is_symlink():
            raise MigrationError(f"target file is a symlink: {self.relative(target)}")
        if target.exists():
            self.backup_root.mkdir(mode=0o700, parents=True, exist_ok=True)
            backup = self.backup_root / self.relative(target).replace("/", "__")
            shutil.copy2(target, backup)
            self.replaced.append((backup, target))
        else:
            self.created_files.append(target)
        temporary = target.with_name(f".{target.name}.legacy-import")
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        try:
            with os.fdopen(descriptor, "wb") as handle:
                handle.write(data)
                handle.flush()
                os.fsync(handle.fileno())
            os.chmod(temporary, mode & 0o777)
            os.replace(temporary, target)
        except BaseException:
            temporary.unlink(missing_ok=True)
            raise
        _fsync_directory(target.parent)

    def relative(self, path: Path) -> str:
        return path.relative_to(self.target_root).as_posix()

    def rollback(self) -> None:
        for backup, original in reversed(self.replaced):
            if backup.exists():
                os.replace(backup, original)
        for path in reversed(self.created_files):
            path.unlink(missing_ok=True)
        for path in reversed(self.created_directories):
            try:
                path.rmdir()
            except OSError:
                pass


@dataclass
class FidelityHooks:
    """Target-environment probes, injectable so tests need no Runtime image."""

    is_default_soul: Callable[[str], bool]
    bundled_skill_names: Callable[[], dict[str, PurePosixPath]]
    telegram_preserves_pending_updates: Callable[[], bool]
    now: Callable[[], datetime]


def default_hooks() -> FidelityHooks:
    return FidelityHooks(
        is_default_soul=_target_default_soul,
        bundled_skill_names=_target_bundled_skill_names,
        telegram_preserves_pending_updates=_target_telegram_preserves_pending,
        now=lambda: datetime.now(timezone.utc),
    )


def _normalize_soul(text: str) -> str:
    return text.replace("\r\n", "\n").strip()


def _target_default_soul(text: str) -> bool:
    try:
        from hermes_cli.default_soul import DEFAULT_SOUL_MD, is_legacy_template_soul
    except Exception:
        return False
    return _normalize_soul(text) == _normalize_soul(
        DEFAULT_SOUL_MD
    ) or is_legacy_template_soul(text)


def _target_bundled_skill_names() -> dict[str, PurePosixPath]:
    candidates = []
    if os.environ.get("HERMES_BUNDLED_SKILLS"):
        candidates.append(Path(os.environ["HERMES_BUNDLED_SKILLS"]))
    if os.environ.get("HERMES_BUNDLED_PLUGINS"):
        candidates.append(Path(os.environ["HERMES_BUNDLED_PLUGINS"]).parent / "skills")
    names: dict[str, PurePosixPath] = {}
    for candidate in candidates:
        for name, relative in skill_names_in(candidate).items():
            names.setdefault(name, relative)
    return names


def _target_telegram_preserves_pending() -> bool:
    try:
        from hermes_cli.plugins import get_bundled_plugins_dir
    except Exception:
        return False
    adapter = get_bundled_plugins_dir() / "platforms/telegram/adapter.py"
    try:
        return "_preserve_pending_updates_on_start" in adapter.read_text(
            encoding="utf-8"
        )
    except OSError:
        return False


def _target_path(target_root: Path, absolute: str) -> Path:
    if not absolute.startswith("/data/"):
        raise MigrationError(f"target path is outside /data: {absolute}")
    return target_root / absolute.removeprefix("/data/")


def _rewrite_text_bytes(data: bytes, path_map: PathMap) -> bytes:
    if len(data) > TEXT_REWRITE_LIMIT_BYTES or b"\0" in data:
        return data
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        return data
    return path_map.rewrite_text(text).encode("utf-8")


class FidelityInstall:
    """Preflight against the stopped target, then apply inside a transaction."""

    def __init__(
        self,
        plan: dict[str, Any],
        bundle: Path,
        target_root: Path,
        inventory: dict[str, Any],
        hooks: FidelityHooks,
    ):
        self.plan = plan
        self.payload = bundle / "payload"
        self.target_root = target_root
        self.inventory = inventory
        self.hooks = hooks
        self.now = hooks.now().astimezone(timezone.utc).replace(microsecond=0)
        self.path_map = PathMap(plan["path_map"]["placed_home_entries"])
        self.hermes_home = target_root / Path(TARGET_HERMES_RELATIVE.as_posix())
        self.result: dict[str, Any] = {}
        self.snapshot = SourceSnapshot(
            self.payload / Path(SOURCE_SNAPSHOT_RELATIVE_PATH.as_posix()), inventory
        )

    def close(self) -> None:
        self.snapshot.close()

    # -- preflight ---------------------------------------------------------

    def preflight(self) -> None:
        config_path = self.hermes_home / "config.yaml"
        if not config_path.is_file() or config_path.is_symlink():
            raise MigrationError(
                "target Hermes config.yaml is missing; boot the target once first"
            )
        self.target_config_bytes = config_path.read_bytes()
        self.target_config = _load_yaml_mapping(
            self.target_config_bytes.decode("utf-8"), "target Hermes config.yaml"
        )
        self.source_config = _source_config(self.snapshot)
        self.source_env = _source_env(self.snapshot)
        self.telegram = telegram_source(self.source_config, self.source_env)
        self._preflight_telegram()
        self._preflight_soul()
        self._preflight_cron()
        self.skill_decisions = self._preflight_skills()
        self._preflight_home()

    def _preflight_telegram(self) -> None:
        if not self.plan["telegram"]["activate"]:
            return
        current = _get(self.target_config, "gateway", "platforms", "telegram")
        if current is not None and not isinstance(current, dict):
            raise MigrationError(
                "target Telegram settings are not in a supported format"
            )
        if isinstance(current, dict) and (
            current.get("token") or current.get("enabled")
        ):
            raise MigrationError(
                "target already has a Telegram connection; refusing to replace it"
            )
        if not self.hooks.telegram_preserves_pending_updates():
            raise MigrationError(
                "target Hermes drops queued Telegram updates on a cold start; "
                "use a Runtime image with hermes-telegram-preserve-pending.patch"
            )
        self.pairing_dir = self._target_pairing_dir()
        for directory in {
            self.hermes_home / "pairing",
            self.hermes_home / "platforms/pairing",
        }:
            for suffix in ("approved", "pending"):
                existing = directory / f"telegram-{suffix}.json"
                if existing.exists():
                    try:
                        value = json.loads(existing.read_text(encoding="utf-8"))
                    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
                        raise MigrationError(
                            "target Telegram pairing store is unreadable"
                        ) from exc
                    if value:
                        raise MigrationError(
                            "target Telegram pairing store is not empty"
                        )

    def _target_pairing_dir(self) -> Path:
        legacy = self.hermes_home / "pairing"
        if legacy.is_dir() and any(legacy.iterdir()):
            return legacy
        return self.hermes_home / "platforms/pairing"

    def _preflight_soul(self) -> None:
        if not self.plan["persona"]["soul"]["present"]:
            return
        target = self.hermes_home / "SOUL.md"
        if target.is_symlink():
            raise MigrationError("target SOUL.md is a symlink")
        if target.exists():
            text = target.read_text(encoding="utf-8")
            if not (not text.strip() or self.hooks.is_default_soul(text)):
                raise MigrationError(
                    "target SOUL.md was customized; refusing to replace it"
                )

    def _preflight_cron(self) -> None:
        jobs_path = self.hermes_home / "cron/jobs.json"
        if not jobs_path.exists():
            return
        try:
            document = json.loads(jobs_path.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise MigrationError("target cron jobs.json is unreadable") from exc
        jobs = document.get("jobs") if isinstance(document, dict) else document
        if jobs:
            raise MigrationError("target already has scheduled jobs; refusing to merge")

    def _target_skill_names(self) -> dict[str, str]:
        names: dict[str, str] = {}
        for name in skill_names_in(self.hermes_home / "skills"):
            names.setdefault(name, "target-hermes-home-skill")
        external = _get(self.target_config, "skills", "external_dirs") or []
        for entry in external if isinstance(external, list) else []:
            if isinstance(entry, str) and entry.startswith("/data/"):
                for name in skill_names_in(_target_path(self.target_root, entry)):
                    names.setdefault(name, "target-managed-skills-baseline")
        for name in self.hooks.bundled_skill_names():
            names.setdefault(name, "target-hermes-bundled-skill")
        return names

    def _preflight_skills(self) -> list[dict[str, str]]:
        target_names = self._target_skill_names()
        decisions = []
        for skill in self.plan["skills"]["user"]:
            clash = target_names.get(skill["name"])
            destination = self.hermes_home / "skills" / skill["path"]
            if clash is None and (destination.exists() or destination.is_symlink()):
                raise MigrationError(
                    f"destination collision: agent/hermes-home/skills/{skill['path']}"
                )
            decisions.append(
                {
                    **skill,
                    "outcome": "review-only" if clash else "activated",
                    "reason": clash or "",
                }
            )
        return decisions

    def _preflight_home(self) -> None:
        for name in self.plan["home"]["placed"]:
            destination = (
                self.target_root / Path(TARGET_LEGACY_RELATIVE.as_posix()) / name
            )
            if destination.exists() or destination.is_symlink():
                raise MigrationError(
                    f"destination collision: {TARGET_LEGACY_RELATIVE}/{name}"
                )
        scripts = self.hermes_home / "scripts"
        for relative in self.plan["scripts"]["files"]:
            if (scripts / relative).exists() or (scripts / relative).is_symlink():
                raise MigrationError(
                    f"destination collision: agent/hermes-home/scripts/{relative}"
                )

    # -- apply -------------------------------------------------------------

    def apply(self, transaction: Transaction) -> dict[str, Any]:
        self._apply_soul(transaction)
        self._apply_agents_review(transaction)
        self._apply_config(transaction)
        self._apply_pairing(transaction)
        self._apply_skills(transaction)
        self._apply_scripts(transaction)
        self._apply_cron(transaction)
        self._apply_home(transaction)
        self.result["path_rewrites"] = self.path_map.counts()
        return self.result

    def _apply_soul(self, transaction: Transaction) -> None:
        if not self.plan["persona"]["soul"]["present"]:
            self.result["soul"] = "absent-in-source"
            return
        data = self.snapshot.read_bytes(".hermes/SOUL.md")
        if hashlib.sha256(data).hexdigest() != self.plan["persona"]["soul"]["sha256"]:
            raise MigrationError("source SOUL.md does not match the approved plan")
        transaction.replace(
            self.hermes_home / "SOUL.md",
            _rewrite_text_bytes(data, self.path_map),
            0o600,
        )
        self.result["soul"] = "activated"

    def _apply_agents_review(self, transaction: Transaction) -> None:
        review = self.target_root / Path(REVIEW_ONLY_RELATIVE.as_posix()) / "agents"
        for path in self.plan["persona"]["agents_md"]["paths"]:
            name = "home-AGENTS.md" if path == "AGENTS.md" else "hermes-AGENTS.md"
            transaction.write_new(review / name, self.snapshot.read_bytes(path), 0o600)
        self.result["agents_md"] = self.plan["persona"]["agents_md"]["policy"]

    def _apply_config(self, transaction: Transaction) -> None:
        config = copy.deepcopy(self.target_config)
        adopted, _ = adopted_settings(self.source_config)
        adopted = self.path_map.rewrite_value(adopted)
        for key, value in adopted.items():
            if key in ("display", "agent", "cron", "memory"):
                current = config.get(key)
                if current is not None and not isinstance(current, dict):
                    raise MigrationError(f"target config {key} is not a mapping")
                merged = dict(current or {})
                merged.update(value)
                config[key] = merged
            else:
                config[key] = value
        target_model = _get(config, "model", "default")
        source_model = self.plan["config"]["source_model"]
        if source_model and source_model == target_model:
            model_outcome = "source-model-kept"
        elif source_model:
            model_outcome = "source-model-not-offered-target-default-kept"
        else:
            model_outcome = "target-default"
        token, home, enabled, _ = self.telegram
        if self.plan["telegram"]["activate"]:
            if not enabled or token is None:
                raise MigrationError(
                    "source Telegram connection does not match the approved plan"
                )
            gateway = config.setdefault("gateway", {})
            platforms = gateway.setdefault("platforms", {})
            if not isinstance(gateway, dict) or not isinstance(platforms, dict):
                raise MigrationError("target gateway platforms are not a mapping")
            telegram = dict(platforms.get("telegram") or {})
            telegram.update(
                {"enabled": True, "token": token, **AGENTD_TELEGRAM_DEFAULTS}
            )
            extra = dict(telegram.get("extra") or {})
            extra["dm_policy"] = "pairing"
            extra[TELEGRAM_PRESERVE_PENDING_KEY] = True
            telegram["extra"] = extra
            if home is not None:
                telegram["home_channel"] = {
                    "platform": "telegram",
                    "chat_id": home,
                    "name": "Telegram",
                }
            platforms["telegram"] = telegram
        mode = stat.S_IMODE((self.hermes_home / "config.yaml").stat().st_mode)
        rendered = _yaml().safe_dump(config, sort_keys=False, allow_unicode=True)
        transaction.replace(
            self.hermes_home / "config.yaml", rendered.encode("utf-8"), mode
        )
        self.result["config"] = {
            "adopted_keys": self.plan["config"]["adopted_keys"],
            "model": model_outcome,
            "telegram": "connected-with-home-channel"
            if self.plan["telegram"]["activate"] and home
            else (
                "connected"
                if self.plan["telegram"]["activate"]
                else "not-configured-in-source"
            ),
        }

    def _apply_pairing(self, transaction: Transaction) -> None:
        if not self.plan["telegram"]["activate"]:
            self.result["pairing"] = {"approved_count": 0, "pending_count": 0}
            return
        merged = merged_pairing(self.snapshot)
        for suffix, value in merged.items():
            if not value:
                continue
            data = json.dumps(value, indent=2, ensure_ascii=False).encode("utf-8")
            transaction.replace(
                self.pairing_dir / f"telegram-{suffix}.json", data, 0o600
            )
        approved = len(merged["approved"])
        pending = len(merged["pending"])
        if (approved, pending) != (
            self.plan["telegram"]["pairing"]["approved_count"],
            self.plan["telegram"]["pairing"]["pending_count"],
        ):
            raise MigrationError(
                "source pairing store does not match the approved plan"
            )
        self.result["pairing"] = {
            "approved_count": approved,
            "pending_count": pending,
            "target_dir": transaction.relative(self.pairing_dir),
        }

    def _copy_tree(
        self, transaction: Transaction, source: Path, target: Path, rewrite: bool
    ) -> None:
        for path in sorted(source.rglob("*")):
            relative = path.relative_to(source)
            destination = target / relative
            info = path.lstat()
            if stat.S_ISLNK(info.st_mode):
                link = os.readlink(path)
                resolved = (path.parent / link).resolve(strict=False)
                try:
                    resolved.relative_to(source.resolve())
                except ValueError as exc:
                    raise MigrationError(
                        f"skill or script symlink escapes its tree: {relative}"
                    ) from exc
                transaction.symlink_new(destination, link)
            elif stat.S_ISDIR(info.st_mode):
                transaction.make_directory(destination, stat.S_IMODE(info.st_mode))
            elif stat.S_ISREG(info.st_mode):
                data = path.read_bytes()
                if rewrite:
                    data = _rewrite_text_bytes(data, self.path_map)
                transaction.write_new(destination, data, stat.S_IMODE(info.st_mode))
            else:
                raise MigrationError(
                    f"special file in skill or script tree: {relative}"
                )

    def _apply_skills(self, transaction: Transaction) -> None:
        source_root = self.payload / "hermes/skills"
        for decision in self.skill_decisions:
            if decision["outcome"] != "activated":
                continue
            relative = Path(decision["path"])
            destination = self.hermes_home / "skills" / relative
            transaction.make_directory(destination, 0o755)
            self._copy_tree(
                transaction, source_root / relative, destination, rewrite=True
            )
        self.result["skills"] = {
            "activated": sorted(
                d["path"] for d in self.skill_decisions if d["outcome"] == "activated"
            ),
            "review_only_clashes": [
                {"path": d["path"], "name": d["name"], "reason": d["reason"]}
                for d in self.skill_decisions
                if d["outcome"] != "activated"
            ],
            "managed_not_activated": len(self.plan["skills"]["managed_not_activated"]),
            "hidden_review_only": self.plan["skills"]["hidden_review_only"],
        }

    def _apply_scripts(self, transaction: Transaction) -> None:
        source_root = self.payload / "hermes/scripts"
        if source_root.is_dir():
            transaction.make_directory(self.hermes_home / "scripts", 0o700)
            self._copy_tree(
                transaction, source_root, self.hermes_home / "scripts", rewrite=True
            )
        self.result["scripts"] = {"activated_count": len(self.plan["scripts"]["files"])}

    def _apply_cron(self, transaction: Transaction) -> None:
        jobs = load_jobs(self.payload)
        if [job["id"] for job in jobs] != [
            job["id"] for job in self.plan["cron"]["jobs"]
        ]:
            raise MigrationError("source cron jobs do not match the approved plan")
        if not jobs:
            self.result["cron"] = {"count": 0}
            return
        timezone_name = self._effective_timezone()
        outcomes: dict[str, list[str]] = {}
        transformed = []
        for job in jobs:
            new_job, outcome = self._transform_job(job, timezone_name)
            transformed.append(new_job)
            outcomes.setdefault(outcome, []).append(job["id"])
        document = {"jobs": transformed, "updated_at": self.now.isoformat()}
        data = (json.dumps(document, indent=2, ensure_ascii=False) + "\n").encode(
            "utf-8"
        )
        transaction.make_directory(self.hermes_home / "cron", 0o700)
        transaction.write_new(self.hermes_home / "cron/jobs.json", data, 0o600)
        self.result["cron"] = {
            "count": len(jobs),
            "timezone": timezone_name or "server-local",
            "outcomes": {key: sorted(value) for key, value in sorted(outcomes.items())},
        }

    def _effective_timezone(self) -> str | None:
        source = self.source_config.get("timezone")
        target = self.target_config.get("timezone")
        for candidate in (source, target):
            if isinstance(candidate, str) and candidate.strip():
                try:
                    from zoneinfo import ZoneInfo

                    ZoneInfo(candidate.strip())
                except Exception:
                    continue
                return candidate.strip()
        return None

    def _active_platforms(self) -> set[str]:
        platforms = {"finitechat"}
        if self.plan["telegram"]["activate"]:
            platforms.add("telegram")
        return platforms

    def _transform_job(
        self, job: dict[str, Any], timezone_name: str | None
    ) -> tuple[dict[str, Any], str]:
        job = json.loads(json.dumps(job))
        for key in ("run_claim", "fire_claim"):
            job.pop(key, None)
        for key in ("prompt", "workdir", "script"):
            if isinstance(job.get(key), str):
                job[key] = self.path_map.rewrite_text(job[key])
        notes: list[str] = []
        target_model = _get(self.target_config, "model")
        target_model = target_model if isinstance(target_model, dict) else {}
        if job.get("model") not in (None, "", target_model.get("default")):
            job["model"] = None
            notes.append("model")
        for key in ("provider", "base_url"):
            if job.get(key) not in (None, "", target_model.get(key)):
                job[key] = None
                notes.append(key)
        pause_reason = self._pause_reason(job)
        schedule = job.get("schedule") if isinstance(job.get("schedule"), dict) else {}
        kind = schedule.get("kind")
        enabled = (
            job.get("enabled", True) is not False
            and not job.get("paused_at")
            and job.get("state") != "paused"
        )
        if pause_reason and enabled:
            self._pause(job, pause_reason)
            return job, f"paused:{pause_reason.split(':', 1)[0]}"
        outcome = "kept-disabled" if not enabled else "scheduled"
        if kind in ("cron", "interval") and enabled:
            job["next_run_at"] = _next_run(schedule, self.now, timezone_name)
            outcome = "recurring-next-run-recomputed"
        elif kind == "once" and enabled:
            run_at = schedule.get("run_at")
            if job.get("last_run_at") or job.get("state") == "completed":
                outcome = "oneshot-already-ran-kept"
            elif not isinstance(run_at, str):
                self._pause(job, "legacy-migration: one-shot has no run_at")
                outcome = "paused:oneshot-without-run-at"
            else:
                try:
                    due = datetime.fromisoformat(run_at.replace("Z", "+00:00"))
                except ValueError as exc:
                    raise MigrationError(
                        f"cron job {job['id']} has an invalid run_at"
                    ) from exc
                if due.tzinfo is None:
                    due = due.replace(tzinfo=_zone(timezone_name))
                if due <= self.now:
                    self._pause(job, ONESHOT_MISSED_REASON)
                    outcome = "paused:oneshot-missed-during-cutover"
                else:
                    job["next_run_at"] = run_at
                    outcome = "oneshot-scheduled"
        if notes:
            outcome += "+inference-route-normalized"
        return job, outcome

    def _pause_reason(self, job: dict[str, Any]) -> str | None:
        active = self._active_platforms()
        for platform in sorted(_deliver_platforms(job)):
            if platform == "origin-missing":
                continue
            if platform not in active:
                return f"delivery-platform-inactive: {platform}"
        if "telegram" in _deliver_tokens(job) and not self.telegram[1]:
            return "delivery-target-missing: telegram home channel"
        script = job.get("script")
        if isinstance(script, str) and script.strip():
            scripts_root = f"{TARGET_HERMES_HOME}/scripts/"
            if script.startswith("/"):
                if not script.startswith(scripts_root):
                    return "script-outside-hermes-scripts"
                relative = script.removeprefix(scripts_root)
            else:
                relative = script
            if relative not in self.plan["scripts"]["files"]:
                return "script-missing"
        return None

    def _pause(self, job: dict[str, Any], reason: str) -> None:
        job["enabled"] = False
        job["state"] = "paused"
        job["paused_at"] = self.now.isoformat()
        job["paused_reason"] = reason

    def _apply_home(self, transaction: Transaction) -> None:
        root = self.target_root / Path(TARGET_LEGACY_RELATIVE.as_posix())
        placed_entries = 0
        skipped_links = 0
        skipped_generated = 0
        for name in self.plan["home"]["placed"]:
            paths = [name] + (
                self.snapshot.under(name) if self.snapshot.members[name].isdir() else []
            )
            for path in paths:
                if self.snapshot.dispositions.get(path) not in ("preserve", "activate"):
                    skipped_generated += 1
                    continue
                if any(
                    self.snapshot.dispositions.get(parent)
                    not in ("preserve", "activate")
                    for parent in _parents(path)
                ):
                    skipped_generated += 1
                    continue
                member = self.snapshot.members[path]
                destination = root / path
                if member.isdir():
                    transaction.make_directory(destination, member.mode)
                elif member.isfile():
                    handle = self.snapshot.archive.extractfile(member)
                    if handle is None:
                        raise MigrationError(
                            f"source snapshot file cannot be read: {path}"
                        )
                    transaction.copy_stream(destination, handle, member.mode)
                elif member.issym():
                    link = self._placed_link(path, member.linkname)
                    if link is None:
                        skipped_links += 1
                        continue
                    transaction.symlink_new(destination, link)
                else:
                    raise MigrationError(
                        f"source snapshot entry is unsupported: {path}"
                    )
                placed_entries += 1
        self.result["home"] = {
            "placed": self.plan["home"]["placed"],
            "placed_entry_count": placed_entries,
            "skipped_generated_entry_count": skipped_generated,
            "skipped_symlink_count": skipped_links,
            "withheld": self.plan["home"]["withheld"],
            "target_root": TARGET_LEGACY_ROOT,
        }

    def _placed_link(self, path: str, link: str) -> str | None:
        if link.startswith("/") or link.startswith("~/"):
            mapped = self.path_map.map_path(link)
            if mapped and (
                mapped.startswith(TARGET_LEGACY_ROOT + "/")
                or mapped.startswith(TARGET_HERMES_HOME + "/")
            ):
                return mapped
            return None
        resolved = PurePosixPath(os.path.normpath(PurePosixPath(path).parent / link))
        if resolved.parts and resolved.parts[0] in self.plan["home"]["placed"]:
            return link
        return None


def _parents(path: str) -> list[str]:
    parts = path.split("/")
    return ["/".join(parts[:index]) for index in range(1, len(parts))]


def _zone(timezone_name: str | None):
    if timezone_name:
        from zoneinfo import ZoneInfo

        return ZoneInfo(timezone_name)
    return datetime.now().astimezone().tzinfo


def _next_run(
    schedule: dict[str, Any], now: datetime, timezone_name: str | None
) -> str:
    """Next run strictly after the install time, as Hermes compute_next_run."""
    local_now = now.astimezone(_zone(timezone_name))
    if schedule.get("kind") == "interval":
        minutes = schedule.get("minutes")
        if (
            not isinstance(minutes, (int, float))
            or isinstance(minutes, bool)
            or minutes <= 0
        ):
            raise MigrationError("legacy interval job has an invalid period")
        return (local_now + timedelta(minutes=minutes)).isoformat()
    expr = schedule.get("expr")
    if not isinstance(expr, str) or not expr.strip():
        raise MigrationError("legacy cron job has no expression")
    try:
        from croniter import croniter
    except ImportError as exc:  # pragma: no cover - a Hermes dependency
        raise MigrationError("full-fidelity cron needs croniter") from exc
    try:
        return croniter(expr, local_now).get_next(datetime).isoformat()
    except Exception as exc:
        raise MigrationError("legacy cron job has an invalid expression") from exc


# --------------------------------------------------------------------------
# Session rewriting


def session_rewriter(
    plan: dict[str, Any],
) -> tuple[Callable[[dict[str, Any]], dict[str, Any]], PathMap]:
    path_map = PathMap(plan["path_map"]["placed_home_entries"])

    def rewrite(session: dict[str, Any]) -> dict[str, Any]:
        for key in ("cwd", "git_repo_root"):
            if isinstance(session.get(key), str):
                session[key] = path_map.rewrite_text(session[key])
        session["messages"] = path_map.rewrite_value(session.get("messages") or [])
        return session

    return rewrite, path_map


def text_file_rewriter(
    plan: dict[str, Any],
) -> tuple[Callable[[bytes], bytes], PathMap]:
    path_map = PathMap(plan["path_map"]["placed_home_entries"])
    return (lambda data: _rewrite_text_bytes(data, path_map)), path_map


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()

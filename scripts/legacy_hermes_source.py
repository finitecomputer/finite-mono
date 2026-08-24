#!/usr/bin/env python3
"""Frozen-source evidence and export operations for legacy Hermes migrations."""

from __future__ import annotations

import hashlib
import importlib.metadata
import json
import os
import re
import sqlite3
import stat
from contextlib import closing
from pathlib import Path
from typing import Any

from legacy_hermes_contract import (
    INTEGRATIONS_INVENTORY_SCHEMA,
    SITES_INVENTORY_SCHEMA,
    SOURCE_EXPORT_BATCH_SIZE,
    SOURCE_INVENTORY_SCHEMA,
    SUPPORTED_SOURCE_HERMES_VERSION,
    MigrationError,
    _copy_sqlite,
    _fsync_directory,
    _memory_database_fact_count,
    _sha256,
    _write_private_json,
)

SOURCE_ACTIVATE_ROOTS = (
    Path(".hermes/memories"),
    Path(".hermes/skills"),
    Path(".hermes/scripts"),
    Path(".hermes/cron/jobs.json"),
    Path("workspace"),
    Path("dev"),
    Path("uploads"),
)
SOURCE_CONVERTED_FILES = (
    Path(".hermes/state.db"),
    Path(".hermes/state.db-wal"),
    Path(".hermes/state.db-shm"),
    Path(".hermes/memory_store.db"),
    Path(".hermes/memory_store.db-wal"),
    Path(".hermes/memory_store.db-shm"),
)
SOURCE_QUARANTINE_ROOTS = (
    Path(".brain"),
    Path(".finite"),
    Path(".codex"),
    Path(".hermes/cron"),
    Path(".hermes/.env"),
    Path(".hermes/auth.json"),
    Path(".hermes/config.json"),
    Path(".hermes/config.yaml"),
    Path(".hermes/credentials"),
    Path(".hermes/tokens"),
)
SOURCE_REBUILD_ROOTS = (
    Path(".agent-browser"),
    Path(".bun"),
    Path(".cache"),
    Path(".cargo"),
    Path(".config/pulse"),
    Path(".hermes/venv"),
    Path(".local"),
    Path(".npm"),
    Path(".npm-global"),
    Path(".rustup"),
    Path("dev/reap-video/venv"),
)
_ENV_NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
_INTEGRATION_POLICIES = {
    "telegram": "controlled-transfer-after-rehearsal",
    "signal": "controlled-transfer-after-rehearsal",
    "google-workspace": "fresh-authorization-required",
    "finitebrain": "fresh-authorization-required",
    "model-provider-credentials": "target-managed-not-copied",
    "other-environment-config": "preserve-disabled-until-supported-setup",
}
_MODEL_PROVIDER_PREFIXES = (
    "ANTHROPIC_",
    "DEEPSEEK_",
    "GEMINI_",
    "MISTRAL_",
    "OPENAI_",
    "OPENROUTER_",
    "TOGETHER_",
    "XAI_",
    "ZAI_",
)


def _require_source_hermes_version() -> None:
    try:
        actual = importlib.metadata.version("hermes-agent")
    except importlib.metadata.PackageNotFoundError as exc:
        raise MigrationError("source Hermes package is not installed") from exc
    if actual != SUPPORTED_SOURCE_HERMES_VERSION:
        raise MigrationError(
            "source Hermes version must be "
            f"{SUPPORTED_SOURCE_HERMES_VERSION}, got {actual}"
        )


def export_source_sessions(output: Path, source_database: Path) -> dict[str, Any]:
    """Snapshot and stream every legacy session through its public API."""
    _require_source_hermes_version()
    output = Path(output)
    source_database = Path(source_database)
    if output.exists() or output.is_symlink():
        raise MigrationError(f"refusing to overwrite session export: {output}")
    if not source_database.is_file() or source_database.is_symlink():
        raise MigrationError(
            f"source session database is missing or unsafe: {source_database}"
        )
    from hermes_state import SessionDB

    temp = output.with_name(f".{output.name}.partial")
    if temp.exists():
        raise MigrationError(f"partial export already exists: {temp}")
    snapshot = output.with_name(f".{output.name}.snapshot.db")
    if snapshot.exists():
        raise MigrationError(f"partial session snapshot already exists: {snapshot}")
    count = 0
    messages = 0
    database = None
    try:
        _copy_sqlite(source_database, snapshot)
        database = SessionDB(db_path=snapshot)
        expected_count = database.session_count()
        output.parent.mkdir(parents=True, exist_ok=True)
        fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        seen_session_ids: set[str] = set()
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            while count < expected_count:
                rows = database.search_sessions(
                    limit=min(SOURCE_EXPORT_BATCH_SIZE, expected_count - count),
                    offset=count,
                )
                if not rows:
                    raise MigrationError(
                        f"session listing returned {count} of {expected_count}; refusing partial export"
                    )
                for row in rows:
                    session_id = row["id"]
                    if session_id in seen_session_ids:
                        raise MigrationError(
                            f"session listing repeated id {session_id}; refusing partial export"
                        )
                    seen_session_ids.add(session_id)
                    session = database.export_session(session_id)
                    if session is None:
                        raise MigrationError(
                            f"session disappeared during export: {session_id}"
                        )
                    handle.write(
                        json.dumps(session, ensure_ascii=False, separators=(",", ":"))
                    )
                    handle.write("\n")
                    count += 1
                    messages += len(session.get("messages") or [])
            if count != expected_count:
                raise MigrationError(
                    f"session listing returned {count} of {expected_count}; refusing partial export"
                )
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temp, output)
        _fsync_directory(output.parent)
    except BaseException:
        temp.unlink(missing_ok=True)
        raise
    finally:
        if database is not None:
            database.close()
        snapshot.unlink(missing_ok=True)
        Path(str(snapshot) + "-wal").unlink(missing_ok=True)
        Path(str(snapshot) + "-shm").unlink(missing_ok=True)
    return {
        "sessions": count,
        "messages": messages,
        "bytes": output.stat().st_size,
        "sha256": _sha256(output),
    }


def snapshot_source_memory(output: Path, source_database: Path) -> dict[str, Any]:
    """Create a SQLite-API snapshot for target-version memory rebuilding."""
    _require_source_hermes_version()
    output = Path(output)
    source_database = Path(source_database)
    if output.name != "memory_store.db":
        raise MigrationError("memory snapshot output must be named memory_store.db")
    if output.exists() or output.is_symlink():
        raise MigrationError(f"refusing to overwrite memory snapshot: {output}")
    if not source_database.is_file() or source_database.is_symlink():
        raise MigrationError(
            f"source memory database is missing or unsafe: {source_database}"
        )
    try:
        _copy_sqlite(source_database, output)
        with closing(sqlite3.connect(output)) as connection:
            checkpoint = connection.execute(
                "PRAGMA wal_checkpoint(TRUNCATE)"
            ).fetchone()
            if checkpoint is None or checkpoint[0] != 0:
                raise MigrationError("memory snapshot failed WAL checkpoint")
            journal_mode = connection.execute("PRAGMA journal_mode=DELETE").fetchone()
            if journal_mode is None or str(journal_mode[0]).lower() != "delete":
                raise MigrationError("memory snapshot could not become self-contained")
        Path(str(output) + "-wal").unlink(missing_ok=True)
        Path(str(output) + "-shm").unlink(missing_ok=True)
        os.chmod(output, 0o600)
        fact_count = _memory_database_fact_count(output)
        with output.open("rb") as handle:
            os.fsync(handle.fileno())
        _fsync_directory(output.parent)
    except BaseException:
        output.unlink(missing_ok=True)
        Path(str(output) + "-wal").unlink(missing_ok=True)
        Path(str(output) + "-shm").unlink(missing_ok=True)
        raise
    return {
        "facts": fact_count,
        "bytes": output.stat().st_size,
        "sha256": _sha256(output),
    }


def _is_below(relative: Path, root: Path) -> bool:
    return relative == root or relative.is_relative_to(root)


def _source_inventory_classification(relative: Path) -> str:
    if any(_is_below(relative, root) for root in SOURCE_REBUILD_ROOTS):
        return "rebuild"
    if any(_is_below(relative, root) for root in SOURCE_ACTIVATE_ROOTS):
        return "activate"
    if relative in SOURCE_CONVERTED_FILES:
        return "converted"
    if any(_is_below(relative, root) for root in SOURCE_QUARANTINE_ROOTS):
        return "quarantine"
    return "preserve"


def _blocked_inventory_root(relative: Path) -> Path:
    parts = relative.parts
    if parts and parts[0] == ".hermes" and len(parts) > 1:
        return Path(*parts[:2])
    return Path(parts[0])


def _empty_inventory_summary() -> dict[str, int]:
    return {
        "entries": 0,
        "directories": 0,
        "regular_files": 0,
        "bytes": 0,
        "symlinks": 0,
        "special_files": 0,
    }


def _source_symlink_is_contained(
    candidate: Path, source_root: Path, link_target: str
) -> bool:
    target = Path(link_target)
    legacy_home = Path("/home/node")
    if target.is_absolute():
        try:
            relative = target.relative_to(legacy_home)
        except ValueError:
            resolved = target.resolve(strict=False)
        else:
            resolved = (source_root / relative).resolve(strict=False)
    else:
        resolved = (candidate.parent / target).resolve(strict=False)
    try:
        resolved.relative_to(source_root)
    except ValueError:
        return False
    return True


def inventory_source_volume(output: Path, source_root: Path) -> dict[str, Any]:
    """Inventory every durable directory, regular file, and symlink."""
    output = Path(output)
    source_root = Path(source_root)
    if not source_root.is_dir() or source_root.is_symlink():
        raise MigrationError(f"source root must be a real directory: {source_root}")
    if output.exists() or output.is_symlink():
        raise MigrationError(f"refusing to overwrite source inventory: {output}")
    resolved_source = source_root.resolve()
    try:
        output.parent.resolve().relative_to(resolved_source)
    except ValueError:
        pass
    else:
        raise MigrationError(
            "source inventory output must be outside the source volume"
        )

    classifications = {
        name: _empty_inventory_summary()
        for name in (
            "activate",
            "converted",
            "preserve",
            "quarantine",
            "rebuild",
            "blocked",
        )
    }
    blocked: dict[str, dict[str, int]] = {}
    entries: list[dict[str, Any]] = []
    directory_count = 0

    def on_walk_error(error: OSError) -> None:
        raise MigrationError(f"could not inventory source volume: {error}") from error

    for directory, dirnames, filenames in os.walk(
        resolved_source,
        topdown=True,
        followlinks=False,
        onerror=on_walk_error,
    ):
        directory_path = Path(directory)
        dirnames.sort()
        filenames.sort()
        real_directories = [
            directory_path / name
            for name in dirnames
            if not (directory_path / name).is_symlink()
        ]
        directory_count += len(real_directories)
        for candidate in real_directories:
            info = candidate.lstat()
            relative = candidate.relative_to(resolved_source)
            classification = _source_inventory_classification(relative)
            summary = classifications[classification]
            summary["entries"] += 1
            summary["directories"] += 1
            entries.append(
                {
                    "path": relative.as_posix(),
                    "disposition": classification,
                    "kind": "directory",
                    "size": 0,
                    "mode": stat.S_IMODE(info.st_mode),
                    "sha256": None,
                    "link_target": None,
                }
            )
        directory_symlinks = [
            directory_path / name
            for name in dirnames
            if (directory_path / name).is_symlink()
        ]
        dirnames[:] = [
            name for name in dirnames if not (directory_path / name).is_symlink()
        ]
        for candidate in directory_symlinks + [
            directory_path / name for name in filenames
        ]:
            try:
                info = candidate.lstat()
            except OSError as exc:
                raise MigrationError(
                    f"could not inspect source entry: {candidate}"
                ) from exc
            relative = candidate.relative_to(resolved_source)
            classification = _source_inventory_classification(relative)
            if not (candidate.is_symlink() or candidate.is_file()) or (
                candidate.is_symlink()
                and not _source_symlink_is_contained(
                    candidate, resolved_source, os.readlink(candidate)
                )
                and classification not in {"quarantine", "rebuild"}
            ):
                classification = "blocked"
            summary = classifications[classification]
            summary["entries"] += 1
            if candidate.is_symlink():
                summary["symlinks"] += 1
                link_target = os.readlink(candidate)
                entry_size = len(link_target.encode("utf-8"))
                entry_sha256 = hashlib.sha256(link_target.encode("utf-8")).hexdigest()
                entry_kind = "symlink"
            elif candidate.is_file():
                summary["regular_files"] += 1
                summary["bytes"] += info.st_size
                link_target = None
                entry_size = info.st_size
                try:
                    entry_sha256 = _sha256(candidate)
                except OSError as exc:
                    raise MigrationError(
                        "could not hash source entry: " + relative.as_posix()
                    ) from exc
                entry_kind = "file"
            else:
                summary["special_files"] += 1
                link_target = None
                entry_size = info.st_size
                entry_sha256 = None
                entry_kind = "special"
            entries.append(
                {
                    "path": relative.as_posix(),
                    "disposition": classification,
                    "kind": entry_kind,
                    "size": entry_size,
                    "mode": None
                    if entry_kind == "symlink"
                    else stat.S_IMODE(info.st_mode),
                    "sha256": entry_sha256,
                    "link_target": link_target,
                }
            )
            if classification == "blocked":
                root = _blocked_inventory_root(relative).as_posix()
                root_summary = blocked.setdefault(root, _empty_inventory_summary())
                root_summary["entries"] += 1
                if candidate.is_symlink():
                    root_summary["symlinks"] += 1
                elif candidate.is_file():
                    root_summary["regular_files"] += 1
                    root_summary["bytes"] += info.st_size
                else:
                    root_summary["special_files"] += 1

    blocked_roots = [
        {"path": path, **summary} for path, summary in sorted(blocked.items())
    ]
    entries.sort(key=lambda entry: entry["path"])
    result = {
        "schema": SOURCE_INVENTORY_SCHEMA,
        "source_root": str(resolved_source),
        "status": "complete" if not blocked_roots else "blocked",
        "policy": {
            "activate": [path.as_posix() for path in SOURCE_ACTIVATE_ROOTS],
            "converted": [path.as_posix() for path in SOURCE_CONVERTED_FILES],
            "quarantine": [path.as_posix() for path in SOURCE_QUARANTINE_ROOTS],
            "rebuild": [path.as_posix() for path in SOURCE_REBUILD_ROOTS],
            "default": "preserve",
        },
        "directories": directory_count,
        "classifications": classifications,
        "entries": entries,
        "blocked_roots": blocked_roots,
    }
    _write_private_json(output, result)
    return result


def inventory_source_sites(
    output: Path,
    control_plane_export: Path,
    source_inventory_path: Path,
    *,
    expected_machine_id: str,
) -> dict[str, Any]:
    """Bind authoritative legacy Sites records to preserved source paths."""
    output = Path(output)
    control_plane_export = Path(control_plane_export)
    source_inventory_path = Path(source_inventory_path)
    if output.exists() or output.is_symlink():
        raise MigrationError(f"refusing to overwrite Sites inventory: {output}")
    if not expected_machine_id.strip():
        raise MigrationError("expected Sites machine id is required")

    try:
        control_plane = json.loads(control_plane_export.read_text(encoding="utf-8"))
        source_inventory = json.loads(source_inventory_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise MigrationError(f"could not read Sites inventory input: {exc}") from exc
    if not isinstance(control_plane, dict):
        raise MigrationError("published endpoint export must be an object")
    if control_plane.get("machineId") != expected_machine_id:
        raise MigrationError("published endpoint export machine id mismatch")
    endpoints = control_plane.get("endpoints")
    if not isinstance(endpoints, list):
        raise MigrationError("published endpoint export has no endpoint list")
    if (
        not isinstance(source_inventory, dict)
        or source_inventory.get("schema") != SOURCE_INVENTORY_SCHEMA
        or source_inventory.get("status") != "complete"
    ):
        raise MigrationError("source volume inventory is incomplete or invalid")
    source_entries = source_inventory.get("entries")
    if not isinstance(source_entries, list):
        raise MigrationError("source volume inventory entries are missing")
    source_paths = {
        entry.get("path")
        for entry in source_entries
        if isinstance(entry, dict) and isinstance(entry.get("path"), str)
    }

    rendered: list[dict[str, Any]] = []
    seen_hostnames: set[str] = set()
    source_paths_required = 0
    source_paths_present = 0
    required_strings = (
        "hostname",
        "label",
        "status",
        "desired_process_state",
        "created_at",
        "updated_at",
    )
    allowed_fields = {
        "hostname",
        "label",
        "target_port",
        "status",
        "run_command",
        "run_cwd",
        "desired_process_state",
        "auth",
        "created_at",
        "updated_at",
    }
    allowed_auth_fields = {"mode", "owner_email", "emails", "org_domain"}
    for endpoint in endpoints:
        if not isinstance(endpoint, dict):
            raise MigrationError("published endpoint record must be an object")
        if set(endpoint) - allowed_fields:
            raise MigrationError("published endpoint record has unknown fields")
        for field in required_strings:
            if not isinstance(endpoint.get(field), str):
                raise MigrationError(f"published endpoint {field} must be a string")
        hostname = endpoint["hostname"].strip()
        if not hostname or hostname in seen_hostnames:
            raise MigrationError("published endpoint hostnames must be unique")
        seen_hostnames.add(hostname)
        auth = endpoint.get("auth")
        if not isinstance(auth, dict) or not isinstance(auth.get("mode"), str):
            raise MigrationError("published endpoint auth record is invalid")
        if set(auth) - allowed_auth_fields:
            raise MigrationError("published endpoint auth has unknown fields")
        target_port = endpoint.get("target_port")
        if target_port is not None and (
            not isinstance(target_port, int)
            or isinstance(target_port, bool)
            or not 1 <= target_port <= 65535
        ):
            raise MigrationError("published endpoint target_port is invalid")
        run_command = endpoint.get("run_command")
        run_cwd = endpoint.get("run_cwd")
        if run_command is not None and not isinstance(run_command, str):
            raise MigrationError("published endpoint run_command is invalid")
        if run_cwd is not None and not isinstance(run_cwd, str):
            raise MigrationError("published endpoint run_cwd is invalid")
        if bool(run_command) != bool(run_cwd):
            raise MigrationError(
                "published endpoint run_command and run_cwd must appear together"
            )

        source = {"relative_path": None, "status": "not-required"}
        if run_cwd:
            source_paths_required += 1
            source_home = Path("/home/node")
            cwd = Path(run_cwd)
            try:
                relative = cwd.relative_to(source_home)
            except ValueError as exc:
                raise MigrationError(
                    f"published site source is outside /home/node: {hostname}"
                ) from exc
            relative_path = relative.as_posix()
            present = relative_path == "." or relative_path in source_paths
            if not present:
                raise MigrationError(
                    f"published site source is missing from snapshot: {hostname}"
                )
            source_paths_present += 1
            source = {
                "relative_path": relative_path,
                "status": "present-in-source-snapshot",
            }
        rendered.append({**endpoint, "source": source})

    rendered.sort(key=lambda endpoint: endpoint["hostname"])
    result = {
        "schema": SITES_INVENTORY_SCHEMA,
        "machine_id": expected_machine_id,
        "status": "complete",
        "endpoint_count": len(rendered),
        "source_paths_required": source_paths_required,
        "source_paths_present": source_paths_present,
        "activation_policy": "manifest-only-not-republished",
        "endpoints": rendered,
    }
    _write_private_json(output, result)
    return result


def _configured_env_names(env_path: Path) -> list[str]:
    if not env_path.exists():
        return []
    if not env_path.is_file() or env_path.is_symlink():
        raise MigrationError("legacy Hermes .env must be a regular file")
    names: set[str] = set()
    try:
        lines = env_path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeDecodeError) as exc:
        raise MigrationError(f"could not read legacy Hermes .env: {exc}") from exc
    for line_number, line in enumerate(lines, start=1):
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
        normalized_value = value.strip()
        if (
            len(normalized_value) >= 2
            and normalized_value[0] == normalized_value[-1]
            and normalized_value[0] in {'"', "'"}
        ):
            normalized_value = normalized_value[1:-1]
        if normalized_value:
            names.add(name)
    return sorted(names)


def _integration_for_env_name(name: str, platform_names: set[str]) -> str:
    if name.startswith("TELEGRAM_"):
        return "telegram"
    if name.startswith("SIGNAL_"):
        return "signal"
    if name.startswith(("GOOGLE_WORKSPACE_", "GMAIL_")):
        return "google-workspace"
    if name.startswith(("FINITE_BRAIN_", "FBRAIN_")):
        return "finitebrain"
    if name.startswith(_MODEL_PROVIDER_PREFIXES):
        return "model-provider-credentials"
    for platform in platform_names:
        prefix = platform.upper().replace("-", "_") + "_"
        if name.startswith(prefix):
            return platform.replace("_", "-")
    return "other-environment-config"


def _configured_yaml_platforms(path: Path) -> dict[str, bool | None]:
    """Read only the simple platforms mapping emitted by Hermes v0.14."""
    if not path.exists():
        return {}
    if not path.is_file() or path.is_symlink():
        raise MigrationError("legacy Hermes config.yaml must be a regular file")
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeDecodeError) as exc:
        raise MigrationError(
            f"could not read legacy Hermes config.yaml: {exc}"
        ) from exc
    if any("\t" in line[: len(line) - len(line.lstrip())] for line in lines):
        raise MigrationError(
            "legacy Hermes config.yaml uses unsupported tab indentation"
        )
    platforms: dict[str, bool | None] = {}
    in_platforms = False
    current: str | None = None
    for line in lines:
        content = line.split("#", 1)[0].rstrip()
        if not content:
            continue
        indent = len(content) - len(content.lstrip(" "))
        stripped = content.strip()
        if indent == 0:
            in_platforms = stripped == "platforms:"
            current = None
            continue
        if not in_platforms:
            continue
        if indent == 2:
            match = re.fullmatch(r"([A-Za-z0-9_-]+):", stripped)
            if match is None:
                raise MigrationError(
                    "legacy Hermes platforms config uses an unsupported shape"
                )
            current = match.group(1).lower()
            if current in platforms:
                raise MigrationError(f"duplicate legacy Hermes platform: {current}")
            platforms[current] = None
            continue
        if indent >= 4 and current is not None and stripped.startswith("enabled:"):
            raw_enabled = stripped.split(":", 1)[1].strip().lower()
            if raw_enabled not in {"true", "false"}:
                raise MigrationError(
                    f"legacy Hermes platform enabled flag is invalid: {current}"
                )
            platforms[current] = raw_enabled == "true"
    return platforms


def inventory_source_integrations(
    output: Path,
    source_root: Path,
    source_inventory_path: Path,
) -> dict[str, Any]:
    """Inventory external connections without serializing credential values."""
    output = Path(output)
    source_root = Path(source_root)
    source_inventory_path = Path(source_inventory_path)
    if output.exists() or output.is_symlink():
        raise MigrationError(f"refusing to overwrite integrations inventory: {output}")
    if not source_root.is_dir() or source_root.is_symlink():
        raise MigrationError(f"source root must be a real directory: {source_root}")
    try:
        source_inventory = json.loads(source_inventory_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise MigrationError(f"could not read source volume inventory: {exc}") from exc
    if (
        not isinstance(source_inventory, dict)
        or source_inventory.get("schema") != SOURCE_INVENTORY_SCHEMA
        or source_inventory.get("status") != "complete"
    ):
        raise MigrationError("source volume inventory is incomplete or invalid")
    if source_inventory.get("source_root") != str(source_root.resolve()):
        raise MigrationError("source root does not match source volume inventory")
    source_entries = source_inventory.get("entries")
    if not isinstance(source_entries, list):
        raise MigrationError("source volume inventory entries are missing")
    source_paths = {
        entry.get("path")
        for entry in source_entries
        if isinstance(entry, dict) and isinstance(entry.get("path"), str)
    }

    config_path = source_root / ".hermes/config.yaml"
    platform_enabled = _configured_yaml_platforms(config_path)
    platform_names = set(platform_enabled)

    evidence: dict[str, dict[str, Any]] = {}

    def record(
        name: str,
        *,
        configured_key: str | None = None,
        evidence_path: str | None = None,
        source_enabled: bool | None = None,
    ) -> None:
        item = evidence.setdefault(
            name,
            {
                "name": name,
                "configured_keys": set(),
                "evidence_paths": set(),
                "source_enabled": None,
            },
        )
        if configured_key is not None:
            item["configured_keys"].add(configured_key)
        if evidence_path is not None:
            item["evidence_paths"].add(evidence_path)
        if source_enabled is not None:
            item["source_enabled"] = source_enabled

    env_path = source_root / ".hermes/.env"
    for env_name in _configured_env_names(env_path):
        name = _integration_for_env_name(env_name, platform_names)
        record(name, configured_key=env_name, evidence_path=".hermes/.env")
    for platform in sorted(platform_names):
        name = platform.replace("_", "-")
        record(
            name,
            evidence_path=f".hermes/config.yaml#platforms.{platform}",
            source_enabled=platform_enabled[platform],
        )
    if ".hermes/google_token.json" in source_paths or any(
        path == ".hermes/gws" or path.startswith(".hermes/gws/")
        for path in source_paths
    ):
        if ".hermes/google_token.json" in source_paths:
            record("google-workspace", evidence_path=".hermes/google_token.json")
        if any(
            path == ".hermes/gws" or path.startswith(".hermes/gws/")
            for path in source_paths
        ):
            record("google-workspace", evidence_path=".hermes/gws")
    if any(path == ".brain" or path.startswith(".brain/") for path in source_paths):
        record("finitebrain", evidence_path=".brain")

    integrations = []
    for name, item in sorted(evidence.items()):
        policy = _INTEGRATION_POLICIES.get(
            name, "preserve-disabled-until-supported-setup"
        )
        integrations.append(
            {
                "name": name,
                "configured_keys": sorted(item["configured_keys"]),
                "evidence_paths": sorted(item["evidence_paths"]),
                "source_enabled": item["source_enabled"],
                "migration_policy": policy,
                "target_state": "inactive",
            }
        )
    result = {
        "schema": INTEGRATIONS_INVENTORY_SCHEMA,
        "status": "complete",
        "activation_policy": "inventory-only-no-secret-values-or-activation",
        "integration_count": len(integrations),
        "integrations": integrations,
    }
    _write_private_json(output, result)
    return result


def check_source_writers(
    source_root: Path, proc_root: Path = Path("/proc")
) -> dict[str, Any]:
    """Fail if Linux procfs exposes writable access below the frozen PVC."""
    source_root = Path(source_root)
    proc_root = Path(proc_root)
    if not source_root.is_dir() or source_root.is_symlink():
        raise MigrationError(f"source root must be a real directory: {source_root}")
    if not proc_root.is_dir() or proc_root.is_symlink():
        raise MigrationError(f"proc root must be a real directory: {proc_root}")
    if proc_root == Path("/proc") and os.geteuid() != 0:
        raise MigrationError("source writer check against /proc must run as root")

    resolved_source = source_root.resolve()
    writable_references: list[str] = []
    for process in sorted(proc_root.iterdir(), key=lambda path: path.name):
        if not process.name.isdigit():
            continue
        fd_root = process / "fd"
        fdinfo_root = process / "fdinfo"
        if fd_root.is_dir():
            try:
                descriptors = list(fd_root.iterdir())
            except FileNotFoundError:
                descriptors = []
            except PermissionError as exc:
                raise MigrationError(
                    f"could not inspect process {process.name} file descriptors"
                ) from exc
            for descriptor in descriptors:
                try:
                    flags_line = next(
                        line
                        for line in (fdinfo_root / descriptor.name)
                        .read_text(encoding="utf-8")
                        .splitlines()
                        if line.startswith("flags:")
                    )
                    flags = int(flags_line.split(":", 1)[1].strip(), 8)
                    if (flags & os.O_ACCMODE) not in (os.O_WRONLY, os.O_RDWR):
                        continue
                    raw_target = os.readlink(descriptor).removesuffix(" (deleted)")
                    if not raw_target.startswith("/"):
                        continue
                    target = Path(raw_target).resolve(strict=False)
                except FileNotFoundError:
                    continue
                except PermissionError as exc:
                    raise MigrationError(
                        f"could not inspect process {process.name} descriptor "
                        f"{descriptor.name}"
                    ) from exc
                except (StopIteration, ValueError) as exc:
                    raise MigrationError(
                        f"invalid fd metadata for process {process.name} "
                        f"descriptor {descriptor.name}"
                    ) from exc
                try:
                    relative = target.relative_to(resolved_source)
                except ValueError:
                    continue
                writable_references.append(
                    f"writable file descriptor pid={process.name} "
                    f"fd={descriptor.name} path={relative.as_posix()}"
                )

        maps_path = process / "maps"
        try:
            mappings = maps_path.read_text(encoding="utf-8").splitlines()
        except FileNotFoundError:
            mappings = []
        except PermissionError as exc:
            raise MigrationError(
                f"could not inspect process {process.name} memory maps"
            ) from exc
        for mapping in mappings:
            fields = mapping.split(maxsplit=5)
            if len(fields) < 6 or "w" not in fields[1]:
                continue
            raw_target = fields[5].removesuffix(" (deleted)")
            if not raw_target.startswith("/"):
                continue
            target = Path(raw_target).resolve(strict=False)
            try:
                relative = target.relative_to(resolved_source)
            except ValueError:
                continue
            writable_references.append(
                f"writable memory map pid={process.name} path={relative.as_posix()}"
            )

    if writable_references:
        rendered = "; ".join(writable_references[:20])
        raise MigrationError(f"source PVC still has writable access: {rendered}")
    return {
        "source_root": str(resolved_source),
        "status": "no-writable-fds-or-memory-maps",
        "writable_references": 0,
    }

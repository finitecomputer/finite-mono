#!/usr/bin/env python3
"""Sealed bundle and compatibility contract for legacy Hermes migrations."""

from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import sqlite3
import stat
import tarfile
import tempfile
from contextlib import closing
from dataclasses import asdict, dataclass
from pathlib import Path, PurePosixPath
from typing import Any

SCHEMA = "finite.legacy-hermes-migration.v2"
SOURCE_INVENTORY_SCHEMA = "finite.legacy-hermes-source-inventory.v2"
SITES_INVENTORY_SCHEMA = "finite.legacy-hermes-sites.v1"
INTEGRATIONS_INVENTORY_SCHEMA = "finite.legacy-hermes-integrations.v1"
SUPPORTED_SOURCE_HERMES_VERSION = "0.14.0"
SUPPORTED_TARGET_HERMES_VERSION = "0.20.0"
SOURCE_EXPORT_BATCH_SIZE = 1_000
RECEIPT_RELATIVE_PATH = Path("migration/legacy-hermes-v2/receipt.json")
SOURCE_SNAPSHOT_RELATIVE_PATH = PurePosixPath("source-home.tar")
SOURCE_SNAPSHOT_TARGET = PurePosixPath(
    "migration/legacy-hermes-v2/preserved/source-home.tar"
)
PROTECTED_RELATIVE_PATHS = (
    Path("agent/identity/identity.json"),
    Path("agent/client.sqlite3"),
)
ALLOWED_PAYLOAD_ROOTS = (
    PurePosixPath("hermes/memories"),
    PurePosixPath("hermes/skills"),
    PurePosixPath("hermes/scripts"),
    PurePosixPath("home/workspace"),
    PurePosixPath("home/dev"),
    PurePosixPath("home/uploads"),
)
SOURCE_ACTIVE_PAYLOAD_MAPPINGS = (
    (PurePosixPath(".hermes/memories"), PurePosixPath("hermes/memories")),
    (PurePosixPath(".hermes/skills"), PurePosixPath("hermes/skills")),
    (PurePosixPath(".hermes/scripts"), PurePosixPath("hermes/scripts")),
    (PurePosixPath(".hermes/cron/jobs.json"), PurePosixPath("hermes/cron/jobs.json")),
    (PurePosixPath("workspace"), PurePosixPath("home/workspace")),
    (PurePosixPath("dev"), PurePosixPath("home/dev")),
    (PurePosixPath("uploads"), PurePosixPath("home/uploads")),
)
PRESERVED_INERT = {
    "default": "every non-activated source entry remains in source-home.tar",
    "quarantined": [
        "Hermes .env, auth.json, Google/OAuth tokens, and provider credentials",
        "Hermes config and gateway/platform routing state",
        "cron execution output and runtime process state",
        "legacy .finite and platform client state",
        "the legacy local Finite Brain working tree and identity",
    ],
    "rebuilt_not_activated": [
        "Hermes-managed venvs, binaries, package/tool caches, and generated state",
    ],
    "preserved_not_activated": [
        "Hermes logs, raw session/auxiliary SQLite files, and cache-backed media",
    ],
}
LEGACY_HERMES_MEDIA_CACHE_ROOTS = (
    "/home/node/.hermes/audio_cache/",
    "/home/node/.hermes/image_cache/",
    "~/.hermes/audio_cache/",
    "~/.hermes/image_cache/",
)
INTEGRATION_MIGRATION_POLICIES = frozenset(
    {
        "controlled-transfer-after-rehearsal",
        "fresh-authorization-required",
        "target-managed-not-copied",
        "preserve-disabled-until-supported-setup",
    }
)


class MigrationError(RuntimeError):
    """A fail-closed migration contract violation."""


def payload_path_for_source(relative: PurePosixPath) -> PurePosixPath | None:
    for source_root, payload_root in SOURCE_ACTIVE_PAYLOAD_MAPPINGS:
        if relative == source_root:
            return payload_root
        if relative.is_relative_to(source_root):
            return payload_root / relative.relative_to(source_root)
    return None


def source_path_for_payload(relative: PurePosixPath) -> PurePosixPath | None:
    for source_root, payload_root in SOURCE_ACTIVE_PAYLOAD_MAPPINGS:
        if relative == payload_root:
            return source_root
        if relative.is_relative_to(payload_root):
            return source_root / relative.relative_to(payload_root)
    return None


def _validate_active_payload_records(
    records: list[dict[str, Any]], inventory: dict[str, Any]
) -> None:
    source_entries = {
        entry["path"]: entry
        for entry in inventory["entries"]
        if isinstance(entry, dict) and isinstance(entry.get("path"), str)
    }
    for record in records:
        payload_relative = PurePosixPath(record["path"])
        source_relative = source_path_for_payload(payload_relative)
        if source_relative is None:
            continue
        expected = source_entries.get(source_relative.as_posix())
        if expected is None or expected.get("disposition") != "activate":
            raise MigrationError("active payload does not match source inventory")
        comparable = {
            "kind": expected.get("kind"),
            "size": expected.get("size"),
            "mode": expected.get("mode"),
            "sha256": expected.get("sha256"),
            "link_target": expected.get("link_target"),
        }
        if comparable != {
            "kind": record.get("kind"),
            "size": record.get("size"),
            "mode": record.get("mode"),
            "sha256": record.get("sha256"),
            "link_target": record.get("link_target"),
        }:
            raise MigrationError("active payload does not match source inventory")


@dataclass(frozen=True)
class SourceMetadata:
    host_id: str
    machine_id: str
    owner_email: str
    hermes_version: str
    image_reference: str
    image_manifest_digest: str
    container_image_id: str
    source_inventory_sha256: str

    def validate(self) -> None:
        for field, value in asdict(self).items():
            if not isinstance(value, str) or not value.strip():
                raise MigrationError(f"source {field} is required")
        for field in ("image_manifest_digest", "container_image_id"):
            digest = getattr(self, field)
            if not digest.startswith("sha256:") or len(digest) != 71:
                raise MigrationError(
                    f"source {field} must be sha256:<64 hex characters>"
                )
            try:
                int(digest.removeprefix("sha256:"), 16)
            except ValueError as exc:
                raise MigrationError(f"source {field} is not hexadecimal") from exc
        _validate_sha256(self.source_inventory_sha256, "source_inventory_sha256")
        if self.hermes_version != SUPPORTED_SOURCE_HERMES_VERSION:
            raise MigrationError(
                "source Hermes version must be "
                f"{SUPPORTED_SOURCE_HERMES_VERSION}, got {self.hermes_version}"
            )


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _validate_sha256(value: str, label: str) -> None:
    if len(value) != 64:
        raise MigrationError(f"{label} must be 64 hexadecimal characters")
    try:
        int(value, 16)
    except ValueError as exc:
        raise MigrationError(f"{label} must be 64 hexadecimal characters") from exc


def _canonical_json_bytes(value: Any) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")


def _fsync_directory(path: Path) -> None:
    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0)
    descriptor = os.open(path, flags)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def _write_private_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, raw_temp = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temp = Path(raw_temp)
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "wb") as handle:
            handle.write(_canonical_json_bytes(value))
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temp, path)
        _fsync_directory(path.parent)
    except BaseException:
        temp.unlink(missing_ok=True)
        raise


def _allowed_payload_file(relative: PurePosixPath) -> bool:
    if relative in (
        PurePosixPath("sessions.jsonl"),
        PurePosixPath("memory_store.db"),
        PurePosixPath("hermes/cron/jobs.json"),
        SOURCE_SNAPSHOT_RELATIVE_PATH,
    ):
        return True
    return any(relative.is_relative_to(root) for root in ALLOWED_PAYLOAD_ROOTS)


def _symlink_root(relative: PurePosixPath) -> PurePosixPath | None:
    for root in ALLOWED_PAYLOAD_ROOTS:
        if relative.is_relative_to(root):
            return root
    return None


def _validate_payload_symlink(payload: Path, candidate: Path) -> None:
    relative = PurePosixPath(candidate.relative_to(payload).as_posix())
    root_relative = _symlink_root(relative)
    if root_relative not in (
        PurePosixPath("home/workspace"),
        PurePosixPath("home/dev"),
        PurePosixPath("home/uploads"),
    ):
        raise MigrationError(
            f"payload symlink is forbidden outside workspace roots: {relative}"
        )
    link_target = os.readlink(candidate)
    root = (payload / Path(root_relative.as_posix())).resolve()
    unresolved = Path(link_target)
    resolved = (
        unresolved.resolve(strict=False)
        if unresolved.is_absolute()
        else (candidate.parent / unresolved).resolve(strict=False)
    )
    try:
        resolved.relative_to(root)
    except ValueError as exc:
        raise MigrationError(
            f"payload symlink escapes its allowed root: {relative}"
        ) from exc


def _walk_payload_entries(payload: Path) -> list[Path]:
    if not payload.is_dir() or payload.is_symlink():
        raise MigrationError(f"payload must be a real directory: {payload}")
    files: list[Path] = []
    for directory, dirnames, filenames in os.walk(payload, followlinks=False):
        directory_path = Path(directory)
        for name in dirnames:
            candidate = directory_path / name
            if candidate.is_symlink():
                _validate_payload_symlink(payload, candidate)
                files.append(candidate)
        dirnames[:] = [
            name for name in dirnames if not (directory_path / name).is_symlink()
        ]
        for name in filenames:
            candidate = directory_path / name
            relative = PurePosixPath(candidate.relative_to(payload).as_posix())
            if not _allowed_payload_file(relative):
                raise MigrationError(f"payload path is not allowed: {relative}")
            info = candidate.lstat()
            if stat.S_ISLNK(info.st_mode):
                _validate_payload_symlink(payload, candidate)
            elif not stat.S_ISREG(info.st_mode):
                raise MigrationError(f"payload special file is forbidden: {relative}")
            files.append(candidate)
    return sorted(files, key=lambda item: item.relative_to(payload).as_posix())


def _session_index(path: Path) -> tuple[dict[str, tuple[int, int, str | None]], int]:
    if not path.is_file() or path.is_symlink():
        raise MigrationError("payload/sessions.jsonl must be a regular file")
    index: dict[str, tuple[int, int, str | None]] = {}
    message_count = 0
    with path.open("rb") as handle:
        while True:
            offset = handle.tell()
            line = handle.readline()
            if not line:
                break
            if not line.strip():
                raise MigrationError(
                    f"sessions.jsonl contains a blank line at byte {offset}"
                )
            try:
                session = json.loads(line)
            except (UnicodeDecodeError, json.JSONDecodeError) as exc:
                raise MigrationError(
                    f"invalid session JSON at byte {offset}: {exc}"
                ) from exc
            if not isinstance(session, dict):
                raise MigrationError(f"session at byte {offset} is not an object")
            session_id = session.get("id")
            if not isinstance(session_id, str) or not session_id.strip():
                raise MigrationError(f"session at byte {offset} has no string id")
            if session_id in index:
                raise MigrationError(f"duplicate session id: {session_id}")
            parent = session.get("parent_session_id")
            if parent is not None and not isinstance(parent, str):
                raise MigrationError(
                    f"session {session_id} has a non-string parent_session_id"
                )
            messages = session.get("messages")
            if not isinstance(messages, list):
                raise MigrationError(f"session {session_id} messages must be a list")
            if any(not isinstance(message, dict) for message in messages):
                raise MigrationError(
                    f"session {session_id} contains a non-object message"
                )
            index[session_id] = (offset, len(line), parent)
            message_count += len(messages)
    if not index:
        raise MigrationError("sessions.jsonl contains no sessions")
    return index, message_count


def _topological_session_ids(
    index: dict[str, tuple[int, int, str | None]],
) -> list[str]:
    ordered: list[str] = []
    marks: dict[str, int] = {}

    def visit(session_id: str) -> None:
        mark = marks.get(session_id, 0)
        if mark == 2:
            return
        if mark == 1:
            raise MigrationError(f"session parent cycle includes {session_id}")
        marks[session_id] = 1
        parent = index[session_id][2]
        if parent in index:
            visit(parent)
        marks[session_id] = 2
        ordered.append(session_id)

    for session_id in index:
        visit(session_id)
    return ordered


def _is_path_field(field: str) -> bool:
    return field == "path" or field.endswith("_path")


def _is_legacy_cache_media_path(value: str) -> bool:
    return value.startswith(LEGACY_HERMES_MEDIA_CACHE_ROOTS)


def _rewrite_message_paths(value: Any) -> tuple[Any, dict[str, int]]:
    counts = {
        "rewritable_count": 0,
        "cache_media_preserved_count": 0,
        "unmapped_source_path_count": 0,
    }

    def rewrite_media_tag(match: re.Match[str]) -> str:
        prefix, path = match.groups()
        rewritten = _rewrite_source_path(path)
        if rewritten != path:
            counts["rewritable_count"] += 1
            return prefix + rewritten
        if _is_legacy_cache_media_path(path):
            counts["cache_media_preserved_count"] += 1
        elif path.startswith(("/home/node/", "~/")):
            counts["unmapped_source_path_count"] += 1
        return match.group(0)

    def visit(candidate: Any, field: str | None = None) -> Any:
        if isinstance(candidate, dict):
            return {key: visit(item, key) for key, item in candidate.items()}
        if isinstance(candidate, list):
            return [visit(item, field) for item in candidate]
        if not isinstance(candidate, str) or field is None:
            return candidate
        if field == "content":
            return re.sub(r"(MEDIA:\s*)(\S+)", rewrite_media_tag, candidate)
        if not _is_path_field(field):
            return candidate
        rewritten = _rewrite_source_path(candidate)
        if rewritten != candidate:
            counts["rewritable_count"] += 1
            return rewritten
        if _is_legacy_cache_media_path(candidate):
            counts["cache_media_preserved_count"] += 1
        elif candidate.startswith(("/home/node/", "~/")):
            counts["unmapped_source_path_count"] += 1
        return candidate

    return visit(value), counts


def _session_path_summary(path: Path) -> dict[str, Any]:
    rewritable_count = 0
    cache_media_preserved_count = 0
    unmapped_source_path_count = 0
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            session = json.loads(line)
            _, counts = _rewrite_message_paths(session.get("messages") or [])
            rewritable_count += counts["rewritable_count"]
            cache_media_preserved_count += counts["cache_media_preserved_count"]
            unmapped_source_path_count += counts["unmapped_source_path_count"]
    return {
        "rewritable_count": rewritable_count,
        "cache_media_preserved_count": cache_media_preserved_count,
        "unmapped_source_path_count": unmapped_source_path_count,
        "preservation_policy": "retained-in-sealed-source-home",
    }


def _target_for_payload(relative: PurePosixPath) -> PurePosixPath | None:
    if relative in (
        PurePosixPath("sessions.jsonl"),
        PurePosixPath("memory_store.db"),
    ):
        return None
    if relative == SOURCE_SNAPSHOT_RELATIVE_PATH:
        return SOURCE_SNAPSHOT_TARGET
    mappings = (
        (PurePosixPath("hermes/memories"), PurePosixPath("agent/hermes-home/memories")),
        (
            PurePosixPath("hermes/skills"),
            PurePosixPath("migration/legacy-hermes-v2/review-only/skills"),
        ),
        (
            PurePosixPath("hermes/scripts"),
            PurePosixPath("migration/legacy-hermes-v2/review-only/scripts"),
        ),
        (
            PurePosixPath("home/workspace"),
            PurePosixPath("workspace/legacy-box1/workspace"),
        ),
        (PurePosixPath("home/dev"), PurePosixPath("workspace/legacy-box1/dev")),
        (PurePosixPath("home/uploads"), PurePosixPath("workspace/legacy-box1/uploads")),
    )
    for source_root, target_root in mappings:
        if relative.is_relative_to(source_root):
            return target_root / relative.relative_to(source_root)
    if relative == PurePosixPath("hermes/cron/jobs.json"):
        return PurePosixPath("migration/legacy-hermes-v2/review-only/cron/jobs.json")
    raise MigrationError(f"payload path has no target mapping: {relative}")


def _cron_summary(payload: Path) -> dict[str, Any]:
    jobs_path = payload / "hermes/cron/jobs.json"
    if not jobs_path.exists():
        return {
            "count": 0,
            "source_enabled_count": 0,
            "target_state": "not-present",
        }
    if not jobs_path.is_file() or jobs_path.is_symlink():
        raise MigrationError("payload/hermes/cron/jobs.json must be a regular file")
    try:
        document = json.loads(jobs_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise MigrationError(f"invalid legacy cron jobs.json: {exc}") from exc
    jobs = document.get("jobs") if isinstance(document, dict) else document
    if not isinstance(jobs, list) or any(not isinstance(job, dict) for job in jobs):
        raise MigrationError("legacy cron jobs.json must contain a list of job objects")
    return {
        "count": len(jobs),
        "source_enabled_count": sum(
            job.get("enabled", True) is not False for job in jobs
        ),
        "target_state": "review-only-not-active",
    }


def _memory_database_fact_count(database_path: Path) -> int:
    if not database_path.is_file() or database_path.is_symlink():
        raise MigrationError(f"memory database must be a regular file: {database_path}")
    try:
        with closing(
            sqlite3.connect(f"file:{database_path}?mode=ro", uri=True)
        ) as connection:
            quick_check = connection.execute("PRAGMA quick_check").fetchone()
            if quick_check is None or quick_check[0] != "ok":
                raise MigrationError(
                    "payload/memory_store.db failed SQLite quick_check"
                )
            facts_table = connection.execute(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'facts'"
            ).fetchone()
            if facts_table is None:
                raise MigrationError(
                    f"memory database has no facts table: {database_path}"
                )
            return int(connection.execute("SELECT COUNT(*) FROM facts").fetchone()[0])
    except sqlite3.Error as exc:
        raise MigrationError(f"invalid memory database {database_path}: {exc}") from exc


def _fresh_target_memory_fact_count(database_path: Path) -> int:
    if not database_path.exists():
        return 0
    if not database_path.is_file() or database_path.is_symlink():
        raise MigrationError(f"target memory database is unsafe: {database_path}")
    try:
        with closing(
            sqlite3.connect(f"file:{database_path}?mode=ro", uri=True)
        ) as connection:
            facts_table = connection.execute(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'facts'"
            ).fetchone()
            if facts_table is None:
                return 0
            return int(connection.execute("SELECT COUNT(*) FROM facts").fetchone()[0])
    except sqlite3.Error as exc:
        raise MigrationError(f"invalid target memory database: {exc}") from exc


def _memory_summary(payload: Path) -> dict[str, Any]:
    database_path = payload / "memory_store.db"
    if not database_path.exists():
        return {"fact_count": 0, "target_state": "not-present"}
    return {
        "fact_count": _memory_database_fact_count(database_path),
        "target_state": "rebuilt-by-target-hermes",
    }


def _read_source_inventory(bundle: Path) -> dict[str, Any]:
    inventory_path = bundle / "source-volume-inventory.json"
    if not inventory_path.is_file() or inventory_path.is_symlink():
        raise MigrationError(
            "bundle source-volume-inventory.json is missing or not a regular file"
        )
    if stat.S_IMODE(inventory_path.stat().st_mode) != 0o600:
        raise MigrationError("source volume inventory must be mode 0600")
    try:
        inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise MigrationError(f"could not read source volume inventory: {exc}") from exc
    if (
        not isinstance(inventory, dict)
        or inventory.get("schema") != SOURCE_INVENTORY_SCHEMA
    ):
        raise MigrationError(
            f"source volume inventory schema must be {SOURCE_INVENTORY_SCHEMA}"
        )
    classifications = inventory.get("classifications")
    if not isinstance(classifications, dict):
        raise MigrationError("source volume inventory classifications are missing")
    blocked = classifications.get("blocked")
    if (
        inventory.get("status") != "complete"
        or not isinstance(blocked, dict)
        or blocked.get("entries") != 0
        or inventory.get("blocked_roots") != []
    ):
        raise MigrationError("source volume inventory has structurally blocked entries")
    entries = inventory.get("entries")
    if not isinstance(entries, list) or any(
        not isinstance(entry, dict) for entry in entries
    ):
        raise MigrationError("source volume inventory entries are missing")
    paths: list[str] = []
    for entry in entries:
        path = entry.get("path")
        kind = entry.get("kind")
        mode = entry.get("mode")
        size = entry.get("size")
        sha256 = entry.get("sha256")
        link_target = entry.get("link_target")
        if not isinstance(path, str):
            raise MigrationError("source volume inventory entry metadata is invalid")
        relative = PurePosixPath(path)
        if (
            not path
            or relative.is_absolute()
            or relative.as_posix() != path
            or any(part in ("", ".", "..") for part in relative.parts)
        ):
            raise MigrationError("source volume inventory entry metadata is invalid")
        if kind == "directory":
            metadata_is_valid = (
                type(size) is int
                and size == 0
                and type(mode) is int
                and 0 <= mode <= 0o7777
                and sha256 is None
                and link_target is None
            )
        elif kind == "file":
            metadata_is_valid = (
                type(size) is int
                and size >= 0
                and type(mode) is int
                and 0 <= mode <= 0o7777
                and isinstance(sha256, str)
                and re.fullmatch(r"[0-9a-f]{64}", sha256) is not None
                and link_target is None
            )
        elif kind == "symlink":
            metadata_is_valid = (
                type(size) is int
                and size >= 0
                and mode is None
                and isinstance(sha256, str)
                and re.fullmatch(r"[0-9a-f]{64}", sha256) is not None
                and isinstance(link_target, str)
                and size == len(link_target.encode("utf-8"))
            )
        else:
            metadata_is_valid = False
        if not metadata_is_valid:
            raise MigrationError("source volume inventory entry metadata is invalid")
        paths.append(path)
    if paths != sorted(paths) or len(paths) != len(set(paths)):
        raise MigrationError("source volume inventory paths are not unique and sorted")
    dispositions = (
        "activate",
        "converted",
        "preserve",
        "quarantine",
        "rebuild",
        "blocked",
    )
    recomputed = {
        disposition: {
            "entries": 0,
            "directories": 0,
            "regular_files": 0,
            "bytes": 0,
            "symlinks": 0,
            "special_files": 0,
        }
        for disposition in dispositions
    }
    for entry in entries:
        disposition = entry.get("disposition")
        kind = entry.get("kind")
        if disposition not in recomputed:
            raise MigrationError("source volume inventory entry is invalid")
        summary = recomputed[disposition]
        summary["entries"] += 1
        if kind == "directory":
            summary["directories"] += 1
        elif kind == "file":
            summary["regular_files"] += 1
            size = entry.get("size")
            if type(size) is not int or size < 0:
                raise MigrationError("source volume inventory file size is invalid")
            summary["bytes"] += size
        else:
            summary["symlinks"] += 1
    if classifications != recomputed:
        raise MigrationError(
            "source volume inventory classification summary does not match entries"
        )
    if inventory.get("directories") != sum(
        summary["directories"] for summary in recomputed.values()
    ):
        raise MigrationError("source volume inventory directory count mismatch")
    policy = inventory.get("policy")
    if not isinstance(policy, dict) or policy.get("default") != "preserve":
        raise MigrationError("source volume inventory default policy must preserve")
    return inventory


def _source_inventory_summary(bundle: Path) -> dict[str, Any]:
    inventory_path = bundle / "source-volume-inventory.json"
    inventory = _read_source_inventory(bundle)
    return {
        "schema": SOURCE_INVENTORY_SCHEMA,
        "sha256": _sha256(inventory_path),
        "classifications": inventory["classifications"],
        "entry_count": len(inventory["entries"]),
        "directory_count": inventory.get("directories"),
    }


def _normalized_tar_member_path(name: str) -> str | None:
    while name.startswith("./"):
        name = name[2:]
    if name in ("", "."):
        return None
    relative = PurePosixPath(name)
    if relative.is_absolute() or any(
        part in ("", ".", "..") for part in relative.parts
    ):
        raise MigrationError(f"source-home snapshot has unsafe path: {name}")
    return relative.as_posix()


def _tar_stream_sha256(handle: Any) -> str:
    digest = hashlib.sha256()
    for chunk in iter(lambda: handle.read(1024 * 1024), b""):
        digest.update(chunk)
    return digest.hexdigest()


def _source_snapshot_summary(bundle: Path, inventory: dict[str, Any]) -> dict[str, Any]:
    archive_path = bundle / "payload" / Path(SOURCE_SNAPSHOT_RELATIVE_PATH.as_posix())
    if not archive_path.is_file() or archive_path.is_symlink():
        raise MigrationError("payload/source-home.tar must be a regular file")
    if stat.S_IMODE(archive_path.stat().st_mode) != 0o600:
        raise MigrationError("source-home snapshot must be mode 0600")
    expected = {entry["path"]: entry for entry in inventory["entries"]}
    observed: dict[str, dict[str, Any]] = {}
    try:
        with tarfile.open(archive_path, mode="r:") as archive:
            for member in archive:
                path = _normalized_tar_member_path(member.name)
                if path is None:
                    continue
                if path in observed:
                    raise MigrationError(
                        f"source-home snapshot contains duplicate path: {path}"
                    )
                if member.isdir():
                    record = {
                        "path": path,
                        "kind": "directory",
                        "size": 0,
                        "mode": member.mode,
                        "sha256": None,
                        "link_target": None,
                    }
                elif member.isfile():
                    extracted = archive.extractfile(member)
                    if extracted is None:
                        raise MigrationError(
                            f"source-home snapshot file cannot be read: {path}"
                        )
                    record = {
                        "path": path,
                        "kind": "file",
                        "size": member.size,
                        "mode": member.mode,
                        "sha256": _tar_stream_sha256(extracted),
                        "link_target": None,
                    }
                elif member.issym():
                    record = {
                        "path": path,
                        "kind": "symlink",
                        "size": len(member.linkname.encode("utf-8")),
                        "mode": None,
                        "sha256": hashlib.sha256(
                            member.linkname.encode("utf-8")
                        ).hexdigest(),
                        "link_target": member.linkname,
                    }
                else:
                    raise MigrationError(
                        f"source-home snapshot contains unsupported entry: {path}"
                    )
                observed[path] = record
    except (tarfile.TarError, OSError) as exc:
        raise MigrationError(f"could not verify source-home snapshot: {exc}") from exc

    comparable_expected = {
        path: {
            "path": path,
            "kind": entry.get("kind"),
            "size": entry.get("size"),
            "mode": entry.get("mode"),
            "sha256": entry.get("sha256"),
            "link_target": entry.get("link_target"),
        }
        for path, entry in expected.items()
    }
    if observed != comparable_expected:
        missing = sorted(set(expected) - set(observed))
        unexpected = sorted(set(observed) - set(expected))
        detail = f"missing={missing[:5]} unexpected={unexpected[:5]}"
        raise MigrationError(
            f"source-home snapshot does not match inventory ({detail})"
        )
    return {
        "path": SOURCE_SNAPSHOT_RELATIVE_PATH.as_posix(),
        "target": SOURCE_SNAPSHOT_TARGET.as_posix(),
        "sha256": _sha256(archive_path),
        "bytes": archive_path.stat().st_size,
        "entry_count": len(observed),
        "default_disposition": "preserve",
        "activation_policy": "explicit-compatible-paths-only",
        "status": "complete",
    }


def _read_root_only_json(path: Path, label: str) -> dict[str, Any]:
    if not path.is_file() or path.is_symlink():
        raise MigrationError(f"{label} is missing or not a regular file")
    if stat.S_IMODE(path.stat().st_mode) != 0o600:
        raise MigrationError(f"{label} must be mode 0600")
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise MigrationError(f"could not read {label}: {exc}") from exc
    if not isinstance(value, dict):
        raise MigrationError(f"{label} must contain an object")
    return value


def _sites_summary(
    bundle: Path, expected_machine_id: str, source_inventory: dict[str, Any]
) -> dict[str, Any]:
    path = bundle / "sites.json"
    sites = _read_root_only_json(path, "Sites inventory")
    if sites.get("schema") != SITES_INVENTORY_SCHEMA:
        raise MigrationError(f"Sites inventory schema must be {SITES_INVENTORY_SCHEMA}")
    if sites.get("machine_id") != expected_machine_id:
        raise MigrationError("Sites inventory machine id mismatch")
    if sites.get("status") != "complete":
        raise MigrationError("Sites inventory is incomplete")
    endpoints = sites.get("endpoints")
    if not isinstance(endpoints, list):
        raise MigrationError("Sites inventory endpoints are missing")
    hostnames: list[str] = []
    inventory_paths = {
        entry["path"]
        for entry in source_inventory["entries"]
        if isinstance(entry, dict) and isinstance(entry.get("path"), str)
    }
    present = 0
    required = 0
    endpoint_fields = {
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
        "source",
    }
    auth_fields = {"mode", "owner_email", "emails", "org_domain"}
    for endpoint in endpoints:
        if not isinstance(endpoint, dict) or set(endpoint) - endpoint_fields:
            raise MigrationError("Sites inventory endpoint record is invalid")
        hostname = endpoint.get("hostname")
        if not isinstance(hostname, str) or not hostname:
            raise MigrationError("Sites inventory endpoint hostname is invalid")
        hostnames.append(hostname)
        auth = endpoint.get("auth")
        if (
            not isinstance(auth, dict)
            or set(auth) - auth_fields
            or not isinstance(auth.get("mode"), str)
        ):
            raise MigrationError("Sites inventory endpoint auth is invalid")
        for field in (
            "label",
            "status",
            "desired_process_state",
            "created_at",
            "updated_at",
        ):
            if not isinstance(endpoint.get(field), str):
                raise MigrationError(f"Sites inventory endpoint {field} is invalid")
        target_port = endpoint.get("target_port")
        if target_port is not None and (
            not isinstance(target_port, int)
            or isinstance(target_port, bool)
            or not 1 <= target_port <= 65535
        ):
            raise MigrationError("Sites inventory endpoint target port is invalid")
        run_command = endpoint.get("run_command")
        run_cwd = endpoint.get("run_cwd")
        if run_command is not None and not isinstance(run_command, str):
            raise MigrationError("Sites inventory run command is invalid")
        if run_cwd is not None and not isinstance(run_cwd, str):
            raise MigrationError("Sites inventory run cwd is invalid")
        if bool(run_command) != bool(run_cwd):
            raise MigrationError("Sites inventory run command and cwd disagree")
        source = endpoint.get("source")
        if not isinstance(source, dict) or set(source) != {"relative_path", "status"}:
            raise MigrationError("Sites inventory source evidence is invalid")
        source_status = source.get("status")
        relative_path = source.get("relative_path")
        if run_cwd:
            try:
                expected_relative = Path(run_cwd).relative_to("/home/node").as_posix()
            except ValueError as exc:
                raise MigrationError(
                    "Sites inventory source path is outside /home/node"
                ) from exc
            if (
                source_status != "present-in-source-snapshot"
                or relative_path != expected_relative
            ):
                raise MigrationError(
                    "Sites inventory source evidence does not match run_cwd"
                )
            if expected_relative != "." and expected_relative not in inventory_paths:
                raise MigrationError(
                    "Sites inventory source is missing from source snapshot"
                )
            if not isinstance(relative_path, str) or not relative_path:
                raise MigrationError("Sites inventory source path is invalid")
            required += 1
            present += 1
        elif source_status == "not-required":
            if relative_path is not None or run_command:
                raise MigrationError("Sites inventory source path is invalid")
        else:
            raise MigrationError("Sites inventory source status is invalid")
    if hostnames != sorted(set(hostnames)):
        raise MigrationError("Sites inventory endpoints must be unique and sorted")
    if sites.get("endpoint_count") != len(endpoints):
        raise MigrationError("Sites inventory endpoint count mismatch")
    if sites.get("source_paths_required") != required:
        raise MigrationError("Sites inventory required source count mismatch")
    if sites.get("source_paths_present") != present:
        raise MigrationError("Sites inventory present source count mismatch")
    if sites.get("activation_policy") != "manifest-only-not-republished":
        raise MigrationError("Sites inventory activation policy is invalid")
    return {
        "sha256": _sha256(path),
        "endpoint_count": len(endpoints),
        "source_paths_required": required,
        "source_paths_present": present,
        "activation_policy": sites["activation_policy"],
        "status": "complete",
    }


def _integrations_summary(
    bundle: Path, source_inventory: dict[str, Any]
) -> dict[str, Any]:
    path = bundle / "integrations.json"
    inventory = _read_root_only_json(path, "Integrations inventory")
    if inventory.get("schema") != INTEGRATIONS_INVENTORY_SCHEMA:
        raise MigrationError(
            f"Integrations inventory schema must be {INTEGRATIONS_INVENTORY_SCHEMA}"
        )
    if inventory.get("status") != "complete":
        raise MigrationError("Integrations inventory is incomplete")
    if (
        inventory.get("activation_policy")
        != "inventory-only-no-secret-values-or-activation"
    ):
        raise MigrationError("Integrations inventory activation policy is invalid")
    integrations = inventory.get("integrations")
    if not isinstance(integrations, list):
        raise MigrationError("Integrations inventory records are missing")
    names: list[str] = []
    inventory_paths = {
        entry["path"]
        for entry in source_inventory["entries"]
        if isinstance(entry, dict) and isinstance(entry.get("path"), str)
    }
    policy_counts = {policy: 0 for policy in sorted(INTEGRATION_MIGRATION_POLICIES)}
    expected_fields = {
        "name",
        "configured_keys",
        "evidence_paths",
        "source_enabled",
        "migration_policy",
        "target_state",
    }
    for integration in integrations:
        if not isinstance(integration, dict) or set(integration) != expected_fields:
            raise MigrationError("Integrations inventory record is invalid")
        name = integration.get("name")
        if not isinstance(name, str) or not name:
            raise MigrationError("Integrations inventory name is invalid")
        names.append(name)
        configured_keys = integration.get("configured_keys")
        if (
            not isinstance(configured_keys, list)
            or configured_keys != sorted(set(configured_keys))
            or any(
                not isinstance(key, str)
                or re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key) is None
                for key in configured_keys
            )
        ):
            raise MigrationError("Integrations inventory configured keys are invalid")
        evidence_paths = integration.get("evidence_paths")
        if (
            not isinstance(evidence_paths, list)
            or evidence_paths != sorted(set(evidence_paths))
            or any(not isinstance(item, str) or not item for item in evidence_paths)
        ):
            raise MigrationError("Integrations inventory evidence paths are invalid")
        for evidence_path in evidence_paths:
            filesystem_path = evidence_path.split("#", 1)[0]
            if not any(
                path == filesystem_path or path.startswith(filesystem_path + "/")
                for path in inventory_paths
            ):
                raise MigrationError(
                    "Integrations inventory evidence is missing from source snapshot"
                )
        if integration.get("source_enabled") not in {True, False, None}:
            raise MigrationError("Integrations inventory enabled state is invalid")
        policy = integration.get("migration_policy")
        if policy not in INTEGRATION_MIGRATION_POLICIES:
            raise MigrationError("Integrations inventory migration policy is invalid")
        policy_counts[policy] += 1
        if integration.get("target_state") != "inactive":
            raise MigrationError("Integrations inventory target state must be inactive")
    if names != sorted(set(names)):
        raise MigrationError("Integrations inventory records must be unique and sorted")
    if inventory.get("integration_count") != len(integrations):
        raise MigrationError("Integrations inventory count mismatch")
    return {
        "sha256": _sha256(path),
        "integration_count": len(integrations),
        "policy_counts": policy_counts,
        "activation_policy": inventory["activation_policy"],
        "status": "complete",
    }


def create_manifest(bundle: Path, source: SourceMetadata) -> dict[str, Any]:
    """Bind a complete source snapshot and compatible active payload."""
    bundle = Path(bundle)
    payload = bundle / "payload"
    source.validate()
    source_inventory = _source_inventory_summary(bundle)
    if source_inventory["sha256"] != source.source_inventory_sha256:
        raise MigrationError("source volume inventory sha256 mismatch")
    inventory = _read_source_inventory(bundle)
    source_snapshot = _source_snapshot_summary(bundle, inventory)
    sites = _sites_summary(bundle, source.machine_id, inventory)
    integrations = _integrations_summary(bundle, inventory)
    files = _walk_payload_entries(payload)
    session_path = payload / "sessions.jsonl"
    session_index, message_count = _session_index(session_path)
    _topological_session_ids(session_index)
    records = []
    for path in files:
        relative = PurePosixPath(path.relative_to(payload).as_posix())
        target = _target_for_payload(relative)
        is_symlink = path.is_symlink()
        link_target = os.readlink(path) if is_symlink else None
        content_sha256 = (
            hashlib.sha256(link_target.encode("utf-8")).hexdigest()
            if link_target is not None
            else _sha256(path)
        )
        records.append(
            {
                "path": relative.as_posix(),
                "target": target.as_posix() if target is not None else None,
                "kind": "symlink" if is_symlink else "file",
                "size": (
                    len(link_target.encode("utf-8"))
                    if link_target is not None
                    else path.stat().st_size
                ),
                "mode": None if is_symlink else stat.S_IMODE(path.stat().st_mode),
                "sha256": content_sha256,
                "link_target": link_target,
            }
        )
    _validate_active_payload_records(records, inventory)
    manifest: dict[str, Any] = {
        "schema": SCHEMA,
        "source": asdict(source),
        "source_inventory": source_inventory,
        "source_snapshot": source_snapshot,
        "sites": sites,
        "integrations": integrations,
        "files": records,
        "sessions": {
            "count": len(session_index),
            "message_count": message_count,
            "routing_state": "reset-by-target-hermes-importer",
        },
        "cron": _cron_summary(payload),
        "memory": _memory_summary(payload),
        "compatibility": {
            "session_paths": _session_path_summary(session_path),
            "legacy_skills": "review-only-not-active",
            "brain_working_tree": "reauthorize-and-sync-not-copied",
        },
        "protected_target_state": [
            path.as_posix() for path in PROTECTED_RELATIVE_PATHS
        ],
        "preserved_inert": PRESERVED_INERT,
    }
    _write_private_json(bundle / "manifest.json", manifest)
    return manifest


def verify_bundle(bundle: Path) -> dict[str, Any]:
    """Verify source completeness, target mappings, metadata, and every digest."""
    bundle = Path(bundle)
    manifest_path = bundle / "manifest.json"
    if not manifest_path.is_file() or manifest_path.is_symlink():
        raise MigrationError("bundle manifest.json is missing or not a regular file")
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise MigrationError(f"could not read bundle manifest: {exc}") from exc
    if not isinstance(manifest, dict) or manifest.get("schema") != SCHEMA:
        raise MigrationError(f"bundle schema must be {SCHEMA}")
    try:
        source = SourceMetadata(**manifest["source"])
    except (KeyError, TypeError) as exc:
        raise MigrationError("bundle source metadata is invalid") from exc
    source.validate()
    source_inventory = _source_inventory_summary(bundle)
    if source_inventory["sha256"] != source.source_inventory_sha256:
        raise MigrationError("source volume inventory sha256 mismatch")
    if manifest.get("source_inventory") != source_inventory:
        raise MigrationError("source volume inventory summary mismatch")
    inventory = _read_source_inventory(bundle)
    source_snapshot = _source_snapshot_summary(bundle, inventory)
    if manifest.get("source_snapshot") != source_snapshot:
        raise MigrationError("source-home snapshot summary mismatch")
    sites = _sites_summary(bundle, source.machine_id, inventory)
    if manifest.get("sites") != sites:
        raise MigrationError("Sites summary mismatch")
    integrations = _integrations_summary(bundle, inventory)
    if manifest.get("integrations") != integrations:
        raise MigrationError("Integrations summary mismatch")
    files = _walk_payload_entries(bundle / "payload")
    actual_paths = [path.relative_to(bundle / "payload").as_posix() for path in files]
    records = manifest.get("files")
    if not isinstance(records, list):
        raise MigrationError("bundle files must be a list")
    expected_paths = [
        record.get("path") for record in records if isinstance(record, dict)
    ]
    if expected_paths != actual_paths or len(expected_paths) != len(records):
        raise MigrationError("bundle file list does not match manifest")
    for record, path in zip(records, files, strict=True):
        relative = PurePosixPath(record["path"])
        expected_target = _target_for_payload(relative)
        rendered_target = (
            expected_target.as_posix() if expected_target is not None else None
        )
        if record.get("target") != rendered_target:
            raise MigrationError(f"target mapping mismatch for {relative}")
        is_symlink = path.is_symlink()
        link_target = os.readlink(path) if is_symlink else None
        expected_kind = "symlink" if is_symlink else "file"
        expected_size = (
            len(link_target.encode("utf-8"))
            if link_target is not None
            else path.stat().st_size
        )
        expected_mode = None if is_symlink else stat.S_IMODE(path.stat().st_mode)
        expected_sha256 = (
            hashlib.sha256(link_target.encode("utf-8")).hexdigest()
            if link_target is not None
            else _sha256(path)
        )
        if record.get("kind") != expected_kind:
            raise MigrationError(f"kind mismatch for {relative}")
        if record.get("size") != expected_size:
            raise MigrationError(f"size mismatch for {relative}")
        if record.get("mode") != expected_mode:
            raise MigrationError(f"mode mismatch for {relative}")
        if record.get("link_target") != link_target:
            raise MigrationError(f"symlink target mismatch for {relative}")
        if record.get("sha256") != expected_sha256:
            raise MigrationError(f"sha256 mismatch for {relative}")
    session_index, message_count = _session_index(bundle / "payload/sessions.jsonl")
    _topological_session_ids(session_index)
    expected_sessions = manifest.get("sessions")
    if not isinstance(expected_sessions, dict):
        raise MigrationError("bundle sessions summary is missing")
    if expected_sessions.get("count") != len(session_index):
        raise MigrationError("session count mismatch")
    if expected_sessions.get("message_count") != message_count:
        raise MigrationError("message count mismatch")
    if manifest.get("cron") != _cron_summary(bundle / "payload"):
        raise MigrationError("cron summary mismatch")
    if manifest.get("memory") != _memory_summary(bundle / "payload"):
        raise MigrationError("memory summary mismatch")
    expected_compatibility = {
        "session_paths": _session_path_summary(bundle / "payload/sessions.jsonl"),
        "legacy_skills": "review-only-not-active",
        "brain_working_tree": "reauthorize-and-sync-not-copied",
    }
    if manifest.get("compatibility") != expected_compatibility:
        raise MigrationError("compatibility summary mismatch")
    if manifest.get("protected_target_state") != [
        path.as_posix() for path in PROTECTED_RELATIVE_PATHS
    ]:
        raise MigrationError("protected target state contract mismatch")
    if manifest.get("preserved_inert") != PRESERVED_INERT:
        raise MigrationError("preserved-inert contract mismatch")
    return manifest


def _rewrite_source_path(value: Any) -> Any:
    if not isinstance(value, str):
        return value
    mappings = (
        ("/home/node/workspace", "/data/workspace/legacy-box1/workspace"),
        ("/home/node/dev", "/data/workspace/legacy-box1/dev"),
        ("/home/node/uploads", "/data/workspace/legacy-box1/uploads"),
    )
    for source, target in mappings:
        if value == source or value.startswith(source + "/"):
            return target + value[len(source) :]
    return value


def _rewrite_link_target(value: str) -> str:
    rewritten = _rewrite_source_path(value)
    return rewritten if isinstance(rewritten, str) else value


def _copy_sqlite(source: Path, target: Path) -> None:
    target.parent.mkdir(parents=True, exist_ok=True)
    if not source.exists():
        sqlite3.connect(target).close()
        return
    if source.is_symlink() or not source.is_file():
        raise MigrationError(f"SQLite source is missing or unsafe: {source}")
    with tempfile.TemporaryDirectory(
        prefix=f".{target.name}.source-", dir=target.parent
    ) as scratch_name:
        staged_source = Path(scratch_name) / source.name
        for suffix in ("", "-wal", "-shm"):
            candidate = Path(str(source) + suffix)
            if candidate.is_symlink():
                raise MigrationError(f"SQLite sidecar is unsafe: {candidate}")
            if not candidate.exists():
                continue
            if not candidate.is_file():
                raise MigrationError(f"SQLite sidecar is unsafe: {candidate}")
            shutil.copyfile(candidate, Path(str(staged_source) + suffix))

        source_connection = sqlite3.connect(staged_source)
        target_connection = sqlite3.connect(target)
        try:
            source_connection.backup(target_connection)
        finally:
            target_connection.close()
            source_connection.close()

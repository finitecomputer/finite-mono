#!/usr/bin/env python3
"""Sealed bundle and compatibility contract for legacy Hermes migrations."""

from __future__ import annotations

import hashlib
import json
import os
import re
import sqlite3
import stat
import tempfile
from contextlib import closing
from dataclasses import asdict, dataclass
from pathlib import Path, PurePosixPath
from typing import Any

SCHEMA = "finite.legacy-hermes-migration.v1"
SOURCE_INVENTORY_SCHEMA = "finite.legacy-hermes-source-inventory.v1"
SUPPORTED_SOURCE_HERMES_VERSION = "0.14.0"
SUPPORTED_TARGET_HERMES_VERSION = "0.20.0"
SOURCE_EXPORT_BATCH_SIZE = 1_000
RECEIPT_RELATIVE_PATH = Path("migration/legacy-hermes-v1/receipt.json")
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
ARCHIVED_ONLY = (
    "Hermes .env, auth.json, Google/OAuth tokens, and provider credentials",
    "Hermes config and gateway/platform routing state",
    "cron execution output and runtime process state",
    "legacy .finite and platform client state",
    "Hermes-managed venvs, binaries, logs, caches, and raw session/auxiliary SQLite files",
    "legacy session media that exists only in Hermes cache paths",
    "the legacy local Finite Brain working tree; the target must reauthorize and sync",
)
LEGACY_HERMES_MEDIA_CACHE_ROOTS = (
    "/home/node/.hermes/audio_cache/",
    "/home/node/.hermes/image_cache/",
    "~/.hermes/audio_cache/",
    "~/.hermes/image_cache/",
)


class MigrationError(RuntimeError):
    """A fail-closed migration contract violation."""


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
        "cache_media_archive_only_count": 0,
        "unmapped_source_path_count": 0,
    }

    def rewrite_media_tag(match: re.Match[str]) -> str:
        prefix, path = match.groups()
        rewritten = _rewrite_source_path(path)
        if rewritten != path:
            counts["rewritable_count"] += 1
            return prefix + rewritten
        if _is_legacy_cache_media_path(path):
            counts["cache_media_archive_only_count"] += 1
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
            counts["cache_media_archive_only_count"] += 1
        elif candidate.startswith(("/home/node/", "~/")):
            counts["unmapped_source_path_count"] += 1
        return candidate

    return visit(value), counts


def _session_path_summary(path: Path) -> dict[str, Any]:
    rewritable_count = 0
    cache_media_archive_only_count = 0
    unmapped_source_path_count = 0
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            session = json.loads(line)
            _, counts = _rewrite_message_paths(session.get("messages") or [])
            rewritable_count += counts["rewritable_count"]
            cache_media_archive_only_count += counts["cache_media_archive_only_count"]
            unmapped_source_path_count += counts["unmapped_source_path_count"]
    return {
        "rewritable_count": rewritable_count,
        "cache_media_archive_only_count": cache_media_archive_only_count,
        "unmapped_source_path_count": unmapped_source_path_count,
        "archive_only_policy": "retained-in-source-recovery-set",
    }


def _target_for_payload(relative: PurePosixPath) -> PurePosixPath | None:
    if relative in (
        PurePosixPath("sessions.jsonl"),
        PurePosixPath("memory_store.db"),
    ):
        return None
    mappings = (
        (PurePosixPath("hermes/memories"), PurePosixPath("agent/hermes-home/memories")),
        (
            PurePosixPath("hermes/skills"),
            PurePosixPath("migration/legacy-hermes-v1/review-only/skills"),
        ),
        (
            PurePosixPath("hermes/scripts"),
            PurePosixPath("migration/legacy-hermes-v1/review-only/scripts"),
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
        return PurePosixPath("migration/legacy-hermes-v1/review-only/cron/jobs.json")
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


def _source_inventory_summary(bundle: Path) -> dict[str, Any]:
    inventory_path = bundle / "source-volume-inventory.json"
    if not inventory_path.is_file() or inventory_path.is_symlink():
        raise MigrationError(
            "bundle source-volume-inventory.json is missing or not a regular file"
        )
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
    unresolved = classifications.get("unresolved")
    if (
        inventory.get("status") != "complete"
        or not isinstance(unresolved, dict)
        or unresolved.get("entries") != 0
        or inventory.get("unresolved_roots") != []
    ):
        raise MigrationError("source volume inventory still has unresolved entries")
    return {
        "schema": SOURCE_INVENTORY_SCHEMA,
        "sha256": _sha256(inventory_path),
        "classifications": classifications,
    }


def create_manifest(bundle: Path, source: SourceMetadata) -> dict[str, Any]:
    """Hash an already-staged allow-listed payload and write manifest.json."""
    bundle = Path(bundle)
    payload = bundle / "payload"
    source.validate()
    source_inventory = _source_inventory_summary(bundle)
    if source_inventory["sha256"] != source.source_inventory_sha256:
        raise MigrationError("source volume inventory sha256 mismatch")
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
    manifest: dict[str, Any] = {
        "schema": SCHEMA,
        "source": asdict(source),
        "source_inventory": source_inventory,
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
        "archived_only": list(ARCHIVED_ONLY),
    }
    _write_private_json(bundle / "manifest.json", manifest)
    return manifest


def verify_bundle(bundle: Path) -> dict[str, Any]:
    """Verify schema, allow-list, metadata, sizes, and every payload digest."""
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
    if manifest.get("archived_only") != list(ARCHIVED_ONLY):
        raise MigrationError("archived-only contract mismatch")
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
    source_connection = sqlite3.connect(f"file:{source}?mode=ro", uri=True)
    target_connection = sqlite3.connect(target)
    try:
        source_connection.backup(target_connection)
    finally:
        target_connection.close()
        source_connection.close()

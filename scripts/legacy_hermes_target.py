#!/usr/bin/env python3
"""Offline transactional installer for a stopped v2 Agent Runtime."""

from __future__ import annotations

import importlib.metadata
import json
import os
import shutil
import sqlite3
import stat
from collections.abc import Callable
from contextlib import closing
from pathlib import Path
from typing import Any

from legacy_hermes_contract import (
    PROTECTED_RELATIVE_PATHS,
    RECEIPT_RELATIVE_PATH,
    SCHEMA,
    SUPPORTED_TARGET_HERMES_VERSION,
    MigrationError,
    _copy_sqlite,
    _fresh_target_memory_fact_count,
    _fsync_directory,
    _memory_database_fact_count,
    _rewrite_link_target,
    _rewrite_message_paths,
    _rewrite_source_path,
    _session_index,
    _sha256,
    _topological_session_ids,
    _validate_sha256,
    _write_private_json,
    verify_bundle,
)


def _default_session_db_factory(db_path: Path):
    from hermes_state import SessionDB

    return SessionDB(db_path=db_path)


def _default_memory_store_factory(db_path: Path):
    from plugins.memory.holographic.store import MemoryStore

    return MemoryStore(db_path=db_path)


def _installed_hermes_version() -> str:
    try:
        return importlib.metadata.version("hermes-agent")
    except importlib.metadata.PackageNotFoundError as exc:
        raise MigrationError("target Hermes package is not installed") from exc


def _checkpoint_and_check(database_path: Path, label: str) -> None:
    with closing(sqlite3.connect(database_path)) as connection:
        checkpoint = connection.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone()
    if checkpoint is None or checkpoint[0] != 0:
        raise MigrationError(f"rebuilt target {label} failed WAL checkpoint")
    with closing(
        sqlite3.connect(f"file:{database_path}?mode=ro", uri=True)
    ) as connection:
        quick_check = connection.execute("PRAGMA quick_check").fetchone()
    if quick_check is None or quick_check[0] != "ok":
        raise MigrationError(f"rebuilt target {label} failed SQLite quick_check")


def _prepare_memory_store(
    source_database: Path,
    imported_database: Path,
    memory_store_factory: Callable[[Path], Any],
) -> int:
    _copy_sqlite(source_database, imported_database)
    store = memory_store_factory(imported_database)
    try:
        store.rebuild_all_vectors()
    finally:
        store.close()
    _checkpoint_and_check(imported_database, "memory_store.db")
    return _memory_database_fact_count(imported_database)


def _import_sessions(
    sessions_path: Path,
    database_path: Path,
    session_db_factory: Callable[[Path], Any],
) -> dict[str, int]:
    index, message_count = _session_index(sessions_path)
    ordered_ids = _topological_session_ids(index)
    database = session_db_factory(database_path)
    imported = 0
    detached = 0
    try:
        with sessions_path.open("rb") as handle:
            for session_id in ordered_ids:
                offset, length, _ = index[session_id]
                handle.seek(offset)
                raw = handle.read(length)
                session = json.loads(raw)
                for field in ("cwd", "git_repo_root"):
                    session[field] = _rewrite_source_path(session.get(field))
                session["messages"], _ = _rewrite_message_paths(
                    session.get("messages") or []
                )
                messages = session.get("messages") or []
                encoded_size = len(raw)
                database._IMPORT_MAX_SESSIONS = 1
                database._IMPORT_MAX_MESSAGES_PER_SESSION = max(10_000, len(messages))
                database._IMPORT_MAX_TOTAL_MESSAGES = max(50_000, len(messages))
                database._IMPORT_MAX_SESSION_BYTES = max(
                    5 * 1024 * 1024, encoded_size + 1024
                )
                database._IMPORT_MAX_TOTAL_BYTES = max(
                    25 * 1024 * 1024, encoded_size + 1024
                )
                result = database.import_sessions([session])
                if not result.get("ok"):
                    raise MigrationError(
                        f"Hermes rejected session {session_id}: {result.get('errors')}"
                    )
                if result.get("skipped") or result.get("imported") != 1:
                    raise MigrationError(
                        f"target already contains session id {session_id}"
                    )
                imported += 1
                detached += int(result.get("detached") or 0)
    finally:
        database.close()
    if imported != len(index):
        raise MigrationError("not every source session was imported")
    return {"imported": imported, "messages": message_count, "detached": detached}


def _protected_hashes(target_root: Path) -> dict[str, str]:
    hashes: dict[str, str] = {}
    for relative in PROTECTED_RELATIVE_PATHS:
        path = target_root / relative
        if not path.is_file() or path.is_symlink():
            raise MigrationError(
                f"protected target file is missing or unsafe: {relative}"
            )
        hashes[relative.as_posix()] = _sha256(path)
    return hashes


def _create_safe_target_parents(
    target_root: Path, target: Path, created_directories: list[Path]
) -> None:
    relative = target.relative_to(target_root)
    cursor = target_root
    for component in relative.parts[:-1]:
        cursor /= component
        if cursor.is_symlink():
            raise MigrationError(f"target parent is a symlink: {cursor}")
        if cursor.exists():
            if not cursor.is_dir():
                raise MigrationError(f"target parent is not a directory: {cursor}")
            continue
        cursor.mkdir(mode=0o700)
        created_directories.append(cursor)


def install_bundle(
    bundle: Path,
    target_root: Path,
    *,
    expected_machine_id: str,
    expected_manifest_sha256: str,
    expected_identity_sha256: str,
    expected_chat_client_sha256: str,
    session_db_factory: Callable[[Path], Any] = _default_session_db_factory,
    memory_store_factory: Callable[[Path], Any] = _default_memory_store_factory,
) -> dict[str, Any]:
    """Install a verified bundle into one stopped, freshly-created target root."""
    bundle = Path(bundle)
    target_root = Path(target_root)
    receipt_path = target_root / RECEIPT_RELATIVE_PATH
    if receipt_path.exists():
        raise MigrationError(f"bundle is already installed: {receipt_path}")
    target_hermes_version = _installed_hermes_version()
    if target_hermes_version != SUPPORTED_TARGET_HERMES_VERSION:
        raise MigrationError(
            "target Hermes version must be "
            f"{SUPPORTED_TARGET_HERMES_VERSION}, got {target_hermes_version}"
        )
    _validate_sha256(expected_manifest_sha256, "expected manifest sha256")
    _validate_sha256(expected_identity_sha256, "expected target identity sha256")
    _validate_sha256(expected_chat_client_sha256, "expected target Chat client sha256")
    manifest = verify_bundle(bundle)
    manifest_sha256 = _sha256(bundle / "manifest.json")
    if manifest_sha256 != expected_manifest_sha256:
        raise MigrationError("manifest sha256 does not match operator approval")
    if manifest["source"]["machine_id"] != expected_machine_id:
        raise MigrationError("source machine id does not match the approved canary")
    if not target_root.is_dir() or target_root.is_symlink():
        raise MigrationError("target root must be an existing real directory")
    protected_before = _protected_hashes(target_root)
    identity_key = PROTECTED_RELATIVE_PATHS[0].as_posix()
    chat_client_key = PROTECTED_RELATIVE_PATHS[1].as_posix()
    if protected_before[identity_key] != expected_identity_sha256:
        raise MigrationError("target identity sha256 does not match operator approval")
    if protected_before[chat_client_key] != expected_chat_client_sha256:
        raise MigrationError(
            "target Chat client sha256 does not match operator approval"
        )

    payload = bundle / "payload"
    created_files: list[Path] = []
    created_directories: list[Path] = []
    copy_plan: list[tuple[Path, Path, int | None, str | None]] = []
    for record in manifest["files"]:
        if record["target"] is None:
            continue
        source = payload / record["path"]
        target = target_root / record["target"]
        try:
            target.resolve(strict=False).relative_to(target_root.resolve())
        except ValueError as exc:
            raise MigrationError(
                f"target mapping escapes durable root: {record['target']}"
            ) from exc
        if target.exists() or target.is_symlink():
            if record["kind"] == "symlink":
                expected_link = _rewrite_link_target(record["link_target"])
                if not target.is_symlink() or os.readlink(target) != expected_link:
                    raise MigrationError(f"destination collision: {record['target']}")
            elif (
                not target.is_file()
                or target.is_symlink()
                or _sha256(target) != record["sha256"]
                or stat.S_IMODE(target.stat().st_mode) != record["mode"]
            ):
                raise MigrationError(f"destination collision: {record['target']}")
            continue
        copy_plan.append((source, target, record["mode"], record.get("link_target")))

    migration_root = receipt_path.parent
    work_root = migration_root / "work"
    preimport_root = migration_root / "preimport-state"
    target_db = target_root / "agent/hermes-home/state.db"
    imported_db = work_root / "state.db"
    target_memory_db = target_root / "agent/hermes-home/memory_store.db"
    imported_memory_db = work_root / "memory_store.db"
    source_memory_db = payload / "memory_store.db"
    if source_memory_db.exists() and _fresh_target_memory_fact_count(target_memory_db):
        raise MigrationError("fresh target memory store already contains facts")
    protected_after: dict[str, str] | None = None
    sessions_result: dict[str, int] | None = None
    memory_fact_count = 0
    swapped_databases: list[Path] = []
    moved_state_files: list[tuple[Path, Path]] = []
    try:
        migration_root.mkdir(parents=True, exist_ok=False)
        os.chmod(migration_root, 0o700)
        work_root.mkdir(mode=0o700)
        _copy_sqlite(target_db, imported_db)
        sessions_result = _import_sessions(
            payload / "sessions.jsonl", imported_db, session_db_factory
        )
        _checkpoint_and_check(imported_db, "state.db")
        if source_memory_db.exists():
            memory_fact_count = _prepare_memory_store(
                source_memory_db,
                imported_memory_db,
                memory_store_factory,
            )
            if memory_fact_count != manifest["memory"]["fact_count"]:
                raise MigrationError("not every structured memory fact was imported")

        for source, target, mode, link_target in copy_plan:
            _create_safe_target_parents(target_root, target, created_directories)
            if link_target is not None:
                target.symlink_to(_rewrite_link_target(link_target))
            else:
                shutil.copy2(source, target)
                assert mode is not None
                os.chmod(target, int(mode))
            created_files.append(target)

        preimport_root.mkdir(mode=0o700)
        database_swaps = [(target_db, imported_db)]
        if source_memory_db.exists():
            database_swaps.append((target_memory_db, imported_memory_db))
        for target_database, rebuilt_database in database_swaps:
            for suffix in ("", "-wal", "-shm"):
                original = Path(str(target_database) + suffix)
                if original.exists() or original.is_symlink():
                    if not original.is_file() or original.is_symlink():
                        raise MigrationError(f"unsafe target SQLite file: {original}")
                    backup = preimport_root / original.name
                    os.replace(original, backup)
                    moved_state_files.append((backup, original))
            os.replace(rebuilt_database, target_database)
            swapped_databases.append(target_database)
            os.chmod(target_database, 0o600)
            with target_database.open("rb") as handle:
                os.fsync(handle.fileno())
            _fsync_directory(target_database.parent)

        protected_after = _protected_hashes(target_root)
        if protected_after != protected_before:
            raise MigrationError(
                "protected Finite identity or Chat state changed during import"
            )
        receipt: dict[str, Any] = {
            "schema": SCHEMA,
            "source": manifest["source"],
            "source_inventory": manifest["source_inventory"],
            "manifest_sha256": manifest_sha256,
            "sessions": sessions_result,
            "cron": manifest["cron"],
            "memory": {
                **manifest["memory"],
                "imported_fact_count": memory_fact_count,
            },
            "compatibility": manifest["compatibility"],
            "protected_state": {
                "identity_sha256": protected_after[identity_key],
                "chat_client_sha256": protected_after[chat_client_key],
            },
            "workspace_root": "/data/workspace/legacy-box1",
            "status": "installed-offline-awaiting-runtime-verification",
        }
        shutil.rmtree(work_root, ignore_errors=True)
        _write_private_json(receipt_path, receipt)
        return receipt
    except BaseException:
        for database in reversed(swapped_databases):
            database.unlink(missing_ok=True)
        for backup, original in reversed(moved_state_files):
            if backup.exists():
                os.replace(backup, original)
        if moved_state_files:
            _fsync_directory(target_db.parent)
        for path in reversed(created_files):
            path.unlink(missing_ok=True)
        for path in reversed(created_directories):
            try:
                path.rmdir()
            except OSError:
                pass
        shutil.rmtree(migration_root, ignore_errors=True)
        raise

#!/usr/bin/env python3
"""Offline transactional installer for a stopped v2 Agent Runtime."""

from __future__ import annotations

import hashlib
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
    SUPPORTED_TARGET_HERMES_VERSIONS,
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
    _read_source_inventory,
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
    transform_fact: Callable[[str], str] | None = None,
) -> int:
    _copy_sqlite(source_database, imported_database)
    store = memory_store_factory(imported_database)
    try:
        if transform_fact is not None:
            # Full fidelity: rewrite legacy paths in fact content through the
            # target MemoryStore before its vectors are rebuilt.
            facts = store.list_facts(
                limit=_memory_database_fact_count(imported_database) + 1
            )
            for fact in facts:
                content = fact.get("content")
                if isinstance(content, str):
                    rewritten = transform_fact(content)
                    if rewritten != content:
                        store.update_fact(int(fact["fact_id"]), content=rewritten)
        store.rebuild_all_vectors()
    finally:
        store.close()
    _checkpoint_and_check(imported_database, "memory_store.db")
    return _memory_database_fact_count(imported_database)


def _import_sessions(
    sessions_path: Path,
    database_path: Path,
    session_db_factory: Callable[[Path], Any],
    rewrite_session: Callable[[dict[str, Any]], dict[str, Any]] | None = None,
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
                if rewrite_session is not None:
                    session = rewrite_session(session)
                else:
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
    mode: str = "default",
    fidelity_hooks: Any = None,
    telegram_mode: str = "rehearsal",
    expected_telegram_fingerprint: str | None = None,
) -> dict[str, Any]:
    """Install a verified bundle into one stopped, freshly-created target root.

    ``mode`` must equal the mode the approved manifest was sealed with. In
    full-fidelity mode ``telegram_mode`` defaults to ``rehearsal``, which
    writes the Telegram block without a token; only ``live`` writes the real
    token and it requires the qualified Runtime fingerprint.
    """
    bundle = Path(bundle)
    target_root = Path(target_root)
    receipt_path = target_root / RECEIPT_RELATIVE_PATH
    if receipt_path.exists():
        raise MigrationError(f"bundle is already installed: {receipt_path}")
    if receipt_path.parent.exists():
        # Only an incomplete rollback leaves this behind, with the backups it
        # could not restore.
        raise MigrationError(
            f"an earlier install left {RECEIPT_RELATIVE_PATH.parent.as_posix()}/ "
            "behind; restore the pre-import archive before retrying"
        )
    target_hermes_version = _installed_hermes_version()
    if target_hermes_version not in SUPPORTED_TARGET_HERMES_VERSIONS:
        raise MigrationError(
            f"unsupported target Hermes version {target_hermes_version}; "
            f"supported: {sorted(SUPPORTED_TARGET_HERMES_VERSIONS)}"
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
    manifest_mode = manifest.get("mode", "default")
    if mode != manifest_mode:
        raise MigrationError(
            f"install mode {mode} does not match the approved manifest mode {manifest_mode}"
        )
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

    fidelity = None
    transform_memory: Callable[..., bytes] | None = None
    rewrite_session = None
    session_paths = None
    memory_paths = None
    workspace_paths = None
    if manifest_mode == "full-fidelity":
        from legacy_hermes_fidelity import (
            FidelityInstall,
            default_hooks,
            session_rewriter,
            text_file_rewriter,
        )

        fidelity = FidelityInstall(
            manifest["fidelity"],
            bundle,
            target_root,
            _read_source_inventory(bundle),
            fidelity_hooks or default_hooks(),
            telegram_mode=telegram_mode,
            expected_telegram_fingerprint=expected_telegram_fingerprint,
        )
        rewrite_session, session_paths = session_rewriter(manifest["fidelity"])
        memory_paths = text_file_rewriter(manifest["fidelity"])
        workspace_paths = text_file_rewriter(manifest["fidelity"])
        transform_memory = memory_paths
    try:
        return _install_bundle_files(
            bundle,
            target_root,
            manifest,
            manifest_sha256,
            target_hermes_version,
            protected_before,
            identity_key,
            chat_client_key,
            receipt_path,
            session_db_factory,
            memory_store_factory,
            fidelity,
            transform_memory,
            rewrite_session,
            (session_paths, memory_paths, workspace_paths),
        )
    finally:
        if fidelity is not None:
            fidelity.close()


def _install_bundle_files(
    bundle: Path,
    target_root: Path,
    manifest: dict[str, Any],
    manifest_sha256: str,
    target_hermes_version: str,
    protected_before: dict[str, str],
    identity_key: str,
    chat_client_key: str,
    receipt_path: Path,
    session_db_factory: Callable[[Path], Any],
    memory_store_factory: Callable[[Path], Any],
    fidelity: Any,
    transform_memory: Callable[..., bytes] | None,
    rewrite_session: Callable[[dict[str, Any]], dict[str, Any]] | None,
    path_counters: tuple[Any, Any, Any],
) -> dict[str, Any]:
    payload = bundle / "payload"
    created_files: list[Path] = []
    created_directories: list[Path] = []
    copy_plan: list[tuple[Path, Path, int | None, str | None, bytes | None]] = []
    for record in manifest["files"]:
        if record["target"] is None:
            continue
        source = payload / record["path"]
        target = target_root / record["target"]
        content: bytes | None = None
        expected_sha256 = record["sha256"]
        if (
            transform_memory is not None
            and record["kind"] == "file"
            and record["target"].startswith("agent/hermes-home/memories/")
        ):
            content = transform_memory(source.read_bytes(), record["target"])
            expected_sha256 = hashlib.sha256(content).hexdigest()
        elif (
            path_counters[2] is not None
            and record["kind"] == "file"
            and record["target"].startswith("workspace/legacy-box1/")
        ):
            # Full fidelity: activated workspace, dev and uploads files get
            # the same path rewrite; binary ones are listed in the receipt.
            content = path_counters[2](source.read_bytes(), record["target"])
            expected_sha256 = hashlib.sha256(content).hexdigest()
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
                or _sha256(target) != expected_sha256
                or stat.S_IMODE(target.stat().st_mode) != record["mode"]
            ):
                raise MigrationError(f"destination collision: {record['target']}")
            continue
        copy_plan.append(
            (source, target, record["mode"], record.get("link_target"), content)
        )
    if fidelity is not None:
        fidelity.preflight()

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
    transaction = None
    try:
        migration_root.mkdir(parents=True, exist_ok=False)
        os.chmod(migration_root, 0o700)
        work_root.mkdir(mode=0o700)
        _copy_sqlite(target_db, imported_db)
        sessions_result = _import_sessions(
            payload / "sessions.jsonl", imported_db, session_db_factory, rewrite_session
        )
        routing_result = None
        if fidelity is not None:
            routing_result = fidelity.apply_routing(
                imported_db, session_db_factory, payload / "sessions.jsonl"
            )
        _checkpoint_and_check(imported_db, "state.db")
        fact_rewriter = None
        if fidelity is not None:
            fact_paths = path_counters[1]

            def fact_rewriter(text: str) -> str:
                return fact_paths.path_map.rewrite_text(text)

        if source_memory_db.exists():
            memory_fact_count = _prepare_memory_store(
                source_memory_db,
                imported_memory_db,
                memory_store_factory,
                fact_rewriter,
            )
            if memory_fact_count != manifest["memory"]["fact_count"]:
                raise MigrationError("not every structured memory fact was imported")

        for source, target, mode, link_target, content in copy_plan:
            _create_safe_target_parents(target_root, target, created_directories)
            if link_target is not None:
                target.symlink_to(_rewrite_link_target(link_target))
            elif content is not None:
                descriptor = os.open(
                    target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600
                )
                with os.fdopen(descriptor, "wb") as handle:
                    handle.write(content)
                assert mode is not None
                os.chmod(target, int(mode))
            else:
                shutil.copy2(source, target)
                assert mode is not None
                os.chmod(target, int(mode))
            created_files.append(target)

        preimport_root.mkdir(mode=0o700)
        fidelity_result = None
        if fidelity is not None:
            from legacy_hermes_fidelity import Transaction

            transaction = Transaction(target_root, preimport_root / "full-fidelity")
            fidelity_result = fidelity.apply(transaction)
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
            "source_snapshot": manifest["source_snapshot"],
            "sites": manifest["sites"],
            "integrations": manifest["integrations"],
            "manifest_sha256": manifest_sha256,
            "target_hermes_version": target_hermes_version,
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
        if fidelity is not None:
            session_paths, memory_paths, workspace_paths = path_counters
            receipt["mode"] = manifest["mode"]
            receipt["fidelity"] = {
                "plan": manifest["fidelity"],
                "installed": {
                    **fidelity_result,
                    "routing": routing_result,
                    "session_path_rewrites": session_paths.counts(),
                    "memory_path_rewrites": memory_paths.counts(),
                    "workspace_path_rewrites": workspace_paths.counts(),
                    "unrewritable_references": fidelity_result[
                        "unrewritable_references"
                    ]
                    + memory_paths.unrewritable
                    + workspace_paths.unrewritable,
                },
            }
        shutil.rmtree(work_root, ignore_errors=True)
        _write_private_json(receipt_path, receipt)
        return receipt
    except BaseException as error:
        failures = _rollback(
            transaction,
            swapped_databases,
            moved_state_files,
            target_db,
            created_files,
            created_directories,
            migration_root,
        )
        if failures:
            error.add_note(
                "rollback incomplete; restore the pre-import archive before any "
                "boot or retry. The unrestored backups were kept in "
                f"{migration_root.relative_to(target_root).as_posix()}/: "
                + "; ".join(failures)
            )
        raise


def _rollback(
    transaction: Any,
    swapped_databases: list[Path],
    moved_state_files: list[tuple[Path, Path]],
    target_db: Path,
    created_files: list[Path],
    created_directories: list[Path],
    migration_root: Path,
) -> list[str]:
    """Undo a failed install step by step; one failing step never skips the rest."""
    failures: list[str] = []

    def attempt(label: str, action: Callable[[], Any]) -> None:
        try:
            action()
        except Exception as exc:
            failures.append(f"{label}: {type(exc).__name__}")

    if transaction is not None:
        try:
            failures.extend(transaction.rollback())
        except Exception as exc:
            failures.append(f"full-fidelity rollback: {type(exc).__name__}")
    # Put each original SQLite file back over the imported one (os.replace is
    # atomic, so a restored database is never briefly absent). Only then
    # remove imported databases that have no restored original.
    restored: set[Path] = set()
    for backup, original in reversed(moved_state_files):
        if not backup.exists():
            continue
        before = len(failures)
        attempt(
            f"restore {original.name}",
            lambda b=backup, o=original: os.replace(b, o),
        )
        if len(failures) == before:
            restored.add(original)
    for database in reversed(swapped_databases):
        if database not in restored:
            attempt(
                f"remove {database.name}", lambda d=database: d.unlink(missing_ok=True)
            )
    if moved_state_files:
        attempt("fsync state dir", lambda: _fsync_directory(target_db.parent))
    for path in reversed(created_files):
        attempt(f"remove {path.name}", lambda p=path: p.unlink(missing_ok=True))
    for path in reversed(created_directories):
        try:
            path.rmdir()
        except OSError:
            pass
    if not failures:
        shutil.rmtree(migration_root, ignore_errors=True)
    # Otherwise keep the whole migration directory: it holds every backup
    # that failed to restore, and its presence blocks a retry until an
    # operator restores the pre-import archive.
    return failures

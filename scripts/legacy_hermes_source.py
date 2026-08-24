#!/usr/bin/env python3
"""Frozen-source evidence and export operations for legacy Hermes migrations."""

from __future__ import annotations

import importlib.metadata
import json
import os
import sqlite3
from contextlib import closing
from pathlib import Path
from typing import Any

from legacy_hermes_contract import (
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

SOURCE_BUNDLE_ROOTS = (
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
SOURCE_ARCHIVE_ONLY_ROOTS = (
    Path(".brain"),
    Path(".finite"),
    Path(".hermes/audio_cache"),
    Path(".hermes/image_cache"),
)
SOURCE_REBUILD_ROOTS = (Path("dev/reap-video/venv"),)


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
    if any(_is_below(relative, root) for root in SOURCE_BUNDLE_ROOTS):
        return "bundle"
    if relative in SOURCE_CONVERTED_FILES:
        return "converted"
    if any(_is_below(relative, root) for root in SOURCE_ARCHIVE_ONLY_ROOTS):
        return "archive-only"
    return "unresolved"


def _unresolved_inventory_root(relative: Path) -> Path:
    parts = relative.parts
    if parts and parts[0] == ".hermes" and len(parts) > 1:
        return Path(*parts[:2])
    return Path(parts[0])


def _empty_inventory_summary() -> dict[str, int]:
    return {
        "entries": 0,
        "regular_files": 0,
        "bytes": 0,
        "symlinks": 0,
        "special_files": 0,
    }


def inventory_source_volume(output: Path, source_root: Path) -> dict[str, Any]:
    """Inventory every non-directory entry on the legacy /home/node volume."""
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
        for name in ("bundle", "converted", "archive-only", "rebuild", "unresolved")
    }
    unresolved: dict[str, dict[str, int]] = {}
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
        directory_count += sum(
            not (directory_path / name).is_symlink() for name in dirnames
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
            if not (candidate.is_symlink() or candidate.is_file()):
                classification = "unresolved"
            summary = classifications[classification]
            summary["entries"] += 1
            if candidate.is_symlink():
                summary["symlinks"] += 1
            elif candidate.is_file():
                summary["regular_files"] += 1
                summary["bytes"] += info.st_size
            else:
                summary["special_files"] += 1
            if classification == "unresolved":
                root = _unresolved_inventory_root(relative).as_posix()
                root_summary = unresolved.setdefault(root, _empty_inventory_summary())
                root_summary["entries"] += 1
                if candidate.is_symlink():
                    root_summary["symlinks"] += 1
                elif candidate.is_file():
                    root_summary["regular_files"] += 1
                    root_summary["bytes"] += info.st_size
                else:
                    root_summary["special_files"] += 1

    unresolved_roots = [
        {"path": path, **summary} for path, summary in sorted(unresolved.items())
    ]
    result = {
        "schema": SOURCE_INVENTORY_SCHEMA,
        "source_root": str(resolved_source),
        "status": "complete" if not unresolved_roots else "review-required",
        "policy": {
            "bundle": [path.as_posix() for path in SOURCE_BUNDLE_ROOTS],
            "converted": [path.as_posix() for path in SOURCE_CONVERTED_FILES],
            "archive-only": [path.as_posix() for path in SOURCE_ARCHIVE_ONLY_ROOTS],
            "rebuild": [path.as_posix() for path in SOURCE_REBUILD_ROOTS],
        },
        "directories": directory_count,
        "classifications": classifications,
        "unresolved_roots": unresolved_roots,
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

"""Bounded, read-only current Brain grant coverage; never reads key payloads."""

from __future__ import annotations

import sqlite3
import time
from contextlib import closing
from pathlib import Path
from typing import Any, Callable


def collect(brain_id: str, database: Path, scratch_copy: Callable, generated_at: str) -> dict[str, Any]:
    report: dict[str, Any] = {
        "schema_version": "finite.status.v1",
        "generated_at": generated_at,
        "brain_id": brain_id,
        "evidence": "read-only scratch copy of local Brain authority",
        "limitation": "Grant presence does not prove decryption. Earlier keys and copies cannot be recalled.",
    }
    try:
        with scratch_copy(database) as scratch:
            with closing(sqlite3.connect(scratch.as_uri() + "?mode=ro", uri=True, timeout=2)) as connection:
                connection.execute("PRAGMA query_only = ON")
                deadline = time.monotonic() + 5
                connection.set_progress_handler(lambda: int(time.monotonic() > deadline), 1000)
                connection.execute("BEGIN")

                def rows(sql: str, limit: int) -> list[tuple]:
                    result = connection.execute(sql, (brain_id, limit + 1)).fetchall()
                    if len(result) > limit:
                        raise ValueError("Brain grant coverage exceeds the probe's capacity")
                    return result

                brain = connection.execute(
                    "SELECT kind, owner_user_id FROM brains WHERE id = ?", (brain_id,)
                ).fetchone()
                if brain is None:
                    raise ValueError("exact Brain ID was not found")
                if brain[0] not in {"personal", "organization"}:
                    raise ValueError("unsupported Brain kind")
                if (brain[0] == "personal") != (brain[1] is not None):
                    raise ValueError("Brain owner does not match its kind")
                members = {row[0] for row in rows("SELECT user_id FROM brain_members WHERE brain_id = ? LIMIT ?", 1000)}
                admins = {row[0] for row in rows("SELECT user_id FROM brain_admins WHERE brain_id = ? LIMIT ?", 1000)}
                agents = {row[0] for row in rows("SELECT agent_npub FROM personal_agents WHERE brain_id = ? AND status = 'active' LIMIT ?", 1)}
                folders = rows("SELECT id, path, access, current_key_version FROM folders WHERE brain_id = ? ORDER BY id LIMIT ?", 1000)
                direct = rows("SELECT folder_id, user_id FROM folder_access WHERE brain_id = ? LIMIT ?", 4000)
                grants = rows("SELECT g.folder_id, g.recipient_npub FROM folder_key_grants g JOIN folders f ON f.brain_id = g.brain_id AND f.id = g.folder_id AND f.current_key_version = g.key_version WHERE g.brain_id = ? LIMIT ?", 10000)
                mounts = rows("SELECT id FROM shared_folder_connections WHERE (source_brain_id = ?1 OR destination_brain_id = ?1) AND status = 'active' LIMIT ?2", 200)
                sources = rows("SELECT folder_id, user_id FROM folder_access_sources WHERE brain_id = ? AND source_kind = 'mount' LIMIT ?", 4000)
                connection.rollback()
        folder_reports = []
        for folder_id, path, access, version in folders:
            if access not in {"owner", "admin_only", "all_members", "restricted"}:
                raise ValueError("unsupported Folder access policy")
            if access == "owner" and brain[0] != "personal":
                raise ValueError("owner Folder policy requires a Personal Brain")
            entitled = {user for folder, user in direct if folder == folder_id}
            if brain[0] == "personal":
                entitled |= {brain[1]} | agents
            if access in {"admin_only", "all_members", "restricted"}:
                entitled |= admins
            if access == "all_members":
                entitled |= members
            holders = {user for folder, user in grants if folder == folder_id}
            folder_reports.append({
                "folder_id": folder_id, "path": path, "current_key_version": version,
                "entitled_count": len(entitled), "current_grant_count": len(holders),
                "unentitled_current_grants": sorted(holders - entitled),
                "missing_current_grants": sorted(entitled - holders),
            })
        unresolved = sum(len(folder["unentitled_current_grants"]) for folder in folder_reports)
        missing = sum(len(folder["missing_current_grants"]) for folder in folder_reports)
        incomplete_scope = bool(mounts or sources)
        report.update({
            "member_count": len(members), "admin_count": len(admins), "folders": folder_reports,
            "unentitled_current_grant_count": unresolved, "missing_current_grant_count": missing,
            "coverage": {"native_folders": "complete", "mounts": "unverified" if incomplete_scope else "complete"},
            "current_access_complete": not (incomplete_scope or unresolved or missing),
            "overall_status": "unknown" if incomplete_scope else "warning" if unresolved or missing else "pass",
            "exit_code": 2 if incomplete_scope else 1 if unresolved or missing else 0,
        })
        if incomplete_scope:
            report["coverage_reason"] = "Mount source entitlement is not certified by this local probe; use the signed admin access report."
    except (OSError, sqlite3.Error, ValueError, RuntimeError) as error:
        report.update({"overall_status": "unknown", "exit_code": 2, "current_access_complete": False, "error": str(error)})
    return report

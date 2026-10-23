#!/usr/bin/env python3
"""Carry Finite Chat history into the agent's Hermes session store, once.

Chat is moving from Finite Chat onto Hermes's own server. Hermes already
stores every conversation the bridge handled, so this only fills what Hermes
cannot know on its own:

1. Each Finite Chat chat's title (and topic, when it is not Home) becomes the
   title of the Hermes session that holds that chat, unless that session
   already has a title.
2. A chat Hermes never saw (no session for that room and chat) is imported
   as an archived, read-only Hermes session through Hermes's own importer.
3. The complete read-only export is written to the agent home, so any
   message Hermes lacks (commands, a few interrupted turns) stays on disk.

It runs during agent prepare, before the gateway or `hermes serve` starts,
and writes a marker when it completes. Every Hermes write goes through
Hermes's SessionDB API; existing sessions keep their messages untouched.
Failure never blocks boot: the marker is not written and the next boot
retries. Titles are only set on untitled sessions and imported ids are
deterministic, so a retry after a partial run changes nothing already done.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time
from collections.abc import Callable, Iterable
from pathlib import Path
from typing import Any

IMPORT_VERSION = 1
EXPORT_FORMAT = "finitechat.history.v1"
MARKER_NAME = "finitechat-history-import.json"
ARCHIVE_RELATIVE = Path("finitechat-archive") / "history-v1.json"
EXPORT_TIMEOUT_SECONDS = 300
HOME_TOPIC_ID = "home"
IMPORTED_ID_PREFIX = "finitechat-import-"
MAX_TITLE_LENGTH = 100
# Hermes's importer caps a single payload; stay well inside it.
IMPORT_BATCH_SESSIONS = 50
CARRIED_KINDS = {"message", "media"}


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    marker = args.hermes_home / MARKER_NAME
    if _completed(marker):
        print("FINITE_HISTORY_IMPORT skipped=already_complete")
        return 0
    try:
        export = _run_export(args)
        _write_archive(args.agent_home / ARCHIVE_RELATIVE, export)
        from hermes_state import SessionDB  # the pinned Hermes environment

        db = SessionDB()
        try:
            existing = list(_all_sessions(db))
            plan = plan_import(export, existing, sanitize=SessionDB.sanitize_title)
            titled = _apply_titles(db, plan["titles"])
            imported, skipped = _apply_imports(db, plan["imports"])
        finally:
            close = getattr(db, "close", None)
            if callable(close):
                close()
        summary = {
            "version": IMPORT_VERSION,
            "completed_at": int(time.time()),
            "chats_with_messages": plan["chats_with_messages"],
            "chats_already_in_hermes": plan["chats_already_in_hermes"],
            "titled": titled,
            "imported": imported,
            "skipped": skipped,
            "archive": str(ARCHIVE_RELATIVE),
        }
        _atomic_write(marker, json.dumps(summary, indent=2, sort_keys=True) + "\n")
        print(
            "FINITE_HISTORY_IMPORT "
            + " ".join(
                f"{key}={summary[key]}"
                for key in (
                    "chats_with_messages",
                    "chats_already_in_hermes",
                    "titled",
                    "imported",
                    "skipped",
                )
            )
        )
    except Exception as error:
        print(f"FINITE_HISTORY_IMPORT_ERROR {type(error).__name__}: {error}", file=sys.stderr)
    return 0


def plan_import(
    export: dict[str, Any],
    sessions: Iterable[dict[str, Any]],
    sanitize: Callable[[str], str | None] = lambda title: title,
) -> dict[str, Any]:
    """Decide titles and imports without touching any store.

    `sessions` are Hermes session rows (id, source, chat_id, thread_id,
    title). Returns titles to set on untitled sessions, archived sessions to
    import for chats Hermes never saw, and counts for the marker. `sanitize`
    is Hermes's own title cleaner, so uniqueness is checked on stored form.
    """
    if export.get("format") != EXPORT_FORMAT:
        raise ValueError(f"unexpected export format {export.get('format')!r}")
    sessions = list(sessions)
    taken = {row["title"] for row in sessions if row.get("title")}
    by_chat: dict[tuple[str, str], list[dict[str, Any]]] = {}
    for row in sessions:
        if row.get("source") == "finitechat" and row.get("chat_id") and row.get("thread_id"):
            by_chat.setdefault((row["chat_id"], row["thread_id"]), []).append(row)
    existing_ids = {row["id"] for row in sessions}

    titles: list[tuple[str, str]] = []
    imports: list[dict[str, Any]] = []
    chats_with_messages = 0
    chats_already_in_hermes = 0
    for room in export.get("rooms", []):
        for topic in room.get("topics", []):
            for chat in topic.get("chats", []):
                messages = chat.get("messages") or []
                if not messages:
                    continue
                chats_with_messages += 1
                wanted = sanitize(_chat_title(topic, chat)) or "Finite Chat"
                matches = by_chat.get((room["room_id"], chat["chat_id"]), [])
                if matches:
                    chats_already_in_hermes += 1
                    for row in sorted(matches, key=lambda row: row.get("started_at") or 0):
                        if not row.get("title"):
                            titles.append((row["id"], _unique(wanted, taken)))
                    continue
                session_id = IMPORTED_ID_PREFIX + _digest(
                    room["room_id"], topic["topic_id"], chat["chat_id"]
                )
                if session_id in existing_ids:
                    continue
                rows = _transcript(messages)
                if not rows:
                    continue
                imports.append(
                    {
                        "id": session_id,
                        "source": "finitechat",
                        "title": _unique(wanted, taken),
                        "archived": True,
                        "started_at": rows[0]["timestamp"],
                        "ended_at": rows[-1]["timestamp"],
                        "end_reason": "finitechat_history_import",
                        "messages": rows,
                    }
                )
    return {
        "titles": titles,
        "imports": imports,
        "chats_with_messages": chats_with_messages,
        "chats_already_in_hermes": chats_already_in_hermes,
    }


def _transcript(messages: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """User/assistant rows for one chat, with edits applied and progress dropped."""
    latest_edit: dict[str, str] = {}
    for message in messages:
        original = message.get("edit_of_message_id")
        if original:
            latest_edit[original] = message.get("text") or ""
    humans = {
        message.get("sender_account_id")
        for message in messages
        if not message.get("is_mine") and not message.get("edit_of_message_id")
    }
    rows = []
    for message in messages:
        if message.get("edit_of_message_id") or message.get("kind") not in CARRIED_KINDS:
            continue
        text = latest_edit.get(message["message_id"], message.get("text") or "")
        names = [
            media.get("filename") for media in message.get("media") or [] if media.get("filename")
        ]
        if names:
            text = "\n".join(filter(None, [text, *(f"[attachment: {name}]" for name in names)]))
        if not text.strip():
            continue
        mine = bool(message.get("is_mine"))
        if not mine and len(humans) > 1 and message.get("sender_display_name"):
            text = f"{message['sender_display_name']}: {text}"
        rows.append(
            {
                # The export is the agent Device's view: its own messages are the
                # assistant's; everyone else in the room is a user.
                "role": "assistant" if mine else "user",
                "content": text,
                "timestamp": float(message.get("timestamp_unix_seconds") or 0),
                "platform_message_id": message["message_id"],
            }
        )
    return rows


def _chat_title(topic: dict[str, Any], chat: dict[str, Any]) -> str:
    title = " ".join(str(chat.get("title") or "").split()) or "Finite Chat"
    if topic.get("topic_id") != HOME_TOPIC_ID and topic.get("title"):
        title = f"{' '.join(str(topic['title']).split())}: {title}"
    return title


def _unique(title: str, taken: set[str]) -> str:
    base = title[:MAX_TITLE_LENGTH].rstrip() or "Finite Chat"
    candidate = base
    number = 2
    while candidate in taken:
        suffix = f" ({number})"
        candidate = base[: MAX_TITLE_LENGTH - len(suffix)].rstrip() + suffix
        number += 1
    taken.add(candidate)
    return candidate


def _digest(*parts: str) -> str:
    return hashlib.sha256("\x1f".join(parts).encode("utf-8")).hexdigest()[:32]


def _all_sessions(db: Any) -> Iterable[dict[str, Any]]:
    offset = 0
    while True:
        page = db.list_sessions_rich(
            limit=500,
            offset=offset,
            include_archived=True,
            include_hidden=True,
            include_children=True,
            project_compression_tips=False,
            compact_rows=True,
        )
        yield from page
        if len(page) < 500:
            return
        offset += len(page)


def _apply_titles(db: Any, titles: list[tuple[str, str]]) -> int:
    applied = 0
    for session_id, title in titles:
        # Derived provenance never replaces a title a person or model chose.
        try:
            if db.set_auto_title(session_id, title, source=db.TITLE_SOURCE_DERIVED):
                applied += 1
        except ValueError:
            # Taken by a session created since the plan; the preview remains.
            continue
    return applied


def _apply_imports(db: Any, imports: list[dict[str, Any]]) -> tuple[int, int]:
    imported = skipped = 0
    for start in range(0, len(imports), IMPORT_BATCH_SESSIONS):
        result = db.import_sessions(imports[start : start + IMPORT_BATCH_SESSIONS])
        if not result.get("ok"):
            raise RuntimeError(f"Hermes refused the import: {result.get('errors')}")
        imported += int(result.get("imported") or 0)
        skipped += int(result.get("skipped") or 0)
    return imported, skipped


def _run_export(args: argparse.Namespace) -> dict[str, Any]:
    config = json.loads((args.agent_home / "config.json").read_text(encoding="utf-8"))
    command = [
        args.finitechat_bin,
        "app",
        "--data-dir",
        str(args.agent_home),
        "--server",
        str(config.get("server_url") or ""),
        "--device-id",
        str(config.get("device_id") or "agent"),
        "export-history",
    ]
    completed = subprocess.run(
        command,
        check=True,
        capture_output=True,
        timeout=EXPORT_TIMEOUT_SECONDS,
    )
    export = json.loads(completed.stdout)
    if export.get("format") != EXPORT_FORMAT:
        raise ValueError(f"unexpected export format {export.get('format')!r}")
    return export


def _write_archive(path: Path, export: dict[str, Any]) -> None:
    # The first complete export is the archive of record; a retry keeps it.
    if path.exists():
        return
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    _atomic_write(path, json.dumps(export, ensure_ascii=False) + "\n")


def _completed(marker: Path) -> bool:
    try:
        return json.loads(marker.read_text(encoding="utf-8")).get("version") == IMPORT_VERSION
    except (OSError, ValueError, AttributeError):
        return False


def _atomic_write(path: Path, content: str) -> None:
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, 0o600)
        os.replace(temporary, path)
    except BaseException:
        Path(temporary).unlink(missing_ok=True)
        raise


def _parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Carry Finite Chat history into the Hermes session store, once."
    )
    parser.add_argument("--agent-home", type=Path, required=True)
    parser.add_argument("--hermes-home", type=Path, required=True)
    parser.add_argument("--finitechat-bin", default="finitechat")
    return parser.parse_args(argv)


if __name__ == "__main__":
    sys.exit(main())

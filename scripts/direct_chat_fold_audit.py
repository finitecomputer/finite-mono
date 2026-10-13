#!/usr/bin/env python3
"""Compare agents' direct-chat fold digests with the hosted device's.

The hosted device's view comes from `finitechat-hosted-device digest` on
lat-2 (one JSON line per user). Each agent's view comes from its
`/data/agent/direct/fold-status.json`, written once at the fold. Both carry
only hashes. Rooms are matched by id. A room is intact when every topic and
chat digest is equal. Exit status 1 when any agent room differs or has no
hosted counterpart.

usage: scripts/direct_chat_fold_audit.py HOSTED_DIGESTS.jsonl FOLD_STATUS.json...
"""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any


def load_hosted(path: Path) -> dict[str, dict[str, Any]]:
    rooms: dict[str, dict[str, Any]] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        user = json.loads(line)
        for room in user.get("rooms", []):
            rooms[room["room_id"]] = {**room, "_account_id": user.get("account_id")}
    return rooms


def chat_differences(direct: dict[str, Any], hosted: dict[str, Any]) -> list[str]:
    def chats(room: dict[str, Any]) -> dict[tuple[str, str], dict[str, Any]]:
        return {
            (topic["topic_id"], chat["chat_id"]): {**chat, "_topic": topic}
            for topic in room.get("topics", [])
            for chat in topic.get("chats", [])
        }

    differences = []
    for topic in hosted.get("topics", []):
        mine = next((t for t in direct.get("topics", []) if t["topic_id"] == topic["topic_id"]), None)
        if mine is None:
            differences.append(f"topic {topic['topic_id']} missing")
        elif (mine["title_sha256"], mine["archived"]) != (topic["title_sha256"], topic["archived"]):
            differences.append(f"topic {topic['topic_id']} title/archive differs")
    direct_chats, hosted_chats = chats(direct), chats(hosted)
    for key, chat in hosted_chats.items():
        mine = direct_chats.get(key)
        if mine is None:
            differences.append(f"chat {key[0]}/{key[1]} missing")
            continue
        for field in ("title_sha256", "archived", "placement_topic_id", "position", "message_count", "digest"):
            if mine.get(field) != chat.get(field):
                detail = (
                    f" ({mine.get('message_count')} vs {chat.get('message_count')} messages)"
                    if field in ("message_count", "digest")
                    else ""
                )
                differences.append(f"chat {key[0]}/{key[1]} {field} differs{detail}")
                break
    for key in direct_chats.keys() - hosted_chats.keys():
        differences.append(f"chat {key[0]}/{key[1]} only on the agent")
    return differences


def audit(hosted: dict[str, dict[str, Any]], fold_statuses: list[Path]) -> tuple[list[dict[str, Any]], bool]:
    results = []
    ok = True
    for path in fold_statuses:
        status = json.loads(path.read_text(encoding="utf-8"))
        if status.get("origin") != "fold":
            results.append({"agent": str(path), "result": status.get("origin", "unknown"), "error": status.get("error")})
            ok &= status.get("origin") == "fresh"
            continue
        for room in status.get("digests", []):
            expected = hosted.get(room["room_id"])
            if expected is None:
                results.append({"agent": str(path), "room_id": room["room_id"], "result": "no hosted counterpart"})
                ok = False
                continue
            account = expected.pop("_account_id", None)
            if account and status.get("owner_account_id") not in (None, "", account):
                results.append({"agent": str(path), "room_id": room["room_id"], "result": "owner differs"})
                ok = False
                continue
            if room == expected:
                results.append({"agent": str(path), "room_id": room["room_id"], "result": "intact"})
            else:
                results.append({
                    "agent": str(path),
                    "room_id": room["room_id"],
                    "result": "differs",
                    "differences": chat_differences(room, expected),
                })
                ok = False
    return results, ok


def main(argv: list[str]) -> int:
    if len(argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    hosted = load_hosted(Path(argv[1]))
    results, ok = audit(hosted, [Path(path) for path in argv[2:]])
    for result in results:
        print(json.dumps(result, sort_keys=True))
    intact = sum(1 for result in results if result["result"] == "intact")
    print(f"{intact}/{len(results)} agent rooms intact", file=sys.stderr)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))

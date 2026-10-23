from __future__ import annotations

import json
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from io import StringIO
from pathlib import Path

from scripts import direct_chat_fold_audit as audit


def room(room_id: str, *, chat_digest: str = "c1", archived: bool = False) -> dict:
    return {
        "room_id": room_id,
        "digest": f"room-{chat_digest}-{archived}",
        "topics": [
            {
                "topic_id": "home",
                "title_sha256": "home-title",
                "archived": False,
                "chats": [
                    {
                        "chat_id": "home-chat",
                        "title_sha256": "chat-title",
                        "archived": archived,
                        "placement_topic_id": "home",
                        "position": "fffV",
                        "message_count": 3,
                        "digest": chat_digest,
                    }
                ],
            }
        ],
    }


class DirectChatFoldAuditTest(unittest.TestCase):
    def run_audit(self, hosted_rooms: list[dict], statuses: list[dict]) -> tuple[int, list[dict]]:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            hosted = root / "hosted.jsonl"
            hosted.write_text(
                "\n".join(
                    json.dumps({"user_storage_id": f"u{i}", "account_id": "a" * 64, "rooms": [r]})
                    for i, r in enumerate(hosted_rooms)
                )
            )
            paths = []
            for index, status in enumerate(statuses):
                path = root / f"agent-{index}.json"
                path.write_text(json.dumps(status))
                paths.append(str(path))
            out = StringIO()
            with redirect_stdout(out), redirect_stderr(StringIO()):
                code = audit.main(["audit", str(hosted), *paths])
        return code, [json.loads(line) for line in out.getvalue().splitlines()]

    def test_matching_rooms_are_intact(self) -> None:
        code, results = self.run_audit(
            [room("room-a"), room("room-b")],
            [
                {"origin": "fold", "owner_account_id": "a" * 64, "digests": [room("room-a")]},
                {"origin": "fold", "owner_account_id": "a" * 64, "digests": [room("room-b")]},
            ],
        )
        self.assertEqual(code, 0)
        self.assertEqual([result["result"] for result in results], ["intact", "intact"])

    def test_differences_name_the_chat_and_fail(self) -> None:
        code, results = self.run_audit(
            [room("room-a"), room("room-b", archived=True)],
            [
                {"origin": "fold", "digests": [room("room-a", chat_digest="c2")]},
                {"origin": "fold", "digests": [room("room-b", archived=False)]},
            ],
        )
        self.assertEqual(code, 1)
        self.assertEqual(results[0]["result"], "differs")
        self.assertIn("chat home/home-chat digest differs", results[0]["differences"][0])
        self.assertIn("archived differs", results[1]["differences"][0])

    def test_unmatched_rooms_owner_changes_and_pending_folds_fail(self) -> None:
        code, results = self.run_audit(
            [room("room-a")],
            [
                {"origin": "fold", "digests": [room("room-missing")]},
                {"origin": "fold", "owner_account_id": "b" * 64, "digests": [room("room-a")]},
                {"origin": "pending", "error": "rooms are behind the chat server"},
            ],
        )
        self.assertEqual(code, 1)
        self.assertEqual(
            [result["result"] for result in results],
            ["no hosted counterpart", "owner differs", "pending"],
        )

    def test_fresh_agents_have_nothing_to_compare(self) -> None:
        code, results = self.run_audit([], [{"origin": "fresh", "digests": []}])
        self.assertEqual(code, 0)
        self.assertEqual(results[0]["result"], "fresh")


if __name__ == "__main__":
    unittest.main()

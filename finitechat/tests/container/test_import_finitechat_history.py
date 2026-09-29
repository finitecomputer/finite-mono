"""Contract tests for the one-time Finite Chat → Hermes history carry."""

from __future__ import annotations

import importlib.util
import json
import os
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[2]
IMPORTER = REPO_ROOT / "containers/agent/import_finitechat_history.py"
GATEWAY = REPO_ROOT / "containers/agent/run_hermes_gateway.sh"
ROOM = "room-owner"


def load_importer() -> Any:
    spec = importlib.util.spec_from_file_location("import_finitechat_history_under_test", IMPORTER)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def message(message_id: str, text: str, *, mine: bool, ts: int, **extra: Any) -> dict[str, Any]:
    return {
        "message_id": message_id,
        "seq": ts,
        "sender_account_id": "agent" if mine else "owner",
        "sender_display_name": "Agent" if mine else "Owner",
        "is_mine": mine,
        "kind": "message",
        "status": "complete",
        "final_delivery": mine,
        "text": text,
        "timestamp_unix_seconds": ts,
        "edit_of_message_id": None,
        "reply_to_message_id": None,
        "media": [],
        **extra,
    }


def export_fixture() -> dict[str, Any]:
    return {
        "format": "finitechat.history.v1",
        "account_id": "agent",
        "device_id": "agent",
        "rooms": [
            {
                "room_id": ROOM,
                "display_name": "Chat with Agent",
                "is_agent_chat": True,
                "topics": [
                    {
                        "topic_id": "home",
                        "title": "Home",
                        "archived": False,
                        "message_count": 5,
                        "chats": [
                            {
                                "chat_id": "segment-seen",
                                "title": "Renamed plan",
                                "archived": False,
                                "message_count": 2,
                                "messages": [
                                    message("m1", "make a plan", mine=False, ts=100),
                                    message("m2", "here is the plan", mine=True, ts=101),
                                ],
                            },
                            {
                                "chat_id": "segment-unseen",
                                "title": "yo",
                                "archived": True,
                                "message_count": 3,
                                "messages": [
                                    message("u1", "yo", mine=False, ts=10),
                                    message("u2", "🔍 searching", mine=True, ts=11, kind="tool"),
                                    message("u3", "draft", mine=True, ts=12),
                                    message(
                                        "u4",
                                        "final answer",
                                        mine=True,
                                        ts=13,
                                        edit_of_message_id="u3",
                                    ),
                                    message(
                                        "u5",
                                        "",
                                        mine=False,
                                        ts=14,
                                        kind="media",
                                        media=[
                                            {
                                                "attachment_id": "a",
                                                "filename": "photo.jpg",
                                                "mime_type": "image/jpeg",
                                            }
                                        ],
                                    ),
                                ],
                            },
                            {
                                "chat_id": "segment-empty",
                                "title": "New chat",
                                "archived": False,
                                "message_count": 0,
                                "messages": [],
                            },
                        ],
                    },
                    {
                        "topic_id": "topic-fun",
                        "title": "Fun Stuff",
                        "archived": False,
                        "message_count": 1,
                        "chats": [
                            {
                                "chat_id": "segment-fun",
                                "title": "yo",
                                "archived": False,
                                "message_count": 1,
                                "messages": [message("f1", "heyo", mine=False, ts=20)],
                            }
                        ],
                    },
                ],
            }
        ],
    }


def seen_session(**overrides: Any) -> dict[str, Any]:
    return {
        "id": "hermes-seen",
        "source": "finitechat",
        "chat_id": ROOM,
        "thread_id": "segment-seen",
        "title": None,
        "started_at": 100,
        **overrides,
    }


class PlanTests(unittest.TestCase):
    def setUp(self) -> None:
        self.importer = load_importer()

    def test_titles_seen_chats_and_imports_only_unseen_ones(self) -> None:
        plan = self.importer.plan_import(export_fixture(), [seen_session()])
        self.assertEqual(plan["chats_with_messages"], 3)
        self.assertEqual(plan["chats_already_in_hermes"], 1)
        self.assertEqual(plan["titles"], [("hermes-seen", "Renamed plan")])
        self.assertEqual([row["title"] for row in plan["imports"]], ["yo", "Fun Stuff: yo"])

    def test_an_existing_title_is_never_replaced(self) -> None:
        plan = self.importer.plan_import(export_fixture(), [seen_session(title="Chosen by person")])
        self.assertEqual(plan["titles"], [])

    def test_imported_transcript_applies_edits_and_drops_progress(self) -> None:
        plan = self.importer.plan_import(export_fixture(), [])
        unseen = next(row for row in plan["imports"] if row["title"] == "yo")
        self.assertTrue(unseen["archived"])
        self.assertEqual(unseen["source"], "finitechat")
        self.assertEqual(
            [(row["role"], row["content"]) for row in unseen["messages"]],
            [("user", "yo"), ("assistant", "final answer"), ("user", "[attachment: photo.jpg]")],
        )
        self.assertEqual(unseen["messages"][1]["platform_message_id"], "u3")
        self.assertEqual((unseen["started_at"], unseen["ended_at"]), (10.0, 14.0))

    def test_titles_stay_unique_against_hermes_and_each_other(self) -> None:
        plan = self.importer.plan_import(
            export_fixture(), [seen_session(), {"id": "other", "source": "cli", "title": "yo"}]
        )
        self.assertEqual([row["title"] for row in plan["imports"]], ["yo (2)", "Fun Stuff: yo"])

    def test_imported_ids_are_deterministic_and_not_reimported(self) -> None:
        first = self.importer.plan_import(export_fixture(), [])
        again = self.importer.plan_import(export_fixture(), [])
        self.assertEqual(
            [row["id"] for row in first["imports"]], [row["id"] for row in again["imports"]]
        )
        done = [
            {"id": row["id"], "source": "finitechat", "title": row["title"]}
            for row in first["imports"]
        ]
        self.assertEqual(self.importer.plan_import(export_fixture(), done)["imports"], [])

    def test_several_people_in_a_room_keep_their_names(self) -> None:
        export = export_fixture()
        chat = export["rooms"][0]["topics"][1]["chats"][0]
        chat["messages"].append(
            message(
                "f2",
                "me too",
                mine=False,
                ts=21,
                sender_account_id="friend",
                sender_display_name="Friend",
            )
        )
        plan = self.importer.plan_import(export, [])
        fun = next(row for row in plan["imports"] if row["title"] == "Fun Stuff: yo")
        self.assertEqual(
            [row["content"] for row in fun["messages"]], ["Owner: heyo", "Friend: me too"]
        )

    def test_rejects_an_unknown_export_format(self) -> None:
        with self.assertRaises(ValueError):
            self.importer.plan_import({"format": "something-else"}, [])


class LauncherTests(unittest.TestCase):
    def test_prepare_runs_the_importer_after_config_and_before_services(self) -> None:
        script = GATEWAY.read_text(encoding="utf-8")
        reconcile = script.index("    run_config_reconciler\n    # One-time carry")
        importer = script.index('python "$history_importer"')
        prepare_exit = script.index('if [[ "${1:-}" == "--prepare-only" ]]')
        self.assertLess(reconcile, importer)
        self.assertLess(importer, prepare_exit)
        self.assertIn(
            '|| echo "run_hermes_gateway: Finite Chat history import unavailable"', script
        )
        # Opt-in per runtime until canaried.
        self.assertIn('"${FINITE_HISTORY_IMPORT:-0}" == "1"', script)


def hermes_state_available() -> bool:
    return importlib.util.find_spec("hermes_state") is not None


@unittest.skipUnless(hermes_state_available(), "requires the pinned Hermes environment")
class RealHermesTests(unittest.TestCase):
    """Runs the importer end to end against Hermes's own SessionDB."""

    def run_importer(
        self, agent_home: Path, hermes_home: Path, export: dict[str, Any] | None, fail: bool = False
    ) -> subprocess.CompletedProcess[str]:
        fake = agent_home.parent / "finitechat"
        payload = json.dumps(export or {})
        fake.write_text(
            textwrap.dedent(f"""\
            #!/usr/bin/env python3
            import sys
            assert sys.argv[1] == "app" and sys.argv[-1] == "export-history", sys.argv
            {"sys.exit(3)" if fail else ""}
            sys.stdout.write({payload!r})
            """),
            encoding="utf-8",
        )
        fake.chmod(0o755)
        return subprocess.run(
            [
                sys.executable,
                str(IMPORTER),
                "--agent-home",
                str(agent_home),
                "--hermes-home",
                str(hermes_home),
                "--finitechat-bin",
                str(fake),
            ],
            env={**os.environ, "HERMES_HOME": str(hermes_home)},
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )

    def test_carries_titles_and_unseen_chats_once_without_touching_existing_messages(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            tmp = Path(raw)
            agent_home = tmp / "agent"
            hermes_home = agent_home / "hermes-home"
            hermes_home.mkdir(parents=True)
            (agent_home / "config.json").write_text(
                json.dumps({"server_url": "http://127.0.0.1:9", "device_id": "agent"}),
                encoding="utf-8",
            )
            previous = os.environ.get("HERMES_HOME")
            self.addCleanup(
                lambda: (
                    os.environ.pop("HERMES_HOME", None)
                    if previous is None
                    else os.environ.__setitem__("HERMES_HOME", previous)
                )
            )
            os.environ["HERMES_HOME"] = str(hermes_home)
            from hermes_state import SessionDB

            db = SessionDB()
            db.create_session("hermes-seen", "finitechat")
            db.record_gateway_session_peer(
                "hermes-seen",
                source="finitechat",
                session_key="agent:main:finitechat:group:room",
                chat_id=ROOM,
                thread_id="segment-seen",
            )
            db.append_message("hermes-seen", role="user", content="make a plan")
            db.append_message("hermes-seen", role="assistant", content="here is the plan")
            before = [(row["role"], row["content"]) for row in db.get_messages("hermes-seen")]
            db.close()

            failed = self.run_importer(agent_home, hermes_home, None, fail=True)
            self.assertEqual(failed.returncode, 0, failed.stderr)
            self.assertIn("FINITE_HISTORY_IMPORT_ERROR", failed.stderr)
            self.assertFalse((hermes_home / "finitechat-history-import.json").exists())

            done = self.run_importer(agent_home, hermes_home, export_fixture())
            self.assertEqual(done.returncode, 0, done.stderr)
            self.assertIn("titled=1 imported=2", done.stdout)
            marker = json.loads(
                (hermes_home / "finitechat-history-import.json").read_text(encoding="utf-8")
            )
            self.assertEqual((marker["titled"], marker["imported"]), (1, 2))
            archive = agent_home / "finitechat-archive/history-v1.json"
            self.assertEqual(
                json.loads(archive.read_text(encoding="utf-8"))["format"], "finitechat.history.v1"
            )
            self.assertEqual(stat.S_IMODE(archive.stat().st_mode), 0o600)

            db = SessionDB()
            self.assertEqual(db.get_session_title("hermes-seen"), "Renamed plan")
            self.assertEqual(
                [(row["role"], row["content"]) for row in db.get_messages("hermes-seen")], before
            )
            imported = [
                row
                for row in db.list_sessions_rich(limit=50, include_archived=True)
                if row["id"].startswith("finitechat-import-")
            ]
            self.assertEqual(sorted(row["title"] for row in imported), ["Fun Stuff: yo", "yo"])
            self.assertTrue(all(row["archived"] for row in imported))
            db.close()

            again = self.run_importer(agent_home, hermes_home, export_fixture())
            self.assertIn("skipped=already_complete", again.stdout)


if __name__ == "__main__":
    unittest.main()

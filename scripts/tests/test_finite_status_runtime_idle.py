from __future__ import annotations

import contextlib
from datetime import datetime, timezone
import errno
import io
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

from scripts import finite_status
from scripts import finite_status_runtime_idle as idle

NOW_MS = 1_790_000_000_000
SECRET = "PRIVATE_CHAT_TEXT_room_secret_msg_secret_lease_secret_nsec1secret"
CHECKS = ["canonical_handle", "containerd_task", "sandbox_state",
          "duplicate_writers", "cni_namespace", "vmm_process"]


def event(lease: dict | None = None, created_ms: int = NOW_MS - 30_000) -> dict:
    item = {"key": SECRET, "room_id": SECRET, "seq": 1, "message_id": SECRET,
            "created_at_ms": created_ms, "event": {"text": SECRET}}
    if lease is not None:
        item["lease"] = lease
    return item


GATEWAY_START_S = NOW_MS / 1000 - 3600
WRITTEN_S = NOW_MS / 1000 - 86_400  # host mtime of a state file nobody touched for a day
DROP = object()  # leaves a field out of a written document
SESSIONS_DDL = ("CREATE TABLE sessions (id TEXT PRIMARY KEY, source TEXT NOT NULL, started_at REAL NOT NULL,"
                " ended_at REAL, end_reason TEXT)")


class AgentRoot:
    """A synthetic `<work_root>/kata/<runtime>` durable root."""

    def __init__(self, base: Path, runtime: str = "runtime-a") -> None:
        self.root = base / "kata" / runtime
        self.agent = self.root / "agent"
        (self.agent / "hermes-home").mkdir(parents=True)
        self.gateway()
        self.write("hermes-inbox.json", {"events": [], "cursors": {SECRET: 4},
                                         "acked": [{"key": SECRET, "acked_at_ms": 1}]})
        self.pid()
        with contextlib.closing(sqlite3.connect(self.agent / "hermes-home" / "state.db")) as db:
            db.execute(SESSIONS_DDL)
            db.execute("INSERT INTO sessions VALUES (?, 'finitechat', ?, NULL, NULL)", (SECRET, GATEWAY_START_S - 99))
            db.commit()

    def pid(self, started_s: float = GATEWAY_START_S) -> None:
        """`hermes gateway run` creates gateway.pid O_EXCL once at process start."""
        path = self.agent / "hermes-home" / "gateway.pid"
        path.write_text(json.dumps({"pid": 7, "start_time": 123}))
        os.utime(path, (started_s, started_s))

    def session(self, started_s: float, ended_s: float | None = None, sid: str = "bg_120000_" + SECRET) -> None:
        with contextlib.closing(sqlite3.connect(self.agent / "hermes-home" / "state.db")) as db:
            db.execute("INSERT INTO sessions VALUES (?, 'finitechat', ?, ?, NULL)", (sid, started_s, ended_s))
            db.commit()

    def write(self, name: str, document: object, written_s: float = WRITTEN_S) -> Path:
        """The guest writes through virtiofsd, so the host clock stamps the mtime."""
        path = self.agent / name
        path.write_text(document if isinstance(document, str) else json.dumps(document))
        os.utime(path, (written_s, written_s))
        return path

    def gateway(self, state: object = "running", active: object = 0, guest_ahead_s: float = 0,
                **extra: object) -> None:
        """Hermes stamps `updated_at` from the guest clock just before the host stamps the mtime."""
        written_s = NOW_MS / 1000 - 600
        updated_at = datetime.fromtimestamp(written_s + guest_ahead_s, timezone.utc).isoformat()
        document = {"pid": 7, "gateway_state": state, "active_agents": active, "updated_at": updated_at,
                    "platforms": {SECRET: {}}, **extra}
        self.write("hermes-home/gateway_state.json",
                   {key: value for key, value in document.items() if value is not DROP}, written_s)


class ObserveTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.home = AgentRoot(Path(self.temporary.name).resolve())
        # The scratch copies of state.db go to the default temp dir; keep them in view.
        self.scratch = Path(self.temporary.name).resolve() / "tmp"
        self.scratch.mkdir()
        self.tempdir = mock.patch.object(tempfile, "tempdir", str(self.scratch))
        self.tempdir.start()

    def tearDown(self) -> None:
        self.tempdir.stop()
        self.temporary.cleanup()

    def observe(self) -> dict:
        result = idle.observe(self.home.root, NOW_MS)
        self.assertNotIn(SECRET, json.dumps(result))
        # Every read, including one that fails, removes its copy of the user's chat database.
        self.assertEqual(sorted(path.name for path in self.scratch.iterdir()), [])
        return result

    def test_running_gateway_with_absent_agentd_and_marker_files_is_idle(self) -> None:
        result = self.observe()
        self.assertEqual((result["verdict"], result["reasons"]), ("idle", []))
        self.assertEqual(result["gateway"], {"state": "running", "active_agents": 0, "updated_age_s": 600,
                                             "clock_offset_ms": 0,
                                             "background": {"gateway_started_age_s": 3600, "open": 0,
                                                            "stale_open": 0, "recently_ended": 0}})
        self.assertEqual(result["hermes_inbox"]["newest_ack_age_s"], (NOW_MS - 1) // 1000)
        self.assertEqual(result["hermes_inbox"]["written_age_s"], 86_400)
        self.assertEqual(result["agentd_inbox"], {"present": False, "events": 0})
        self.assertEqual(result["running_markers"], {"present": False, "messages": 0})
        (self.home.agent / "hermes-inbox.json").unlink()
        self.assertEqual(self.observe()["verdict"], "idle")

    def test_each_turn_signal_is_busy_with_counts_and_ages_only(self) -> None:
        cases = {
            "active_agents": lambda: self.home.gateway(active=2),
            "inbox_pending": lambda: self.home.write("hermes-inbox.json", {"events": [
                event(), event({"state": "pending"}, created_ms=NOW_MS - 90_000)]}),
            "inbox_leased": lambda: self.home.write("hermes-inbox.json", {"events": [
                event({"state": "leased", "lease_id": SECRET, "leased_at_ms": NOW_MS - 840_000}),
                event({"state": "leased", "lease_id": SECRET, "leased_at_ms": NOW_MS - 5_000})]}),
            "agentd_inbox": lambda: self.home.write("agentd-inbox.json", {"events": [
                {"key": SECRET, "room_id": SECRET, "seq": 2, "message_id": SECRET, "delivery": {}}]}),
        }
        for reason, arrange in cases.items():
            with self.subTest(reason):
                self.tearDown(); self.setUp()
                arrange()
                result = self.observe()
                self.assertEqual((result["verdict"], result["reasons"]), ("busy", [reason]))
        self.assertEqual(result["agentd_inbox"], {"present": True, "events": 1})
        self.tearDown(); self.setUp()
        cases["inbox_leased"]()
        self.assertEqual(self.observe()["hermes_inbox"], {
            "present": True, "pending": 0, "leased": 2, "oldest_pending_age_s": None,
            "oldest_lease_age_s": 840, "newest_lease_age_s": 5, "newest_ack_age_s": None,
            "written_age_s": 86_400})
        cases["inbox_pending"]()
        self.assertEqual(self.observe()["hermes_inbox"]["oldest_pending_age_s"], 90)

    def test_leftover_running_markers_are_reported_separately_from_busy(self) -> None:
        self.home.write("hermes-running.json", {"messages": [
            {"room_id": SECRET, "conversation_id": None, "message_id": SECRET}] * 3})
        result = self.observe()
        self.assertEqual((result["verdict"], result["reasons"]), ("unfinished_markers", ["running_markers"]))
        self.assertEqual(result["running_markers"], {"present": True, "messages": 3})
        self.home.gateway(active=1)
        self.assertEqual(self.observe()["reasons"], ["active_agents", "running_markers"])

    def test_missing_or_non_running_gateway_is_never_idle(self) -> None:
        (self.home.agent / "hermes-home" / "gateway_state.json").unlink()
        self.assertEqual(self.observe()["reasons"], ["gateway_missing"])
        for state in ("starting", "draining", "stopped", "degraded", "startup_failed", "new_state"):
            with self.subTest(state):
                self.home.gateway(state=state)
                result = self.observe()
                self.assertEqual((result["verdict"], result["reasons"]), ("unknown", ["gateway_not_running"]))
        self.assertEqual(result["gateway"]["state"], "unrecognized")

    def test_empty_objects_load_as_empty_but_null_or_mistyped_fields_fail(self) -> None:
        # The finitechat serde loaders default absent fields and ignore unknown
        # ones, but reject a JSON null document or a present null field.
        for name in ("hermes-inbox.json", "agentd-inbox.json", "hermes-running.json"):
            self.home.write(name, {"future_field": SECRET})
        self.assertEqual(self.observe()["verdict"], "idle")
        cases = [
            ("hermes_inbox_malformed", "hermes-inbox.json", "null"),
            ("agentd_inbox_malformed", "agentd-inbox.json", "null"),
            ("running_markers_malformed", "hermes-running.json", "null"),
            ("hermes_inbox_malformed", "hermes-inbox.json", {"events": None}),
            ("hermes_inbox_malformed", "hermes-inbox.json", {"events": [], "cursors": None}),
            ("hermes_inbox_malformed", "hermes-inbox.json", {"events": [], "cursors": {"r": -1}}),
            ("hermes_inbox_malformed", "hermes-inbox.json", {"events": [], "acked": None}),
            ("hermes_inbox_malformed", "hermes-inbox.json", {"events": [], "acked": [{"key": "k"}]}),
            ("agentd_inbox_malformed", "agentd-inbox.json", {"events": [], "cursors": []}),
            ("running_markers_malformed", "hermes-running.json", {"messages": [], "x": 1} | {"messages": None}),
        ]
        for reason, name, document in cases:
            with self.subTest(reason=reason, document=document):
                self.tearDown(); self.setUp()
                self.home.write(name, document)
                result = self.observe()
                self.assertEqual((result["verdict"], result["reasons"]), ("unknown", [reason]))
        self.tearDown(); self.setUp()
        self.home.write("hermes-home/gateway_state.json", "null")
        self.assertEqual(self.observe()["reasons"], ["gateway_malformed"])

    def test_malformed_state_and_unknown_leases_fail_unknown(self) -> None:
        cases = [
            ("gateway_malformed", lambda: self.home.gateway(active="0")),
            ("gateway_malformed", lambda: self.home.gateway(active=False)),
            ("gateway_malformed", lambda: self.home.gateway(active=-1)),
            ("gateway_malformed", lambda: self.home.gateway(active=None)),
            ("gateway_malformed", lambda: self.home.gateway(state=None)),
            ("gateway_malformed", lambda: self.home.write("hermes-home/gateway_state.json", "{")),
            ("gateway_malformed", lambda: self.home.write("hermes-home/gateway_state.json", "")),
            ("hermes_inbox_unknown_lease", lambda: self.home.write(
                "hermes-inbox.json", {"events": [event({"state": "expired"})]})),
            ("hermes_inbox_unknown_lease", lambda: self.home.write(
                "hermes-inbox.json", {"events": [event({"state": "leased", "leased_at_ms": 1})]})),
            ("hermes_inbox_malformed", lambda: self.home.write(
                "hermes-inbox.json", {"events": [event({"state": "leased", "lease_id": "x", "leased_at_ms": "1"})]})),
            ("hermes_inbox_malformed", lambda: self.home.write("hermes-inbox.json", {"events": [event(None) | {"lease": None}]})),
            ("hermes_inbox_malformed", lambda: self.home.write("hermes-inbox.json", {"events": {}})),
            ("hermes_inbox_malformed", lambda: self.home.write("hermes-inbox.json", {"events": [7]})),
            ("agentd_inbox_malformed", lambda: self.home.write("agentd-inbox.json", [])),
            ("agentd_inbox_malformed", lambda: self.home.write("agentd-inbox.json", "0")),
            ("running_markers_malformed", lambda: self.home.write("hermes-running.json", {"messages": None})),
            ("running_markers_malformed", lambda: self.home.write("hermes-running.json", b"\xff".decode("latin-1"))),
        ]
        for reason, arrange in cases:
            with self.subTest(reason):
                self.tearDown(); self.setUp()
                arrange()
                result = self.observe()
                self.assertEqual((result["verdict"], result["reasons"]), ("unknown", [reason]))

    def test_unknown_keeps_observed_busy_signals(self) -> None:
        self.home.gateway(active=3)
        self.home.write("agentd-inbox.json", "not json")
        result = self.observe()
        self.assertEqual((result["verdict"], result["reasons"]), ("unknown", ["agentd_inbox_malformed", "active_agents"]))

    def test_symlinks_special_files_oversize_and_missing_roots_fail_unknown(self) -> None:
        outside = Path(self.temporary.name).resolve() / "outside.json"
        outside.write_text(json.dumps({"events": []}))
        cases = [
            ("gateway_symlink", lambda: self._replace_with_symlink("hermes-home/gateway_state.json", outside)),
            ("hermes_inbox_symlink", lambda: self._replace_with_symlink("hermes-inbox.json", outside)),
            ("agentd_inbox_symlink", lambda: (self.home.agent / "agentd-inbox.json").symlink_to(outside)),
            ("running_markers_symlink", lambda: (self.home.agent / "hermes-running.json").symlink_to(
                Path(self.temporary.name) / "dangling")),
            ("agentd_inbox_not_regular", lambda: os.mkfifo(self.home.agent / "agentd-inbox.json")),
            ("running_markers_not_regular", lambda: (self.home.agent / "hermes-running.json").mkdir()),
            ("agent_root_symlink", lambda: self._replace_with_symlink("hermes-home", outside.parent)),
            ("agent_root_missing", lambda: os.rename(self.home.agent, self.home.root / "moved")),
        ]
        for reason, arrange in cases:
            with self.subTest(reason):
                self.tearDown(); self.setUp()
                outside.parent.mkdir(exist_ok=True); outside.write_text(json.dumps({"events": []}))
                arrange()
                result = self.observe()
                self.assertEqual((result["verdict"], result["reasons"]), ("unknown", [reason]))
        self.tearDown(); self.setUp()
        with mock.patch.object(idle, "MAX_STATE_BYTES", 64):
            self.home.write("hermes-inbox.json", {"events": [], "pad": "x" * 64})
            self.assertEqual(self.observe()["reasons"], ["gateway_oversize", "hermes_inbox_oversize"])

    def test_an_entry_acked_within_the_quiet_window_is_busy(self) -> None:
        # A /bg or /btw command entry is acked when its child launches; the
        # old Runtime keeps no other trace of a /btw child.
        for acked_ms, verdict in ((NOW_MS - 60_000, "busy"), (NOW_MS - 1_799_000, "busy"),  # 29 min 59 s
                                  (NOW_MS - 1_800_000, "idle"), (NOW_MS + 5_000, "busy")):
            with self.subTest(age_ms=NOW_MS - acked_ms):
                self.home.write("hermes-inbox.json", {"events": [], "acked": [
                    {"key": SECRET, "acked_at_ms": 1}, {"key": SECRET + "2", "acked_at_ms": acked_ms}]})
                result = self.observe()
                self.assertEqual((result["verdict"], result["reasons"]),
                                 (verdict, ["recent_ack"] if verdict == "busy" else []))
        self.assertEqual(self.observe()["hermes_inbox"]["newest_ack_age_s"], 0)

    def test_open_bg_session_of_the_current_gateway_is_busy_at_any_age(self) -> None:
        self.home.session(GATEWAY_START_S + 1)
        result = self.observe()
        self.assertEqual((result["verdict"], result["reasons"]), ("busy", ["background_sessions"]))
        self.assertEqual(result["gateway"]["background"], {"gateway_started_age_s": 3600, "open": 1,
                                                           "stale_open": 0, "recently_ended": 0})
        self.tearDown(); self.setUp()
        self.home.session(GATEWAY_START_S + 1, sid="20260921_120000_" + SECRET)  # a chat session, not /bg
        self.assertEqual(self.observe()["verdict"], "idle")

    def test_open_bg_session_from_an_earlier_gateway_process_is_reported_not_busy(self) -> None:
        # A stop or crash leaves a killed child's row open forever.
        self.home.session(GATEWAY_START_S - 5)
        result = self.observe()
        self.assertEqual((result["verdict"], result["gateway"]["background"]["stale_open"]), ("idle", 1))
        self.home.pid(GATEWAY_START_S - 10)  # the same row under the process that ran it
        self.assertEqual(self.observe()["reasons"], ["background_sessions"])
        self.tearDown(); self.setUp()
        self.home.session(GATEWAY_START_S)  # launched in the same instant the process started
        self.assertEqual(self.observe()["gateway"]["background"]["open"], 1)

    def test_bg_session_ended_within_the_delivery_window_is_busy(self) -> None:
        # The row is ended before the result is sent to the chat; the window is 5 minutes.
        for ended_ago_s, verdict in ((10, "busy"), (299, "busy"), (300, "busy"), (301, "idle")):
            with self.subTest(ended_ago_s=ended_ago_s):
                self.tearDown(); self.setUp()
                self.home.session(GATEWAY_START_S + 1, NOW_MS / 1000 - ended_ago_s)
                self.assertEqual(self.observe()["verdict"], verdict)

    def test_guest_times_are_compared_on_the_guest_clock(self) -> None:
        # Each case is true in host time; the guest stamps it skew_s behind (negative: ahead).
        now_s = NOW_MS / 1000

        def ack(host_age_s: float, skew_s: float) -> None:
            self.home.write("hermes-inbox.json", {"events": [], "acked": [
                {"key": SECRET, "acked_at_ms": int((now_s - host_age_s - skew_s) * 1000)}]})

        cases = [
            ("ack 29m59s ago, guest 30s behind", 30, lambda: ack(1799, 30), ["recent_ack"]),
            ("ack 30m ago, guest 30s behind", 30, lambda: ack(1800, 30), []),
            ("ack 30m30s ago, guest 60s ahead", -60, lambda: ack(1830, -60), []),
            ("/bg replayed 3s after the gateway started, guest 10s behind", 10,
             lambda: self.home.session(GATEWAY_START_S + 3 - 10), ["background_sessions"]),
            ("/bg ended 4m59s ago, guest 30s behind", 30,
             lambda: self.home.session(GATEWAY_START_S + 60 - 30, now_s - 299 - 30), ["background_sessions"]),
            ("/bg ended 5m1s ago, guest 30s behind", 30,
             lambda: self.home.session(GATEWAY_START_S + 60 - 30, now_s - 301 - 30), []),
            ("child of the previous process killed 5s before restart, guest 10s ahead", -10,
             lambda: self.home.session(GATEWAY_START_S - 5 + 10), []),
        ]
        for label, skew_s, arrange, reasons in cases:
            with self.subTest(label):
                self.tearDown(); self.setUp()
                self.home.gateway(guest_ahead_s=-skew_s)
                arrange()
                result = self.observe()
                self.assertEqual((result["verdict"], result["reasons"]), ("busy" if reasons else "idle", reasons))
                self.assertEqual(result["gateway"]["clock_offset_ms"], skew_s * 1000)

    def test_an_unmeasured_or_large_clock_offset_is_unknown(self) -> None:
        for guest_ahead_s, reasons in ((60, []), (-60, []), (61, ["clock_skew"]), (-61, ["clock_skew"])):
            with self.subTest(guest_ahead_s=guest_ahead_s):
                self.home.gateway(guest_ahead_s=guest_ahead_s)
                result = self.observe()
                self.assertEqual((result["verdict"], result["reasons"]), ("unknown" if reasons else "idle", reasons))
                self.assertEqual(result["gateway"]["clock_offset_ms"], -guest_ahead_s * 1000)
        for updated_at in (DROP, None, 7, "", "yesterday", "2026-09-21T10:00:00"):  # the last has no zone
            with self.subTest(updated_at=updated_at):
                self.home.gateway(updated_at=updated_at)
                result = self.observe()
                self.assertEqual((result["verdict"], result["reasons"]), ("unknown", ["clock_unavailable"]))
                self.assertIsNone(result["gateway"]["clock_offset_ms"])
        self.home.gateway(active=1, updated_at=None)
        self.assertEqual(self.observe()["reasons"], ["clock_unavailable", "active_agents"])

    def test_an_inbox_written_within_the_quiet_window_is_busy_whatever_its_ack_stamps(self) -> None:
        # An ack is written no later than the inbox file's host mtime, so this holds
        # even if the guest clock drifted after gateway_state.json was last written.
        stale_ack = {"events": [], "acked": [{"key": SECRET, "acked_at_ms": NOW_MS - 2_400_000}]}
        for written_ago_s, verdict in ((600, "busy"), (1799, "busy"), (1800, "idle")):
            with self.subTest(written_ago_s=written_ago_s):
                self.home.write("hermes-inbox.json", stale_ack, NOW_MS / 1000 - written_ago_s)
                result = self.observe()
                self.assertEqual((result["verdict"], result["reasons"]),
                                 (verdict, ["recent_ack"] if verdict == "busy" else []))
                self.assertEqual(result["hermes_inbox"]["written_age_s"], written_ago_s)

    def test_rows_still_in_the_wal_are_read_from_a_scratch_copy_without_touching_live_files(self) -> None:
        home = self.home.agent / "hermes-home"
        writer = sqlite3.connect(home / "state.db")
        try:
            writer.execute("PRAGMA journal_mode=WAL")
            writer.execute("PRAGMA wal_autocheckpoint=0")
            writer.execute("INSERT INTO sessions VALUES ('bg_1', 'finitechat', ?, NULL, NULL)", (GATEWAY_START_S + 1,))
            writer.commit()
            before = {p.name: (p.stat().st_mtime_ns, p.stat().st_size) for p in home.iterdir()}
            self.assertGreater(before["state.db-wal"][1], 0)
            self.assertEqual(self.observe()["reasons"], ["background_sessions"])
            self.assertEqual({p.name: (p.stat().st_mtime_ns, p.stat().st_size) for p in home.iterdir()}, before)
        finally:
            writer.close()

    def test_missing_unsafe_or_unreadable_session_state_fails_unknown(self) -> None:
        def home() -> Path:
            return self.home.agent / "hermes-home"

        def outside() -> Path:
            return Path(self.temporary.name).resolve() / "outside.db"

        cases = [
            ("background_pid_missing", lambda: (home() / "gateway.pid").unlink()),
            ("background_not_regular", lambda: ((home() / "gateway.pid").unlink(),
                                                (home() / "gateway.pid").symlink_to(outside()))),
            ("background_missing", lambda: (home() / "state.db").unlink()),
            ("background_symlink", lambda: (os.rename(home() / "state.db", outside()),
                                            (home() / "state.db").symlink_to(outside()))),
            ("background_not_regular", lambda: ((home() / "state.db").unlink(), (home() / "state.db").mkdir())),
            ("background_malformed", lambda: (home() / "state.db").write_bytes(b"not a database" * 100)),
            ("background_malformed", lambda: self._drop_sessions_table()),
        ]
        for reason, arrange in cases:
            with self.subTest(reason):
                self.tearDown(); self.setUp()
                arrange()
                result = self.observe()
                self.assertEqual((result["verdict"], result["reasons"]), ("unknown", [reason]))
        self.tearDown(); self.setUp()
        with mock.patch.object(idle, "MAX_SESSION_DB_BYTES", 64):
            self.assertEqual(self.observe()["reasons"], ["background_oversize"])
        with mock.patch.dict(sys.modules, {"sqlite3": None}):
            self.assertEqual(self.observe()["reasons"], ["background_sqlite_unavailable"])

    def test_a_copy_torn_by_a_live_writer_is_retried_then_unknown(self) -> None:
        real = idle._copy_stable
        calls = []

        def torn(directory, name, target, required):
            calls.append(name)
            if len(calls) == 1:
                raise idle.Unreadable("unstable")
            return real(directory, name, target, required)

        with mock.patch.object(idle, "_copy_stable", torn):
            self.assertEqual(self.observe()["verdict"], "idle")
        with mock.patch.object(idle, "_copy_stable", side_effect=idle.Unreadable("unstable")) as always:
            self.assertEqual(self.observe()["reasons"], ["background_unstable"])
        self.assertEqual(always.call_count, 3)

    def test_a_live_write_during_the_copy_is_unstable_never_read(self) -> None:
        live = self.home.agent / "hermes-home" / "state.db"
        real_read = os.read
        bumps = iter(range(1, 1_000_000))

        def read_while_writer_commits(fd, size):
            data = real_read(fd, size)
            stamp = int(GATEWAY_START_S * 1e9) + next(bumps)
            os.utime(live, ns=(stamp, stamp))
            return data

        with mock.patch.object(idle.os, "read", read_while_writer_commits):
            self.assertEqual(self.observe()["reasons"], ["background_unstable"])

    def test_a_checkpoint_between_the_db_and_wal_copies_is_unstable_never_idle(self) -> None:
        # Hermes checkpoints PASSIVE every 50 writes; the next write then resets
        # the WAL in place (same size), so a db copied before the checkpoint and a
        # WAL copied after it no longer carry the open /bg row between them.
        home = self.home.agent / "hermes-home"
        writer = sqlite3.connect(home / "state.db", isolation_level=None)
        real = idle._copy_stable
        copies = []

        def checkpoint_after_db_copy(directory, name, target, required):
            signature = real(directory, name, target, required)
            copies.append(name)
            if name == "state.db" and len(copies) == 1:
                writer.execute("PRAGMA wal_checkpoint(PASSIVE)")
                writer.execute("INSERT INTO messages VALUES (1, 'a later turn on another page')")
            return signature

        try:
            writer.execute("PRAGMA journal_mode=WAL")
            writer.execute("PRAGMA wal_autocheckpoint=0")
            writer.execute("CREATE TABLE messages (id INTEGER PRIMARY KEY, body TEXT)")
            writer.execute("PRAGMA wal_checkpoint(TRUNCATE)")
            writer.execute("INSERT INTO sessions VALUES ('bg_1', 'finitechat', ?, NULL, NULL)", (GATEWAY_START_S + 1,))
            with mock.patch.object(idle, "_copy_stable", checkpoint_after_db_copy):
                result = self.observe()
            self.assertEqual((result["verdict"], result["reasons"]), ("busy", ["background_sessions"]))
            self.assertEqual(copies, ["state.db", "state.db-wal", "state.db", "state.db-wal"])
        finally:
            writer.close()

    def test_a_db_closed_or_replaced_between_the_copies_is_retried(self) -> None:
        home = self.home.agent / "hermes-home"
        writer = sqlite3.connect(home / "state.db", isolation_level=None)
        writer.execute("PRAGMA journal_mode=WAL")
        writer.execute("PRAGMA wal_autocheckpoint=0")
        writer.execute("INSERT INTO sessions VALUES ('bg_1', 'finitechat', ?, NULL, NULL)", (GATEWAY_START_S + 1,))
        real = idle._copy_stable
        events = iter(["close", "replace"])
        copies = []

        def change_after_db_copy(directory, name, target, required):
            signature = real(directory, name, target, required)
            copies.append(name)
            event = next(events, None) if name == "state.db" else None
            if event == "close":  # the last connection checkpoints and removes the WAL
                writer.close()
            elif event == "replace":  # a new inode under the same name, same size and mtime
                replacement = home / "replacement.db"
                replacement.write_bytes((home / "state.db").read_bytes())
                info = (home / "state.db").stat()
                os.utime(replacement, ns=(info.st_atime_ns, info.st_mtime_ns))
                os.replace(replacement, home / "state.db")
            return signature

        with mock.patch.object(idle, "_copy_stable", change_after_db_copy):
            result = self.observe()
        self.assertEqual((result["verdict"], result["reasons"]), ("busy", ["background_sessions"]))
        self.assertEqual(copies, ["state.db", "state.db-wal"] * 3)  # each change was retried

    def test_a_file_renamed_over_the_source_mid_copy_is_unstable(self) -> None:
        # The open descriptor still reads the old inode, so only a by-name check sees it.
        home = self.home.agent / "hermes-home"
        writer = sqlite3.connect(home / "state.db")
        try:
            writer.execute("PRAGMA journal_mode=WAL")
            writer.execute("PRAGMA wal_autocheckpoint=0")
            writer.execute("INSERT INTO sessions VALUES ('bg_1', 'finitechat', ?, NULL, NULL)", (GATEWAY_START_S + 1,))
            writer.commit()
            wal = (home / "state.db-wal").read_bytes()
            real_read = os.read

            def rename_during_wal_copy(fd, size):
                data = real_read(fd, size)
                if os.fstat(fd).st_size == len(wal) and data:
                    (home / "next.db-wal").write_bytes(wal)
                    os.replace(home / "next.db-wal", home / "state.db-wal")
                return data

            with mock.patch.object(idle.os, "read", rename_during_wal_copy):
                self.assertEqual(self.observe()["reasons"], ["background_unstable"])
        finally:
            writer.close()

    def test_a_wal_larger_than_one_copy_chunk_is_read_to_its_last_frame(self) -> None:
        home = self.home.agent / "hermes-home"
        writer = sqlite3.connect(home / "state.db", isolation_level=None)
        try:
            writer.execute("PRAGMA journal_mode=WAL")
            writer.execute("PRAGMA wal_autocheckpoint=0")
            writer.execute("CREATE TABLE messages (id INTEGER PRIMARY KEY, body BLOB)")
            for _ in range(40):
                writer.execute("INSERT INTO messages (body) VALUES (?)", (os.urandom(60_000),))
            writer.execute("INSERT INTO sessions VALUES ('bg_1', 'finitechat', ?, NULL, NULL)", (GATEWAY_START_S + 1,))
            self.assertGreater((home / "state.db-wal").stat().st_size, 2 * 1024 * 1024)
            self.assertEqual(self.observe()["reasons"], ["background_sessions"])
        finally:
            writer.close()

    def test_short_or_failed_writes_never_read_as_an_older_state(self) -> None:
        home = self.home.agent / "hermes-home"
        writer = sqlite3.connect(home / "state.db", isolation_level=None)
        writer.execute("PRAGMA journal_mode=WAL")
        writer.execute("PRAGMA wal_autocheckpoint=0")
        writer.execute("CREATE TABLE messages (id INTEGER PRIMARY KEY, body BLOB)")
        for _ in range(40):
            writer.execute("INSERT INTO messages (body) VALUES (?)", (os.urandom(60_000),))
        writer.execute("INSERT INTO sessions VALUES ('bg_1', 'finitechat', ?, NULL, NULL)", (GATEWAY_START_S + 1,))
        self.addCleanup(writer.close)
        real_write = os.write

        def half(fd, data):  # a filling filesystem writes what fits and returns that count
            return real_write(fd, bytes(data)[: max(1, len(data) // 2)])

        def drops_tail(fd, data):  # a write that loses bytes yet reports them all
            real_write(fd, bytes(data)[:-1])
            return len(data)

        def full(fd, data):
            raise OSError(errno.ENOSPC, "no space")

        for write, expected in ((half, ("busy", ["background_sessions"])),
                                (drops_tail, ("unknown", ["background_unreadable"])),
                                (full, ("unknown", ["background_unreadable"]))):
            with self.subTest(write.__name__), mock.patch.object(idle.os, "write", write):
                result = self.observe()
                self.assertEqual((result["verdict"], result["reasons"]), expected)

    def test_copies_left_by_a_killed_read_are_removed_by_the_next_read(self) -> None:
        # SIGKILL, OOM or a reboot mid-copy skips cleanup and leaves the user's chat database in TMPDIR.
        now = time.time()
        outside = Path(self.temporary.name).resolve() / "outside"
        outside.mkdir()
        (outside / "keep").write_text(SECRET)

        def entry(name: str, age_s: float, file_age_s: float | None = None, kind: str = "dir") -> None:
            path = self.scratch / name
            if kind == "dir":
                path.mkdir()
                (path / "state.db").write_text(SECRET)
                stamp = now - (age_s if file_age_s is None else file_age_s)
                os.utime(path / "state.db", (stamp, stamp))
            elif kind == "file":
                path.write_text(SECRET)
            else:
                path.symlink_to(outside)
            os.utime(path, (now - age_s, now - age_s), follow_symlinks=kind != "link")

        entry("finite-status-idle.killed", 11 * 60)
        entry("finite-status-idle.recent", 9 * 60)
        entry("finite-status-idle.copying", 3600, file_age_s=0)  # a concurrent read's large copy
        entry("finite-status-sqlite.other", 3600)
        entry("finite-status-idle.file", 3600, kind="file")
        entry("finite-status-idle.link", 3600, kind="link")
        result = idle.observe(self.home.root, NOW_MS)
        self.assertEqual(result["verdict"], "idle")
        self.assertEqual(sorted(path.name for path in self.scratch.iterdir()), [
            "finite-status-idle.copying", "finite-status-idle.file", "finite-status-idle.link",
            "finite-status-idle.recent", "finite-status-sqlite.other"])
        self.assertEqual((outside / "keep").read_text(), SECRET)
        entry("finite-status-idle.another-user", 3600)
        with mock.patch.object(idle.os, "geteuid", return_value=os.geteuid() + 1):
            idle.observe(self.home.root, NOW_MS)
        self.assertTrue((self.scratch / "finite-status-idle.another-user").is_dir())

    def _drop_sessions_table(self) -> None:
        with contextlib.closing(sqlite3.connect(self.home.agent / "hermes-home" / "state.db")) as db:
            db.execute("DROP TABLE sessions")
            db.commit()

    @unittest.skipIf(os.geteuid() == 0, "root bypasses file modes")
    def test_permission_denied_is_unknown(self) -> None:
        path = self.home.agent / "hermes-home" / "gateway_state.json"
        path.chmod(0)
        try:
            self.assertEqual(self.observe()["reasons"], ["gateway_unreadable"])
        finally:
            path.chmod(0o600)

    def _replace_with_symlink(self, relative: str, target: Path) -> None:
        path = self.home.agent / relative
        if path.is_dir():
            for child in path.iterdir():
                child.unlink()
            path.rmdir()
        else:
            path.unlink()
        path.symlink_to(target)


class TargetTests(unittest.TestCase):
    """The exact assignment is proven by the lifecycle probe before any read."""

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.home = AgentRoot(Path(self.temporary.name).resolve())
        self.home.gateway(active=1)
        self.scratch = Path(self.temporary.name).resolve() / "tmp"
        self.scratch.mkdir()
        self.tempdir = mock.patch.object(tempfile, "tempdir", str(self.scratch))
        self.tempdir.start()

    def tearDown(self) -> None:
        left = list(self.scratch.iterdir())
        self.tempdir.stop()
        self.temporary.cleanup()
        self.assertEqual(left, [])

    def probe(self, verdict: str = "operable", state_root: object = None, machine: str = "machine-a") -> dict:
        checks = [{"name": name, "status": "pass", "detail": "qualified", "evidence": {}} for name in CHECKS]
        checks[0]["evidence"] = {"container_id": "a" * 64, "container_name": machine,
                                 "state_root": str(self.home.root) if state_root is None else state_root}
        if verdict != "operable":
            checks[1].update(status="fail", finding="control_channel_unavailable")
        return {"schema": "finite.lifecycle-probe.v1", "verdict": verdict,
                "reason": None if verdict == "operable" else "control_channel_unavailable",
                "runtime": {"project_id": "project-a", "agent_runtime_id": "runtime-a",
                            "source_machine_id": machine, "container_name": machine},
                "checks": checks}

    def run_idle(self, probe: dict, host: str = "finite-lat-3") -> dict:
        with (
            mock.patch.dict(finite_status.os.environ, {"FINITE_STATUS_LIFECYCLE_PROBE_BIN": "/bin/sh"}),
            mock.patch.object(finite_status.socket, "gethostname", return_value="finite-lat-3"),
            mock.patch.object(finite_status, "read_environment_values", return_value={}),
            mock.patch.object(finite_status, "run_read_only",
                              return_value=subprocess.CompletedProcess([], 0, json.dumps(probe), "")) as run,
        ):
            report = finite_status.collect_runtime_idle("project-a", "runtime-a", "machine-a", host)
        command = run.call_args.args[0]
        self.assertEqual(command[1:], ["lifecycle-probe", "--project-id", "project-a",
                                       "--agent-runtime-id", "runtime-a", "--source-machine-id", "machine-a"])
        self.assertNotIn(SECRET, json.dumps(report))
        return report

    def test_operable_exact_assignment_reads_its_durable_root(self) -> None:
        report = self.run_idle(self.probe())
        section = report["sections"]["runtime_idle"]
        self.assertEqual((report["overall_status"], report["exit_code"]), ("red", 1))
        self.assertEqual((section["verdict"], section["reasons"]), ("busy", ["active_agents"]))
        self.assertEqual(section["lifecycle"], {"verdict": "operable", "reason": None})
        self.home.gateway(active=0)
        report = self.run_idle(self.probe())
        self.assertEqual((report["overall_status"], report["exit_code"]), ("green", 0))
        self.assertEqual(set(report["sections"]["runtime_idle"]), {
            "status", "project_id", "agent_runtime_id", "source_machine_id", "source_host_id", "lifecycle",
            "verdict", "reasons", "gateway", "hermes_inbox", "agentd_inbox", "running_markers"})

    def test_wrong_target_or_unproven_root_is_unknown_without_reading_state(self) -> None:
        other = AgentRoot(Path(self.temporary.name).resolve() / "other", "runtime-b")
        link = Path(self.temporary.name).resolve() / "linked" / "kata" / "runtime-a"
        link.parent.mkdir(parents=True)
        link.symlink_to(self.home.root)
        cases = [
            (self.probe(machine="machine-b"), "lifecycle_not_operable"),
            (self.probe("degraded"), "lifecycle_not_operable"),
            (self.probe(state_root=str(other.root)), "state_root_mismatch"),
            (self.probe(state_root=str(self.home.agent)), "state_root_mismatch"),
            (self.probe(state_root="kata/runtime-a"), "state_root_mismatch"),
            (self.probe(state_root=str(link)), "state_root_mismatch"),
            (self.probe(state_root=str(self.home.root / ".." / "runtime-a")), "state_root_mismatch"),
            (self.probe(state_root=None) | {"checks": []}, "lifecycle_not_operable"),
        ]
        for probe, reason in cases:
            with self.subTest(reason), mock.patch.object(idle, "observe") as observe:
                report = self.run_idle(probe)
                observe.assert_not_called()
                section = report["sections"]["runtime_idle"]
                self.assertEqual((report["exit_code"], section["verdict"], section["reasons"]),
                                 (2, "unknown", [reason]))

    def test_wrong_host_refuses_before_provider_access(self) -> None:
        with (
            mock.patch.object(finite_status.socket, "gethostname", return_value="finite-lat-4"),
            mock.patch.object(finite_status, "run_read_only") as run,
        ):
            with self.assertRaises(finite_status.CollectionError):
                finite_status.collect_runtime_idle("project-a", "runtime-a", "machine-a", "finite-lat-3")
        run.assert_not_called()

    def test_cli_emits_json_and_requires_four_simple_identifiers(self) -> None:
        stdout = io.StringIO()
        with (
            mock.patch.object(finite_status, "collect_runtime_idle",
                              return_value={"exit_code": 0, "sections": {}}) as collect,
            contextlib.redirect_stdout(stdout),
            self.assertRaises(SystemExit) as exit_,
        ):
            finite_status.main(["--runtime-idle", "project-a", "runtime-a", "machine-a", "finite-lat-3"])
        collect.assert_called_once_with("project-a", "runtime-a", "machine-a", "finite-lat-3")
        self.assertEqual((exit_.exception.code, json.loads(stdout.getvalue())["exit_code"]), (0, 0))
        for arguments in (["project-a", "../runtime", "machine-a", "finite-lat-3"],
                          ["project-a", "runtime-a", "machine-a"]):
            with self.subTest(arguments), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                finite_status.parse_args(["--runtime-idle", *arguments])
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            finite_status.parse_args(["--runtime-idle", "p", "r", "m", "h", "--runtime-lifecycle", "p", "r", "m", "h"])


if __name__ == "__main__":
    unittest.main()

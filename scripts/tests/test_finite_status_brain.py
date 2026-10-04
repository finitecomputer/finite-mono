from __future__ import annotations

import hashlib
import contextlib
import io
import json
import sqlite3
import tempfile
import subprocess
import sys
import unittest
from pathlib import Path

from scripts.finite_status import parse_args, scratch_copy_sqlite
from scripts.finite_status_brain import collect


class BrainGrantCoverageTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.database = Path(self.temporary.name) / "authority.sqlite3"
        with contextlib.closing(sqlite3.connect(self.database)) as connection, connection:
            connection.executescript("""
                CREATE TABLE brains(id, kind, owner_user_id);
                CREATE TABLE brain_members(brain_id, user_id);
                CREATE TABLE brain_admins(brain_id, user_id);
                CREATE TABLE personal_agents(brain_id, agent_npub, status);
                CREATE TABLE folders(brain_id, id, path, access, current_key_version);
                CREATE TABLE folder_access(brain_id, folder_id, user_id);
                CREATE TABLE folder_access_sources(brain_id, folder_id, user_id, source_kind);
                CREATE TABLE folder_key_grants(brain_id, folder_id, key_version, recipient_npub);
                CREATE TABLE shared_folder_connections(id, source_brain_id, destination_brain_id, status);
                INSERT INTO brains VALUES('org', 'organization', NULL);
                INSERT INTO brain_members VALUES('org', 'admin'), ('org', 'member');
                INSERT INTO brain_admins VALUES('org', 'admin');
                INSERT INTO folders VALUES('org', 'private', 'Private', 'restricted', 2);
                INSERT INTO folder_key_grants VALUES('org', 'private', 2, 'admin'),
                    ('org', 'private', 2, 'member'), ('org', 'private', 2, 'removed'),
                    ('org', 'private', 1, 'historical');
            """)

    def report(self, brain="org"):
        before = hashlib.sha256(self.database.read_bytes()).hexdigest()
        report = collect(brain, self.database, scratch_copy_sqlite, "2026-10-03T00:00:00Z")
        self.assertEqual(before, hashlib.sha256(self.database.read_bytes()).hexdigest())
        return report

    def test_current_unentitled_member_and_removed_recipient_are_both_reported(self):
        report = self.report()
        self.assertEqual(report["overall_status"], "warning")
        self.assertFalse(report["current_access_complete"])
        self.assertEqual(report["folders"][0]["unentitled_current_grants"], ["member", "removed"])
        self.assertEqual(report["folders"][0]["missing_current_grants"], [])
        self.assertNotIn("historical", str(report))
        self.assertEqual(report, self.report())

    def test_missing_survivor_grant_and_mount_scope_cannot_look_complete(self):
        with contextlib.closing(sqlite3.connect(self.database)) as connection, connection:
            connection.execute("DELETE FROM folder_key_grants WHERE recipient_npub = 'admin'")
            connection.execute("INSERT INTO shared_folder_connections VALUES('mount', 'source', 'org', 'active')")
        report = self.report()
        self.assertEqual(report["overall_status"], "unknown")
        self.assertEqual(report["coverage"]["mounts"], "unverified")
        self.assertEqual(report["folders"][0]["missing_current_grants"], ["admin"])

    def test_native_all_members_coverage_can_pass_and_ignores_historical_grants(self):
        with contextlib.closing(sqlite3.connect(self.database)) as connection, connection:
            connection.execute("DELETE FROM folder_key_grants WHERE recipient_npub = 'removed'")
            connection.execute("UPDATE folders SET access = 'all_members'")
        report = self.report()
        self.assertEqual(report["exit_code"], 0)
        self.assertTrue(report["current_access_complete"])

    def test_personal_owner_agent_and_admin_entitlements_follow_folder_policy(self):
        with contextlib.closing(sqlite3.connect(self.database)) as connection, connection:
            connection.execute("UPDATE brains SET kind = 'personal', owner_user_id = 'owner'")
            connection.execute("INSERT INTO personal_agents VALUES('org', 'agent', 'active')")
            connection.execute("DELETE FROM folder_key_grants")
        report = self.report()
        self.assertEqual(report["folders"][0]["missing_current_grants"], ["admin", "agent", "owner"])
        with contextlib.closing(sqlite3.connect(self.database)) as connection, connection:
            connection.execute("UPDATE folders SET access = 'owner'")
        self.assertEqual(self.report()["folders"][0]["missing_current_grants"], ["agent", "owner"])

    def test_retained_mount_provenance_is_unverified_without_active_connection(self):
        with contextlib.closing(sqlite3.connect(self.database)) as connection, connection:
            connection.execute("INSERT INTO folder_access_sources VALUES('org', 'private', 'member', 'mount')")
        self.assertEqual(self.report()["coverage"]["mounts"], "unverified")

    def test_live_wal_is_copied_before_reading_and_original_is_unchanged(self):
        with contextlib.closing(sqlite3.connect(self.database)) as connection:
            connection.execute("PRAGMA journal_mode = WAL")
            connection.execute("INSERT INTO folder_key_grants VALUES('org', 'private', 2, 'wal-recipient')")
            connection.commit()
            self.assertTrue(self.database.with_name("authority.sqlite3-wal").exists())
            report = self.report()
            self.assertIn("wal-recipient", report["folders"][0]["unentitled_current_grants"])

    def test_capacity_and_absent_brain_fail_without_truncating(self):
        self.assertEqual(self.report("absent")["overall_status"], "unknown")
        with contextlib.closing(sqlite3.connect(self.database)) as connection, connection:
            connection.executemany("INSERT INTO brain_members VALUES('org', ?)", [(str(i),) for i in range(1000)])
        report = self.report()
        self.assertEqual(report["overall_status"], "unknown")
        self.assertNotIn("folders", report)

    def test_canonical_command_exposes_only_an_exact_id_and_explicit_database(self):
        options = parse_args(["--brain-access", "org", "--brain-database", str(self.database)])
        self.assertEqual(options.brain_access, "org")
        self.assertEqual(options.brain_database, self.database)
        with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
            parse_args(["--brain-access", "org' OR 1=1"])

    def test_actual_command_emits_scoped_json_for_results_and_unreadable_state(self):
        command = Path(__file__).resolve().parents[1] / "finite-status"
        for path, exit_code in [(self.database, 1), (self.database.with_name("absent.sqlite3"), 2)]:
            result = subprocess.run(
                [sys.executable, str(command), "--brain-access", "org", "--brain-database", str(path)],
                capture_output=True, text=True, timeout=15,
            )
            self.assertEqual(result.returncode, exit_code, result.stderr)
            report = json.loads(result.stdout)
            self.assertEqual(report["brain_id"], "org")
            self.assertFalse(report["current_access_complete"])

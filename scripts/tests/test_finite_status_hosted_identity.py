from __future__ import annotations

import contextlib
import io
import json
import sqlite3
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from scripts import finite_status
from scripts import finite_status_brain_roster as roster
from scripts import finite_status_hosted_identity as census

KEY = "a1" * 32
OTHER = "b2" * 32
SECRET = "NEVER-EXPORT-IDENTITY-SECRET-OR-MESSAGE-BODY"


class HostedCensusTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "users").mkdir()
        self.accounts = []

    def add_root(self, number=1, key=KEY, stored=None, account=True):
        storage = f"{number:064x}"
        home = self.root / "users" / storage
        identity = home / "finite-home/identity/identity.json"
        identity.parent.mkdir(parents=True)
        identity.write_text(json.dumps({"public_key_hex": key, "secret_key_hex": SECRET}))
        store = home / "chat/client.sqlite3"
        store.parent.mkdir()
        with contextlib.closing(sqlite3.connect(store)) as connection:
            connection.execute("CREATE TABLE client_device_states (account_id TEXT, device_id TEXT, ciphertext TEXT)")
            for value in [key] if stored is None else stored:
                connection.execute("INSERT INTO client_device_states VALUES (?, 'hosted-web', ?)", (value, SECRET))
            connection.commit()
        if account:
            self.accounts.append({"storage": storage, "account": f"account-{number}",
                                  "email": f"{number}@example.test", "link_status": "linked"})
        return home, identity, store

    def collect(self, **kwargs):
        return census.collect(self.root, [KEY], self.accounts,
                              finite_status.scratch_copy_sqlite, finite_status.sqlite_json_query, **kwargs)

    def test_current_selector_match_is_load_only_and_secret_free(self):
        _, identity, store = self.add_root()
        before = identity.read_bytes(), store.read_bytes()
        result = self.collect()
        self.assertTrue(result["coverage"]["complete"])
        self.assertTrue(result["roots"][0]["matches"][0]["currentAccountCandidate"])
        self.assertEqual(result["duplicateClaims"], {})
        self.assertNotIn(SECRET, json.dumps(result))
        self.assertEqual(before, (identity.read_bytes(), store.read_bytes()))

    def test_orphan_duplicate_prevents_unique_mapping(self):
        self.add_root()
        self.add_root(2, account=False)
        result = self.collect()
        self.assertTrue(result["coverage"]["complete"])
        self.assertEqual(len(result["duplicateClaims"][KEY]), 2)
        self.assertIsNone(result["roots"][1]["matches"][0]["account"])

    def test_stale_or_multi_key_store_is_not_current_identity_proof(self):
        self.add_root(stored=[KEY, OTHER])
        result = self.collect()
        self.assertFalse(result["roots"][0]["selectorMatchesStore"])
        self.assertFalse(result["roots"][0]["matches"][0]["currentAccountCandidate"])

    def test_non_target_public_keys_and_contacts_are_not_returned(self):
        self.add_root(key=OTHER)
        result = self.collect()
        self.assertTrue(result["coverage"]["complete"])
        self.assertEqual(result["roots"][0]["matches"], [])
        self.assertNotIn(OTHER, json.dumps(result))
        self.assertNotIn("1@example.test", json.dumps(result))

    def test_missing_counterpart_and_symlink_fail_coverage(self):
        home, identity, _ = self.add_root()
        identity.unlink()
        self.assertFalse(self.collect()["coverage"]["complete"])
        identity.symlink_to(home / "nonexistent")
        self.assertFalse(self.collect()["coverage"]["complete"])

    def test_identity_change_across_copy_is_detected(self):
        _, identity, _ = self.add_root()
        original = census.public_selector
        calls = 0

        def changing(path):
            nonlocal calls
            calls += 1
            if calls == 2:
                identity.write_text(json.dumps({"public_key_hex": OTHER, "secret": SECRET}))
            return original(path)

        with mock.patch.object(census, "public_selector", side_effect=changing):
            result = self.collect()
        self.assertFalse(result["coverage"]["complete"])
        self.assertNotIn(SECRET, json.dumps(result))

    def test_size_time_and_space_guards_mark_incomplete(self):
        self.add_root()
        with mock.patch.object(census, "MAX_TOTAL_BYTES", 1):
            self.assertFalse(self.collect()["coverage"]["complete"])
        self.assertFalse(self.collect(free_bytes=lambda: 0)["coverage"]["complete"])
        ticks = iter([0, census.BUDGET_SECONDS + 1])
        self.assertFalse(self.collect(clock=lambda: next(ticks))["coverage"]["complete"])

    def test_root_churn_and_query_failure_do_not_claim_complete(self):
        self.add_root()
        original = finite_status.sqlite_json_query

        def changed(database, sql):
            (self.root / "users" / ("ff" * 32)).mkdir(exist_ok=True)
            return original(database, sql)

        with mock.patch.object(finite_status, "sqlite_json_query", side_effect=changed):
            self.assertFalse(self.collect()["coverage"]["complete"])
        with mock.patch.object(finite_status, "sqlite_json_query", side_effect=RuntimeError(SECRET)):
            result = self.collect()
        self.assertFalse(result["coverage"]["complete"])
        self.assertNotIn(SECRET, json.dumps(result))

    def test_malformed_identity_and_root_bounds_fail_closed(self):
        _, identity, _ = self.add_root()
        identity.write_text(SECRET)
        result = self.collect()
        self.assertFalse(result["coverage"]["complete"])
        self.assertNotIn(SECRET, json.dumps(result))
        with mock.patch.object(census, "MAX_ROOTS", 0), self.assertRaises(census.CensusError):
            self.collect()

    def wrapper(self, key_text=None, stdout=None, returncode=0):
        key_file = self.root / "keys.json"
        key_file.write_text(json.dumps([KEY]) if key_text is None else key_text)
        payload = {"checked_at": "2026-10-21T00:00:00Z", "accounts": self.accounts}
        response = mock.Mock(returncode=returncode,
                             stdout=json.dumps(payload) if stdout is None else stdout,
                             stderr=SECRET)
        original = finite_status.run_read_only

        def only_psql(command, **kwargs):
            return response if command[0] == "psql" else original(command, **kwargs)

        with mock.patch.object(finite_status, "run_read_only", side_effect=only_psql) as run, \
                mock.patch.object(finite_status, "postgres_environment", return_value={}), \
                mock.patch.object(roster, "HOSTED_DATA_ROOT", self.root):
            return finite_status.collect_brain_hosted_census(key_file), run

    def test_wrapper_collects_read_only_and_removes_scratch(self):
        _, identity, store = self.add_root()
        before = identity.read_bytes(), store.read_bytes()
        copies = []
        original = finite_status.scratch_copy_sqlite

        @contextlib.contextmanager
        def track_copy(database):
            with original(database) as scratch:
                copies.append(scratch.parent)
                yield scratch

        with mock.patch.object(finite_status, "scratch_copy_sqlite", side_effect=track_copy):
            report, run = self.wrapper()
        self.assertTrue(report["coverage"]["complete"])
        self.assertTrue(report["roots"][0]["matches"][0]["currentAccountCandidate"])
        self.assertEqual(report["evidence"]["coreCheckedAt"], "2026-10-21T00:00:00Z")
        self.assertEqual(report["evidence"]["hostedDataRoot"], str(self.root))
        self.assertNotIn(SECRET, json.dumps(report))
        self.assertEqual(before, (identity.read_bytes(), store.read_bytes()))
        self.assertTrue(copies)
        self.assertTrue(all(not directory.exists() for directory in copies))
        calls = [call for call in run.call_args_list if call.args[0][0] == "psql"]
        self.assertEqual(len(calls), 1)
        sql = calls[0].kwargs["input_text"]
        self.assertIn("REPEATABLE READ READ ONLY", sql)
        self.assertTrue(sql.rstrip().endswith("ROLLBACK;"))
        self.assertIn("ON_ERROR_STOP=1", calls[0].args[0])

    def test_wrapper_rejects_invalid_and_oversized_key_input_before_psql(self):
        invalid = [json.dumps(value) for value in (
            [], {}, [KEY.upper()], [KEY, KEY], [KEY[:-1]], [None],
            [f"{number:064x}" for number in range(513)])]
        invalid += [SECRET, " " * (64 * 1024) + SECRET]
        key_file = self.root / "invalid.json"
        for content in invalid:
            with self.subTest(length=len(content)), \
                    mock.patch.object(finite_status, "run_read_only") as run:
                key_file.write_text(content)
                with self.assertRaises(finite_status.CollectionError) as caught:
                    finite_status.collect_brain_hosted_census(key_file)
                self.assertNotIn(SECRET, str(caught.exception))
                run.assert_not_called()
        key_file.unlink()
        with mock.patch.object(finite_status, "run_read_only") as run:
            with self.assertRaises(finite_status.CollectionError):
                finite_status.collect_brain_hosted_census(key_file)
            run.assert_not_called()

    def test_wrapper_accepts_key_count_ceiling(self):
        self.add_root()
        keys = [KEY] + [f"{number:064x}" for number in range(511)]
        report, _ = self.wrapper(json.dumps(keys))
        self.assertEqual(report["coverage"]["targetKeyCount"], 512)
        self.assertTrue(report["coverage"]["complete"])

    def test_wrapper_refuses_psql_failure_and_output_overflow_without_stderr(self):
        for arguments in ({"returncode": 1}, {"stdout": SECRET * (2 * 1024 * 1024 // len(SECRET) + 1)}):
            with self.subTest(arguments=list(arguments)), \
                    mock.patch.object(census, "collect") as collect:
                with self.assertRaises(finite_status.CollectionError) as caught:
                    self.wrapper(**arguments)
                self.assertEqual(str(caught.exception), "read-only hosted account census failed or exceeded bound")
                self.assertNotIn(SECRET, str(caught.exception))
                collect.assert_not_called()

    def test_wrapper_redacts_invalid_core_payload_before_store_access(self):
        payloads = [SECRET, "null", "[]", json.dumps({"checked_at": SECRET}),
                    json.dumps({"accounts": []}),
                    json.dumps({"accounts": [], "checked_at": SECRET}),
                    json.dumps({"accounts": {}, "checked_at": "2026-10-21T00:00:00Z"}),
                    json.dumps({"accounts": [SECRET], "checked_at": "2026-10-21T00:00:00Z"})]
        for payload in payloads:
            with self.subTest(payload_length=len(payload)), \
                    mock.patch.object(census, "collect") as collect:
                with self.assertRaises(finite_status.CollectionError) as caught:
                    self.wrapper(stdout=payload)
                self.assertNotIn(SECRET, str(caught.exception))
                collect.assert_not_called()

    def test_census_cli_routes_exact_input_and_preserves_coverage_exit_code(self):
        for exit_code in (0, 2):
            expected = {"exit_code": exit_code, "coverage": {"complete": exit_code == 0}}
            with self.subTest(exit_code=exit_code), \
                    mock.patch.object(finite_status, "collect_brain_hosted_census", return_value=expected) as collect, \
                    mock.patch.object(finite_status, "collect_live", side_effect=AssertionError("fleet read")), \
                    contextlib.redirect_stdout(io.StringIO()) as output:
                with self.assertRaises(SystemExit) as stopped:
                    finite_status.main(["--brain-hosted-census", "targets.json"])
            self.assertEqual(stopped.exception.code, exit_code)
            self.assertEqual(json.loads(output.getvalue()), expected)
            collect.assert_called_once_with(Path("targets.json"))


if __name__ == "__main__":
    unittest.main()

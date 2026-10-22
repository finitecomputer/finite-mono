from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock
from uuid import uuid4

from scripts import finite_status, finite_status_brain_bindings as bindings


class PrivateBindingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.input = Path(self.temp.name) / "accounts.json"
        self.input.write_text('["account-a"]')
        self.row = {"accountId": "account-a", "email": "a@example.test",
                    "linkStatus": "linked", "workosSubject": "user_synthetic"}
        self.payload = {"checkedAt": "2026-10-21T00:00:00Z", "bindings": [self.row]}

    def collect(self, payload=None, returncode=0):
        result = subprocess.CompletedProcess([], returncode, json.dumps(self.payload if payload is None else payload), "private diagnostic")
        with mock.patch.object(finite_status, "postgres_environment", return_value={}), \
                mock.patch.object(finite_status, "run_read_only", return_value=result) as run:
            report = finite_status.collect_brain_scoped_bindings(self.input)
        return report, run

    def test_exact_account_binding_and_missing_account_coverage(self):
        report, run = self.collect()
        self.assertEqual(report["bindings"], [self.row])
        self.assertEqual(report["exit_code"], 0)
        self.assertTrue(report["coverage"]["complete"])
        self.assertIn('account_ids=["account-a"]', run.call_args.args[0])
        self.assertEqual(run.call_args.kwargs["input_text"], bindings.QUERY)
        report, _ = self.collect({**self.payload, "bindings": []})
        self.assertFalse(report["coverage"]["complete"])
        self.assertEqual(report["missingAccountIds"], ["account-a"])
        self.assertEqual(report["exit_code"], 2)

    def test_untrusted_input_never_reaches_query_or_errors(self):
        for value in [[], ["account-a", "account-a"], ["account-a'); DROP TABLE users;--"],
                      ["a"] * 101, [None], {"accountId": "a"}, "private input"]:
            self.input.write_text(json.dumps(value))
            with mock.patch.object(finite_status, "run_read_only") as run:
                with self.assertRaisesRegex(finite_status.CollectionError, "^private Core account binding export unavailable or invalid$"):
                    finite_status.collect_brain_scoped_bindings(self.input)
                run.assert_not_called()
        self.input.write_bytes(b" " * (bindings.MAX_BYTES + 1))
        with self.assertRaises(finite_status.CollectionError):
            finite_status.collect_brain_scoped_bindings(self.input)

    def test_invalid_binding_results_fail_closed_without_private_diagnostics(self):
        cases = [None, [], {}, {**self.payload, "checkedAt": "private invalid time"},
                 {**self.payload, "bindings": [self.row, self.row]}]
        cases += [{**self.payload, "bindings": [{**self.row, key: value}]} for key, value in [
            ("accountId", "other"), ("workosSubject", None), ("workosSubject", "x" * 201),
            ("email", None), ("email", "a\nprivate@example.test"), ("linkStatus", "pending"),
            ("unrequested", "private unexpected field")]]
        for payload in cases:
            with self.subTest(payload=payload):
                with self.assertRaisesRegex(finite_status.CollectionError, "^private Core account binding export unavailable or invalid$"):
                    self.collect(payload=[] if payload is None else payload)
        for code in (1, 2):
            with self.assertRaises(finite_status.CollectionError):
                self.collect(returncode=code)

    @unittest.skipUnless(os.environ.get("FC_CORE_POSTGRES_TEST_URL"), "requires disposable Core Postgres")
    def test_real_postgres_selects_only_requested_binding_from_read_only_snapshot(self):
        schema = "binding_probe_" + uuid4().hex
        command = ["psql", "--no-psqlrc", "-qAt", "--set", "ON_ERROR_STOP=1",
                   "--dbname", os.environ["FC_CORE_POSTGRES_TEST_URL"]]
        environment = {**os.environ, "PGOPTIONS": f"-c search_path={schema}"}

        def cleanup():
            result = subprocess.run(command, input=f"DROP SCHEMA IF EXISTS {schema} CASCADE;",
                                    text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)

        self.addCleanup(cleanup)
        migration = (Path(__file__).resolve().parents[2] /
                     "finitecomputer-v2/crates/finite-saas-core/migrations/0017_rfc3339_reads.sql").read_text()
        timestamp_function = migration[migration.index("CREATE OR REPLACE FUNCTION core_rfc3339"):]
        timestamp_function = timestamp_function[:timestamp_function.index("$$;") + 3]
        fixture = f"CREATE SCHEMA {schema};\nSET search_path={schema};\n" + timestamp_function + """
CREATE TABLE users(id text, normalized_email text, link_status text, workos_user_id text);
INSERT INTO users VALUES ('account-a','a@example.test','linked','user_synthetic'),
  ('unrequested','private@example.test','linked','user_not_requested');
"""
        result = subprocess.run(command, input=fixture, text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        with mock.patch.dict(finite_status.CONTRACT["database"], {"name": os.environ["FC_CORE_POSTGRES_TEST_URL"]}), \
                mock.patch.object(finite_status, "postgres_environment", return_value=environment):
            report = finite_status.collect_brain_scoped_bindings(self.input)
        self.assertTrue(report["coverage"]["complete"])
        self.assertEqual(report["bindings"], [self.row])
        self.assertIsNotNone(finite_status.parse_time(report["generated_at"]))
        self.assertNotIn("user_not_requested", json.dumps(report))
        self.assertNotIn("private@example.test", json.dumps(report))


if __name__ == "__main__":
    unittest.main()

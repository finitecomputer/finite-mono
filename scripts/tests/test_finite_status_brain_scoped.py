from __future__ import annotations

import json
import os
import subprocess
import unittest

from scripts import finite_status_brain_scoped as scoped
from scripts.tests.test_finite_status_brain_roster import npub

KEY = "11" * 32


def record(kind="managed_agent", **overrides):
    return {"id": "opaque-record", "kind": kind, "key": KEY, "account": "a", "project": "p",
            "account_matches": True, "human_any": False, "human_other": False, **overrides}


def pin(**overrides):
    return {"id": "r", "npub": npub(KEY), "project": "p", "account": "a", "project_present": True,
            "phase": None, "active": True, "foreign_link": False, **overrides}


class ScopedStatusTests(unittest.TestCase):
    def test_same_binding_conflicts_as_core_and_no_contact_dependency(self):
        cases = [
            (record(), [pin()], None),
            (record(), [pin(phase="retirement_requested")], None),
            (record(), [pin(active=False, phase="archived")], None),
            (record(), [pin(), pin(id="legacy", active=False)], None),
            (record(), [pin(active=False)], "agentBindingChanged"),
            (record(), [pin(phase="archived")], "agentBindingChanged"),
            (record(), [], "agentBindingChanged"),
            (record(), [pin(foreign_link=True)], "agentBindingChanged"),
            (record(), [pin(project_present=False)], "agentBindingChanged"),
            (record(), [pin(project="other")], "agentBindingChanged"),
            (record(), [pin(account="other")], "agentBindingChanged"),
            (record(human_any=True), [pin()], "identityEvidenceConflict"),
            (record(account_matches=False), [pin()], "accountBindingChanged"),
            (record("account_chat_key"), [], None),
            (record("account_chat_key", human_any=True), [], None),
            (record("account_chat_key", human_other=True), [], "identityEvidenceConflict"),
            (record("account_chat_key"), [pin()], "identityEvidenceConflict"),
            (record("account_chat_key", account_matches=False), [], "accountBindingChanged"),
        ]
        for kind in ("historical_identity", "invitation_key"):
            cases.extend([(record(kind), [], None), (record(kind), [pin()], "annotationSuperseded"),
                          (record(kind, human_any=True), [], "annotationSuperseded")])
        for row, pins, expected in cases:
            with self.subTest(row=row, pins=pins):
                self.assertEqual(scoped.binding_conflict(row, pins), expected)

    def test_summary_contact_free_bounds_and_schema_states(self):
        facts = {"schema_present": True, "active": 1, "retired": 2,
                 "records": [record()], "pins": [pin(account="other")]}
        result = scoped.identity_summary(facts)
        self.assertEqual(result["conflictCount"], 1)
        self.assertEqual(result["conflicts"], [{"recordId": "opaque-record", "reason": "agentBindingChanged"}])
        self.assertNotIn(KEY, json.dumps(result))
        self.assertNotIn(npub(KEY), json.dumps(result))
        self.assertEqual(scoped.identity_summary({**facts, "active": 2})["state"], "unavailable")
        self.assertEqual(scoped.identity_summary({"schema_present": False})["state"], "notConfigured")
        self.assertEqual(scoped.identity_summary(None)["state"], "notChecked")
        with self.assertRaises(ValueError):
            scoped.identity_summary({**facts, "active": True})

    @unittest.skipUnless(os.environ.get("FC_CORE_POSTGRES_TEST_URL"), "requires disposable Core Postgres")
    def test_actual_postgres_read_only_query_and_binding_drift(self):
        fixture = f"""
CREATE TEMP TABLE brain_scoped_records(id text, public_key_hex text, kind text, account_id text,
  project_id text, workos_subject text, retired_at timestamptz);
CREATE TEMP TABLE users(id text, link_status text, workos_user_id text);
CREATE TEMP TABLE projects(id text, owner_user_id text);
CREATE TEMP TABLE agent_runtimes(id text, project_id text, health_reporting_npub text, offboarding_phase text);
CREATE TEMP TABLE project_runtime_links(agent_runtime_id text, project_id text, active boolean);
CREATE TEMP TABLE account_brain_principals(public_key_hex text, status text, user_id text);
INSERT INTO users VALUES ('a','linked','private-subject');
INSERT INTO projects VALUES ('p','a');
INSERT INTO agent_runtimes VALUES ('r','p','{npub(KEY)}',NULL);
INSERT INTO project_runtime_links VALUES ('r','p',true);
INSERT INTO brain_scoped_records VALUES ('opaque-record','{KEY}','managed_agent','a','p','private-subject',NULL);
"""
        for drift, expected in (("", None), ("UPDATE users SET workos_user_id='changed';", "accountBindingChanged"),
                                ("UPDATE projects SET owner_user_id='other';", "agentBindingChanged"),
                                (f"INSERT INTO account_brain_principals VALUES ('{KEY}','active','other');", "identityEvidenceConflict")):
            # Build session-local temporary fixtures first, then query them
            # from a real read-only transaction. Closing psql discards them.
            sql = fixture + drift + "\nBEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;\n" + scoped.IDENTITY_QUERY + "ROLLBACK;\n"
            result = subprocess.run(["psql", "--no-psqlrc", "-qAt", "--set", "ON_ERROR_STOP=1", "--dbname", os.environ["FC_CORE_POSTGRES_TEST_URL"]],
                                    input=sql, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            facts = json.loads(result.stdout)["scoped_record_facts"]
            self.assertNotIn("private-subject", result.stdout)
            summary = scoped.identity_summary(facts)
            self.assertEqual(summary["conflictCount"], int(expected is not None))
            if expected:
                self.assertEqual(summary["conflicts"][0]["reason"], expected)


if __name__ == "__main__":
    unittest.main()

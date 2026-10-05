from __future__ import annotations

import contextlib
import http.server
import io
import threading
import json
import os
import sqlite3
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from scripts import finite_status
from scripts import finite_status_brain_roster as roster

# Canonical NIP-19 vector plus synthetic keys built from it.
VECTOR_HEX = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d"
VECTOR_NPUB = "npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6"
SECRET_MARKER = "WRAPPED-GRANT-PAYLOAD-MUST-NOT-LEAK"


def npub(hex_key: str) -> str:
    """Encode test keys with the same rules npub_to_hex decodes."""
    charset = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
    data, accumulator, bits = [], 0, 0
    for byte in bytes.fromhex(hex_key):
        accumulator = (accumulator << 8) | byte
        bits += 8
        while bits >= 5:
            bits -= 5
            data.append((accumulator >> bits) & 31)
    if bits:
        data.append((accumulator << (5 - bits)) & 31)
    expanded = [ord(c) >> 5 for c in "npub"] + [0] + [ord(c) & 31 for c in "npub"]
    polymod = roster._bech32_polymod(expanded + data + [0] * 6) ^ 1
    checksum = [(polymod >> 5 * (5 - i)) & 31 for i in range(6)]
    return "npub1" + "".join(charset[d] for d in data + checksum)


KEYS = {name: npub(f"{index:02x}" * 32) for index, name in enumerate(
    ["admin", "member", "agent", "removed", "added", "approver", "target", "guest", "human",
     "wrapper"], start=1)}


class KeyFormatTests(unittest.TestCase):
    def test_only_canonical_npubs_decode(self):
        self.assertEqual(roster.npub_to_hex(VECTOR_NPUB), VECTOR_HEX)
        self.assertEqual(roster.npub_to_hex(npub(VECTOR_HEX)), VECTOR_HEX)
        for invalid in [
            VECTOR_NPUB.upper(),
            VECTOR_NPUB[:-1] + ("q" if VECTOR_NPUB[-1] != "q" else "p"),
            "nsec1" + VECTOR_NPUB[5:],
            VECTOR_NPUB[:20],
            VECTOR_HEX,
            VECTOR_NPUB + "q",
            VECTOR_NPUB[:-1],
            "npub1",
            "npub1-not-bech32",
            "",
        ]:
            self.assertIsNone(roster.npub_to_hex(invalid), invalid)


def build_brain_store(path: Path, oversized: str | None = None) -> None:
    """Synthetic Brain store with the columns the probe reads. `oversized`
    names a column to give one over-long key."""
    connection = sqlite3.connect(path)
    connection.executescript(f"""
CREATE TABLE brains (id TEXT PRIMARY KEY, kind TEXT, name TEXT, owner_user_id TEXT, created_at TEXT);
CREATE TABLE folders (brain_id TEXT, id TEXT, path TEXT, current_key_version INTEGER);
CREATE TABLE brain_members (brain_id TEXT, user_id TEXT, delegated_by_npub TEXT,
  origin_kind TEXT, origin_ref TEXT);
CREATE TABLE brain_admins (brain_id TEXT, user_id TEXT);
CREATE TABLE personal_agents (brain_id TEXT, owner_npub TEXT, agent_npub TEXT, status TEXT);
CREATE TABLE folder_access (brain_id TEXT, folder_id TEXT, user_id TEXT);
CREATE TABLE folder_key_grants (id TEXT, brain_id TEXT, folder_id TEXT, key_version INTEGER,
  issuer_npub TEXT, recipient_npub TEXT, format TEXT, wrapped_event_json TEXT,
  access_change_event_json TEXT, created_at TEXT, delegated_by_npub TEXT, origin_kind TEXT,
  origin_ref TEXT);
CREATE TABLE brain_invite_tokens (brain_id TEXT, redeemed_by_npub TEXT, redeemed_at TEXT);
CREATE TABLE brain_invitations (brain_id TEXT, user_id TEXT, status TEXT, target_kind TEXT,
  accepted_at TEXT, claimed_by_npub TEXT);
CREATE TABLE share_links (brain_id TEXT, recipient_npub TEXT, status TEXT, accepted_at TEXT);
CREATE TABLE shared_folder_invitations (source_brain_id TEXT, destination_admin_npub TEXT,
  status TEXT, accepted_at TEXT);
CREATE TABLE brain_record_index (brain_id TEXT, actor_npub TEXT, accepted_at TEXT);
CREATE TABLE brain_approval_nonces (brain_id TEXT, signer_npub TEXT, applied_at TEXT);
CREATE TABLE identity_aliases (npub TEXT, preferred_nip05 TEXT, nip05_verified_at TEXT,
  updated_at TEXT);
INSERT INTO brains VALUES ('content-brain','organization','Content',NULL,'2026-09-01T00:00:00Z'),
  ('other','organization','Other',NULL,'2026-09-01T00:00:00Z');
INSERT INTO folders VALUES ('content-brain','marketing','Marketing Analytics',2),
  ('other','x','X',1), ('content-brain','long','{"p" * 2000}',1);
INSERT INTO brain_members VALUES
  ('content-brain','{KEYS["admin"]}',NULL,'bootstrap',NULL),
  ('content-brain','{KEYS["member"]}','{KEYS["admin"]}','invitation','invite-1'),
  ('content-brain','{KEYS["agent"]}','{KEYS["admin"]}','direct',NULL),
  ('content-brain','{KEYS["added"]}','{KEYS["admin"]}','direct',NULL),
  ('content-brain','{KEYS["approver"]}','{KEYS["admin"]}','direct',NULL),
  ('content-brain','{KEYS["target"]}','{KEYS["approver"]}','approval','approval-event'),
  ('content-brain','not-an-npub',NULL,'direct',NULL),
  ('other','{KEYS["human"]}',NULL,'direct',NULL);
INSERT INTO brain_admins VALUES ('content-brain','{KEYS["admin"]}'),
  ('content-brain','{KEYS["target"]}');
INSERT INTO folder_access VALUES ('content-brain','marketing','{KEYS["guest"]}');
INSERT INTO folder_key_grants VALUES
  ('g1','content-brain','marketing',2,'{KEYS["admin"]}','{KEYS["member"]}','NIP-59',
   '{SECRET_MARKER}','{SECRET_MARKER}','2026-10-01T00:00:00Z','{KEYS["admin"]}','invitation','invite-1'),
  ('g2','content-brain','marketing',2,'{KEYS["wrapper"]}','{KEYS["removed"]}','NIP-59',
   '{SECRET_MARKER}',NULL,'2026-10-01T00:00:00Z','{KEYS["wrapper"]}','direct',NULL),
  ('g3','content-brain','marketing',1,'{KEYS["admin"]}','{KEYS["guest"]}','NIP-59',
   '{SECRET_MARKER}',NULL,'2026-09-01T00:00:00Z',NULL,'direct',NULL),
  ('g4','content-brain','marketing',2,'{KEYS["admin"]}','{KEYS["agent"]}','NIP-59',
   '{SECRET_MARKER}',NULL,'2026-10-01T00:00:00Z',NULL,'direct',NULL);
INSERT INTO brain_invitations VALUES
  ('content-brain','{KEYS["member"]}','accepted','npub','2026-10-01T01:00:00Z',NULL),
  ('content-brain','{KEYS["added"]}','pending','npub',NULL,NULL);
INSERT INTO brain_record_index VALUES ('content-brain','{KEYS["agent"]}','2026-10-02T00:00:00Z'),
  ('content-brain','{KEYS["agent"]}','2026-10-01T00:00:00Z');
INSERT INTO brain_approval_nonces VALUES ('content-brain','{KEYS["approver"]}','2026-10-03T00:00:00Z');
INSERT INTO identity_aliases VALUES ('{KEYS["added"]}','added@example.test','2026-05-01T00:00:00Z','2026-05-01T00:00:00Z'),
  ('{KEYS["agent"]}','jules-agent@finite.vip','2026-09-01T00:00:00Z','2026-09-01T00:00:00Z'),
  ('{npub("ee" * 32)}','jules-agent@finite.vip','2026-08-01T00:00:00Z','2026-08-01T00:00:00Z');
""")
    if oversized == "member":
        connection.execute("INSERT INTO brain_members VALUES ('content-brain', ?, NULL, 'direct', NULL)",
                           ("k" * 300,))
    if oversized == "grantor":
        connection.execute("UPDATE folder_key_grants SET delegated_by_npub = ? WHERE id = 'g1'",
                           ("d" * 300,))
    connection.commit()
    connection.close()


def query(database: Path, sql: str) -> list[dict]:
    return finite_status.sqlite_json_query(database, sql)


class BrainScratchTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.database = Path(self.scratch.name) / "brain.sqlite3"
        build_brain_store(self.database)

    def tearDown(self):
        self.scratch.cleanup()

    def collect(self, brain_id="content-brain") -> dict:
        return roster.collect_brain(self.database, brain_id, finite_status.scratch_copy_sqlite, query)

    def test_exact_brain_roster_reads_metadata_and_never_payloads(self):
        before = self.database.read_bytes()
        brain = self.collect()
        self.assertEqual(self.database.read_bytes(), before)
        self.assertNotIn(SECRET_MARKER, json.dumps(brain))
        keys = roster.roster_keys(brain["tables"])
        by_npub = {row["npub"]: row for row in keys.values()}
        self.assertNotIn(KEYS["human"], by_npub, "other Brains are filtered out")
        self.assertNotIn(KEYS["wrapper"], by_npub, "a grant issuer is not a roster member")
        self.assertEqual(by_npub[KEYS["removed"]]["roles"], {"currentGrantOnly"})
        self.assertEqual(by_npub[KEYS["guest"]]["roles"], {"explicitFolderAccess"})
        self.assertEqual(by_npub[KEYS["guest"]]["grants"], [], "old key versions are not current")
        self.assertEqual(by_npub[KEYS["target"]]["membership"]["admittedBy"], KEYS["approver"])
        self.assertEqual(
            [p["kind"] for p in by_npub[KEYS["approver"]]["participation"]], ["appliedApproval"])
        self.assertEqual(by_npub[KEYS["target"]]["participation"], [])
        self.assertEqual(by_npub[KEYS["added"]]["participation"], [], "pending invitation")
        self.assertEqual(
            by_npub[KEYS["agent"]]["participation"],
            [{"kind": "authenticatedBrainAction", "firstAt": "2026-10-01T00:00:00Z"}])
        grant = by_npub[KEYS["member"]]["grants"][0]
        self.assertEqual(grant["path"], "Marketing Analytics")
        self.assertEqual(grant["grantDelegatedBy"], KEYS["admin"])
        paths = {row["id"]: row["path"] for row in brain["tables"]["folders"]}
        self.assertIsNone(paths["long"], "oversized text arrives as NULL")
        references = roster.reference_keys(brain["tables"], keys)
        self.assertEqual(references[("content-brain", KEYS["wrapper"])],
                         {"issuedCurrentGrants", "delegatedCurrentGrants"})
        self.assertNotIn(("content-brain", KEYS["admin"]), references, "roster keys are not references")
        inputs = roster.core_inputs(brain)
        self.assertIn(KEYS["wrapper"], json.loads(inputs["npubs"]))
        self.assertNotIn("not-an-npub", inputs["npubs"], "invalid keys are never sent to Core")

    def test_unknown_or_invalid_brain_ids_fail_closed(self):
        for brain_id in ("missing-brain", "content'; DROP TABLE brains; --", "", None):
            with self.assertRaises(roster.RosterError, msg=repr(brain_id)):
                self.collect(brain_id)

    def test_oversized_member_or_grantor_keys_fail_closed(self):
        for column in ("member", "grantor"):
            build_brain_store(self.database.with_name(f"{column}.sqlite3"), oversized=column)
            with self.assertRaises(roster.RosterError, msg=column):
                roster.collect_brain(self.database.with_name(f"{column}.sqlite3"), "content-brain",
                                     finite_status.scratch_copy_sqlite, query)

    def test_missing_required_key_fails_closed(self):
        tables = self.collect()["tables"]
        tables["admins"].append({"brain_id": "content-brain", "user_id": None})
        with self.assertRaises(roster.RosterError):
            roster.roster_keys(tables)

    def test_key_bound_covers_roster_and_reference_keys(self):
        with mock.patch.object(roster, "MAX_KEYS", 8), self.assertRaises(roster.RosterError):
            self.collect()

    def test_row_bounds_fail_closed(self):
        with mock.patch.object(roster, "MAX_ROWS", 2), self.assertRaises(roster.RosterError):
            self.collect()


def core_fixture() -> dict:
    return {
        "checked_at": "2026-10-05T15:00:00Z",
        "brain_identity_schema": True,
        "agents": [
            {"npub": KEYS["agent"], "runtime": "runtime-a", "project": "project-a",
             "project_present": True, "display_name": "Jules Agent", "agent_email": None,
             "account": "account-1", "offboarding_phase": None, "active_link": True,
             "foreign_active_link": False, "health_reported_at": "2026-10-05T14:59:00Z"},
            {"npub": KEYS["removed"], "runtime": "runtime-b", "project": "project-b",
             "project_present": True, "display_name": "Old Agent", "agent_email": None,
             "account": "account-1", "offboarding_phase": "archived", "active_link": False,
             "foreign_active_link": False, "health_reported_at": None},
            {"npub": KEYS["approver"], "runtime": "runtime-c", "project": "project-c",
             "project_present": True, "display_name": "Split", "agent_email": None,
             "account": "account-2", "offboarding_phase": None, "active_link": True,
             "foreign_active_link": False, "health_reported_at": None},
            {"npub": KEYS["approver"], "runtime": "runtime-d", "project": "project-d",
             "project_present": True, "display_name": "Split", "agent_email": None,
             "account": "account-1", "offboarding_phase": None, "active_link": True,
             "foreign_active_link": False, "health_reported_at": None},
        ],
        "humans": [{"hex": roster.npub_to_hex(KEYS["member"]), "account": "account-2",
                    "status": "active", "first_observed_at": "2026-10-05T14:17:00Z",
                    "last_observed_at": "2026-10-05T14:17:00Z"}],
        "accounts": [
            {"account": "account-1", "email": "jules@acme.example", "link_status": "linked"},
            {"account": "account-2", "email": "ray@other.example", "link_status": "linked"},
        ],
        "scopes": [{"account": "account-2", "brain_server": "https://brain.finite.computer",
                    "brain_id": "content-brain", "established_at": "2026-10-05T14:17:00Z",
                    "revoked_at": None}],
    }


class ClassificationTests(unittest.TestCase):
    def report(self) -> dict:
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        database = Path(scratch.name) / "brain.sqlite3"
        build_brain_store(database)
        brain = roster.collect_brain(database, "content-brain", finite_status.scratch_copy_sqlite, query)
        return roster.build_report(brain, core_fixture(), "content-brain")

    def test_core_identity_and_raw_scopes_without_a_disclosure_decision(self):
        report = self.report()
        brain = report["brains"][0]
        roster_rows = {row["npub"]: row for row in brain["roster"]}
        agent = roster_rows[KEYS["agent"]]
        self.assertEqual(agent["coreIdentity"]["state"], "agentPin")
        self.assertEqual(agent["coreIdentity"]["probeDerivedLifecycle"], "live")
        self.assertIsNone(agent["coreIdentity"]["agentPins"][0]["offboardingPhase"])
        pin = agent["coreIdentity"]["agentPins"][0]
        self.assertEqual(pin["displayName"], "Jules Agent")
        self.assertEqual(pin["runtimeId"], "runtime-a")
        self.assertEqual(pin["currentOwnerAccount"]["email"], "jules@acme.example")
        # Identified for the operator; no consent row exists for this Brain.
        self.assertEqual(agent["sharingScopes"], [])
        member = roster_rows[KEYS["member"]]
        self.assertEqual(member["coreIdentity"]["state"], "humanAssociation")
        self.assertEqual(member["coreIdentity"]["humanAssociations"][0]["account"]["email"],
                         "ray@other.example")
        self.assertEqual([scope["brain_id"] for scope in member["sharingScopes"]], ["content-brain"])
        removed = roster_rows[KEYS["removed"]]
        self.assertEqual(removed["coreIdentity"]["probeDerivedLifecycle"], "retired")
        self.assertEqual(removed["coreIdentity"]["agentPins"][0]["offboardingPhase"], "archived")
        approver = roster_rows[KEYS["approver"]]
        self.assertEqual(approver["coreIdentity"]["state"], "conflict")
        self.assertEqual(
            sorted(p["displayName"] for p in approver["coreIdentity"]["agentPins"]), ["Split", "Split"],
            "every matching record is shown")
        self.assertNotIn("probeDerivedLifecycle", approver["coreIdentity"])
        self.assertEqual(roster_rows[KEYS["added"]]["coreIdentity"]["state"], "noCoreRecord")
        self.assertEqual(roster_rows[KEYS["added"]]["storedNip05"]["name"], "added@example.test")
        self.assertEqual(roster_rows["not-an-npub"]["keyFormat"], "invalid")
        self.assertEqual(roster_rows["not-an-npub"]["coreIdentity"]["state"], "keyInvalid")
        self.assertEqual([row["npub"] for row in brain["referenceKeys"]], [KEYS["wrapper"]])
        self.assertEqual(brain["referenceKeys"][0]["coreIdentity"]["state"], "noCoreRecord")
        self.assertEqual(report["evidence"]["coreCheckedAt"], "2026-10-05T15:00:00Z")
        self.assertTrue(report["evidence"]["brainDatabase"].endswith("brain.sqlite3"))
        text = json.dumps(report)
        self.assertNotIn(SECRET_MARKER, text)
        self.assertNotIn("wouldDescribe", text)

    def test_inconsistent_runtime_records_never_name_a_single_owner(self):
        core = core_fixture()
        core["agents"][0]["foreign_active_link"] = True
        identity = roster.core_identity(
            KEYS["agent"], core, {row["account"]: row for row in core["accounts"]})
        self.assertEqual(identity["state"], "inconsistent")
        self.assertNotIn("probeDerivedLifecycle", identity)
        self.assertEqual(identity["agentPins"][0]["foreignActiveLink"], True)

    def test_report_byte_bound_fails_closed(self):
        with mock.patch.object(roster, "MAX_REPORT_BYTES", 1000), self.assertRaises(roster.RosterError):
            self.report()


class QueryContractTests(unittest.TestCase):
    def test_queries_are_select_only_and_never_read_payload_columns(self):
        texts = list(roster.brain_queries("content-brain").values()) + [
            roster.alias_query([VECTOR_NPUB]), roster.CORE_QUERY]
        with self.assertRaises(roster.RosterError):
            roster.alias_query(["npub1'); DROP TABLE x; --"])
        for text in texts:
            upper = text.upper()
            for token in ("INSERT ", "UPDATE ", "DELETE ", "ALTER ", "DROP ", "CREATE ", "ATTACH "):
                self.assertNotIn(token, upper)
            for column in ("wrapped_event_json", "access_change_event_json", "ciphertext",
                           "secret", "payload_json"):
                self.assertNotIn(column, text)

    def test_parse_core_output_requires_agent_evidence_and_bounds(self):
        with self.assertRaises(roster.RosterError):
            roster.parse_core_output("")
        with self.assertRaises(roster.RosterError):
            roster.parse_core_output(json.dumps({"humans": []}))
        with mock.patch.object(roster, "MAX_ROWS", 1), self.assertRaises(roster.RosterError):
            roster.parse_core_output(json.dumps({"agents": [{}, {}]}))


class CommandTests(unittest.TestCase):
    def test_brain_id_is_validated_and_mode_is_opt_in_json(self):
        for arguments in (["--brain-roster", "bad id"], ["--brain-roster-all"]):
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as exit:
                finite_status.main(arguments)
            self.assertEqual(exit.exception.code, 2, arguments)
        stdout = io.StringIO()
        report = {"schema_version": roster.SCHEMA_VERSION, "exit_code": 0, "brains": []}
        with mock.patch.object(finite_status, "collect_brain_roster", return_value=report) as collect, \
                contextlib.redirect_stdout(stdout), self.assertRaises(SystemExit) as exit:
            finite_status.main(["--brain-roster", "content-brain"])
        collect.assert_called_once_with("content-brain")
        self.assertEqual(exit.exception.code, 0)
        self.assertEqual(json.loads(stdout.getvalue())["schema_version"], roster.SCHEMA_VERSION)

    def test_core_failure_is_a_collection_error(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        database = Path(scratch.name) / "brain.sqlite3"
        build_brain_store(database)
        failed = subprocess.CompletedProcess([], 1, "", "ERROR: permission denied\n")
        real_run = finite_status.run_read_only

        def run(command, **kwargs):
            return failed if command[0] == "psql" else real_run(command, **kwargs)

        with mock.patch.object(roster, "BRAIN_DATABASE", database), \
                mock.patch.object(finite_status, "postgres_environment", return_value={}), \
                mock.patch.object(finite_status, "run_read_only", side_effect=run), \
                self.assertRaises(finite_status.CollectionError):
            finite_status.collect_brain_roster("content-brain")


TOKEN = "synthetic-operator-token-must-not-leak"


def inspect_answer(hex_key: str, *bindings: tuple[str, bool]) -> bytes:
    return json.dumps({
        "kind": "native", "pubkey": hex_key,
        "vip_emails": [{"email": name, "localpart": name.split("@")[0], "domain": "finite.vip",
                        "created_at": 1_780_000_000, "disabled": disabled,
                        "disabled_at": 1_780_000_100 if disabled else None,
                        "email_challenges": ["never-selected"]}
                       for name, disabled in bindings],
    }).encode()


class DirectoryTests(unittest.TestCase):
    def test_answers_are_selected_strictly_and_echo_the_exact_key(self):
        agent, hex_key = KEYS["agent"], roster.npub_to_hex(KEYS["agent"])
        bound = roster.directory_result(agent, 200, inspect_answer(hex_key, ("jules-agent@finite.vip", False)))
        self.assertEqual(bound["state"], "bound")
        self.assertEqual(bound["bindings"], [{"name": "jules-agent@finite.vip", "disabled": False,
                                              "createdAt": 1_780_000_000, "disabledAt": None}])
        self.assertNotIn("never-selected", json.dumps(bound))
        multi = roster.directory_result(agent, 200, inspect_answer(
            hex_key, ("a@finite.vip", False), ("b@finite.vip", False), ("c@finite.vip", True)))
        self.assertTrue(multi["multipleActiveNames"])
        self.assertEqual(len(multi["bindings"]), 3, "every binding is kept")
        self.assertEqual(roster.directory_result(agent, 200, inspect_answer(
            hex_key, ("old@finite.vip", True)))["state"], "disabledOnly")
        self.assertEqual(roster.directory_result(
            agent, 404, b'{"error":"principal_not_found"}'), {"state": "noBinding"})
        self.assertEqual(roster.directory_result(agent, 503, b"{}")["state"], "unavailable")
        other = roster.npub_to_hex(KEYS["member"])
        bad_name = inspect_answer(hex_key, ("Bad Name@finite.vip", False))
        inconsistent = json.loads(inspect_answer(hex_key, ("a@finite.vip", False)))
        inconsistent["vip_emails"][0]["disabled_at"] = 5
        for status, body in [
            (200, inspect_answer(other, ("a@finite.vip", False))),
            (200, json.dumps({"kind": "vip_email", "pubkey": hex_key}).encode()),
            (200, bad_name),
            (200, json.dumps(inconsistent).encode()),
            (200, inspect_answer(hex_key, *[(f"n{i}@finite.vip", False) for i in range(17)])),
            (200, b"x" * (roster.MAX_DIRECTORY_RESPONSE_BYTES + 1)),
            (200, b"not json"),
            (404, b'{"error":"invalid_identifier"}'),
        ]:
            self.assertEqual(roster.directory_result(agent, status, body)["state"],
                             "invalidResponse", body[:80])

    def test_malformed_tokens_are_refused_and_transport_errors_are_redacted(self):
        self.assertTrue(roster.valid_directory_token(TOKEN))
        for bad in ("", "short", "a" * 513, "tok\r\nX-Injected: 1" + "a" * 20,
                    "token-with-\u00e9-" + "a" * 20, "token with space" + "a" * 20, None):
            self.assertFalse(roster.valid_directory_token(bad), repr(bad))
        # Even if a bad header value reached the transport, only the error
        # class name comes back.
        leaky = "secret-fragment-QQQQ\nX-Bad: 1"
        with mock.patch.object(roster, "DIRECTORY_INSPECT_URL", "http://127.0.0.1:9/api"):
            post = roster.http_directory_post(leaky)
            with self.assertRaises(roster.DirectoryUnavailable) as raised:
                post(b"{}")
        self.assertNotIn("QQQQ", str(raised.exception))
        self.assertIsNone(raised.exception.__context__)
        self.assertIsNone(raised.exception.__cause__)

    def test_collection_is_exact_key_and_budgeted(self):
        requests = []

        def post(body: bytes) -> tuple[int, bytes]:
            requests.append(json.loads(body))
            if len(requests) == 2:
                raise roster.DirectoryUnavailable("URLError")
            return 404, b'{"error":"principal_not_found"}'

        ticks = iter([0, 0, 1, 10_000])
        result = roster.collect_directory(
            [KEYS["admin"], KEYS["agent"], "not-an-npub", KEYS["member"]], post, lambda: next(ticks))
        self.assertEqual(requests, [{"identifier": KEYS["admin"]}, {"identifier": KEYS["agent"]}])
        self.assertEqual(result["results"][KEYS["admin"]], {"state": "noBinding"})
        self.assertEqual(result["results"][KEYS["agent"]],
                         {"state": "unavailable", "reason": "URLError"})
        self.assertNotIn("not-an-npub", result["results"])
        self.assertEqual(result["results"][KEYS["member"]]["reason"], "Directory budget exhausted")
        self.assertTrue(result["budget_exhausted"])

    def test_http_client_sends_only_the_header_token_and_never_follows_redirects(self):
        seen = []
        hex_key = roster.npub_to_hex(KEYS["agent"])

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self):  # noqa: N802
                body = self.rfile.read(int(self.headers["content-length"]))
                seen.append((self.path, self.headers.get("x-finite-operator-token"), json.loads(body)))
                if json.loads(body)["identifier"] == KEYS["member"]:
                    self.send_response(302)
                    self.send_header("location", "http://127.0.0.1:9/elsewhere")
                    self.end_headers()
                    return
                answer = inspect_answer(hex_key, ("jules-agent@finite.vip", False))
                self.send_response(200)
                self.send_header("content-length", str(len(answer)))
                self.end_headers()
                self.wfile.write(answer)

            def log_message(self, *args):
                pass

        server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.shutdown)
        url = f"http://127.0.0.1:{server.server_port}/api/v1/operator/inspect"
        with mock.patch.object(roster, "DIRECTORY_INSPECT_URL", url), \
                mock.patch.dict(os.environ, {"http_proxy": "http://127.0.0.1:9",
                                             "HTTP_PROXY": "http://127.0.0.1:9"}):
            post = roster.http_directory_post(TOKEN)
            result = roster.collect_directory([KEYS["agent"], KEYS["member"]], post)
        self.assertEqual(result["results"][KEYS["agent"]]["state"], "bound")
        self.assertEqual(result["results"][KEYS["member"]]["state"], "unavailable")
        self.assertEqual([path for path, _, _ in seen], ["/api/v1/operator/inspect"] * 2)
        self.assertTrue(all(token == TOKEN for _, token, _ in seen))
        self.assertNotIn(TOKEN, json.dumps(result))

    def test_alias_name_evidence_includes_rows_outside_the_roster(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        database = Path(scratch.name) / "brain.sqlite3"
        build_brain_store(database)
        hex_key = roster.npub_to_hex(KEYS["agent"])

        def directory(npubs):
            self.assertIn(KEYS["agent"], npubs)
            return roster.collect_directory(npubs, lambda body: (
                (200, inspect_answer(hex_key, ("jules-agent@finite.vip", False)))
                if json.loads(body)["identifier"] == KEYS["agent"]
                else (404, b'{"error":"principal_not_found"}')))

        brain = roster.collect_brain(database, "content-brain", finite_status.scratch_copy_sqlite,
                                     query, directory)
        report = roster.build_report(brain, core_fixture(), "content-brain")
        (evidence,) = report["aliasNameEvidence"]
        self.assertEqual(evidence["name"], "jules-agent@finite.vip")
        self.assertEqual(evidence["directoryKeys"], [{"npub": KEYS["agent"], "disabled": False}])
        rows = {row["npub"]: row["matchesActiveDirectoryKey"] for row in evidence["brainAliasRows"]}
        self.assertEqual(rows, {KEYS["agent"]: True, npub("ee" * 32): False})
        roster_rows = {row["npub"]: row for row in report["brains"][0]["roster"]}
        self.assertEqual(roster_rows[KEYS["agent"]]["identityDirectory"]["state"], "bound")
        self.assertEqual(roster_rows[KEYS["admin"]]["identityDirectory"], {"state": "noBinding"})
        self.assertEqual(roster_rows["not-an-npub"]["identityDirectory"], {"state": "keyInvalid"})
        self.assertEqual(report["evidence"]["identityDirectory"]["state"], "checked")
        self.assertNotIn(SECRET_MARKER, json.dumps(report))

    def test_end_to_end_token_stays_internal_and_missing_token_is_explicit(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        database = Path(scratch.name) / "brain.sqlite3"
        build_brain_store(database)
        token_file = Path(scratch.name) / "identity-operator.env"
        token_file.write_text(f"FINITE_IDENTITY_OPERATOR_TOKEN={TOKEN}\n")
        core_stdout = json.dumps({"checked_at": "2026-10-05T15:00:00Z", "brain_identity_schema": True,
                                  "agents": [], "accounts": []}) + "\n"
        real_run = finite_status.run_read_only

        def run(command, **kwargs):
            self.assertNotIn(TOKEN, " ".join(command))
            if command[0] == "psql":
                return subprocess.CompletedProcess(command, 0, core_stdout, "")
            return real_run(command, **kwargs)

        def unreachable(token):
            def post(body):
                raise roster.DirectoryUnavailable("URLError")
            return post

        bad_token = "bad\u00e9token-fragment-ZZZZ\tvalue"
        bad_file = Path(scratch.name) / "bad.env"
        bad_file.write_text(f"FINITE_IDENTITY_OPERATOR_TOKEN='{bad_token}'\n")
        for token_path, expected in ((token_file, "checked"), (Path(scratch.name) / "absent", "notConfigured"),
                                     (bad_file, "unavailable")):
            with mock.patch.object(roster, "BRAIN_DATABASE", database), \
                    mock.patch.object(roster, "DIRECTORY_TOKEN_FILE", token_path), \
                    mock.patch.object(roster, "http_directory_post", side_effect=unreachable), \
                    mock.patch.object(finite_status, "postgres_environment", return_value={}), \
                    mock.patch.object(finite_status, "run_read_only", side_effect=run):
                report = finite_status.collect_brain_roster("content-brain")
            self.assertEqual(report["evidence"]["identityDirectory"]["state"], expected)
            self.assertNotIn(TOKEN, json.dumps(report))
            self.assertNotIn("token-fragment-ZZZZ", json.dumps(report))


@unittest.skipUnless(os.environ.get("FC_CORE_POSTGRES_TEST_URL"), "requires disposable Core Postgres")
class CorePostgresTests(unittest.TestCase):
    def test_core_query_matches_exact_pins_and_hides_ids(self):
        member_hex = roster.npub_to_hex(KEYS["member"])
        fixture = f"""
BEGIN;
CREATE FUNCTION core_rfc3339(ts timestamptz) RETURNS text LANGUAGE sql IMMUTABLE
  AS $$ SELECT to_char(ts AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') $$;
CREATE TEMP TABLE users(id text, normalized_email text, link_status text);
CREATE TEMP TABLE projects(id text, owner_user_id text, display_name text, agent_email text);
CREATE TEMP TABLE agent_runtimes(id text, project_id text, health_reporting_npub text,
  offboarding_phase text, health_reported_at timestamptz);
CREATE TEMP TABLE project_runtime_links(project_id text, agent_runtime_id text, active boolean);
CREATE TEMP TABLE account_brain_principals(user_id text, public_key_hex text, status text,
  first_observed_at timestamptz, last_observed_at timestamptz);
CREATE TEMP TABLE account_brain_sharing_scopes(user_id text, brain_server text, brain_id text,
  established_at timestamptz, revoked_at timestamptz);
INSERT INTO users VALUES ('user_jules','jules@acme.example','linked'),('user_ray','ray@other.example','linked');
INSERT INTO projects VALUES ('project_1','user_jules','Jules Agent','{"a" * 300}@finite.vip');
INSERT INTO agent_runtimes VALUES ('runtime_1','project_1','{KEYS["agent"]}',NULL,now()),
  ('runtime_x','project_1','npub1unrelated',NULL,now());
INSERT INTO project_runtime_links VALUES ('project_1','runtime_1',true);
INSERT INTO account_brain_principals VALUES ('user_ray','{member_hex}','active',now(),now());
INSERT INTO account_brain_sharing_scopes VALUES ('user_ray','https://brain.finite.computer',
  'content-brain',now(),NULL),('user_ray','https://brain.finite.computer','elsewhere',now(),NULL);
"""
        command = ["psql", "--no-psqlrc", "--tuples-only", "--no-align", "--quiet",
                   "--set", "ON_ERROR_STOP=1", "--dbname", os.environ["FC_CORE_POSTGRES_TEST_URL"]]
        for name, value in {
            "npubs": json.dumps([KEYS["agent"], KEYS["member"]]),
            "hexes": json.dumps([member_hex]),
            "brain_ids": json.dumps(["content-brain"]),
            "row_limit": "50001",
            "name_limit": "256",
            "email_limit": "254",
        }.items():
            command.extend(["--set", f"{name}={value}"])
        # Same invocation shape as the probe: a script file, stdin closed,
        # bounded by subprocess and server-side timeouts.
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        script = Path(scratch.name) / "core.sql"
        script.write_text(fixture + roster.CORE_QUERY + "ROLLBACK;\n")
        environment = dict(os.environ, PGOPTIONS="-c idle_in_transaction_session_timeout=20000"
                                                 " -c statement_timeout=20000")
        result = subprocess.run([*command, "--file", str(script)], stdin=subprocess.DEVNULL,
                                text=True, capture_output=True, timeout=60, env=environment)
        self.assertEqual(result.returncode, 0, result.stderr[-2000:])
        core = roster.parse_core_output(result.stdout)
        self.assertEqual([agent["npub"] for agent in core["agents"]], [KEYS["agent"]])
        self.assertEqual(core["agents"][0]["display_name"], "Jules Agent")
        self.assertIsNone(core["agents"][0]["agent_email"], "oversized text is withheld in SQL")
        self.assertEqual(len(core["humans"]), 1)
        self.assertEqual([scope["brain_id"] for scope in core["scopes"]], ["content-brain"])
        emails = {account["email"] for account in core["accounts"]}
        self.assertEqual(emails, {"jules@acme.example", "ray@other.example"})
        # Real Core ids are operator metadata; unrelated rows never appear.
        self.assertEqual(core["agents"][0]["project"], "project_1")
        self.assertEqual(core["agents"][0]["runtime"], "runtime_1")
        self.assertEqual(core["agents"][0]["account"], "user_jules")
        self.assertNotIn("runtime_x", json.dumps(core))


if __name__ == "__main__":
    unittest.main()

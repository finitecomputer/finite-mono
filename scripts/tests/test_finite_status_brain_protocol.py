from __future__ import annotations

import copy
import http.server
import json
import threading
import time
import unittest
from pathlib import Path

from scripts import finite_status_brain_protocol as probe
from scripts.tests.test_finite_status_brain_roster import npub

FIXTURE = Path(__file__).resolve().parents[2] / "finitecomputer-v2/docs/contracts/brain-identity-descriptions-v3.example.json"
ENV = {"FC_CORE_BRAIN_IDENTITY_BIND": "127.0.0.1:4202",
       "FC_CORE_BRAIN_IDENTITY_BRAIN_SERVER": "https://brain.example.test",
       "FC_CORE_BRAIN_DESCRIPTION_TOKEN": "synthetic-private-credential"}


def brain(keys):
    return {"tables": {"brains": [{"id": "brain_example", "kind": "organization"}],
                       "admins": [{"brain_id": "brain_example", "user_id": npub(keys[0])}],
                       "members": [{"brain_id": "brain_example", "user_id": npub(k)} for k in keys],
                       "participation": [{"brain_id": "brain_example", "npub": npub(k), "kind": "approved", "first_at": "2026-01-01T00:00:00Z"} for k in keys],
                       "personal_agents": [], "folder_access": [], "folders": [], "current_grants": []}}


class ProtocolTests(unittest.TestCase):
    def setUp(self):
        self.response = json.loads(FIXTURE.read_text())
        self.keys = [row["publicKeyHex"] for row in self.response["results"]]
        self.request = {"brainId": "brain_example", "publicKeysHex": self.keys,
                        "participatingPublicKeysHex": self.keys}
        self.brain = brain(self.keys)

    def test_shared_28_row_fixture_and_private_summary(self):
        used, conflicts = probe.validate(self.request, self.response)
        seen = []
        def post(payload, seconds):
            seen.append((json.loads(payload), seconds))
            return 200, json.dumps(self.response).encode()
        result = probe.collect(self.brain, "brain_example", ENV, post=post)
        self.assertEqual(result["servedVersion"], probe.VERSION)
        self.assertEqual((result["checkedKeys"], result["operatorUsedKeys"], result["conflictKeys"]), (28, used, conflicts))
        self.assertFalse(result["viewerVerified"])
        self.assertEqual(set(seen[0][0]["publicKeysHex"]), set(self.keys))
        self.assertLessEqual(seen[0][1], 3)
        for private in ("@", ENV["FC_CORE_BRAIN_DESCRIPTION_TOKEN"], self.keys[0], "Previous example Agent"):
            self.assertNotIn(private, json.dumps(result))

    def test_invalid_provenance_never_certifies_v3(self):
        mutations = [
            lambda r: r.update(version="finite-core-brain-identity-descriptions-v2"),
            lambda r: r.update(brainId="other"),
            lambda r: r["results"].pop(),
            lambda r: r["results"].append(r["results"][0]),
            lambda r: r["results"][1].pop("operatorRecord"),
            lambda r: r["results"][1]["operatorRecord"].update(state="conflict", reason="accountBindingChanged"),
            lambda r: r["results"][2]["operatorAnnotation"].update(label="unicode—claim"),
            lambda r: r["results"][2]["operatorAnnotation"].update(label="newline\nclaim"),
            lambda r: r["results"][2]["operatorAnnotation"].update(observedAt="2026-02-30T00:00:00Z"),
            lambda r: r["results"][2].update(accountEmail="leak@example.test"),
            lambda r: r["results"][1]["operatorRecord"].update(observedAt="2026-02-30T00:00:00Z"),
        ]
        for mutate in mutations:
            value = copy.deepcopy(self.response)
            mutate(value)
            result = probe.collect(self.brain, "brain_example", ENV, post=lambda *_: (200, json.dumps(value).encode()))
            self.assertEqual(result["state"], "unavailable", value)
            self.assertIsNone(result["servedVersion"])

    def test_no_fallback_no_admin_no_config_and_timeout(self):
        for status in (400, 401, 404, 500, 302):
            calls = []
            def post(*_):
                calls.append(True)
                return status, b'{"error":"invalid descriptions request body"}'
            result = probe.collect(self.brain, "brain_example", ENV, post=post)
            self.assertEqual(result["state"], "unavailable")
            self.assertEqual(len(calls), 1)
        self.assertEqual(probe.collect(self.brain, "brain_example", {})["state"], "notConfigured")
        self.brain["tables"]["admins"] = []
        self.assertEqual(probe.collect(self.brain, "brain_example", ENV)["state"], "notChecked")
        self.assertEqual(probe.collect(brain(self.keys), "brain_example", ENV, post=lambda *_: (200, b"{}"), clock=iter([0, 16]).__next__)["state"], "unavailable")

    def test_nonparticipant_normal_result_and_operator_hints_are_rejected(self):
        request = {**self.request, "participatingPublicKeysHex": []}
        with self.assertRaises(ValueError):
            probe.validate(request, self.response)
        row = copy.deepcopy(self.response["results"][0])
        row["source"]["kind"] = "operatorScopedRecord"
        row["operatorRecord"] = {"state": "used", "revision": row["source"]["revision"], "observedAt": row["source"]["observedAt"]}
        row["responsibleAccount"]["humanPublicKeysHex"] = [self.keys[1]]
        self.response["results"][0] = row
        with self.assertRaises(ValueError):
            probe.validate(self.request, self.response)

    def test_loopback_transport_is_bounded_and_does_not_follow_redirects(self):
        seen = []
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass
            def do_POST(self):
                seen.append((self.path, self.headers.get(probe.HEADER), self.rfile.read(int(self.headers["Content-Length"]))))
                self.send_response(302)
                self.send_header("Location", "http://example.invalid/credential-leak")
                self.end_headers()
                self.wfile.write(b"redirect")
        with http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
            worker = threading.Thread(target=server.serve_forever, daemon=True)
            worker.start()
            try:
                post = probe.http_post(f"127.0.0.1:{server.server_port}", "synthetic-token")
                self.assertEqual(post(b"{}", 1), (302, b"redirect"))
                self.assertEqual(seen, [(probe.PATH, "synthetic-token", b"{}")])
            finally:
                server.shutdown()
                worker.join()
        for bind in ("example.com:4202", "0.0.0.0:4202", "192.0.2.1:4202", "127.0.0.1:0", "127.0.0.1:65536"):
            with self.assertRaises(ValueError):
                probe.http_post(bind, "token")

    def test_slow_header_drip_has_an_absolute_deadline(self):
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass
            def do_POST(self):
                self.rfile.read(int(self.headers["Content-Length"]))
                try:
                    self.wfile.write(b"HTTP/1.1 200 OK\r\n")
                    for _ in range(30):
                        self.wfile.write(b"X-Drip: yes\r\n")
                        self.wfile.flush()
                        time.sleep(.03)
                    self.wfile.write(b"Content-Length: 2\r\n\r\n{}")
                except OSError:
                    pass
        with http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
            worker = threading.Thread(target=server.serve_forever, daemon=True)
            worker.start()
            try:
                started = time.monotonic()
                try:
                    _, body = probe.http_post(f"127.0.0.1:{server.server_port}", "token")(b"{}", .15)
                    self.assertNotEqual(body, b"{}")
                except (OSError, probe.http.client.HTTPException):
                    pass
                self.assertLess(time.monotonic() - started, .75)
            finally:
                server.shutdown()
                worker.join()

    def test_every_page_and_only_its_participating_subset_is_checked(self):
        keys = [format(n, "064x") for n in range(1, 202)]
        fixture = brain(keys)
        fixture["tables"]["participation"] = fixture["tables"]["participation"][:3]
        seen = []
        def post(payload, seconds):
            request = json.loads(payload)
            seen.append(request)
            self.assertLessEqual(len(request["publicKeysHex"]), 100)
            self.assertEqual(set(request["participatingPublicKeysHex"]), set(request["publicKeysHex"]) & set(keys[:3]))
            return 200, json.dumps({"version":probe.VERSION,"brainId":"brain_example","checkedAt":"2026-01-01T00:00:00Z",
                                   "results":[{"publicKeyHex":key,"state":"notShared"} for key in request["publicKeysHex"]]}).encode()
        result = probe.collect(fixture, "brain_example", ENV, post=post)
        self.assertEqual(result["state"], "checked")
        self.assertEqual(result["checkedKeys"], 201)
        self.assertEqual(len(seen), 3)
        self.assertEqual({k for request in seen for k in request["publicKeysHex"]}, set(keys))


if __name__ == "__main__":
    unittest.main()

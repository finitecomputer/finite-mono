"""Native Hermes auth/routing and bounded product CLI contract tests.

Defaults to sealed packaged modules/plugins. Source mode is a fast local test,
not packaging evidence. Every CLI and HOME in these tests is disposable.
"""

import asyncio
import base64
import hashlib
import http.client
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
OWNER_NPUB = "npub1" + "q" * 58
AGENT_NPUB = "npub1" + "p" * 58
MEMBER_NPUB = "npub1" + "m" * 58
ADMIN_NPUB = "npub1" + "d" * 58
GUEST_NPUB = "npub1" + "g" * 58
INVITATION = "invitation-" + "0" * 32


if os.environ.get("FINITE_INVENTORY_SOURCE_TEST") == "1":
    name = "hermes_cli.finite_dashboard_reads"
    spec = importlib.util.spec_from_file_location(
        name, ROOT / "finite-agentd/integrations/hermes/finite_dashboard_reads.py"
    )
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)


class ProductInventoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.scratch = tempfile.TemporaryDirectory()
        cls.home = Path(cls.scratch.name)
        cls.bin = cls.home / "bin"
        cls.bin.mkdir()
        bundled = cls.home / "bundled"
        bundled.mkdir()
        source_mode = os.environ.get("FINITE_INVENTORY_SOURCE_TEST") == "1"
        for product in ("finite-brain", "finite-sites"):
            source = (
                (ROOT / product / "integrations/hermes" / product)
                if source_mode
                else (Path(os.environ["HERMES_BUNDLED_PLUGINS"]) / product)
            )
            shutil.copytree(source, bundled / product)
        cls.environment = patch.dict(
            os.environ,
            {
                "HERMES_HOME": str(cls.home / "hermes"),
                "HERMES_BUNDLED_PLUGINS": str(bundled),
                "FINITE_HOME": str(cls.home / "agent"),
                "PATH": f"{cls.bin}{os.pathsep}{os.environ['PATH']}",
            },
        )
        cls.environment.start()
        (cls.home / "hermes").mkdir()
        from hermes_cli import finite_dashboard_reads, web_server
        from starlette.testclient import TestClient

        cls.reads = finite_dashboard_reads
        cls.server = web_server
        cls.client = TestClient(web_server.app, raise_server_exceptions=False)
        cls.client.headers[web_server._SESSION_HEADER_NAME] = web_server._SESSION_TOKEN
        for name in ("fbrain", "fsite"):
            binary = cls.bin / name
            binary.write_text(
                f"#!{sys.executable}\n"
                + """
import json, os, pathlib, sys, time
home = pathlib.Path(os.environ["FINITE_HOME"])
with (home / "calls").open("a") as calls:
    calls.write(json.dumps(sys.argv[1:]) + "\\n")
mode = (home / "mode").read_text() if (home / "mode").exists() else "ok"
if mode == "failure":
    print("private CLI diagnostics", file=sys.stderr)
    sys.exit(1)
if mode == "service-failure":
    print(json.dumps({"inventory_error_version": 1, "kind": "request"}), file=sys.stderr)
    sys.exit(1)
if mode == "oversize-error":
    sys.stderr.write("x" * 8193)
    sys.exit(1)
if mode == "invalid-error-version":
    print(json.dumps({"inventory_error_version": True, "kind": "request"}), file=sys.stderr)
    sys.exit(1)
if mode == "oversize":
    sys.stdout.write("x" * (512 * 1024 + 1))
    sys.exit(0)
if mode == "slow":
    (home / "pid").write_text(str(os.getpid()))
    time.sleep(60)
if sys.argv[1:3] == ["access", "list"]:
    print((home / "access.json").read_text())
    sys.exit(0)
if sys.argv[1:3] == ["brain", "personal-agent-consent"]:
    # Signs as the Agent and never mints an identity; no flag required.
    print((home / "consent.json").read_text())
    sys.exit(0)
if sys.argv[1:4] == ["invite", "brain", "list"]:
    print((home / "invitations.json").read_text())
    sys.exit(0)
if sys.argv[1] in ("invite", "admin"):
    step = " ".join(sys.argv[1:4])
    if (home / "hold").exists():
        (home / "held").write_text(step)
        while (home / "hold").exists():
            time.sleep(0.02)
    failing = (home / "fail").read_text().split(",") if (home / "fail").exists() else []
    if step in failing:
        print("private CLI diagnostics", file=sys.stderr)
        sys.exit(1)
    if (home / "slow-change").exists():
        (home / "pid").write_text(str(os.getpid()))
        time.sleep(60)
    if sys.argv[1:4] == ["invite", "brain", "create"]:
        print((home / "receipt.json").read_text())
    else:
        print(json.dumps({"state": "complete", "private": "receipt"}))
    sys.exit(0)
if "--existing-identity" not in sys.argv:
    sys.exit(2)
if sys.argv[1:3] == ["brain", "metadata"]:
    if (home / "missing-metadata").exists(): sys.exit(1)
    payload = json.loads((home / "metadata.json").read_text())
    payload["brainId"] = sys.argv[3].removeprefix("--brain=")
elif sys.argv[1:3] == ["brain", "list"]:
    payload = json.loads((home / "brains.json").read_text())
elif sys.argv[1:3] == ["project", "list"]:
    payload = json.loads((home / "sites.json").read_text())
else:
    sys.exit(2)
print(json.dumps(payload))
"""
            )
            binary.chmod(0o700)

    @classmethod
    def tearDownClass(cls):
        cls.client.close()
        cls.environment.stop()
        cls.scratch.cleanup()

    def setUp(self):
        self.agent = self.home / "agent"
        shutil.rmtree(self.agent, ignore_errors=True)
        self.agent.mkdir()
        self.brains = {
            "brains": [
                {
                    "brainId": "agent-brain",
                    "name": "Agent brain",
                    "kind": "organization",
                    "role": "guest",
                    "inviteCode": "private-code",
                },
                {
                    "brainId": "invitation",
                    "name": "Invitation",
                    "kind": "organization",
                    "role": "invited",
                    "inviteCode": "private-code",
                },
            ]
        }
        self.metadata = {
            "folders": [
                {"id": "folder", "name": "Visible folder", "accessUserIds": ["private-principal"]}
            ],
            "mountedFolders": [{"id": "mount", "name": "Not included"}],
            "members": ["private-principal"],
        }
        self.sites = {
            "projects": [
                {"project_id": "repo-only", "site": None},
                {
                    "project_id": "site-project",
                    "role": "editor",
                    "git_remote_url": "https://finite.site/site-project.git",
                    "project_visibility": "public",
                    "site": {
                        "name": "Agent site",
                        "url": "https://agent.finite.site",
                        "visibility": "private",
                        "status": "published",
                        "active_version": 1,
                    },
                },
            ]
        }
        self.consent = {
            "version": "finite-brain-personal-agent-consent-v1",
            "agentNpub": AGENT_NPUB,
            "ownerNpub": OWNER_NPUB,
            "brainId": "personal-0123456789abcdef",
            "brainServer": "https://brain.example",
            "expiresAt": 1790000300,
            "consent": {
                "id": "a" * 64,
                "pubkey": "b" * 64,
                "created_at": 1790000000,
                "kind": 30078,
                "tags": [["d", "personal-0123456789abcdef"], ["p", "c" * 64]],
                "content": '{"brainId":"personal-0123456789abcdef"}',
                "sig": "d" * 128,
            },
        }
        self.write_data()

    def write_data(self):
        for name, payload in (
            ("brains", self.brains),
            ("metadata", self.metadata),
            ("sites", self.sites),
            ("consent", self.consent),
        ):
            (self.agent / f"{name}.json").write_text(json.dumps(payload))

    def get(self, product="brain", suffix=""):
        return self.client.get(f"/api/plugins/finite-{product}/overview{suffix}")

    def test_native_auth_precedes_cli_and_fixed_get_rejects_parameters(self):
        response = self.client.get(
            "/api/plugins/finite-brain/overview",
            headers={self.server._SESSION_HEADER_NAME: "invalid"},
        )
        self.assertEqual(response.status_code, 401)
        self.assertFalse((self.agent / "calls").exists())
        for product in ("brain", "sites"):
            self.assertEqual(self.get(product, "?profile=other&command=whoami").status_code, 400)
            self.assertEqual(
                self.client.post(f"/api/plugins/finite-{product}/overview").status_code, 405
            )
        self.assertFalse((self.agent / "calls").exists())

    def test_brain_projects_guest_folders_and_pending_without_private_fields(self):
        response = self.get()
        self.assertEqual(response.status_code, 200, response.text)
        self.assertEqual(response.headers["cache-control"], "no-store")
        rows = response.json()["brains"]
        self.assertEqual(rows[0]["folders"], [{"id": "folder", "name": "Visible folder"}])
        self.assertEqual(rows[0]["role"], "guest")
        self.assertIsNone(rows[1]["folders"])
        self.assertNotIn("private", response.text)
        self.assertNotIn("Not included", response.text)
        calls = [json.loads(line) for line in (self.agent / "calls").read_text().splitlines()]
        self.assertEqual(
            calls,
            [
                ["brain", "list", "--json", "--existing-identity"],
                ["brain", "metadata", "--brain=agent-brain", "--json", "--existing-identity"],
            ],
        )
        (self.agent / "missing-metadata").touch()
        response = self.get()
        self.assertEqual(response.status_code, 200)
        self.assertIsNone(response.json()["brains"][0]["folders"])

    def test_scoped_annotations_are_bounded_dated_and_never_owner_contacts(self):
        annotation = {
            "kind": "historical_identity",
            "label": "Previous example Agent",
            "observedAt": "2026-01-01T00:00:00Z",
        }
        row = {
            "npub": "npub1a",
            "brainRole": "admin",
            "folders": [],
            "description": {"state": "notShared"},
            "operatorAnnotation": annotation,
            "operatorRecord": {
                "state": "used",
                "revision": "opaque",
                "observedAt": annotation["observedAt"],
            },
        }
        report = {
            "version": "finite-brain-access-report-v1",
            "brainId": "agent-brain",
            "identities": [row],
        }

        def fetch(candidate):
            report["identities"] = [candidate]
            (self.agent / "access.json").write_text(json.dumps(report))
            response = self.client.get("/api/plugins/finite-brain/identities/agent-brain")
            self.assertEqual(response.status_code, 200, response.text)
            return response.json()["identities"][0]

        self.assertEqual(fetch(row)["operatorAnnotation"], annotation)
        for annotation_change in (
            {"kind": "owner"},
            {"label": "x" * 121},
            {"label": "Unicode \u2014"},
            {"label": " leading"},
            {"observedAt": "2026-02-30T00:00:00Z"},
            {"observedAt": "2026-01-01T00:00:00+00:00"},
            {"ownerEmail": "secret@example.test"},
        ):
            projected = fetch({**row, "operatorAnnotation": {**annotation, **annotation_change}})
            self.assertIsNone(projected["operatorAnnotation"])
            self.assertIsNone(projected["ownerEmail"])
            self.assertIsNone(projected["name"])
        self.assertIsNone(
            fetch({**row, "operatorRecord": {"state": "conflict"}})["operatorAnnotation"]
        )

    def test_brain_identities_project_roster_fields_and_stored_public_name(self):
        agent = {
            "state": "resolved",
            "kind": "agent",
            "displayName": "Ada",
            "lifecycle": "active",
            "responsibleAccount": {"email": "sam@example.test", "id": "private-account"},
            "source": {"kind": "private-source", "observedAt": "2026-10-05T00:00:00Z"},
        }
        unshared = {"state": "notShared", "kind": "human", "accountEmail": "private@example.test"}
        folder = {"folderId": "folder", "path": "private-path", "state": "ready"}
        stored = {"name": "cara@finite.vip", "storedAt": "2026-09-17T20:39:30Z"}
        named: dict[str, object] = {
            "npub": "npub1c",
            "brainRole": "member",
            "description": {"state": "unavailable", "reason": "private-reason"},
            "storedNip05": stored,
            "folders": [],
        }
        report = {
            "version": "finite-brain-access-report-v1",
            "brainId": "agent-brain",
            "currentAccessComplete": False,
            "identities": [
                {
                    "npub": "npub1a",
                    "brainRole": "member",
                    "description": agent,
                    "folders": [folder],
                },
                {"npub": "npub1b", "brainRole": "admin", "description": unshared, "folders": []},
                named,
                {
                    "npub": "npub1d",
                    "brainRole": "member",
                    "description": {"state": "unknown"},
                    "storedNip05": {"name": "dev@finite.vip"},
                    "folders": [],
                },
            ],
        }
        (self.agent / "access.json").write_text(json.dumps(report))
        response = self.client.get("/api/plugins/finite-brain/identities/agent-brain")
        self.assertEqual(response.status_code, 200, response.text)
        unnamed = {"publicName": None, "publicNameStoredAt": None}
        empty = {
            "kind": None,
            "name": None,
            "email": None,
            "ownerEmail": None,
            "lifecycle": None,
            "operatorAnnotation": None,
        }
        self.assertEqual(
            response.json()["identities"],
            [
                {
                    "npub": "npub1a",
                    "role": "member",
                    "kind": "agent",
                    "name": "Ada",
                    "email": None,
                    "ownerEmail": "sam@example.test",
                    "lifecycle": "active",
                    "operatorAnnotation": None,
                    **unnamed,
                    "folders": [{"id": "folder", "state": "ready"}],
                },
                {"npub": "npub1b", "role": "admin", **empty, **unnamed, "folders": []},
                {
                    "npub": "npub1c",
                    "role": "member",
                    **empty,
                    "publicName": "cara@finite.vip",
                    "publicNameStoredAt": "2026-09-17T20:39:30Z",
                    "folders": [],
                },
                {
                    "npub": "npub1d",
                    "role": "member",
                    **empty,
                    "publicName": "dev@finite.vip",
                    "publicNameStoredAt": None,
                    "folders": [],
                },
            ],
        )
        self.assertNotIn("private", response.text)
        calls = [json.loads(line) for line in (self.agent / "calls").read_text().splitlines()]
        self.assertEqual(
            calls,
            [
                ["brain", "list", "--json", "--existing-identity"],
                ["access", "list", "--brain", "agent-brain", "--json"],
            ],
        )
        for malformed in (
            "cara@finite.vip",
            {"name": ""},
            {"name": "cara@finite.vip", "storedAt": "yesterday"},
            {"name": "cara@finite.vip", "storedAt": "2026-13-01T00:00:00Z"},
            {"name": "cara@finite.vip", "storedAt": 1790000000},
        ):
            named["storedNip05"] = malformed
            (self.agent / "access.json").write_text(json.dumps(report))
            response = self.client.get("/api/plugins/finite-brain/identities/agent-brain")
            self.assertEqual(response.status_code, 503, malformed)
        named["storedNip05"] = stored
        report["brainId"] = "other-brain"
        (self.agent / "access.json").write_text(json.dumps(report))
        response = self.client.get("/api/plugins/finite-brain/identities/agent-brain")
        self.assertEqual(response.status_code, 503)
        before = len((self.agent / "calls").read_text().splitlines())
        for brain in ("--server", "a" * 129, "invitation", "not-a-member"):
            response = self.client.get(f"/api/plugins/finite-brain/identities/{brain}")
            self.assertEqual(response.status_code, 403)
        calls = (self.agent / "calls").read_text().splitlines()[before:]
        self.assertNotIn("access", "".join(calls))

    def test_sites_use_site_visibility_and_exclude_source_only_repositories(self):
        response = self.get("sites")
        self.assertEqual(response.status_code, 200, response.text)
        data = response.json()
        self.assertEqual(data["sourceOnlyProjects"], 1)
        self.assertEqual(len(data["sites"]), 1)
        self.assertEqual(data["sites"][0]["visibility"], "private")
        self.assertTrue(data["sites"][0]["canEdit"])
        self.assertNotIn("updated", response.text)
        self.sites["projects"][1]["site"]["url"] = "javascript:alert(1)"
        self.write_data()
        self.assertEqual(self.get("sites").status_code, 503)

    def test_different_agent_homes_and_missing_or_revoked_access_never_reuse_data(self):
        first = self.get().json()
        self.brains["brains"][0]["name"] = "Different agent brain"
        self.write_data()
        self.assertNotEqual(first, self.get().json())
        (self.agent / "mode").write_text("failure")
        for product in ("brain", "sites"):
            response = self.get(product)
            self.assertEqual(response.status_code, 403)
            self.assertNotIn("private CLI", response.text)
            self.assertNotIn("Agent brain", response.text)

    def test_unrecognized_diagnostic_contracts_fail_closed(self):
        for mode in ("oversize-error", "invalid-error-version"):
            (self.agent / "mode").write_text(mode)
            for product in ("brain", "sites"):
                response = self.get(product)
                self.assertEqual(response.status_code, 403)
                self.assertNotIn("inventory_error", response.text)

    def test_typed_service_failure_is_retryable_without_leaking_diagnostics(self):
        (self.agent / "mode").write_text("service-failure")
        for product in ("brain", "sites"):
            response = self.get(product)
            self.assertEqual(response.status_code, 503)
            self.assertNotIn("inventory_error", response.text)

    def test_bounds_fail_without_truncation_and_timeout_reaps_cli(self):
        self.brains["brains"] = [
            {"brainId": f"brain-{i}", "name": "Brain", "kind": "personal", "role": "member"}
            for i in range(33)
        ]
        self.write_data()
        self.assertEqual(self.get().status_code, 503)
        self.assertEqual(len((self.agent / "calls").read_text().splitlines()), 1)
        (self.agent / "mode").write_text("oversize")
        self.assertEqual(self.get().status_code, 503)
        (self.agent / "mode").write_text("slow")
        with patch.object(self.reads, "READ_SECONDS", 2):
            self.assertEqual(self.get().status_code, 503)
        pid = int((self.agent / "pid").read_text())
        with self.assertRaises(ProcessLookupError):
            os.kill(pid, 0)

    @unittest.skipUnless(
        os.environ.get("FBRAIN_TEST_BINARY") and os.environ.get("FSITE_TEST_BINARY"),
        "real CLI binaries supplied by the CLI integration gate",
    )
    def test_real_clis_sign_with_two_agent_identities_not_the_human_identity(self):
        binaries = self.home / "real-bin"
        binaries.mkdir(exist_ok=True)
        for name, variable in (("fbrain", "FBRAIN_TEST_BINARY"), ("fsite", "FSITE_TEST_BINARY")):
            (binaries / name).symlink_to(Path(os.environ[variable]).resolve())
        identities = {}
        for label in ("agent-one", "agent-two", "human"):
            home = self.home / label
            result = subprocess.run(
                [str(binaries / "fsite"), "auth", "import", "--output", "json"],
                input=os.urandom(32).hex(),
                text=True,
                capture_output=True,
                check=True,
                env={**os.environ, "FINITE_HOME": str(home)},
            )
            identities[json.loads(result.stdout)["pubkey"]] = label
        observed = []
        upstream_status = 200

        class Handler(BaseHTTPRequestHandler):
            def do_GET(handler):
                header = handler.headers.get("Authorization", "")
                event = json.loads(base64.b64decode(header.removeprefix("Nostr ")))
                label = identities[event["pubkey"]]
                observed.append((label, handler.path))
                if upstream_status != 200:
                    handler.send_error(upstream_status)
                    return
                if handler.path == "/v1/brains":
                    data = {
                        "brains": [
                            {
                                "brainId": "--server" if label == "agent-one" else label,
                                "name": label,
                                "kind": "personal",
                                "role": "personal_agent",
                            }
                        ]
                    }
                elif handler.path == f"/v1/brains/{label}/metadata":
                    data = {"brainId": label, "folders": [{"id": "visible", "name": label}]}
                elif handler.path == "/api/v2/projects":
                    data = {
                        "projects": [
                            {
                                "project_id": label,
                                "slug": label,
                                "project_visibility": "private",
                                "role": "owner",
                                "git_remote_url": f"https://finite.site/{label}.git",
                                "site": {
                                    "name": label,
                                    "url": f"https://{label}.finite.site",
                                    "status": "published",
                                    "visibility": "private",
                                    "active_version": 1,
                                    "branch": "main",
                                    "path": "dist",
                                    "spa": False,
                                    "created": False,
                                },
                            }
                        ]
                    }
                else:
                    handler.send_error(403)
                    return
                body = json.dumps(data).encode()
                handler.send_response(200)
                handler.send_header("Content-Type", "application/json")
                handler.send_header("Content-Length", str(len(body)))
                handler.end_headers()
                handler.wfile.write(body)

            def log_message(self, *_):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        url = f"http://127.0.0.1:{server.server_port}"
        try:
            for label in ("agent-one", "agent-two"):
                identity_file = self.home / label / "identity/identity.json"
                before = identity_file.read_bytes()
                with patch.dict(
                    os.environ,
                    {
                        "FINITE_HOME": str(self.home / label),
                        "FBRAIN_CONFIG_DIR": str(self.home / label / "brain-config"),
                        "FINITE_BRAIN_SERVER_URL": url,
                        "FINITE_SITES_API": url,
                        "PATH": f"{binaries}{os.pathsep}{os.environ['PATH']}",
                    },
                ):
                    brains = self.get()
                    sites = self.get("sites")
                self.assertEqual(brains.status_code, 200, brains.text)
                self.assertEqual(sites.status_code, 200, sites.text)
                self.assertEqual(brains.json()["brains"][0]["name"], label)
                self.assertEqual(sites.json()["sites"][0]["name"], label)
                self.assertEqual(before, identity_file.read_bytes())
            self.assertEqual(
                [label for label, _ in observed], ["agent-one"] * 3 + ["agent-two"] * 3
            )
            with patch.dict(
                os.environ,
                {
                    "FINITE_HOME": str(self.home / "agent-two"),
                    "FINITE_BRAIN_SERVER_URL": url,
                    "FINITE_SITES_API": url,
                    "PATH": f"{binaries}{os.pathsep}{os.environ['PATH']}",
                },
            ):
                for upstream_status in (503, 403):
                    for product in ("brain", "sites"):
                        self.assertEqual(self.get(product).status_code, upstream_status)
                with patch.dict(os.environ, {"FINITE_HOME": str(self.home / "missing-signer")}):
                    for product in ("brain", "sites"):
                        self.assertEqual(self.get(product).status_code, 403)
                    self.assertFalse((self.home / "missing-signer").exists())
            # The consent route runs the real CLI as the Agent. Signing is
            # local: no Brain request, and the identity file stays unchanged.
            human = json.loads(
                subprocess.run(
                    [str(binaries / "fbrain"), "auth", "status", "--json"],
                    capture_output=True,
                    text=True,
                    check=True,
                    env={**os.environ, "FINITE_HOME": str(self.home / "human")},
                ).stdout
            )["npub"]
            agent_one = self.home / "agent-one"
            before = (agent_one / "identity/identity.json").read_bytes()
            with patch.dict(
                os.environ,
                {
                    "FINITE_HOME": str(agent_one),
                    "FBRAIN_CONFIG_DIR": str(agent_one / "brain-config"),
                    "FINITE_BRAIN_SERVER_URL": url,
                    "PATH": f"{binaries}{os.pathsep}{os.environ['PATH']}",
                },
            ):
                response = self.read_consent(human)
            self.assertEqual(response.status_code, 200, response.text)
            consent = response.json()
            keys = {label: key for key, label in identities.items()}
            self.assertEqual(consent["ownerNpub"], human)
            self.assertEqual(consent["brainId"], f"personal-{keys['human'][:16]}")
            self.assertEqual(consent["consent"]["pubkey"], keys["agent-one"])
            self.assertEqual(consent["consent"]["kind"], 30078)
            self.assertEqual(json.loads(consent["consent"]["content"])["brainServer"], url)
            self.assertEqual(before, (agent_one / "identity/identity.json").read_bytes())
        finally:
            server.shutdown()
            worker.join()
            server.server_close()

    def test_flag_shaped_valid_brain_ids_are_data_not_options(self):
        self.brains["brains"][0]["brainId"] = "--server"
        self.write_data()
        response = self.get()
        self.assertEqual(response.status_code, 200, response.text)
        self.assertEqual(response.json()["brains"][0]["folders"][0]["name"], "Visible folder")
        self.assertIn("--brain=--server", (self.agent / "calls").read_text())

    def test_ids_cannot_become_cli_flags_or_path_traversal(self):
        for invalid_id in ("../other", "brain?query", "", "x" * 129):
            self.brains["brains"][0]["brainId"] = invalid_id
            self.write_data()
            self.assertEqual(self.get().status_code, 503)

    def read_consent(self, owner=OWNER_NPUB, suffix=""):
        return self.client.get(f"/api/plugins/finite-brain/personal-agent-consent/{owner}{suffix}")

    def test_personal_agent_consent_signs_as_agent_for_the_requested_owner(self):
        response = self.read_consent()
        self.assertEqual(response.status_code, 200, response.text)
        self.assertEqual(response.headers["cache-control"], "no-store")
        self.assertEqual(
            response.json(),
            {
                "version": 1,
                "agentNpub": AGENT_NPUB,
                "ownerNpub": OWNER_NPUB,
                "brainId": "personal-0123456789abcdef",
                "consent": self.consent["consent"],
            },
        )
        calls = [json.loads(line) for line in (self.agent / "calls").read_text().splitlines()]
        self.assertEqual(
            calls,
            [["brain", "personal-agent-consent", "--owner", OWNER_NPUB, "--json"]],
        )

    def test_personal_agent_consent_refuses_malformed_owner_before_cli(self):
        response = self.client.get(
            f"/api/plugins/finite-brain/personal-agent-consent/{OWNER_NPUB}",
            headers={self.server._SESSION_HEADER_NAME: "invalid"},
        )
        self.assertEqual(response.status_code, 401)
        for owner in (
            "npub1short",
            "--owner",
            OWNER_NPUB.upper(),
            "npub1" + "b" * 58,
            OWNER_NPUB + "q",
            "nsec1" + "q" * 58,
        ):
            self.assertEqual(self.read_consent(owner).status_code, 503, owner)
        self.assertEqual(self.read_consent(suffix="?owner=other").status_code, 400)
        self.assertEqual(
            self.client.post(
                f"/api/plugins/finite-brain/personal-agent-consent/{OWNER_NPUB}"
            ).status_code,
            405,
        )
        self.assertFalse((self.agent / "calls").exists())

    def test_personal_agent_consent_refuses_unexpected_cli_output(self):
        mutations = {
            "other owner": lambda data: data.update(ownerNpub="npub1" + "z" * 58),
            "self consent": lambda data: data.update(agentNpub=OWNER_NPUB),
            "malformed agent": lambda data: data.update(agentNpub="npub1agent"),
            "organization id": lambda data: data.update(brainId="acme"),
            "uppercase id": lambda data: data.update(brainId="personal-0123456789ABCDEF"),
            "flag id": lambda data: data.update(brainId="--server"),
            "old version": lambda data: data.update(
                version="finite-brain-personal-agent-consent-v0"
            ),
            "missing consent": lambda data: data.pop("consent"),
            "unsigned": lambda data: data["consent"].pop("sig"),
            "short id": lambda data: data["consent"].update(id="a" * 63),
            "boolean kind": lambda data: data["consent"].update(kind=True),
            "string time": lambda data: data["consent"].update(created_at="1790000000"),
            "flat tags": lambda data: data["consent"].update(tags=["d", "x"]),
            "empty content": lambda data: data["consent"].update(content=""),
        }
        for name, mutate in mutations.items():
            with self.subTest(name):
                data = json.loads(json.dumps(self.consent))
                mutate(data)
                (self.agent / "consent.json").write_text(json.dumps(data))
                response = self.read_consent()
                self.assertEqual(response.status_code, 503, response.text)
                self.assertNotIn("npub", response.text)
        (self.agent / "mode").write_text("failure")
        response = self.read_consent()
        self.assertEqual(response.status_code, 403)
        self.assertNotIn("private CLI", response.text)

    # Admin roster changes: invite, revoke invitation, remove.

    def admin_fixture(self, role="admin", kind="organization"):
        self.brains["brains"][0].update(role=role, kind=kind)
        self.write_data()

        def row(npub, brain_role):
            return {"npub": npub, "brainRole": brain_role, "description": {}, "folders": []}

        report = {
            "version": "finite-brain-access-report-v1",
            "brainId": "agent-brain",
            "brainKind": kind,
            "actingKey": {"npub": AGENT_NPUB, "brainRole": role},
            "identities": [
                row(AGENT_NPUB, role),
                row(MEMBER_NPUB, "member"),
                row(ADMIN_NPUB, "admin"),
                row(GUEST_NPUB, "guest"),
                row(OWNER_NPUB, "noCurrentRole"),
            ],
        }
        (self.agent / "access.json").write_text(json.dumps(report))

        def invitation(suffix, status="pending", **extra):
            return {
                "id": "invitation-" + suffix * 32,
                "brainId": "agent-brain",
                "folderOnly": False,
                "status": status,
                "userId": MEMBER_NPUB,
                "inviteCode": "invite-private",
                "acceptPath": "/v1/private",
                "identities": [{"npub": MEMBER_NPUB, "nip05": "Mia@finite.vip"}],
                "expiresAt": "2026-11-02T00:00:00Z",
                "expired": False,
                **extra,
            }

        self.invitations = {
            "invitations": [
                invitation("0"),
                invitation("1", "accepted"),
                invitation("2", "revoked"),
                invitation("3", expired=True, userId=None, identities=[]),
            ]
        }
        (self.agent / "invitations.json").write_text(json.dumps(self.invitations))
        (self.agent / "receipt.json").write_text(
            json.dumps(invitation("4", deliveryStatus="in_app"))
        )
        return report

    def calls(self):
        path = self.agent / "calls"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def change_client(self):
        """One event loop for the client's life, as under uvicorn, so a
        removal job keeps running between requests."""
        from starlette.testclient import TestClient

        server = self.server

        async def app(scope, receive, send):
            if scope["type"] == "lifespan":
                while True:
                    message = await receive()
                    kind = message["type"].rsplit(".", 1)[-1]
                    await send({"type": f"lifespan.{kind}.complete"})
                    if kind == "shutdown":
                        return
            await server.app(scope, receive, send)

        client = TestClient(app, raise_server_exceptions=False)
        client.headers[server._SESSION_HEADER_NAME] = server._SESSION_TOKEN
        return client

    def wait_for_job(self, client, job_id, brain="agent-brain"):
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            state = client.get(f"/api/plugins/finite-brain/removals/{brain}/{job_id}").json()
            if state["job"]["state"] != "running":
                return state["job"]["state"]
            time.sleep(0.02)
        self.fail("removal job did not finish within 60 s")

    def test_identities_advertise_changes_only_to_organization_admins(self):
        for role, kind, actions in (
            ("admin", "organization", ["invite", "revokeInvitation", "remove"]),
            ("member", "organization", []),
            ("owner", "organization", []),
            ("personal_agent", "personal", []),
            ("admin", "personal", []),
        ):
            with self.subTest(role=role, kind=kind):
                report = self.admin_fixture(role, kind)
                if role == "personal_agent":
                    report["actingKey"]["brainRole"] = "personalAgent"
                    (self.agent / "access.json").write_text(json.dumps(report))
                response = self.client.get("/api/plugins/finite-brain/identities/agent-brain")
                self.assertEqual(response.status_code, 200, response.text)
                self.assertEqual(
                    response.json()["changes"], {"actingNpub": AGENT_NPUB, "actions": actions}
                )
        # The fresh report must agree with the Brain list that this key is an Admin.
        for acting, kind in (("member", "organization"), ("admin", "personal")):
            report = self.admin_fixture()
            report["actingKey"]["brainRole"] = acting
            report["brainKind"] = kind
            (self.agent / "access.json").write_text(json.dumps(report))
            response = self.client.get("/api/plugins/finite-brain/identities/agent-brain")
            self.assertEqual(response.json()["changes"]["actions"], [], (acting, kind))
        report = self.admin_fixture()
        report["actingKey"] = {"npub": "npub1short", "brainRole": "admin"}
        (self.agent / "access.json").write_text(json.dumps(report))
        response = self.client.get("/api/plugins/finite-brain/identities/agent-brain")
        self.assertEqual(response.json()["changes"], {"actingNpub": None, "actions": []})

    def test_invite_by_npub_or_public_name_signs_one_member_invitation(self):
        self.admin_fixture()
        for target in (MEMBER_NPUB, "mia@finite.vip", "_@example.finite.vip"):
            with self.subTest(target=target):
                (self.agent / "calls").unlink(missing_ok=True)
                response = self.client.post(
                    "/api/plugins/finite-brain/invitations/agent-brain", json={"target": target}
                )
                self.assertEqual(response.status_code, 200, response.text)
                self.assertEqual(response.headers["cache-control"], "no-store")
                self.assertEqual(
                    response.json(),
                    {
                        "version": 1,
                        "invitation": {
                            "id": "invitation-" + "4" * 32,
                            "targetNpub": MEMBER_NPUB,
                            "targetName": "mia@finite.vip",
                            "expiresAt": "2026-11-02T00:00:00Z",
                            "expired": False,
                        },
                    },
                )
                self.assertNotIn("private", response.text)
                self.assertEqual(
                    self.calls(),
                    [
                        ["brain", "list", "--json", "--existing-identity"],
                        [
                            "invite",
                            "brain",
                            "create",
                            "--brain",
                            "agent-brain",
                            "--target",
                            target,
                            "--json",
                        ],
                    ],
                )
        # A receipt the plugin cannot read still reports the created invitation.
        (self.agent / "receipt.json").write_text("not json")
        response = self.client.post(
            "/api/plugins/finite-brain/invitations/agent-brain", json={"target": MEMBER_NPUB}
        )
        self.assertEqual(response.json(), {"version": 1, "invitation": None})

    def test_change_inputs_are_refused_before_any_command(self):
        self.admin_fixture()
        url = "/api/plugins/finite-brain/invitations/agent-brain"
        for target in (
            "--server",
            "-x@finite.vip",
            "mia@finite.vip --json",
            "Mia@finite.vip",
            "mia@localhost",
            "mia@finite.vip\n",
            "a" * 65 + "@finite.vip",
            "b" * 64,
            MEMBER_NPUB + "q",
            "npub1" + "b" * 58,
            "nsec1" + "q" * 58,
            "",
            7,
            None,
            ["mia@finite.vip"],
        ):
            with self.subTest(target=target):
                self.assertEqual(self.client.post(url, json={"target": target}).status_code, 400)
        for body, headers in (
            ({"target": MEMBER_NPUB, "role": "admin"}, {}),
            ({"target": MEMBER_NPUB, "folder": "x"}, {}),
            ({}, {}),
            ([MEMBER_NPUB], {}),
        ):
            self.assertEqual(self.client.post(url, json=body, headers=headers).status_code, 400)
        self.assertEqual(
            self.client.post(
                url,
                content=json.dumps({"target": MEMBER_NPUB}),
                headers={"content-type": "text/plain"},
            ).status_code,
            400,
        )
        self.assertEqual(self.client.post(url, content=b"{" + b" " * 2048 + b"}").status_code, 400)
        self.assertEqual(
            self.client.post(url + "?server=x", json={"target": MEMBER_NPUB}).status_code, 400
        )
        for npub in ("--target", MEMBER_NPUB.upper(), "mia@finite.vip", 1):
            response = self.client.post(
                "/api/plugins/finite-brain/removals/agent-brain", json={"npub": npub}
            )
            self.assertEqual(response.status_code, 400, npub)
        for invitation in ("--id", "invitation-" + "0" * 31, "invitation-" + "A" * 32, "x"):
            response = self.client.delete(
                f"/api/plugins/finite-brain/invitations/agent-brain/{invitation}"
            )
            self.assertEqual(response.status_code, 400, invitation)
        response = self.client.request(
            "DELETE",
            f"/api/plugins/finite-brain/invitations/agent-brain/{INVITATION}",
            content=b"{}",
        )
        self.assertEqual(response.status_code, 400)
        self.assertEqual(self.calls(), [])
        for brain in ("--server", "-x", "a" * 129):
            for response in (
                self.client.post(
                    f"/api/plugins/finite-brain/invitations/{brain}", json={"target": MEMBER_NPUB}
                ),
                self.client.delete(f"/api/plugins/finite-brain/invitations/{brain}/{INVITATION}"),
                self.client.post(
                    f"/api/plugins/finite-brain/removals/{brain}", json={"npub": MEMBER_NPUB}
                ),
            ):
                self.assertEqual(response.status_code, 400, brain)
                self.assertEqual(response.json(), {"error": "invalidInput"})
        self.assertEqual(self.calls(), [])

    def test_native_auth_precedes_every_change(self):
        self.admin_fixture()
        bad = {self.server._SESSION_HEADER_NAME: "invalid"}
        for method, path, body in (
            ("POST", "invitations/agent-brain", {"target": MEMBER_NPUB}),
            ("DELETE", f"invitations/agent-brain/{INVITATION}", None),
            ("POST", "removals/agent-brain", {"npub": MEMBER_NPUB}),
            ("GET", "invitations/agent-brain", None),
            ("GET", "removals/agent-brain/" + "0" * 32, None),
        ):
            response = self.client.request(
                method, f"/api/plugins/finite-brain/{path}", json=body, headers=bad
            )
            self.assertEqual(response.status_code, 401, path)
        self.assertEqual(self.calls(), [])

    def test_changes_refuse_agents_that_are_not_organization_admins(self):
        for role, kind in (
            ("member", "organization"),
            ("owner", "organization"),
            ("invited", "organization"),
            ("personal_agent", "personal"),
            ("admin", "personal"),
        ):
            with self.subTest(role=role, kind=kind):
                self.admin_fixture(role, kind)
                (self.agent / "calls").unlink(missing_ok=True)
                for method, path, body in (
                    ("POST", "invitations/agent-brain", {"target": MEMBER_NPUB}),
                    ("DELETE", f"invitations/agent-brain/{INVITATION}", None),
                    ("POST", "removals/agent-brain", {"npub": MEMBER_NPUB}),
                ):
                    response = self.client.request(
                        method, f"/api/plugins/finite-brain/{path}", json=body
                    )
                    self.assertEqual(response.status_code, 403, path)
                    self.assertEqual(response.json(), {"error": "notAdmin"})
                self.assertEqual(
                    self.client.get(
                        "/api/plugins/finite-brain/invitations/agent-brain"
                    ).status_code,
                    403,
                )
                # Only the no-mint membership proof ran; nothing could mint or change.
                self.assertEqual(
                    {tuple(call) for call in self.calls()},
                    {("brain", "list", "--json", "--existing-identity")},
                )
        self.admin_fixture()
        self.brains["brains"][0]["brainId"] = "other-brain"
        self.write_data()
        response = self.client.post(
            "/api/plugins/finite-brain/invitations/agent-brain", json={"target": MEMBER_NPUB}
        )
        self.assertEqual(response.status_code, 403)

    def test_pending_invitations_project_only_this_brains_pending_rows(self):
        self.admin_fixture()
        response = self.client.get("/api/plugins/finite-brain/invitations/agent-brain")
        self.assertEqual(response.status_code, 200, response.text)
        self.assertEqual(response.headers["cache-control"], "no-store")
        self.assertEqual(
            response.json(),
            {
                "version": 1,
                "brainId": "agent-brain",
                "invitations": [
                    {
                        "id": INVITATION,
                        "targetNpub": MEMBER_NPUB,
                        "targetName": "mia@finite.vip",
                        "expiresAt": "2026-11-02T00:00:00Z",
                        "expired": False,
                    },
                    {
                        "id": "invitation-" + "3" * 32,
                        "targetNpub": None,
                        "targetName": None,
                        "expiresAt": "2026-11-02T00:00:00Z",
                        "expired": True,
                    },
                ],
            },
        )
        self.assertNotIn("private", response.text)
        self.assertIn(["invite", "brain", "list", "--brain", "agent-brain", "--json"], self.calls())
        pending = self.invitations["invitations"][0]
        for name, change in (
            # The Agent's own inbox has no folderOnly field; never show it as this list.
            ("inbox shape", lambda row: row.pop("folderOnly")),
            ("other brain", lambda row: row.update(brainId="other")),
            ("folder invitation", lambda row: row.update(folderOnly=True)),
            ("flag id", lambda row: row.update(id="--id")),
            ("bad npub", lambda row: row.update(userId="npub1x")),
            ("bad time", lambda row: row.update(expiresAt="soon")),
        ):
            with self.subTest(name):
                broken = json.loads(json.dumps(self.invitations))
                change(broken["invitations"][0])
                (self.agent / "invitations.json").write_text(json.dumps(broken))
                response = self.client.get("/api/plugins/finite-brain/invitations/agent-brain")
                self.assertEqual(response.status_code, 503)
        # The name is optional: an unexpected identity list hides only the name.
        for name, identities in (
            ("too many", [pending["identities"][0]] * 33),
            ("not a list", "x"),
        ):
            with self.subTest(name):
                odd = json.loads(json.dumps(self.invitations))
                odd["invitations"][0]["identities"] = identities
                (self.agent / "invitations.json").write_text(json.dumps(odd))
                response = self.client.get("/api/plugins/finite-brain/invitations/agent-brain")
                self.assertEqual(response.status_code, 200, response.text)
                first = response.json()["invitations"][0]
                self.assertEqual(
                    (first["id"], first["targetNpub"], first["targetName"]),
                    (INVITATION, MEMBER_NPUB, None),
                )
        self.assertEqual(pending["id"], INVITATION)

    def test_revoke_only_this_brains_pending_invitation(self):
        self.admin_fixture()
        url = f"/api/plugins/finite-brain/invitations/agent-brain/{INVITATION}"
        response = self.client.delete(url)
        self.assertEqual(response.status_code, 200, response.text)
        self.assertEqual(response.json(), {"version": 1})
        self.assertEqual(
            self.calls()[-1], ["invite", "brain", "revoke", "--id", INVITATION, "--json"]
        )
        for invitation in ("1" * 32, "2" * 32, "9" * 32):
            (self.agent / "calls").unlink()
            response = self.client.delete(
                f"/api/plugins/finite-brain/invitations/agent-brain/invitation-{invitation}"
            )
            self.assertEqual(response.status_code, 422)
            self.assertEqual(response.json(), {"error": "notPending"})
            self.assertNotIn("revoke", json.dumps(self.calls()))

    def test_remove_member_runs_one_background_removal(self):
        self.admin_fixture()
        with self.change_client() as client:
            response = client.post(
                "/api/plugins/finite-brain/removals/agent-brain", json={"npub": MEMBER_NPUB}
            )
            self.assertEqual(response.status_code, 202, response.text)
            self.assertEqual(response.headers["cache-control"], "no-store")
            job = response.json()["job"]
            self.assertEqual(job["state"], "running")
            self.assertEqual(self.wait_for_job(client, job["id"]), "removed")
            # A job ID only answers for its own Brain.
            other = client.get(f"/api/plugins/finite-brain/removals/other/{job['id']}")
            self.assertEqual(other.json()["job"]["state"], "unknown")
        self.assertEqual(
            self.calls(),
            [
                ["brain", "list", "--json", "--existing-identity"],
                ["access", "list", "--brain", "agent-brain", "--json"],
                [
                    "admin",
                    "member",
                    "remove",
                    "--brain",
                    "agent-brain",
                    "--target",
                    MEMBER_NPUB,
                    "--json",
                ],
            ],
        )

    def test_remove_admin_demotes_first_and_keeps_nonzero_results_unknown(self):
        self.admin_fixture()
        demote = [
            "admin",
            "role",
            "revoke",
            "admin",
            "--brain",
            "agent-brain",
            "--target",
            ADMIN_NPUB,
            "--json",
        ]
        remove = [
            "admin",
            "member",
            "remove",
            "--brain",
            "agent-brain",
            "--target",
            ADMIN_NPUB,
            "--json",
        ]
        with self.change_client() as client:
            for failing, state, steps in (
                ("", "removed", [demote, remove]),
                ("admin member remove", "unknown", [demote, remove]),
                ("admin role revoke", "unknown", [demote]),
            ):
                with self.subTest(failing=failing):
                    (self.agent / "fail").write_text(failing)
                    (self.agent / "calls").unlink(missing_ok=True)
                    response = client.post(
                        "/api/plugins/finite-brain/removals/agent-brain", json={"npub": ADMIN_NPUB}
                    )
                    self.assertEqual(response.status_code, 202, response.text)
                    job = response.json()["job"]["id"]
                    self.assertEqual(self.wait_for_job(client, job), state)
                    self.assertEqual(self.calls()[2:], steps)
                    status = client.get(f"/api/plugins/finite-brain/removals/agent-brain/{job}")
                    self.assertNotIn("private", status.text)

    def test_removal_refuses_own_key_and_rows_without_a_member_role(self):
        self.admin_fixture()
        for npub, code in (
            (AGENT_NPUB, "ownKey"),
            (GUEST_NPUB, "notRemovable"),
            (OWNER_NPUB, "notRemovable"),
            ("npub1" + "z" * 58, "notRemovable"),
        ):
            with self.subTest(npub=npub):
                response = self.client.post(
                    "/api/plugins/finite-brain/removals/agent-brain", json={"npub": npub}
                )
                self.assertEqual(response.status_code, 422)
                self.assertEqual(response.json(), {"error": code})
        self.assertNotIn("remove", json.dumps(self.calls()))
        self.assertNotIn("revoke", json.dumps(self.calls()))
        # Without a valid acting key the own-key rule cannot be checked.
        for acting in ({"npub": "npub1short", "brainRole": "admin"}, None):
            with self.subTest(acting=acting):
                report = self.admin_fixture()
                report["actingKey"] = acting
                (self.agent / "access.json").write_text(json.dumps(report))
                (self.agent / "calls").unlink(missing_ok=True)
                response = self.client.post(
                    "/api/plugins/finite-brain/removals/agent-brain", json={"npub": MEMBER_NPUB}
                )
                self.assertEqual(response.status_code, 503)
                self.assertEqual(response.json(), {"error": "unavailable"})
                self.assertNotIn("admin", json.dumps(self.calls()))

    def test_one_change_at_a_time_and_lost_jobs_read_as_unknown(self):
        self.admin_fixture()
        (self.agent / "hold").touch()
        with self.change_client() as client:
            response = client.post(
                "/api/plugins/finite-brain/removals/agent-brain", json={"npub": MEMBER_NPUB}
            )
            job = response.json()["job"]["id"]
            for _ in range(500):
                if (self.agent / "held").exists():
                    break
                time.sleep(0.02)
            before = len(self.calls())
            for method, path, body in (
                ("POST", "invitations/agent-brain", {"target": MEMBER_NPUB}),
                ("DELETE", f"invitations/agent-brain/{INVITATION}", None),
                ("POST", "removals/agent-brain", {"npub": ADMIN_NPUB}),
            ):
                response = client.request(method, f"/api/plugins/finite-brain/{path}", json=body)
                self.assertEqual(response.status_code, 409, path)
                self.assertEqual(response.json(), {"error": "busy"})
            self.assertEqual(len(self.calls()), before)
            # Reads stay available while a change runs.
            self.assertEqual(
                client.get("/api/plugins/finite-brain/identities/agent-brain").status_code, 200
            )
            (self.agent / "hold").unlink()
            self.assertEqual(self.wait_for_job(client, job), "removed")
            response = client.post(
                "/api/plugins/finite-brain/invitations/agent-brain", json={"target": MEMBER_NPUB}
            )
            self.assertEqual(response.status_code, 200)
        # A restarted Hermes has no record of the job: unknown, never a guess.
        response = self.client.get(f"/api/plugins/finite-brain/removals/agent-brain/{'f' * 32}")
        self.assertEqual(
            response.json(), {"version": 1, "job": {"id": "f" * 32, "state": "unknown"}}
        )
        response = self.client.get("/api/plugins/finite-brain/removals/agent-brain/--x")
        self.assertEqual(response.json()["job"], {"id": None, "state": "unknown"})

    def test_failed_and_timed_out_changes_report_without_diagnostics(self):
        self.admin_fixture()
        (self.agent / "fail").write_text("invite brain create,invite brain revoke")
        for method, path, body in (
            ("POST", "invitations/agent-brain", {"target": MEMBER_NPUB}),
            ("DELETE", f"invitations/agent-brain/{INVITATION}", None),
        ):
            response = self.client.request(method, f"/api/plugins/finite-brain/{path}", json=body)
            self.assertEqual(response.status_code, 502, path)
            self.assertEqual(response.json(), {"error": "unknown"})
        (self.agent / "fail").unlink()
        (self.agent / "slow-change").touch()
        plugin = sys.modules["hermes_dashboard_plugin_finite-brain"]
        with patch.object(plugin, "READ_SECONDS", 2):
            response = self.client.post(
                "/api/plugins/finite-brain/invitations/agent-brain", json={"target": MEMBER_NPUB}
            )
        # The command started, so the server may have created it.
        self.assertEqual(response.status_code, 504)
        self.assertEqual(response.json(), {"error": "unknown"})
        with self.assertRaises(ProcessLookupError):
            os.kill(int((self.agent / "pid").read_text()), 0)
        (self.agent / "mode").write_text("failure")
        (self.agent / "slow-change").unlink()
        response = self.client.post(
            "/api/plugins/finite-brain/invitations/agent-brain", json={"target": MEMBER_NPUB}
        )
        self.assertEqual(response.status_code, 403)
        self.assertNotIn("private", response.text)

    @unittest.skipUnless(
        os.environ.get("FBRAIN_TEST_BINARY") and os.environ.get("FBRAIN_SERVER_TEST_BINARY"),
        "real fbrain and Brain server binaries supplied by the caller",
    )
    def test_real_brain_server_roster_changes_are_signed_by_the_agent(self):
        """Native router and auth, the real fbrain and a real local Brain server."""
        import socket

        lab = self.home / "roster-lab"
        binaries = lab / "bin"
        binaries.mkdir(parents=True)
        real = Path(os.environ["FBRAIN_TEST_BINARY"]).resolve()
        # A pass-through shim that can pause one removal step, to hold the
        # Agent's change slot open while the real command waits.
        (binaries / "fbrain").write_text(
            f"#!{sys.executable}\n"
            "import os, pathlib, sys, time\n"
            "hold = pathlib.Path(os.environ['ROSTER_LAB']) / 'hold'\n"
            "while sys.argv[1:4] == ['admin', 'member', 'remove'] and hold.exists():\n"
            "    time.sleep(0.02)\n"
            f"os.execv({str(real)!r}, [{str(real)!r}, *sys.argv[1:]])\n"
        )
        (binaries / "fbrain").chmod(0o700)
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        # Forward actual signed requests to the actual store, then inject a
        # failure only after a successful commit. This does not fake CLI exits.
        fault = {"mode": None, "route": None, "attempts": 0, "commits": 0, "hits": 0}

        class FaultProxy(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def forward(self):
                if (
                    fault["mode"] == "mounts-after-remove"
                    and fault["commits"]
                    and self.command == "GET"
                    and self.path.endswith("/mounts")
                ):
                    fault["hits"] += 1
                    self.send_response(503)
                    self.end_headers()
                    return
                body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
                if (self.command, self.path) == fault["route"]:
                    fault["attempts"] += 1
                connection = http.client.HTTPConnection("127.0.0.1", port, timeout=30)
                try:
                    connection.request(self.command, self.path, body, dict(self.headers))
                    response = connection.getresponse()
                    payload = response.read()
                    if (self.command, self.path) == fault["route"] and 200 <= response.status < 300:
                        fault["commits"] += 1
                        if fault["mode"] == "lost-response":
                            fault["hits"] += 1
                            self.close_connection = True
                            self.connection.shutdown(socket.SHUT_RDWR)
                            return
                    self.send_response(response.status)
                    for key, value in response.getheaders():
                        if key.lower() not in ("content-length", "transfer-encoding", "connection"):
                            self.send_header(key, value)
                    self.send_header("Content-Length", str(len(payload)))
                    self.end_headers()
                    self.wfile.write(payload)
                finally:
                    connection.close()

            do_GET = do_POST = do_DELETE = do_PUT = forward

        proxy = ThreadingHTTPServer(("127.0.0.1", 0), FaultProxy)
        proxy_thread = threading.Thread(target=proxy.serve_forever, daemon=True)
        proxy_thread.start()
        self.addCleanup(proxy.server_close)
        self.addCleanup(proxy.shutdown)
        url = f"http://127.0.0.1:{proxy.server_port}"
        log = (lab / "server.log").open("w")
        server = subprocess.Popen(
            [os.environ["FBRAIN_SERVER_TEST_BINARY"]],
            env={
                "PATH": os.environ["PATH"],
                "HOME": str(lab),
                "FINITE_BRAIN_ADDR": f"127.0.0.1:{port}",
                "FINITE_BRAIN_PUBLIC_BASE_URL": url,
                "FINITE_BRAIN_DB": str(lab / "brain.sqlite3"),
                "FINITE_BRAIN_PROTECTED_RATE_LIMIT": "100000:60",
            },
            stdout=log,
            stderr=subprocess.STDOUT,
        )
        self.addCleanup(log.close)
        self.addCleanup(server.wait)
        self.addCleanup(server.terminate)
        deadline = time.monotonic() + 60
        while True:
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                    break
            except OSError:
                if server.poll() is not None or time.monotonic() > deadline:
                    log.flush()
                    self.fail(
                        "Brain server did not start:\n"
                        + (lab / "server.log").read_text(errors="replace")[-4000:]
                    )
                time.sleep(0.05)

        def agent_env(label):
            return {
                "FINITE_HOME": str(lab / label),
                "FBRAIN_CONFIG_DIR": str(lab / label / "brain-config"),
                "FINITE_BRAIN_SERVER_URL": url,
                "ROSTER_LAB": str(lab),
                "PATH": f"{binaries}{os.pathsep}{os.environ['PATH']}",
            }

        def cli(label, *args, cwd=None, check=True):
            result = subprocess.run(
                [str(real), *args],
                env={**os.environ, **agent_env(label), "HOME": str(lab / label)},
                cwd=cwd or lab,
                input=os.urandom(32).hex() if args[:2] == ("auth", "import") else None,
                capture_output=True,
                text=True,
            )
            if check:
                self.assertEqual(result.returncode, 0, result.stderr)
                return json.loads(result.stdout) if "--json" in args else result.stdout
            return result

        npub = {}
        for label in ("alice", "bob", "carol", "dave"):
            cli(label, "auth", "import", "--json")
            npub[label] = cli(label, "auth", "status", "--json")["npub"]
        brain = cli("alice", "brain", "create", "organization", "Roster Lab", "--json")["brainId"]
        trees = {label: lab / f"{label}-tree" for label in ("alice", "bob")}
        cli("alice", "open", brain, str(trees["alice"]), "--json")
        cli(
            "alice",
            "folder",
            "create",
            "Shared Notes",
            "--access",
            "all_members",
            "--json",
            cwd=trees["alice"],
        )
        cli("alice", "sync", "now", "--json", cwd=trees["alice"])
        folder = trees["alice"] / "Shared Notes"
        (folder / "before.md").write_text("# Before\n\nWritten before the removal.\n")
        cli("alice", "sync", "now", "--json", cwd=trees["alice"])

        def key_version():
            metadata = cli("alice", "brain", "metadata", "--brain", brain, "--json")
            return {item["id"]: item["currentKeyVersion"] for item in metadata["folders"]}

        def roster(label="alice"):
            with patch.dict(os.environ, agent_env(label)):
                response = self.client.get(f"/api/plugins/finite-brain/identities/{brain}")
            self.assertEqual(response.status_code, 200, response.text)
            return response.json()

        def change(label, method, path, body=None):
            with patch.dict(os.environ, agent_env(label)):
                return self.client.request(method, f"/api/plugins/finite-brain/{path}", json=body)

        def invite_and_accept(label):
            response = change("alice", "POST", f"invitations/{brain}", {"target": npub[label]})
            self.assertEqual(response.status_code, 200, response.text)
            invitation = response.json()["invitation"]
            self.assertEqual(invitation["targetNpub"], npub[label])
            inbox = cli(label, "invite", "brain", "list", "--json")["invitations"]
            self.assertEqual([item["id"] for item in inbox], [invitation["id"]])
            cli(label, "invite", "brain", "accept", invitation["id"], "--json")
            # An admin's ordinary sync wraps existing Folder Keys for the new Member.
            cli("alice", "sync", "now", "--json", cwd=trees["alice"])

        timings = {}

        def remove(label, client):
            with patch.dict(os.environ, agent_env("alice")):
                started = time.monotonic()
                response = client.post(
                    f"/api/plugins/finite-brain/removals/{brain}", json={"npub": npub[label]}
                )
                self.assertEqual(response.status_code, 202, response.text)
                state = self.wait_for_job(client, response.json()["job"]["id"], brain)
                timings[label] = round(time.monotonic() - started, 3)
            return state

        changes = roster()["changes"]
        self.assertEqual(changes["actingNpub"], npub["alice"])
        self.assertEqual(changes["actions"], ["invite", "revokeInvitation", "remove"])

        # Invite through the route; the invitee accepts with its own CLI.
        invite_and_accept("bob")
        cli("bob", "open", brain, str(trees["bob"]), "--json")
        self.assertIn("Written before", (trees["bob"] / "Shared Notes/before.md").read_text())
        rows = {row["npub"]: row for row in roster()["identities"]}
        self.assertEqual(rows[npub["bob"]]["role"], "member")
        invite_and_accept("carol")
        cli(
            "alice",
            "admin",
            "role",
            "grant",
            "admin",
            "--brain",
            brain,
            "--target",
            npub["carol"],
            "--json",
        )

        # Revoke through the route: the invitee no longer sees it.
        response = change("alice", "POST", f"invitations/{brain}", {"target": npub["dave"]})
        invitation = response.json()["invitation"]["id"]
        listed = change("alice", "GET", f"invitations/{brain}").json()["invitations"]
        self.assertEqual([item["id"] for item in listed], [invitation])
        response = change("alice", "DELETE", f"invitations/{brain}/{invitation}")
        self.assertEqual(response.status_code, 200, response.text)
        self.assertEqual(change("alice", "GET", f"invitations/{brain}").json()["invitations"], [])
        self.assertEqual(cli("dave", "invite", "brain", "list", "--json")["invitations"], [])
        self.assertNotEqual(
            cli("dave", "invite", "brain", "accept", invitation, check=False).returncode, 0
        )

        # Refusals change nothing.
        before = key_version()
        for label, method, path, body, status in (
            ("alice", "POST", f"removals/{brain}", {"npub": npub["alice"]}, 422),
            ("alice", "POST", f"removals/{brain}", {"npub": npub["dave"]}, 422),
            ("bob", "POST", f"invitations/{brain}", {"target": npub["dave"]}, 403),
            ("bob", "POST", f"removals/{brain}", {"npub": npub["carol"]}, 403),
            ("dave", "POST", f"invitations/{brain}", {"target": npub["dave"]}, 403),
            ("alice", "POST", f"invitations/{brain}", {"target": "--server"}, 400),
            ("alice", "POST", f"invitations/{brain}", {"target": f"{npub['dave']} --json"}, 400),
            ("alice", "POST", "invitations/--server", {"target": npub["dave"]}, 400),
        ):
            response = change(label, method, path, body)
            self.assertEqual(response.status_code, status, (label, path, body, response.text))
        self.assertEqual(change("alice", "GET", f"invitations/{brain}").json()["invitations"], [])
        self.assertEqual(cli("dave", "invite", "brain", "list", "--json")["invitations"], [])
        # A real Personal Brain, created the way the dashboard does: the owner
        # signs the creation and the Agent signs its consent.
        cli("owner", "auth", "import", "--json")
        owner = cli("owner", "auth", "status", "--json")["npub"]
        consent = cli("alice", "brain", "personal-agent-consent", "--owner", owner, "--json")
        body = json.dumps(
            {
                "brainId": consent["brainId"],
                "kind": "personal",
                "name": "Personal Brain",
                "personalAgentNpub": npub["alice"],
                "personalAgentConsent": consent["consent"],
            }
        ).encode()
        signed = cli(
            "owner", "signer", "sign", "--kind", "http-auth", "--content", "",
            "--tag", f"u,{url}/v1/brains", "--tag", "method,POST",
            "--tag", f"nonce,{os.urandom(16).hex()}",
            "--tag", f"payload,{hashlib.sha256(body).hexdigest()}", "--json",
        )  # fmt: skip
        created = urllib.request.urlopen(
            urllib.request.Request(
                f"{url}/v1/brains",
                data=body,
                method="POST",
                headers={
                    "content-type": "application/json",
                    "authorization": "Nostr "
                    + base64.b64encode(signed["eventJson"].encode()).decode(),
                },
            ),
            timeout=10,
        )
        self.assertEqual(created.status, 200)
        personal = consent["brainId"]
        self.assertEqual(roster()["changes"]["actions"], ["invite", "revokeInvitation", "remove"])
        response = change("alice", "POST", f"invitations/{personal}", {"target": npub["dave"]})
        self.assertEqual(response.status_code, 403)
        with patch.dict(os.environ, agent_env("alice")):
            self.assertEqual(
                self.client.get(f"/api/plugins/finite-brain/identities/{personal}").json()[
                    "changes"
                ]["actions"],
                [],
            )
        self.assertEqual(key_version(), before)
        self.assertEqual({row["role"] for row in roster()["identities"]}, {"admin", "member"})

        with self.change_client() as client:
            # Remove a Member: the Folder is re-keyed and later writes stay unreadable.
            self.assertEqual(remove("bob", client), "removed")
            after = key_version()
            self.assertEqual(after["shared-notes"], before["shared-notes"] + 1)
            self.assertNotIn(npub["bob"], {row["npub"] for row in roster()["identities"]})
            (folder / "after.md").write_text("# After\n\nSecret written after the removal.\n")
            cli("alice", "sync", "now", "--json", cwd=trees["alice"])
            cli("bob", "sync", "now", "--json", cwd=trees["bob"], check=False)
            leaked = [
                path
                for path in trees["bob"].rglob("*")
                if path.is_file() and b"Secret written after" in path.read_bytes()
            ]
            self.assertEqual(leaked, [])

            # Remove an Admin: demoted first, then removed.
            self.assertEqual(remove("carol", client), "removed")
            self.assertNotIn(npub["carol"], {row["npub"] for row in roster()["identities"]})

            # One change at a time, with the real command paused mid-removal.
            invite_and_accept("dave")
            (lab / "hold").touch()
            with patch.dict(os.environ, agent_env("alice")):
                response = client.post(
                    f"/api/plugins/finite-brain/removals/{brain}", json={"npub": npub["dave"]}
                )
                job = response.json()["job"]["id"]
                busy = client.post(
                    f"/api/plugins/finite-brain/invitations/{brain}", json={"target": npub["bob"]}
                )
                self.assertEqual(busy.status_code, 409)
                (lab / "hold").unlink()
                self.assertEqual(self.wait_for_job(client, job, brain), "removed")
        self.assertEqual({row["npub"] for row in roster()["identities"]}, {npub["alice"]})

        # A Member or Admin can be gone even though its command exits nonzero:
        # the final mounts GET failed, or the committed DELETE response was lost.
        # Report uncertainty and let a separate fresh roster establish the truth.
        with self.change_client() as client:
            for mode in ("mounts-after-remove", "lost-response"):
                for admin in (False, True):
                    label = f"fault-{mode}-{admin}"
                    cli(label, "auth", "import", "--json")
                    npub[label] = cli(label, "auth", "status", "--json")["npub"]
                    invite_and_accept(label)
                    if admin:
                        cli(
                            "alice",
                            "admin",
                            "role",
                            "grant",
                            "admin",
                            "--brain",
                            brain,
                            "--target",
                            npub[label],
                            "--json",
                        )
                    before_fault = key_version()["shared-notes"]
                    fault.update(
                        mode=mode,
                        route=("DELETE", f"/v1/admin/brains/{brain}/members/{npub[label]}"),
                        attempts=0,
                        commits=0,
                        hits=0,
                    )
                    state = remove(label, client)
                    fault["mode"] = None
                    with self.subTest(mode=mode, admin=admin):
                        # Prove that this is a post-commit error, without a retry.
                        self.assertEqual(fault["commits"], 1)
                        self.assertEqual(fault["attempts"], 1)
                        self.assertEqual(fault["hits"], 1)
                        self.assertEqual(key_version()["shared-notes"], before_fault + 1)
                        self.assertNotIn(
                            npub[label], {row["npub"] for row in roster()["identities"]}
                        )
                        self.assertEqual(state, "unknown")

        # The same uncertainty rule covers a committed invite/revoke whose
        # response never reached the CLI. Neither operation is sent twice.
        label = "fault-invitation"
        cli(label, "auth", "import", "--json")
        npub[label] = cli(label, "auth", "status", "--json")["npub"]
        fault.update(
            mode="lost-response",
            route=("POST", f"/v1/brains/{brain}/invitations"),
            attempts=0,
            commits=0,
            hits=0,
        )
        response = change("alice", "POST", f"invitations/{brain}", {"target": npub[label]})
        fault["mode"] = None
        self.assertEqual((fault["commits"], fault["hits"]), (1, 1))
        self.assertEqual(fault["attempts"], 1)
        self.assertEqual(response.status_code, 502, response.text)
        self.assertEqual(response.json(), {"error": "unknown"})
        inbox = cli(label, "invite", "brain", "list", "--json")["invitations"]
        self.assertEqual(len(inbox), 1)
        invitation = inbox[0]["id"]
        fault.update(
            mode="lost-response",
            route=("DELETE", f"/v1/invitations/{invitation}"),
            attempts=0,
            commits=0,
            hits=0,
        )
        response = change("alice", "DELETE", f"invitations/{brain}/{invitation}")
        fault["mode"] = None
        self.assertEqual((fault["commits"], fault["hits"]), (1, 1))
        self.assertEqual(fault["attempts"], 1)
        self.assertEqual(response.status_code, 502, response.text)
        self.assertEqual(response.json(), {"error": "unknown"})
        self.assertEqual(cli(label, "invite", "brain", "list", "--json")["invitations"], [])
        print(f"\nreal Brain removal timings (seconds): {json.dumps(timings)}")


class ProcessCancellationTests(unittest.IsolatedAsyncioTestCase):
    async def test_cancelled_request_reaps_process(self):
        # Independent loop; do not reuse the native server's bound semaphore.
        from hermes_cli import finite_dashboard_reads
        from hermes_cli.finite_dashboard_reads import read_json

        with tempfile.TemporaryDirectory() as directory:
            pid_file = Path(directory) / "pid"
            with patch.object(finite_dashboard_reads, "_PROCESSES", asyncio.Semaphore(1)):
                task = asyncio.create_task(
                    read_json(
                        sys.executable,
                        "-c",
                        "import os,sys,time; open(sys.argv[1], 'w').write(str(os.getpid())); time.sleep(60)",
                        str(pid_file),
                    )
                )
                async with asyncio.timeout(5):
                    while not pid_file.exists():
                        await asyncio.sleep(0.01)
                task.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await task
                with self.assertRaises(ProcessLookupError):
                    os.kill(int(pid_file.read_text()), 0)

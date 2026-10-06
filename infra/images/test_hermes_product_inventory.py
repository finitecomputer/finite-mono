"""Native Hermes auth/routing and bounded product CLI contract tests.

Defaults to sealed packaged modules/plugins. Source mode is a fast local test,
not packaging evidence. Every CLI and HOME in these tests is disposable.
"""

import asyncio
import base64
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
OWNER_NPUB = "npub1" + "q" * 58
AGENT_NPUB = "npub1" + "p" * 58


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

    def test_brain_identities_project_only_core_resolved_roster_fields(self):
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
            ],
        }
        (self.agent / "access.json").write_text(json.dumps(report))
        response = self.client.get("/api/plugins/finite-brain/identities/agent-brain")
        self.assertEqual(response.status_code, 200, response.text)
        empty = {"kind": None, "name": None, "email": None, "ownerEmail": None}
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
                    "folders": [{"id": "folder", "state": "ready"}],
                },
                {"npub": "npub1b", "role": "admin", **empty, "folders": []},
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

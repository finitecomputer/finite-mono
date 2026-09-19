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
if mode == "oversize":
    sys.stdout.write("x" * (512 * 1024 + 1))
    sys.exit(0)
if mode == "slow":
    (home / "pid").write_text(str(os.getpid()))
    time.sleep(60)
if "--existing-identity" not in sys.argv:
    sys.exit(2)
if sys.argv[1:3] == ["brain", "metadata"]:
    if (home / "missing-metadata").exists(): sys.exit(1)
    payload = json.loads((home / "metadata.json").read_text())
    payload["brainId"] = sys.argv[3]
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
        self.write_data()

    def write_data(self):
        for name, payload in (
            ("brains", self.brains),
            ("metadata", self.metadata),
            ("sites", self.sites),
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
                ["brain", "metadata", "agent-brain", "--json", "--existing-identity"],
            ],
        )
        (self.agent / "missing-metadata").touch()
        response = self.get()
        self.assertEqual(response.status_code, 200)
        self.assertIsNone(response.json()["brains"][0]["folders"])

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

        class Handler(BaseHTTPRequestHandler):
            def do_GET(handler):
                header = handler.headers.get("Authorization", "")
                event = json.loads(base64.b64decode(header.removeprefix("Nostr ")))
                label = identities[event["pubkey"]]
                observed.append((label, handler.path))
                if handler.path == "/v1/brains":
                    data = {
                        "brains": [
                            {
                                "brainId": label,
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
        finally:
            server.shutdown()
            worker.join()
            server.server_close()

    def test_ids_cannot_become_cli_flags_or_path_traversal(self):
        for invalid_id in ("--server", "../other", "brain?query", ""):
            self.brains["brains"][0]["brainId"] = invalid_id
            self.write_data()
            self.assertEqual(self.get().status_code, 503)


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

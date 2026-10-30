"""The dashboard's Agent export through native Hermes routing.

Defaults to the sealed packaged plugin (`HERMES_BUNDLED_PLUGINS`). With
FINITE_EXPORT_SOURCE_TEST=1 it loads the plugin from source: a fast local
test, not packaging evidence. The agent, its CLIs and every HOME are
synthetic and disposable.
"""

import errno
import hashlib
import io
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest
import zipfile
from datetime import datetime, timedelta
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
BASE = "/api/plugins/finite-export"
PERSONAL = "personal-0123456789abcdef"
MiB = 1024 * 1024
# Synthetic credentials: none may reach the archive.
OPENROUTER_KEY = "sk-or-v1-" + "7" * 48
TELEGRAM_TOKEN = "123456789:" + "A" * 35
IDENTITY_SECRET = "c0ffee" * 10 + "abcd"
NOTE_TOKEN = "ghp_" + "Z9" * 18
LOG_TOKEN = "xoxb-" + "4" * 40
ARCHIVED_KEY = "sk-" + "q8" * 24
LONG_JWT = "eyJ" + "h" * 40 + "." + "p" * 8000 + "." + "s" * 40

FAKE_FBRAIN = """
import json, os, pathlib, sys, time
fixtures = pathlib.Path(os.environ["FINITE_EXPORT_TEST_FIXTURES"])
args = sys.argv[1:]
if (fixtures / "slow").exists():
    time.sleep(60)
if args[:2] == ["brain", "list"]:
    print((fixtures / "brains.json").read_text())
elif args[:1] == ["open"]:
    tree = pathlib.Path(os.environ["FBRAIN_WORKING_TREE_ROOT"]) / args[1]
    print(json.dumps({"path": str(tree), "syncStatus": "synced"}))
elif args[:2] == ["sync", "now"]:
    print(json.dumps({"conflicts": []}))
elif args[:2] == ["folder", "list"]:
    print(json.dumps({"folders": []}))
else:
    sys.exit(2)
"""

FAKE_FSITE = """
import os, pathlib, sys
fixtures = pathlib.Path(os.environ["FINITE_EXPORT_TEST_FIXTURES"])
if sys.argv[1:3] == ["project", "list"]:
    print((fixtures / "projects.json").read_text())
else:
    sys.exit(2)
"""


def run(*argv, cwd=None):
    subprocess.run(argv, cwd=cwd, check=True, capture_output=True)


class AgentExportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.scratch = tempfile.TemporaryDirectory()
        home = Path(cls.scratch.name)
        cls.agent = home / "agent"
        cls.hermes = cls.agent / "hermes-home"
        cls.workspace = home / "workspace"
        cls.fixtures = home / "fixtures"
        bundled = home / "bundled"
        source = (
            ROOT / "finite-agentd/integrations/hermes/finite-export"
            if os.environ.get("FINITE_EXPORT_SOURCE_TEST") == "1"
            else Path(os.environ["HERMES_BUNDLED_PLUGINS"]) / "finite-export"
        )
        shutil.copytree(source, bundled / "finite-export")
        bin_dir = home / "bin"
        for directory in (cls.hermes, cls.workspace, cls.fixtures, bin_dir):
            directory.mkdir(parents=True)
        for name, body in (("fbrain", FAKE_FBRAIN), ("fsite", FAKE_FSITE)):
            (bin_dir / name).write_text(f"#!{sys.executable}\n{body}")
            (bin_dir / name).chmod(0o755)
        cls.environment = patch.dict(
            os.environ,
            {
                "HERMES_HOME": str(cls.hermes),
                "HERMES_BUNDLED_PLUGINS": str(bundled),
                "FINITE_HOME": str(cls.agent),
                "FINITECHAT_WORKSPACE": str(cls.workspace),
                "FBRAIN_WORKING_TREE_ROOT": str(cls.workspace / "finitebrain"),
                "FINITE_AGENT_NAME": "Moss",
                "FINITE_EXPORT_TEST_FIXTURES": str(cls.fixtures),
                "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
            },
        )
        cls.environment.start()
        cls.make_agent(home)
        from hermes_cli import web_server
        from starlette.testclient import TestClient

        cls.client = TestClient(web_server.app, raise_server_exceptions=False)
        cls.client.headers[web_server._SESSION_HEADER_NAME] = web_server._SESSION_TOKEN
        cls.anonymous = TestClient(web_server.app, raise_server_exceptions=False)

    @classmethod
    def make_agent(cls, home):
        from hermes_state import SessionDB

        sessions = SessionDB(cls.hermes / "state.db")
        sessions.create_session("chat-1", "finitechat")
        sessions.append_message("chat-1", "user", "Plan the garden for spring.")
        sessions.append_message("chat-1", "assistant", "Tomatoes along the south fence.")
        sessions.close()
        (cls.hermes / "SOUL.md").write_text("You are Moss, a careful gardener's agent.\n")
        (cls.hermes / "memories").mkdir(exist_ok=True)
        (cls.hermes / "memories/MEMORY.md").write_text("The garden faces south.\n")
        skill = cls.hermes / "skills/garden/seed-planner"
        skill.mkdir(parents=True)
        (skill / "SKILL.md").write_text(
            "---\nname: seed-planner\ndescription: Plan seed starts.\n---\nCount weeks back.\n"
        )
        (cls.hermes / ".env").write_text(f"OPENROUTER_API_KEY={OPENROUTER_KEY}\n")
        (cls.hermes / "config.yaml").write_text(
            f"gateway:\n  platforms:\n    telegram:\n      token: {TELEGRAM_TOKEN}\n"
        )
        (cls.agent / "identity").mkdir()
        (cls.agent / "identity/identity.json").write_text(
            json.dumps({"kind": "nostr-secp256k1", "secret_hex": IDENTITY_SECRET})
        )

        workspace = cls.workspace
        (workspace / "notes").mkdir()
        (workspace / "notes/plan.md").write_text("# Spring plan\nStart peppers in February.\n")
        (workspace / "notes/keys.txt").write_text(f"github: {NOTE_TOKEN}\n")
        # Past the old 2 MiB scan, and across the scan's 1 MiB chunk boundary.
        log = "x" * (MiB - 20) + f"slack {LOG_TOKEN}\n" + "y" * (2 * MiB)
        (workspace / "notes/service.log").write_text(log)
        # Starts well before the boundary and ends well after it.
        log = "a" * (MiB - 5001) + f" {LONG_JWT}\n" + "b" * 1024
        (workspace / "notes/session.log").write_text(log)
        (workspace / "data").mkdir()
        (workspace / "data/minified.json").write_text("0" * (17 * MiB))
        (workspace / "backups").mkdir()
        with zipfile.ZipFile(workspace / "backups/old-laptop.zip", "w") as backup:
            backup.writestr(".env", f"OPENAI_API_KEY={ARCHIVED_KEY}\n")
        (workspace / "media").mkdir()
        cls.clip = os.urandom(30 * MiB)
        (workspace / "media/walkthrough.mp4").write_bytes(cls.clip)
        (workspace / "harvest.csv").write_text("week,kg\n" + "1,2.5\n" * (6 * MiB // 6))
        (workspace / "app/node_modules/left-pad").mkdir(parents=True)
        (workspace / "app/package.json").write_text("{}")
        (workspace / "app/node_modules/left-pad/index.js").write_text("module.exports = 1;\n")
        (workspace / "finite-export").mkdir()
        (workspace / "finite-export/earlier-export.zip").write_bytes(b"PK earlier export")
        brain = workspace / "finitebrain" / PERSONAL
        brain.mkdir(parents=True)
        (brain / "garden.md").write_text("# Garden\nCompost every autumn.\n")

        site = home / "recipes.git"
        work = home / "recipes-work"
        run("git", "init", "--quiet", "--bare", "--initial-branch=main", str(site))
        run("git", "init", "--quiet", "--initial-branch=main", str(work))
        (work / "index.html").write_text("<h1>Recipes</h1>\n")
        run("git", "add", "index.html", cwd=work)
        run(
            "git", "-c", "user.name=Moss", "-c", "user.email=moss@example.invalid",
            "commit", "--quiet", "-m", "site", cwd=work,
        )  # fmt: skip
        run("git", "push", "--quiet", str(site), "main", cwd=work)

        (cls.fixtures / "brains.json").write_text(
            json.dumps(
                {
                    "brains": [
                        {
                            "brainId": PERSONAL,
                            "name": "Moss notes",
                            "kind": "personal",
                            "role": "owner",
                        },
                        {
                            "brainId": "team-garden",
                            "name": "Team",
                            "kind": "organization",
                            "role": "member",
                        },
                        {
                            "brainId": "pending",
                            "name": "Invite",
                            "kind": "organization",
                            "role": "invited",
                        },
                    ]
                }
            )
        )
        site_record = {
            "url": "https://recipes.finite.site",
            "visibility": "public",
            "branch": "main",
        }
        (cls.fixtures / "projects.json").write_text(
            json.dumps(
                {
                    "projects": [
                        {"slug": "recipes", "role": "owner", "site": site_record,
                         "git_remote_url": site.as_uri()},
                        {"slug": "team-site", "role": "editor",
                         "site": {**site_record, "url": "https://team-site.finite.site"}},
                        {"slug": "scratch", "role": "owner", "site": None},
                    ]
                }
            )
        )  # fmt: skip

    @classmethod
    def tearDownClass(cls):
        cls.stop_jobs()
        cls.environment.stop()
        cls.scratch.cleanup()

    @classmethod
    def stop_jobs(cls):
        for kind in ("survey", "export"):
            try:
                record = json.loads((cls.agent / "exports" / kind / "job.json").read_text())
                os.killpg(record["pid"], signal.SIGKILL)
            except (OSError, ValueError, KeyError):
                pass

    def setUp(self):
        # The plugin's free-space rules never depend on the test machine's disk.
        disk = patch("shutil.disk_usage", return_value=SimpleNamespace(free=10**15))
        disk.start()
        self.addCleanup(disk.stop)
        self.stop_jobs()
        (self.fixtures / "slow").unlink(missing_ok=True)
        shutil.rmtree(self.agent / "exports", ignore_errors=True)

    def status(self):
        response = self.client.get(f"{BASE}/status")
        self.assertEqual(response.status_code, 200, response.text)
        self.assertEqual(response.headers["cache-control"], "no-store")
        return response.json()

    def wait(self, kind, timeout=180):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            job = self.status()[kind]
            if job and job["state"] != "running":
                return job
            time.sleep(0.1)
        self.fail(f"the {kind} did not finish")

    def assert_stopped(self, group):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            # A status read reaps the killed wrapper; until then it is a zombie,
            # which Linux still counts in its process group.
            self.status()
            try:
                os.killpg(group, 0)
            except ProcessLookupError:
                return
            except PermissionError:  # macOS answers EPERM for a group of zombies
                return
            time.sleep(0.05)
        self.fail("the export kept running")

    def survey(self):
        response = self.client.post(f"{BASE}/survey")
        self.assertEqual(response.status_code, 202, response.text)
        self.assertEqual(response.json()["survey"]["state"], "running")
        survey = self.wait("survey")
        self.assertEqual(survey["state"], "ready", survey)
        return survey["summary"]

    def start_export(self, brains, sites):
        return self.client.post(f"{BASE}/export", json={"brains": brains, "sites": sites})

    def test_owner_exports_everything_and_downloads_it(self):
        self.assertEqual(self.status(), {"version": 1, "survey": None, "export": None})
        summary = self.survey()
        self.assertEqual(summary["conversations"], 1)
        self.assertEqual(summary["memories"], 1)
        self.assertEqual(
            summary["brains"],
            [
                {"id": PERSONAL, "name": "Moss notes", "kind": "personal", "bytes": 31},
                {"id": "team-garden", "name": "Team", "kind": "organization", "bytes": None},
            ],
        )
        self.assertEqual(
            summary["sites"],
            [
                {"slug": "recipes", "url": "https://recipes.finite.site", "owned": True},
                {"slug": "team-site", "url": "https://team-site.finite.site", "owned": False},
            ],
        )
        # Media and large files count; dependencies, archives and earlier exports
        # don't. The survey judges secrets by name; the copy reads every text file.
        self.assertEqual(summary["files"], 8)
        self.assertGreater(summary["fileBytes"], 36 * MiB)

        response = self.start_export([PERSONAL], ["recipes"])
        self.assertEqual(response.status_code, 202, response.text)
        export = self.wait("export")
        self.assertEqual(export["state"], "ready", export)
        self.assertGreater(export["bytes"], 25 * MiB)
        self.assertRegex(export["fileName"], r"^finite-export-moss-\d{8}-\d{4}\.zip$")
        self.assertEqual(export["problems"], 0)
        kept = datetime.fromisoformat(export["expiresAt"]) - datetime.fromisoformat(
            export["finishedAt"]
        )
        self.assertEqual(kept, timedelta(days=7))
        # The staging copy is gone; only the zip and its record remain.
        self.assertEqual(list((self.agent / "exports/export/tmp").iterdir()), [])

        self.assertEqual(self.anonymous.get(f"{BASE}/download").status_code, 401)
        download = self.client.get(f"{BASE}/download")
        self.assertEqual(download.status_code, 200)
        self.assertEqual(download.headers["content-type"], "application/zip")
        self.assertIn(f'filename="{export["fileName"]}"', download.headers["content-disposition"])
        self.assertEqual(len(download.content), export["bytes"])
        resumed = self.client.get(f"{BASE}/download", headers={"Range": "bytes=100-199"})
        self.assertEqual(resumed.status_code, 206)
        self.assertEqual(resumed.content, download.content[100:200])

        for secret in (
            OPENROUTER_KEY, TELEGRAM_TOKEN, IDENTITY_SECRET, NOTE_TOKEN, LOG_TOKEN, ARCHIVED_KEY,
            LONG_JWT,
        ):  # fmt: skip
            self.assertNotIn(secret.encode(), download.content)
        with zipfile.ZipFile(io.BytesIO(download.content)) as archive:
            names = set(archive.namelist())
            manifest = json.loads(archive.read("manifest.json"))
            self.assertEqual(manifest["format"], "finite-agent-export/1")
            self.assertTrue(manifest["choices"]["everything"])
            for entry in manifest["files"]:
                digest = hashlib.sha256(archive.read(entry["path"])).hexdigest()
                self.assertEqual(digest, entry["sha256"], entry["path"])
            self.assertEqual(archive.read("workspace/media/walkthrough.mp4"), self.clip)
            info = archive.getinfo("workspace/media/walkthrough.mp4")
            self.assertEqual(info.compress_type, zipfile.ZIP_STORED)
            for name in (
                "workspace/harvest.csv",
                "workspace/notes/plan.md",
                f"brains/{PERSONAL}/files/garden.md",
                "sites/recipes/source/index.html",
                "chats/index.json",
                "agent/SOUL.md",
                "memories/MEMORY.md",
            ):
                self.assertIn(name, names)
            leaked = [
                name
                for name in names
                if name.endswith(
                    (
                        ".env",
                        "identity.json",
                        "config.yaml",
                        "keys.txt",
                        ".log",
                        ".zip",
                        "minified.json",
                    )
                )
                or "node_modules" in name
                or "earlier-export" in name
                or name.startswith(
                    ("brains/team-garden", "sites/team-site", "workspace/finitebrain")
                )
            ]
            self.assertEqual(leaked, [])
            skipped = {
                entry.get("kind") or entry["path"]: entry["reason"] for entry in manifest["skipped"]
            }
            for name in ("keys.txt", "service.log", "session.log"):
                self.assertEqual(skipped[f"workspace/notes/{name}"], "looks like it holds a secret")
            self.assertEqual(
                skipped["workspace/data/minified.json"], "a line too long to check for credentials"
            )
            self.assertIn("archives", skipped)
            chats = json.loads(archive.read("chats/index.json"))
        self.assertEqual(len(chats["conversations"]), 1)

        self.assertEqual(self.client.delete(f"{BASE}/export").status_code, 200)
        self.assertIsNone(self.status()["export"])
        self.assertEqual(self.client.get(f"{BASE}/download").status_code, 404)
        self.assertFalse((self.agent / "exports/export").exists())

    def test_choices_are_checked_before_anything_runs(self):
        self.assertEqual(self.start_export([], []).json(), {"error": "surveyFirst"})
        self.survey()
        bad = (
            {"brains": [PERSONAL]},
            {"brains": [PERSONAL], "sites": [], "extra": True},
            {"brains": "all", "sites": []},
            {"brains": ["--out=/tmp"], "sites": []},
            {"brains": [PERSONAL, PERSONAL], "sites": []},
        )
        for body in bad:
            response = self.client.post(f"{BASE}/export", json=body)
            self.assertEqual(response.status_code, 400, body)
        self.assertEqual(
            self.client.post(f"{BASE}/export", content=b'{"brains":[],"sites":[]}').status_code, 400
        )
        self.assertEqual(self.client.post(f"{BASE}/survey?again=1").status_code, 400)
        for brains, sites in ((["pending"], []), ([], ["scratch"]), (["unknown"], [])):
            response = self.start_export(brains, sites)
            self.assertEqual(
                (response.status_code, response.json()), (422, {"error": "notOffered"})
            )
        with patch("shutil.disk_usage", return_value=SimpleNamespace(free=0)):
            response = self.start_export([PERSONAL], [])
        self.assertEqual(
            (response.status_code, response.json()), (507, {"error": "notEnoughSpace"})
        )
        self.assertIsNone(self.status()["export"])

    def test_one_job_at_a_time_and_cancel_stops_it(self):
        self.survey()
        (self.fixtures / "slow").touch()
        self.assertEqual(self.start_export([PERSONAL], []).status_code, 202)
        self.assertEqual(self.status()["export"]["state"], "running")
        self.assertEqual(self.client.post(f"{BASE}/survey").json(), {"error": "busy"})
        self.assertEqual(self.start_export([], []).json(), {"error": "busy"})
        record = json.loads((self.agent / "exports/export/job.json").read_text())
        self.assertEqual(self.client.delete(f"{BASE}/export").status_code, 200)
        self.assertIsNone(self.status()["export"])
        self.assert_stopped(record["pid"])

    def test_a_running_export_stops_before_the_shared_disk_runs_low(self):
        self.survey()
        (self.fixtures / "slow").touch()
        self.assertEqual(self.start_export([PERSONAL], []).status_code, 202)
        record = json.loads((self.agent / "exports/export/job.json").read_text())
        with patch("shutil.disk_usage", return_value=SimpleNamespace(free=0)):
            self.assertEqual(self.status()["export"]["state"], "failed")
        self.assertFalse((self.agent / "exports/export/tmp").exists())
        self.assert_stopped(record["pid"])

    def test_a_full_disk_still_stops_the_export(self):
        self.survey()
        (self.fixtures / "slow").touch()
        self.assertEqual(self.start_export([PERSONAL], []).status_code, 202)
        record = json.loads((self.agent / "exports/export/job.json").read_text())
        full = OSError(errno.ENOSPC, "No space left on device")
        with (
            patch("shutil.disk_usage", return_value=SimpleNamespace(free=0)),
            patch("pathlib.Path.write_text", side_effect=full),
        ):
            self.status()
        self.assert_stopped(record["pid"])
        self.assertFalse((self.agent / "exports/export/tmp").exists())
        self.assertIn(self.status()["export"]["state"], ("interrupted", "failed"))

    def test_a_job_that_died_reads_as_interrupted_and_old_ones_expire(self):
        self.survey()
        export = self.agent / "exports/export"
        (export / "tmp/staging").mkdir(parents=True)
        dead = subprocess.Popen([sys.executable, "-c", "pass"])
        dead.wait()
        (export / "job.json").write_text(
            json.dumps({"version": 1, "pid": dead.pid, "pidStart": "1", "startedAt": time.time()})
        )
        self.assertEqual(self.status()["export"]["state"], "interrupted")
        self.assertFalse((export / "tmp").exists())
        eight_days_ago = time.time() - 8 * 24 * 3600
        os.utime(self.agent / "exports/survey/exit", (eight_days_ago, eight_days_ago))
        (export / "job.json").write_text(
            json.dumps({"version": 1, "pid": dead.pid, "startedAt": eight_days_ago})
        )
        self.assertEqual(self.status(), {"version": 1, "survey": None, "export": None})
        self.assertFalse(export.exists())


if __name__ == "__main__":
    unittest.main()

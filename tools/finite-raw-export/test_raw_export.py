"""The raw-folder exporter's own tests. Its match with `finite-mover-host`'s
classifier on the host reader's fixture is checked from there
(tests/host.rs, `the_raw_exporter_matches_the_classifier`), which also runs
this file."""

import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import raw_export  # noqa: E402

TOKEN = "ghp_abcdefghij0123456789"


def write(root, path, data):
    target = Path(root, path)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data if isinstance(data, bytes) else data.encode())
    return target


class Shapes(unittest.TestCase):
    """The same cases as finite-mover-host's src/shapes.rs tests."""

    def test_each_token_shape_is_found(self):
        for text in [
            "key: sk-proj4abcdefghijklmnop",
            "sk_live_abcdefgh12345678",
            "rk_live_abcdefgh12345678",
            "ghp_abcdefghij0123456789",
            "github_pat_abcdefghij0123456789",
            "glpat-abcdefghij0123456789",
            "xoxb-123456789012-abc",
            "AIzaSyA1234567890abcdefghijklmnopqrs",
            "AKIA1234567890ABCDEF",
            "ntn_abcdefghij0123456789",
            "hf_abcdefghij0123456789abcdefghij",
            "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw1",
            "nsec1abcdefghij0123456789",
            "eyJhbGciOiJIUzI1NiIsInR5c.eyJzdWIiOiIxMjM0NTY3ODkwIiwib.SflKxwRJSMeKKF2QT4fw",
            "-----BEGIN OPENSSH PRIVATE KEY-----",
            "-----BEGIN PRIVATE KEY-----",
            "https://discord.com/api/webhooks/123/abcDEF",
            "clone https://alice:hunter22secret@example.org/repo",
        ]:
            self.assertTrue(raw_export.holds_secret(text.encode()), text)

    def test_placeholders_and_words_are_not_secrets(self):
        for text in [
            "sk-xxxxxxxxxxxxxxxxxxxxxxxx",
            "sk-your-key-goes-here-please",
            "coindesk-dl0123456789abcdefghij",
            "sk-is-a-slug-for-everything",
            "AKIAIOSFODNN7EXAMPLE",
            "https://USER:TOKEN@host",
            "https://user:${PASSWORD}@host",
            "https://user:pw@host",
            "-----BEGIN PUBLIC KEY-----",
            "nsec1short",
            "plain prose with no credential in it",
        ]:
            self.assertFalse(raw_export.holds_secret(text.encode()), text)

    def test_chunks_utf16_and_long_runs_read_as_the_host_reads_them(self):
        split = b"a" * (raw_export.CHUNK - 10) + b" ghp_abcdefghij0123456789 tail"
        self.assertEqual(raw_export.scan(split), "secret")
        self.assertEqual(raw_export.scan(b"SQLite format 3\0" + TOKEN.encode()), "binary")
        utf16 = b"\xff\xfe" + ("token " + TOKEN).encode("utf-16-le")
        self.assertEqual(raw_export.scan(utf16), "secret")
        endless = b"a" * (raw_export.CARRY_MAX + 2 * raw_export.CHUNK)
        self.assertEqual(raw_export.scan(endless), "uncheckable")


class Config(unittest.TestCase):
    ORIGINAL = (
        "timezone: America/Chicago\n"
        "agent:\n  personality: 'kind and brief'  \n  max_turns: 90\n"
        "model:\n  default: glm-5-3-flash # the default\n  api_key: sk-or-v1-0123456789abcdef0123456789abcdef\n"
        "gateway:\n  platforms:\n    telegram:\n      enabled: true\n      token: '123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw1'\n"
        "mcp_servers:\n  - name: x\n"
    ).encode()
    EXPECTED = (
        'timezone: "America/Chicago"\n'
        'agent:\n  personality: "kind and brief"\n'
        'model:\n  default: "glm-5-3-flash"\n'
        "gateway:\n  platforms:\n    telegram:\n      enabled: true\n"
    ).encode()

    def test_the_copy_holds_only_the_allow_list(self):
        self.assertEqual(raw_export.public_config(self.ORIGINAL), self.EXPECTED)
        quoted = b'model:\n  default: "gpt-x" # the default\n'
        self.assertEqual(raw_export.public_config(quoted), b'model:\n  default: "gpt-x"\n')

    def test_without_pyyaml_there_is_no_copy(self):
        # `import yaml` fails while the module is None.
        saved = sys.modules.get("yaml")
        sys.modules["yaml"] = None
        try:
            self.assertIsNone(raw_export.public_config(self.ORIGINAL))
        finally:
            if saved is None:
                del sys.modules["yaml"]
            else:
                sys.modules["yaml"] = saved

    def test_a_value_shaped_like_a_credential_is_dropped(self):
        copy = raw_export.public_config(b"model: gpt-x\npersonality: use sk-proj4abcdefghijklmnop\n")
        self.assertEqual(copy, b'model:\n  default: "gpt-x"\n')
        self.assertIsNone(raw_export.public_config(b"\xff not text"))


class Export(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory(prefix="raw-export-")
        self.root = Path(self.dir.name, "data")
        self.out = Path(self.dir.name, "out", "raw.zip")
        write(self.root, "agent/hermes-home/SOUL.md", "kind\n")
        write(self.root, "workspace/notes.md", "notes\n")

    def tearDown(self):
        self.dir.cleanup()

    def export(self, **options):
        return raw_export.export_raw(self.root, self.out, **options)

    def names(self):
        with zipfile.ZipFile(self.out) as archive:
            return sorted(archive.namelist())

    def test_a_live_database_is_copied_whole_without_its_sidecars(self):
        database = self.root / "agent/hermes-home/state.db"
        conn = sqlite3.connect(database)
        conn.execute("PRAGMA journal_mode=WAL")
        conn.execute("CREATE TABLE messages (body TEXT)")
        conn.execute("INSERT INTO messages VALUES ('hello from the old agent')")
        conn.commit()
        # The writer stays open, as the agent's does: rows sit in the WAL.
        self.assertTrue(database.with_name("state.db-wal").exists())
        summary = self.export()
        conn.close()
        names = self.names()
        self.assertIn("agent/hermes-home/state.db", names)
        self.assertNotIn("agent/hermes-home/state.db-wal", names)
        copied = Path(self.dir.name, "copied.db")
        with zipfile.ZipFile(self.out) as archive:
            copied.write_bytes(archive.read("agent/hermes-home/state.db"))
        rows = sqlite3.connect(copied).execute("SELECT body FROM messages").fetchall()
        self.assertEqual(rows, [("hello from the old agent",)])
        self.assertEqual(summary["classes"]["history"]["files"], 2)
        self.assertEqual(list(self.out.parent.glob(".raw-export-*")), [], "no temporary copy is left")

    def test_a_linked_sidecar_never_brings_rows_from_outside(self):
        # A database whose WAL is another database's, by a hard link.
        outside = Path(self.dir.name, "outside.db")
        conn = sqlite3.connect(outside)
        conn.execute("PRAGMA journal_mode=WAL")
        conn.execute("PRAGMA wal_autocheckpoint=0")
        conn.execute("CREATE TABLE messages (body TEXT)")
        conn.commit()
        database = self.root / "agent/hermes-home/state.db"
        main_file = outside.read_bytes()
        database.write_bytes(main_file)
        conn.execute("INSERT INTO messages VALUES ('a private row from outside')")
        conn.commit()
        os.link(outside.with_name("outside.db-wal"), database.with_name("state.db-wal"))
        # Neither the outside rows nor a history missing its WAL: no zip.
        with self.assertRaisesRegex(ValueError, "agent/hermes-home/state.db"):
            self.export()
        conn.close()
        self.assertFalse(self.out.exists(), "no partial zip is left")
        self.assertEqual(list(self.out.parent.glob(".raw-export-*")), [], "no temporary copy is left")
        self.assertEqual(database.read_bytes(), main_file, "the tree is untouched")

    def test_a_linked_or_odd_main_database_stops_the_export(self):
        # A second name for the main file, with the writer still open: it
        # used to be left out as "not a file" while its WAL went alone.
        database = self.root / "agent/hermes-home/state.db"
        conn = sqlite3.connect(database)
        conn.execute("PRAGMA journal_mode=WAL")
        conn.execute("CREATE TABLE messages (body TEXT)")
        conn.execute("INSERT INTO messages VALUES ('committed history')")
        conn.commit()
        os.link(database, Path(self.dir.name, "backup.db"))
        try:
            with self.assertRaisesRegex(ValueError, "agent/hermes-home/state.db"):
                self.export()
        finally:
            conn.close()
        self.assertFalse(self.out.exists(), "no partial zip is left")
        for sidecar in database.parent.glob("state.db*"):
            sidecar.unlink()

        # Not a plain file at all: a link elsewhere, or a pipe.
        elsewhere = Path(self.dir.name, "elsewhere.db")
        sqlite3.connect(elsewhere).close()
        os.symlink(elsewhere, database)
        with self.assertRaisesRegex(ValueError, "agent/hermes-home/state.db"):
            self.export()
        database.unlink()
        os.mkfifo(self.root / "agent/hermes-home/memory_store.db")
        with self.assertRaisesRegex(ValueError, "agent/hermes-home/memory_store.db"):
            self.export()
        self.assertFalse(self.out.exists(), "no partial zip is left")
        self.assertEqual(list(self.out.parent.glob(".raw-export-*")), [], "no temporary copy is left")

    def test_a_site_already_in_the_tree_is_staged_from_it(self):
        write(self.root, "sites/garden/index.html", "<h1>in the tree</h1>\n")
        clone = Path(self.dir.name, "garden")
        write(clone, "index.html", "<h1>cloned</h1>\n")
        write(clone, "about.html", "<h1>about</h1>\n")
        facts = {
            "sites/garden.json": b'{"url": "https://garden.finite.site"}',
            "workspace/finitebrain/b1.json": b'{"name": "from the fact"}',
        }
        write(self.root, "workspace/finitebrain/b1.json", '{"name": "from the tree"}')
        self.export(sites={"garden": clone}, facts=facts)
        with zipfile.ZipFile(self.out) as archive:
            names = archive.namelist()
            self.assertEqual(len(names), len(set(names)), "no name twice")
            self.assertEqual(archive.read("sites/garden/index.html"), b"<h1>in the tree</h1>\n")
            self.assertEqual(archive.read("sites/garden.json"), facts["sites/garden.json"])
            self.assertEqual(archive.read("workspace/finitebrain/b1.json"), b'{"name": "from the tree"}')
        self.assertNotIn("sites/garden/about.html", names, "the tree's copy wins whole")

    def test_export_pys_own_checks_can_be_added(self):
        def secret_reason(name):
            return "may hold a secret" if name == "notes.md" else None

        summary = self.export(extra_name_check=secret_reason, extra_text_check=lambda text: b"kind" in text)
        self.assertEqual(self.names(), [".finite-stage/summary.json"])
        reasons = {line["path"]: line["why"] for line in summary["left"]}
        self.assertEqual(reasons["workspace/notes.md"], "credential-name")
        self.assertEqual(reasons["agent/hermes-home/SOUL.md"], "token-in-text")

    def test_sites_and_fact_files_come_in_the_layouts_places(self):
        site = Path(self.dir.name, "garden")
        write(site, "index.html", "<h1>garden</h1>\n")
        write(site, ".git/HEAD", "ref: refs/heads/main\n")
        write(site, ".env", "TOKEN=1\n")
        facts = {
            "agent/profile.json": b'{"name": "Old Agent", "picture_url": null}',
            "sites/garden.json": b'{"url": "https://garden.finite.site", "visibility": "public", "path": ".", "spa": false}',
        }
        summary = self.export(sites={"garden": site}, facts=facts)
        names = self.names()
        for name in ["sites/garden/index.html", "agent/profile.json", "sites/garden.json"]:
            self.assertIn(name, names)
        self.assertNotIn("sites/garden/.env", names)
        self.assertNotIn("sites/garden/.git/HEAD", names)
        whys = {line["path"]: line["why"] for line in summary["left"]}
        self.assertEqual(whys["sites/garden/.git"], "cache-dir")
        self.assertEqual(whys["sites/garden/.env"], "credential-name")
        for bad in [{"agent/hermes-home/config.yaml": b"{}"}, {"agent/profile.json": f'{{"n": "{TOKEN}"}}'.encode()}]:
            with self.assertRaises(ValueError):
                raw_export.export_raw(self.root, Path(self.dir.name, "bad.zip"), facts=bad)

    def test_the_zip_never_holds_itself_or_anything_unread(self):
        self.out = self.root / "agent/exports/raw.zip"
        os.symlink("/etc/passwd", self.root / "workspace/passwd")
        os.link(self.root / "workspace/notes.md", self.root / "workspace/notes-again.md")
        self.export()
        names = self.names()
        self.assertEqual(
            names,
            [".finite-stage/summary.json", "agent/hermes-home/SOUL.md"],
            "no link, no hard-linked file, no zip of itself",
        )

    def test_the_command_prints_one_json_line(self):
        result = raw_export.main(["--root", str(self.root), "--out", str(self.out), "--id", "agent-1"])
        self.assertEqual(result, 0)
        with zipfile.ZipFile(self.out) as archive:
            summary = json.loads(archive.read(".finite-stage/summary.json"))
        self.assertEqual(summary["id"], "agent-1")
        self.assertEqual(summary["staged"]["files"], 2)


class BuildRawPatch(unittest.TestCase):
    """export-py-build-raw.patch applies to finite-mono's export.py, and its
    `build --raw` writes the raw-folder zip (no fbrain, fsite or hermes on
    PATH: no brains, no sites, a default profile)."""

    def patched(self, scratch):
        """export.py with the patch applied, and an agent's folder for it."""
        here = Path(__file__).resolve().parent
        repo = here.parents[1]
        dashboard = "finite-agentd/integrations/hermes/finite-export/dashboard"
        target = Path(scratch, dashboard)
        target.mkdir(parents=True)
        (target / "export.py").write_bytes((repo / dashboard / "export.py").read_bytes())
        (target / "raw_export.py").write_bytes((here / "raw_export.py").read_bytes())
        applied = subprocess.run(
            ["git", "apply", str(here / "export-py-build-raw.patch")],
            cwd=scratch,
            capture_output=True,
            text=True,
        )
        self.assertEqual(applied.returncode, 0, applied.stderr)
        root = Path(scratch, "data")
        write(root, "agent/hermes-home/SOUL.md", "kind\n")
        write(root, "agent/hermes-home/.env", "OPENROUTER_API_KEY='sk-or-v1-0123456789abcdef0123'\n")
        write(root, "workspace/notes.md", "notes\n")
        env = {
            "PATH": "/usr/bin:/bin",
            "FINITE_HOME": str(root / "agent"),
            "HERMES_HOME": str(root / "agent/hermes-home"),
            "FINITECHAT_WORKSPACE": str(root / "workspace"),
            "FINITE_AGENT_NAME": "Old Agent",
        }

        def run(*options):
            return subprocess.run(
                [sys.executable, "-I", str(target / "export.py"), "build", "--raw", "--no-report", *options],
                env=env,
                capture_output=True,
                text=True,
                timeout=300,
            )

        return root, run

    def test_raw_refuses_the_options_it_does_not_honor(self):
        # These choose what goes in for `build`; the raw zip takes the
        # folder by its own rules, so it must not quietly ignore them.
        with tempfile.TemporaryDirectory(prefix="raw-export-patch-") as scratch:
            root, run = self.patched(scratch)
            for options in [["--no-workspace"], ["--everything"], ["--max-file-mb", "5"]]:
                refused = run(*options)
                self.assertEqual(refused.returncode, 2, options)
                self.assertIn(options[0], refused.stderr)
            self.assertFalse((root / "agent/exports").exists(), "no zip was made")

    def test_the_patch_applies_and_builds(self):
        with tempfile.TemporaryDirectory(prefix="raw-export-patch-") as scratch:
            root, run = self.patched(scratch)
            built = run()
            self.assertEqual(built.returncode, 0, built.stdout + built.stderr)
            result = json.loads(built.stdout)
            self.assertTrue(result["zip"].startswith(str(root / "agent/exports")))
            with zipfile.ZipFile(result["zip"]) as archive:
                names = sorted(archive.namelist())
                profile = json.loads(archive.read("agent/profile.json"))
            self.assertEqual(
                names,
                [
                    ".finite-stage/summary.json",
                    "agent/hermes-home/SOUL.md",
                    "agent/profile.json",
                    "workspace/notes.md",
                ],
            )
            self.assertEqual(profile, {"name": "Old Agent", "picture_url": None})


if __name__ == "__main__":
    unittest.main()

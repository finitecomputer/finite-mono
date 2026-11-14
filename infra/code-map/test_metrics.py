"""Exercise committed snapshots and history semantics against a synthetic repo."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("metrics.py")


class SnapshotTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.git("init", "-q")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        (self.repo / "lib.rs").write_text("pub fn answer() -> i32 { 42 }\n")
        (self.repo / "notes.md").write_text("Not source code\n")
        self.git("add", ".")
        self.commit("initial", "2026-08-01T12:00:00+00:00")
        self.initial = self.git("rev-parse", "HEAD").strip()

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.repo), *args], text=True)

    def commit(self, message, date):
        subprocess.run(
            ["git", "-C", str(self.repo), "commit", "-qm", message],
            env={**os.environ, "GIT_AUTHOR_DATE": date, "GIT_COMMITTER_DATE": date},
            check=True,
        )

    def metrics(self, ref="HEAD"):
        output = self.root / "output"
        subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                str(self.repo),
                "--ref",
                ref,
                "--output",
                str(output),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
        )
        return (
            json.loads((output / "summary.json").read_text()),
            json.loads((output / "finite-mono.cc.json").read_text()),
        )

    def attributes(self, data, path):
        return data["lenses"]["metrics"]["attributes"][
            hashlib.sha256(path.encode()).hexdigest()[:16]
        ]

    def test_analyzes_commit_not_worktree_and_excludes_docs(self):
        (self.repo / "lib.rs").write_text("invalid uncommitted source\n" * 100)
        (self.repo / "untracked.rs").write_text("fn secret_draft() {}\n")
        summary, data = self.metrics()
        self.assertEqual([f["path"] for f in summary["files"]], ["lib.rs"])
        self.assertEqual(self.attributes(data, "lib.rs")["rloc"], 1)
        self.assertEqual(self.attributes(data, "lib.rs")["touches_30d"], 0)

    def test_history_and_test_classification(self):
        (self.repo / "lib.rs").write_text(
            "pub fn answer() -> i32 { if true { 43 } else { 42 } }\n"
        )
        (self.repo / "tests").mkdir()
        (self.repo / "tests/check.rs").write_text(
            "#[test]\nfn check() { assert!(true); }\n"
        )
        self.git("add", ".")
        self.commit("change", "2026-09-09T12:00:00+00:00")
        summary, data = self.metrics()
        self.assertEqual(summary["history_since"], "2026-08-10")
        attrs = self.attributes(data, "lib.rs")
        self.assertEqual(
            (attrs["touches_30d"], attrs["added_30d"], attrs["deleted_30d"]), (1, 1, 1)
        )
        test = self.attributes(data, "tests/check.rs")
        self.assertEqual(test["main_file_loc"], 0)
        self.assertEqual(test["dedicated_test_loc"], test["rloc"])
        self.assertEqual(
            sum(a["rloc"] for a in data["lenses"]["metrics"]["attributes"].values()),
            sum(c["code"] for c in summary["components"].values()),
        )

    def test_explicit_old_ref_is_stable(self):
        (self.repo / "new.rs").write_text("fn added() {}\n")
        self.git("add", ".")
        self.commit("new file", "2026-09-09T12:00:00+00:00")
        summary, _ = self.metrics(self.initial)
        self.assertEqual(summary["commit"], self.initial)
        self.assertEqual(summary["history_since"], "2026-07-02")
        self.assertEqual([f["path"] for f in summary["files"]], ["lib.rs"])


if __name__ == "__main__":
    unittest.main()

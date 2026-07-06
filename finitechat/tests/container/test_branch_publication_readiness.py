"""Unit checks for branch publication readiness reporting."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPO_ROOT / "scripts" / "hermes-branch-publication-readiness.py"

spec = importlib.util.spec_from_file_location("hermes_branch_publication_readiness", SCRIPT_PATH)
assert spec is not None
readiness = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(readiness)


class BranchPublicationReadinessTest(unittest.TestCase):
    def test_classifies_source_changes_as_candidates(self) -> None:
        status, report = readiness.build_report(
            branch="codex/hermes-sidecar-hardening",
            status_lines=[
                " M integrations/hermes/README.md",
                "?? scripts/hermes-hardening-audit.py",
                "?? .env.example",
            ],
            include_ignored=False,
            commit_message="Test commit",
        )

        self.assertEqual(status, 0)
        self.assertEqual(report["status"], "ready")
        self.assertIn("integrations/hermes/README.md", report["candidate_paths"])
        self.assertIn("scripts/hermes-hardening-audit.py", report["candidate_paths"])
        self.assertIn(".env.example", report["candidate_paths"])
        self.assertEqual(report["blocked_paths"], [])
        self.assertIn("git push -u origin", report["suggested_commands"]["push"])

    def test_blocks_env_and_generated_paths(self) -> None:
        status, report = readiness.build_report(
            branch="codex/hermes-sidecar-hardening",
            status_lines=[
                "?? .env",
                "?? target/hermes-hardening-audit.json",
                "?? scripts/__pycache__/x.pyc",
                " M integrations/hermes/README.md",
            ],
            include_ignored=False,
            commit_message="Test commit",
        )

        self.assertEqual(status, 2)
        self.assertEqual(report["status"], "blocked")
        blocked = [item["path"] for item in report["blocked_paths"]]
        self.assertIn(".env", blocked)
        self.assertIn("target/hermes-hardening-audit.json", blocked)
        self.assertIn("scripts/__pycache__/x.pyc", blocked)
        self.assertIn("blocked generated or sensitive paths", report["errors"][0])

    def test_clean_worktree_is_not_blocked(self) -> None:
        status, report = readiness.build_report(
            branch="codex/hermes-sidecar-hardening",
            status_lines=[],
            include_ignored=False,
            commit_message="Test commit",
        )

        self.assertEqual(status, 0)
        self.assertEqual(report["status"], "clean")
        self.assertEqual(report["candidate_paths"], [])
        self.assertEqual(report["blocked_paths"], [])
        self.assertEqual(report["errors"], [])
        self.assertIn("no local source changes", report["notes"][0])
        self.assertIsNone(report["suggested_commands"]["stage"])
        self.assertIsNone(report["suggested_commands"]["commit"])
        self.assertIn("git push -u origin", report["suggested_commands"]["push"])

    def test_ignored_paths_are_counted_but_not_blocking(self) -> None:
        status, report = readiness.build_report(
            branch="codex/hermes-sidecar-hardening",
            status_lines=[
                " M integrations/hermes/README.md",
                "!! target/debug/build.log",
            ],
            include_ignored=True,
            commit_message="Test commit",
        )

        self.assertEqual(status, 0)
        self.assertEqual(report["ignored_count"], 1)
        self.assertEqual(report["ignored_sample"], ["target/debug/build.log"])


if __name__ == "__main__":
    unittest.main()

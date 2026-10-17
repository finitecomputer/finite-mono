#!/usr/bin/env python3
"""Isolated Runner release gate in deploy-lat{3,4,5}-closure-cache.

Each case runs the real deploy script in --validate-only mode (it exits before
any SSH) inside a throwaway clone of a local bare origin, so the gate's git
reads are exercised against real refs.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
BRANCH = "release/runner-hosts-test"


def git(cwd: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args],
        cwd=cwd,
        env=git_env(),
        text=True,
        capture_output=True,
        check=True,
    ).stdout.strip()


def git_env() -> dict[str, str]:
    return {
        **os.environ,
        "GIT_CONFIG_GLOBAL": os.devnull,
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_AUTHOR_NAME": "test",
        "GIT_AUTHOR_EMAIL": "test@example.invalid",
        "GIT_COMMITTER_NAME": "test",
        "GIT_COMMITTER_EMAIL": "test@example.invalid",
    }


class IsolatedReleaseGate:
    host = 0
    schema = ""
    install_paths = False

    def setUp(self) -> None:
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.temp = Path(temp.name)
        self.origin = self.temp / "origin.git"
        seed = self.temp / "seed"
        git(self.temp, "init", "-q", "--bare", "-b", "main", str(self.origin))
        git(self.temp, "init", "-q", "-b", "main", str(seed))
        git(seed, "commit", "-q", "--allow-empty", "-m", "main")
        self.main_rev = git(seed, "rev-parse", "HEAD")
        git(seed, "checkout", "-q", "-b", BRANCH)
        git(seed, "commit", "-q", "--allow-empty", "-m", "release head B")
        self.release_rev = git(seed, "rev-parse", "HEAD")
        git(seed, "remote", "add", "origin", str(self.origin))
        git(seed, "push", "-q", "origin", "main", BRANCH)
        self.seed = seed
        self.work = self.temp / "work"
        git(self.temp, "clone", "-q", str(self.origin), str(self.work))

    def artifact(self, rev: str) -> Path:
        artifact = self.temp / f"artifact-{rev[:12]}"
        (artifact / "nix-cache").mkdir(parents=True)
        (artifact / "nix-cache" / "nix-cache-info").write_text(
            "StoreDir: /nix/store\n", encoding="utf-8"
        )
        manifest = {
            "schema": self.schema,
            "host": f"finite-lat-{self.host}",
            "repository": "finitecomputer/finite-mono",
            "rev": rev,
            "system": "/nix/store/"
            + "b" * 32
            + f"-nixos-system-finite-lat-{self.host}-26.05.test",
            "cache": "nix-cache",
        }
        if self.install_paths:
            manifest["disko"] = "/nix/store/" + "c" * 32 + "-disko"
            manifest["kexec"] = "/nix/store/" + "d" * 32 + "-kexec-tarball"
        (artifact / "manifest.json").write_text(
            json.dumps(manifest) + "\n", encoding="utf-8"
        )
        return artifact

    def validate(self, rev: str, branch: str | None) -> subprocess.CompletedProcess[str]:
        env = git_env()
        env.pop("FINITE_ISOLATED_RELEASE_BRANCH", None)
        if branch is not None:
            env["FINITE_ISOLATED_RELEASE_BRANCH"] = branch
        return subprocess.run(
            [
                str(ROOT / f"scripts/deploy-lat{self.host}-closure-cache"),
                "--validate-only",
                str(self.artifact(rev)),
            ],
            cwd=self.work,
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )

    def advance_origin_release(self) -> str:
        git(self.seed, "commit", "-q", "--allow-empty", "-m", "release head C")
        git(self.seed, "push", "-q", "origin", BRANCH)
        return git(self.seed, "rev-parse", "HEAD")

    def test_exact_release_head_is_accepted(self) -> None:
        result = self.validate(self.release_rev, BRANCH)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"isolated-release={BRANCH}", result.stdout)
        self.assertIn(f"rev={self.release_rev}", result.stdout)

    def test_stale_tracking_ref_does_not_pass_as_the_release_head(self) -> None:
        # The clone fetches only main, so its release tracking ref stays at B
        # while origin moves to C.
        git(self.work, "config", "remote.origin.fetch", "+refs/heads/main:refs/remotes/origin/main")
        new_head = self.advance_origin_release()
        result = self.validate(self.release_rev, BRANCH)
        self.assertEqual(result.returncode, 65, result.stdout + result.stderr)
        self.assertIn(f"is not the head of origin/{BRANCH} ({new_head})", result.stderr)
        accepted = self.validate(new_head, BRANCH)
        self.assertEqual(accepted.returncode, 0, accepted.stderr)

    def test_release_branch_deleted_on_origin_is_refused(self) -> None:
        git(self.work, "config", "remote.origin.fetch", "+refs/heads/main:refs/remotes/origin/main")
        git(self.seed, "push", "-q", "origin", "--delete", BRANCH)
        result = self.validate(self.release_rev, BRANCH)
        self.assertEqual(result.returncode, 65, result.stdout + result.stderr)
        self.assertIn(f"cannot fetch origin {BRANCH}", result.stderr)

    def test_older_release_commit_is_refused(self) -> None:
        new_head = self.advance_origin_release()
        result = self.validate(self.release_rev, BRANCH)
        self.assertEqual(result.returncode, 65)
        self.assertIn(f"({new_head})", result.stderr)

    def test_only_release_runner_hosts_branches_are_accepted(self) -> None:
        result = self.validate(self.release_rev, "fix/runner-hosts-test")
        self.assertEqual(result.returncode, 64)
        self.assertIn("must name release/runner-hosts-*", result.stderr)

    def test_without_opt_in_only_main_revisions_pass(self) -> None:
        refused = self.validate(self.release_rev, None)
        self.assertEqual(refused.returncode, 65)
        self.assertIn("is not on origin/main", refused.stderr)
        accepted = self.validate(self.main_rev, None)
        self.assertEqual(accepted.returncode, 0, accepted.stderr)
        self.assertNotIn("isolated-release=", accepted.stdout)


class Lat3IsolatedReleaseGateTests(IsolatedReleaseGate, unittest.TestCase):
    host = 3
    schema = "finite.lat3.nixos-closure.v1"


class Lat4IsolatedReleaseGateTests(IsolatedReleaseGate, unittest.TestCase):
    host = 4
    schema = "finite.lat4.nixos-closure.v2"
    install_paths = True


class Lat5IsolatedReleaseGateTests(IsolatedReleaseGate, unittest.TestCase):
    host = 5
    schema = "finite.lat5.nixos-closure.v2"
    install_paths = True


if __name__ == "__main__":
    unittest.main()

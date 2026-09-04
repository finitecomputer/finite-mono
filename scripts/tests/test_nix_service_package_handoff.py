from __future__ import annotations

import json
import os
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HANDOFF = ROOT / "scripts" / "ci" / "nix-service-package-handoff"
REVISION = "a" * 40
DEVFINITY = "/nix/store/" + "b" * 32 + "-devfinity"
DEPENDENCY = "/nix/store/" + "c" * 32 + "-dependency"


class NixServicePackageHandoffTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.bin_dir = self.root / "bin"
        self.bin_dir.mkdir()
        self.restore_marker = self.root / "restored"
        self.command_log = self.root / "commands.log"
        self.write_executable(
            "git",
            """
            #!/usr/bin/env bash
            set -euo pipefail
            if [[ "$1" == "rev-parse" && "$2" == "HEAD" ]]; then
              printf '%s\n' "$FAKE_REVISION"
              exit 0
            fi
            exit 2
            """,
        )
        self.write_executable(
            "nix",
            """
            #!/usr/bin/env bash
            set -euo pipefail
            printf '<%s>' "$@" >> "$FAKE_COMMAND_LOG"
            printf '\n' >> "$FAKE_COMMAND_LOG"

            if [[ "$1" == "path-info" && "${2:-}" == "--recursive" ]]; then
              printf '%s\n%s\n' "$FAKE_DEPENDENCY_PATH" "$FAKE_DEVFINITY_PATH"
              exit 0
            fi
            if [[ "$1" == "path-info" ]]; then
              exit 0
            fi
            if [[ "$1" == "eval" ]]; then
              printf '%s' "$FAKE_EVALUATED_PATH"
              exit 0
            fi
            if [[ "$1" == "copy" ]]; then
              for ((index = 1; index <= $#; index++)); do
                argument="${!index}"
                if [[ "$argument" == "--to" ]]; then
                  next=$((index + 1))
                  destination="${!next}"
                  destination="${destination#file://}"
                  mkdir -p "$destination"
                  printf 'StoreDir: /nix/store\n' > "$destination/nix-cache-info"
                  exit 0
                fi
                if [[ "$argument" == "--from" ]]; then
                  : > "$FAKE_RESTORE_MARKER"
                  exit 0
                fi
              done
            fi
            exit 2
            """,
        )
        self.environment = os.environ.copy()
        self.environment.update(
            {
                "FAKE_COMMAND_LOG": str(self.command_log),
                "FAKE_DEPENDENCY_PATH": DEPENDENCY,
                "FAKE_DEVFINITY_PATH": DEVFINITY,
                "FAKE_EVALUATED_PATH": DEVFINITY,
                "FAKE_RESTORE_MARKER": str(self.restore_marker),
                "FAKE_REVISION": REVISION,
                "GITHUB_REPOSITORY": "finitecomputer/finite-mono",
                "GITHUB_RUN_ATTEMPT": "2",
                "GITHUB_RUN_ID": "run-123",
                "PATH": f"{self.bin_dir}:{os.environ['PATH']}",
            }
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def write_executable(self, name: str, body: str) -> None:
        path = self.bin_dir / name
        path.write_text(textwrap.dedent(body).lstrip(), encoding="utf-8")
        path.chmod(path.stat().st_mode | stat.S_IXUSR)

    def run_handoff(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(HANDOFF), *arguments],
            cwd=self.root,
            env=self.environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

    def pack(self) -> Path:
        package_outputs = self.root / "package-outputs"
        package_outputs.write_text(
            f"/nix/store/{'d' * 32}-other\n{DEVFINITY}\n", encoding="utf-8"
        )
        artifact = self.root / "artifact"
        completed = self.run_handoff(
            "pack", REVISION, str(package_outputs), str(artifact)
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        return artifact

    def test_pack_creates_a_revision_bound_file_cache(self) -> None:
        artifact = self.pack()

        manifest = json.loads((artifact / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["schema"], "finite.ci.devfinity-nix-handoff.v1")
        self.assertEqual(manifest["repository"], "finitecomputer/finite-mono")
        self.assertEqual(manifest["rev"], REVISION)
        self.assertEqual(manifest["devfinity"], DEVFINITY)
        self.assertEqual(manifest["closure_path_count"], 2)
        self.assertEqual(manifest["github_run_id"], "run-123")
        self.assertEqual(manifest["github_run_attempt"], "2")
        self.assertTrue((artifact / "nix-cache" / "nix-cache-info").is_file())
        self.assertEqual(
            (artifact / "store-paths.txt").read_text(encoding="utf-8"),
            f"{DEVFINITY}\n{DEPENDENCY}\n",
        )

    def test_restore_imports_the_exact_checkout_output(self) -> None:
        artifact = self.pack()

        completed = self.run_handoff("restore", REVISION, str(artifact))

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertTrue(self.restore_marker.is_file())
        commands = self.command_log.read_text(encoding="utf-8")
        self.assertIn(
            "<eval><--raw><.#packages.x86_64-linux.devfinity.outPath>", commands
        )
        self.assertIn("<copy><--no-check-sigs>", commands)
        self.assertIn(
            f"<--from><file://{(artifact / 'nix-cache').resolve()}>", commands
        )

    def test_restore_rejects_a_manifest_from_another_revision(self) -> None:
        artifact = self.pack()
        manifest_path = artifact / "manifest.json"
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        manifest["rev"] = "e" * 40
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

        completed = self.run_handoff("restore", REVISION, str(artifact))

        self.assertEqual(completed.returncode, 65)
        self.assertIn("manifest rev", completed.stderr)
        self.assertFalse(self.restore_marker.exists())

    def test_restore_rejects_a_different_evaluated_output(self) -> None:
        artifact = self.pack()
        self.environment["FAKE_EVALUATED_PATH"] = (
            "/nix/store/" + "f" * 32 + "-devfinity"
        )

        completed = self.run_handoff("restore", REVISION, str(artifact))

        self.assertEqual(completed.returncode, 65)
        self.assertIn("checkout evaluates devfinity", completed.stderr)
        self.assertFalse(self.restore_marker.exists())

    def test_pack_rejects_an_ambiguous_output_list(self) -> None:
        package_outputs = self.root / "package-outputs"
        package_outputs.write_text(f"{DEVFINITY}\n{DEVFINITY}\n", encoding="utf-8")

        completed = self.run_handoff(
            "pack", REVISION, str(package_outputs), str(self.root / "artifact")
        )

        self.assertEqual(completed.returncode, 65)
        self.assertIn("exactly one devfinity output", completed.stderr)

    def test_pack_rejects_a_file_as_the_artifact_destination(self) -> None:
        package_outputs = self.root / "package-outputs"
        package_outputs.write_text(f"{DEVFINITY}\n", encoding="utf-8")
        artifact = self.root / "artifact"
        artifact.write_text("not a directory\n", encoding="utf-8")

        completed = self.run_handoff(
            "pack", REVISION, str(package_outputs), str(artifact)
        )

        self.assertEqual(completed.returncode, 65)
        self.assertIn("is not a directory", completed.stderr)


if __name__ == "__main__":
    unittest.main()

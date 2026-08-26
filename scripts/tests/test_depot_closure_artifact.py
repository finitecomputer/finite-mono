from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
HELPER = ROOT / "scripts" / "fetch_depot_nixos_closure.py"
SPEC = importlib.util.spec_from_file_location("fetch_depot_nixos_closure", HELPER)
assert SPEC is not None and SPEC.loader is not None
fetcher = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fetcher)


class DepotClosureArtifactTests(unittest.TestCase):
    def test_helper_targets_only_github_and_native_depot(self) -> None:
        self.assertEqual(fetcher.DEPOT_REPOSITORY, "finitecomputer/finite-mono")
        self.assertEqual(
            fetcher.GITHUB_CLONE_URL,
            "https://github.com/finitecomputer/finite-mono.git",
        )
        source = HELPER.read_text(encoding="utf-8")
        self.assertNotIn("gh workflow", source)
        self.assertNotIn("gh run", source)

    def test_dispatch_selects_main_and_the_exact_revision(self) -> None:
        revision = "a" * 40
        with mock.patch.object(
            fetcher, "command_json", return_value={"run_id": "run-123"}
        ) as command:
            run_id = fetcher.dispatch("lat1-nixos-closure.yml", revision)

        self.assertEqual(run_id, "run-123")
        self.assertEqual(
            command.call_args.args[0],
            [
                "depot",
                "ci",
                "dispatch",
                "--org",
                "scthc5h66g",
                "--repo",
                "finitecomputer/finite-mono",
                "--workflow",
                "lat1-nixos-closure.yml",
                "--ref",
                "main",
                "--input",
                f"rev={revision}",
                "--output",
                "json",
            ],
        )

    def test_artifact_selection_requires_exact_run_workflow_and_name(self) -> None:
        document = {
            "artifacts": [
                {
                    "artifact_id": "wrong-run",
                    "run_id": "other",
                    "workflow_path": "lat1-nixos-closure.yml",
                    "name": "lat1-nixos-closure-" + "a" * 40,
                },
                {
                    "artifact_id": "artifact-123",
                    "run_id": "run-123",
                    "workflow_path": "lat1-nixos-closure.yml",
                    "name": "lat1-nixos-closure-" + "a" * 40,
                },
            ]
        }

        self.assertEqual(
            fetcher.select_artifact(
                document,
                "run-123",
                "lat1-nixos-closure.yml",
                "lat1-nixos-closure-" + "a" * 40,
            ),
            "artifact-123",
        )

    def test_artifact_selection_rejects_duplicates(self) -> None:
        item = {
            "artifact_id": "artifact-123",
            "run_id": "run-123",
            "workflow_path": "lat1-nixos-closure.yml",
            "name": "lat1-nixos-closure-" + "a" * 40,
        }
        with self.assertRaises(fetcher.ClosureFetchError):
            fetcher.select_artifact(
                {"artifacts": [item, dict(item, artifact_id="artifact-456")]},
                "run-123",
                "lat1-nixos-closure.yml",
                item["name"],
            )

    def test_extract_rejects_parent_traversal(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "artifact.zip"
            destination = root / "output"
            destination.mkdir()
            with zipfile.ZipFile(archive, "w") as bundle:
                bundle.writestr("../escape", "bad")

            with self.assertRaises(fetcher.ClosureFetchError):
                fetcher.safe_extract(archive, destination)

            self.assertFalse((root / "escape").exists())

    def test_manifest_requires_github_repository_and_complete_cache(self) -> None:
        revision = "a" * 40
        with tempfile.TemporaryDirectory() as temporary:
            artifact = Path(temporary)
            (artifact / "nix-cache").mkdir()
            (artifact / "nix-cache" / "nix-cache-info").write_text(
                "StoreDir: /nix/store\n", encoding="utf-8"
            )
            manifest = {
                "schema": "finite.lat1.nixos-closure.v1",
                "repository": "finitecomputer/finite-mono",
                "rev": revision,
                "cache": "nix-cache",
            }
            (artifact / "manifest.json").write_text(
                json.dumps(manifest), encoding="utf-8"
            )

            fetcher.validate_manifest(
                artifact,
                revision,
                "finite.lat1.nixos-closure.v1",
                "lat1-nixos-closure-" + revision,
            )

            manifest["repository"] = "other/repository"
            (artifact / "manifest.json").write_text(
                json.dumps(manifest), encoding="utf-8"
            )
            with self.assertRaises(fetcher.ClosureFetchError):
                fetcher.validate_manifest(
                    artifact,
                    revision,
                    "finite.lat1.nixos-closure.v1",
                    "lat1-nixos-closure-" + revision,
                )


if __name__ == "__main__":
    unittest.main()

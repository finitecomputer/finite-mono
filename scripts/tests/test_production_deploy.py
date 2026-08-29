from __future__ import annotations

import json
import tempfile
import unittest
from contextlib import redirect_stderr
from io import StringIO
from pathlib import Path
from unittest.mock import patch

from scripts import production_deploy


ROOT = Path(__file__).resolve().parents[2]

VALID_MANIFEST = {
    "environment": "production",
    "scope": "lat1-nixos",
    "classification": "ordinary",
    "risky_path_policy": "lat1-v1",
    "mutation_enabled": False,
    "rollback_policy": "previous-lat1-closure",
    "closure_transport": "cachix",
    "cachix_cache_name": "finite",
    "cachix_substituter": "https://finite.cachix.org",
    "cachix_trusted_public_key": "finite.cachix.org-1:Sg/y/5ax+IxMrPXS4moFro6YFdqa+a2gzDYAesRcVsk=",
    "legacy_file_cache_fallback": "explicit-only",
    "required_gates": ["ci-gate"],
}


class ProductionDeployTests(unittest.TestCase):
    def test_valid_manifest_accepts_minimal_production_shape(self) -> None:
        manifest = production_deploy.validate_manifest(dict(VALID_MANIFEST))
        self.assertEqual(manifest["environment"], "production")
        self.assertEqual(manifest["scope"], "lat1-nixos")
        self.assertEqual(manifest["closure_transport"], "cachix")
        self.assertEqual(manifest["cachix_cache_name"], "finite")
        self.assertFalse(manifest["mutation_enabled"])

    def test_manifest_rejects_extra_keys(self) -> None:
        manifest = dict(VALID_MANIFEST)
        manifest["dashboard_digest"] = "sha256:" + "a" * 64
        with self.assertRaisesRegex(production_deploy.DeployConfigError, "unknown"):
            production_deploy.validate_manifest(manifest)

    def test_manifest_rejects_runtime_scope(self) -> None:
        manifest = dict(VALID_MANIFEST)
        manifest["scope"] = "runtime-rollout"
        with self.assertRaisesRegex(production_deploy.DeployConfigError, "lat1-nixos"):
            production_deploy.validate_manifest(manifest)

    def test_risky_path_classifier_flags_known_persistence_paths(self) -> None:
        risky = production_deploy.classify_paths(
            [
                "docs/README.md",
                "finitecomputer-v2/crates/finite-saas-core/migrations/0020_new.sql",
                "finite-brain/crates/finite-brain-store/src/schema.rs",
                "infra/nixos/modules/backups.nix",
            ]
        )
        self.assertEqual(
            [(entry["path"], entry["reason"]) for entry in risky],
            [
                (
                    "finitecomputer-v2/crates/finite-saas-core/migrations/0020_new.sql",
                    "core-postgres-migration",
                ),
                (
                    "finite-brain/crates/finite-brain-store/src/schema.rs",
                    "brain-sqlite-schema",
                ),
                ("infra/nixos/modules/backups.nix", "lat1-recovery-module"),
            ],
        )

    def test_ordinary_classification_refuses_risky_paths(self) -> None:
        risky = production_deploy.classify_paths(
            ["finite-sites/crates/finitesites-store/src/lib.rs"]
        )
        with self.assertRaisesRegex(production_deploy.DeployConfigError, "ordinary"):
            production_deploy.validate_classification(dict(VALID_MANIFEST), risky)

    def test_schema_change_classification_accepts_risky_paths(self) -> None:
        manifest = dict(VALID_MANIFEST)
        manifest["classification"] = "schema-change"
        risky = production_deploy.classify_paths(
            ["finitechat/crates/finitechat-server/src/store/schema.rs"]
        )
        production_deploy.validate_classification(manifest, risky)

    def test_ci_source_uses_source_sha_for_non_production_merge(self) -> None:
        source = "a" * 40
        with (
            patch.object(production_deploy, "resolve_rev", side_effect=lambda rev: rev),
            patch.object(production_deploy, "commit_parents", return_value=["b" * 40]),
        ):
            resolved = production_deploy.resolve_ci_source(source, "c" * 40)
        self.assertEqual(resolved["source_sha"], source)
        self.assertEqual(resolved["ci_source_sha"], source)
        self.assertEqual(resolved["ci_source_reason"], "source-sha")

    def test_ci_source_uses_second_parent_for_production_merge(self) -> None:
        source = "a" * 40
        production_base = "b" * 40
        promoted_source = "c" * 40
        tree_by_rev = {source: "tree", promoted_source: "tree"}
        with (
            patch.object(production_deploy, "resolve_rev", side_effect=lambda rev: rev),
            patch.object(
                production_deploy,
                "commit_parents",
                return_value=[production_base, promoted_source],
            ),
            patch.object(
                production_deploy,
                "tree_sha",
                side_effect=lambda rev: tree_by_rev[rev],
            ),
        ):
            resolved = production_deploy.resolve_ci_source(source, production_base)
        self.assertEqual(resolved["source_sha"], source)
        self.assertEqual(resolved["ci_source_sha"], promoted_source)
        self.assertEqual(resolved["ci_source_reason"], "production-merge-second-parent")

    def test_ci_source_rejects_merge_commit_with_unmatched_tree(self) -> None:
        source = "a" * 40
        production_base = "b" * 40
        promoted_source = "c" * 40
        tree_by_rev = {source: "merge-tree", promoted_source: "promoted-tree"}
        with (
            patch.object(production_deploy, "resolve_rev", side_effect=lambda rev: rev),
            patch.object(
                production_deploy,
                "commit_parents",
                return_value=[production_base, promoted_source],
            ),
            patch.object(
                production_deploy,
                "tree_sha",
                side_effect=lambda rev: tree_by_rev[rev],
            ),
        ):
            with self.assertRaisesRegex(production_deploy.DeployConfigError, "tree"):
                production_deploy.resolve_ci_source(source, production_base)

    def test_record_is_minimal_and_generated_from_plan(self) -> None:
        plan = {
            "schema": production_deploy.PLAN_SCHEMA,
            "environment": "production",
            "scope": "lat1-nixos",
            "source_sha": "a" * 40,
            "production_branch": "production",
            "manifest_sha256": "b" * 64,
            "classification": "ordinary",
            "mutation_enabled": False,
            "closure_transport": "cachix",
            "cachix_cache_name": "finite",
            "cachix_substituter": "https://finite.cachix.org",
        }
        record = production_deploy.build_record(
            plan,
            outcome="dry_run_blocked_before_mutation",
            mutation_boundary_crossed=False,
            system_path=None,
            closure_transport="cachix",
            cachix_cache_name="finite",
            closure_store_path_count=1234,
            closure_size_bytes=5678,
            closure_size_report="closure-size.txt",
            override_reason=None,
        )
        self.assertEqual(record["schema"], production_deploy.RECORD_SCHEMA)
        self.assertEqual(record["source_sha"], "a" * 40)
        self.assertFalse(record["mutation_boundary_crossed"])
        self.assertIsNone(record["system_path"])
        self.assertEqual(record["closure_transport"], "cachix")
        self.assertEqual(record["cachix_cache_name"], "finite")
        self.assertEqual(record["closure_store_path_count"], 1234)
        self.assertEqual(record["closure_size_bytes"], 5678)
        self.assertEqual(record["closure_size_report"], "closure-size.txt")
        self.assertEqual(record["outcome"], "dry_run_blocked_before_mutation")

    def test_record_command_rejects_non_plan_input(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            input_path = root / "not-plan.json"
            output_path = root / "record.json"
            input_path.write_text(
                json.dumps({"schema": "other"}) + "\n", encoding="utf-8"
            )
            with redirect_stderr(StringIO()):
                exit_code = production_deploy.main(
                    [
                        "record",
                        "--plan",
                        str(input_path),
                        "--output",
                        str(output_path),
                        "--outcome",
                        "dry_run_blocked_before_mutation",
                    ]
                )
            self.assertEqual(exit_code, 2)
            self.assertFalse(output_path.exists())

    def test_workflows_keep_mvp_deploy_conductor_shape(self) -> None:
        deploy = (ROOT / ".github/workflows/production-deploy.yml").read_text(
            encoding="utf-8"
        )
        plan = (ROOT / ".github/workflows/production-deploy-plan.yml").read_text(
            encoding="utf-8"
        )
        lat1_closure = (ROOT / ".github/workflows/lat1-nixos-closure.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("environment: production", deploy)
        self.assertIn("if: needs.prepare.outputs.mutation_enabled == 'true'", deploy)
        self.assertIn('ci_push_before=""', deploy)
        self.assertIn('git merge-base --is-ancestor "$source_sha" origin/main', deploy)
        self.assertIn('--push-before "$ci_push_before"', deploy)
        self.assertIn("CACHIX_CACHE_NAME: finite", deploy)
        self.assertGreaterEqual(deploy.count("Configure Cachix read-only cache"), 2)
        self.assertGreaterEqual(deploy.count("skipPush: true"), 2)
        self.assertIn("Publish lat1 closure to Cachix", deploy)
        self.assertIn("scripts/publish-lat1-nixos-cachix-closure", deploy)
        self.assertLess(
            deploy.index("Publish lat1 closure to Cachix"),
            deploy.index("Upload lat1 closure artifact"),
        )
        self.assertIn("deployment-transport.json", deploy)
        self.assertIn("Prove lat1 closure substitutes before mutation", deploy)
        self.assertIn("FINITE_LAT1_DEPLOY_MODE=realise-only", deploy)
        self.assertIn("--closure-store-path-count", deploy)
        self.assertIn("--closure-size-bytes", deploy)
        self.assertIn("Configure Cachix read-only cache", plan)
        self.assertIn("skipPush: true", plan)
        self.assertIn("Configure Cachix read-only cache", lat1_closure)
        self.assertIn("skipPush: true", lat1_closure)
        self.assertIn("Publish lat1 closure to Cachix", lat1_closure)
        self.assertIn("scripts/publish-lat1-nixos-cachix-closure", lat1_closure)
        self.assertIn("if: ${{ ! inputs.include_file_cache }}", lat1_closure)
        self.assertIn("FINITE_LAT1_BUILD_FILE_CACHE", lat1_closure)
        self.assertIn("Stage status collector on lat1", deploy)
        self.assertIn("finite-status-before.json", deploy)
        self.assertIn('"nodes"]["nixpkgs"]["locked"]["rev"]', deploy)
        self.assertIn("github:NixOS/nixpkgs/$REMOTE_NIXPKGS_REV#python3", deploy)
        self.assertIn("--command python3 ./finite-status --json", deploy)
        self.assertIn("python3 -m json.tool", deploy)
        self.assertNotIn("require-production-disabled", plan)


if __name__ == "__main__":
    unittest.main()

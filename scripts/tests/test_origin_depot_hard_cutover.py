from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GITHUB_WORKFLOWS = ROOT / ".github" / "workflows"
CUTOVER_WIZARD = ROOT / "scripts" / "origin-depot-hard-cutover-wizard"
HISTORICAL_WORKFLOW_DOCS = {
    "docs/ci-gate-mvp-plan.md",
    "docs/fedimint-monorepo-structure-analysis.md",
    "docs/monorepo-migration-log.md",
    "docs/origin-depot-migration-plan.md",
    "docs/research/2026-08-24-cursor-origin-ci-fit.md",
    "docs/research/2026-08-24-depot-vs-buildkite.md",
    "finitecomputer-v2/docs/carry-over-manifest.md",
    "level-up.md",
}
SOURCE_AUTHORITY_SURFACES = (
    "CONTRIBUTING.md",
    "Cargo.toml",
    "finite-brain/development.md",
    "finitechat/docs/testflight-runbook.md",
    "finitecomputer-v2/deploy/finite-computer/images/runtime.Dockerfile",
    "infra/images/deepseek-v4-vllm.Dockerfile",
    "infra/images/private-limiter.Dockerfile",
)
SECRET_POLICY_SURFACES = (
    "CONTRIBUTING.md",
    "infra/README.md",
    "infra/runbooks/README.md",
)


class OriginDepotHardCutoverTests(unittest.TestCase):
    def test_github_actions_has_no_workflows_after_hard_cutover(self) -> None:
        workflows = sorted(path.name for path in GITHUB_WORKFLOWS.glob("*.yml"))

        self.assertEqual(workflows, [])

    def test_executable_contracts_do_not_name_github_workflow_paths(self) -> None:
        legacy_fragment = ".github" + "/workflows"
        split_legacy_path = re.compile(r"""["']\.github["']\s*/\s*["']workflows["']""")
        violations: list[str] = []

        for path in ROOT.rglob("*"):
            if not path.is_file() or any(
                part in {".git", ".github", ".direnv", "target"} for part in path.parts
            ):
                continue
            if not (
                path.suffix in {".py", ".rs"}
                or path.name.endswith("Dockerfile")
                or ("scripts" in path.parts and path.suffix == "")
            ):
                continue
            try:
                text = path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue
            if path in {CUTOVER_WIZARD, Path(__file__).resolve()}:
                continue
            if legacy_fragment in text or split_legacy_path.search(text):
                violations.append(str(path.relative_to(ROOT)))

        self.assertEqual(violations, [])

    def test_obsolete_github_actions_operator_helpers_are_removed(self) -> None:
        helpers = sorted(
            path.name
            for path in (ROOT / "finitechat" / "scripts").glob("hermes-github-*")
        )

        self.assertEqual(helpers, [])

    def test_authoritative_docs_name_only_native_depot_workflows(self) -> None:
        legacy_fragment = ".github" + "/workflows"
        violations: list[str] = []

        for path in ROOT.rglob("*.md"):
            if any(part in {".git", ".direnv", "target"} for part in path.parts):
                continue
            relative = str(path.relative_to(ROOT))
            if relative in HISTORICAL_WORKFLOW_DOCS:
                continue
            if legacy_fragment in path.read_text(encoding="utf-8"):
                violations.append(relative)

        self.assertEqual(violations, [])

    def test_active_source_authority_references_do_not_point_at_github(self) -> None:
        legacy_source = "github.com" + "/finitecomputer/finite-mono"
        violations = [
            path
            for path in SOURCE_AUTHORITY_SURFACES
            if legacy_source in (ROOT / path).read_text(encoding="utf-8")
        ]

        self.assertEqual(violations, [])

    def test_production_rollout_handoff_is_depot_native(self) -> None:
        runbook = (ROOT / "infra" / "runbooks" / "deploy-core.md").read_text(
            encoding="utf-8"
        )
        wizard = CUTOVER_WIZARD.read_text(encoding="utf-8")

        self.assertNotIn("gh workflow", runbook)
        self.assertNotIn("gh run", runbook)
        self.assertIn("scripts/fetch_depot_nixos_closure.py lat1", runbook)
        self.assertGreaterEqual(runbook.count("scripts/finite-status --json"), 2)
        self.assertIn("scripts/fetch_depot_nixos_closure.py", wizard)

    def test_production_image_runbooks_use_native_depot_dispatch(self) -> None:
        runtime = (ROOT / "infra" / "runbooks" / "runtime-image.md").read_text()
        images = (ROOT / "infra" / "images" / "README.md").read_text()

        for document in (runtime, images):
            self.assertIn("depot ci dispatch", document)
            self.assertIn("--org scthc5h66g", document)
            self.assertIn("--repo finite-co/finite-mono", document)
            self.assertIn('--input rev="$REV"', document)
            self.assertIn("--input publish_production=true", document)
        self.assertIn("--workflow runtime-image.yml", runtime)
        self.assertIn("--workflow service-images.yml", images)
        self.assertIn("--workflow deepseek-v4-vllm-image.yml", images)
        self.assertNotIn("gh workflow", runtime)

    def test_new_closure_manifests_name_origin_source_authority(self) -> None:
        for name in (
            "build-lat1-nixos-closure-artifact",
            "build-lat3-nixos-closure-artifact",
        ):
            with self.subTest(script=name):
                source = (ROOT / "scripts" / name).read_text(encoding="utf-8")
                self.assertIn('"repository": "finite-co/finite-mono"', source)
                self.assertNotIn('"repository": "finitecomputer/finite-mono"', source)

    def test_deployment_record_reads_releases_from_release_repository(self) -> None:
        changelog = (ROOT / "infra" / "deployment-changelog.md").read_text(
            encoding="utf-8"
        )

        self.assertIn(
            "gh release list --repo finitecomputer/finite-releases", changelog
        )
        self.assertNotIn("gh release list --repo finitecomputer/finite-mono", changelog)

    def test_release_dispatches_select_the_finite_depot_organization(self) -> None:
        runbook = (ROOT / "infra" / "runbooks" / "release-cli.md").read_text(
            encoding="utf-8"
        )

        self.assertEqual(
            runbook.count("depot ci dispatch"),
            runbook.count("--org scthc5h66g"),
        )

    def test_secret_policy_does_not_claim_private_source_is_public(self) -> None:
        violations = [
            path
            for path in SECRET_POLICY_SURFACES
            if "repo is public" in (ROOT / path).read_text(encoding="utf-8")
        ]

        self.assertEqual(violations, [])

    def test_native_depot_ci_runs_the_hard_cutover_contract(self) -> None:
        justfile = (ROOT / "justfile").read_text(encoding="utf-8")
        workflow = (ROOT / ".depot" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("origin-depot-hard-cutover-contract:", justfile)
        self.assertIn(
            "python3 -m unittest scripts.tests.test_origin_depot_hard_cutover",
            justfile,
        )
        self.assertIn(
            "nix develop --command just origin-depot-hard-cutover-contract",
            workflow,
        )

    def test_native_depot_workflows_do_not_describe_github_as_ci_authority(
        self,
    ) -> None:
        legacy_fragment = ".github" + "/workflows"
        violations: list[str] = []

        for path in (ROOT / ".depot" / "workflows").glob("*.yml"):
            workflow = path.read_text(encoding="utf-8")
            if legacy_fragment in workflow or "Actions secret" in workflow:
                violations.append(path.name)

        self.assertEqual(violations, [])

    def test_cutover_wizard_opens_publish_gates_only_after_acceptance(self) -> None:
        wizard = CUTOVER_WIZARD.read_text(encoding="utf-8")

        self.assertIn("TOTAL_STAGES=11", wizard)
        for secret in (
            "CACHIX_AUTH_TOKEN",
            "OPENROUTER_API_KEY",
            "FINITE_PRIVATE_SMOKE_API_KEY",
            "PHALA_CLOUD_API_KEY",
            "FINITE_RELEASES_GITHUB_TOKEN",
            "FINITE_GHCR_USERNAME",
            "FINITE_GHCR_TOKEN",
        ):
            with self.subTest(secret=secret):
                self.assertIn(f'set_depot_secret "{secret}"', wizard)
        for variable in (
            "DEPOT_PROJECT_ID",
            "PHALA_EXPECTED_WORKSPACE_ID",
            "PHALA_EXPECTED_WORKSPACE_SLUG",
        ):
            with self.subTest(variable=variable):
                self.assertIn(f'set_depot_var "{variable}"', wizard)
        release_enable = wizard.index(
            'set_depot_var "FINITE_RELEASE_PUBLISH_ENABLED" "true"'
        )
        image_enable = wizard.index(
            'set_depot_var "FINITE_GHCR_PRODUCTION_PUBLISH_ENABLED" "true"'
        )
        acceptance_gate = wizard.index(
            "Does the evidence bundle record every required positive, negative, rollback,"
        )
        legacy_disable = wizard.index('actions/permissions" -F enabled=false')
        hard_cut = wizard.index('stage "Hard-cut execution lanes"')
        release_legacy_disable = wizard.index(
            "for workflow in release-finitechat.yml release-fsite.yml release-fbrain.yml",
            hard_cut,
        )
        image_legacy_disable = wizard.index(
            "for workflow in service-images.yml runtime-image.yml deepseek-v4-vllm-image.yml",
            hard_cut,
        )
        remote_handoff = wizard.index('git remote set-url origin "$ORIGIN_CLONE_URL"')
        self.assertLess(acceptance_gate, release_enable)
        self.assertLess(acceptance_gate, image_enable)
        self.assertLess(remote_handoff, release_legacy_disable)
        self.assertLess(release_legacy_disable, release_enable)
        self.assertLess(image_legacy_disable, image_enable)
        self.assertLess(release_enable, legacy_disable)
        self.assertLess(image_enable, legacy_disable)
        canary_enable = wizard.index(
            'set_depot_var "FINITE_RELEASE_CANARY_PUBLISH_ENABLED" "true"'
        )
        canary_remove = wizard.index(
            'remove_depot_var "FINITE_RELEASE_CANARY_PUBLISH_ENABLED"'
        )
        self.assertLess(canary_enable, acceptance_gate)
        self.assertLess(acceptance_gate, canary_remove)
        self.assertLess(canary_remove, legacy_disable)
        self.assertIn("RELEASE_CANARY_ARMED=true", wizard)
        self.assertIn("trap cleanup_cutover EXIT", wizard)
        self.assertIn(
            'variant.get("attributes")\n'
            '            == [{"key": "repository", "value": repository}]',
            wizard,
        )
        self.assertIn('variant.get("name") == "default"', wizard)
        self.assertIn(
            'verify_depot_var "FINITE_RELEASE_PUBLISH_ENABLED" "true"', wizard
        )
        self.assertIn(
            'verify_depot_var "FINITE_GHCR_PRODUCTION_PUBLISH_ENABLED" "true"',
            wizard,
        )
        self.assertIn('push "$ORIGIN_CLONE_URL" --all', wizard)
        self.assertIn('push "$ORIGIN_CLONE_URL" --tags', wizard)
        self.assertIn(
            'diff -u "$scratch_dir/github-refs" "$scratch_dir/origin-refs-after"',
            wizard,
        )
        self.assertIn('actions/permissions" -F enabled=false', wizard)
        self.assertIn('gh secret delete "$name" --repo "$GITHUB_REPO"', wizard)
        self.assertIn('gh variable delete "$name" --repo "$GITHUB_REPO"', wizard)
        self.assertIn("--visibility private", wizard)
        self.assertIn('gh repo archive "$GITHUB_REPO" --yes', wizard)
        self.assertIn("origin-accepted-main:.depot/workflows/$workflow", wizard)
        self.assertIn("post-disable authoritative Release event", wizard)
        self.assertIn("post-disable authoritative image event", wizard)
        self.assertIn("post-disable authoritative CI/validation event", wizard)

    def test_operator_starts_from_the_staged_pr_after_parent_merge(self) -> None:
        plan = (ROOT / "docs" / "origin-depot-migration-plan.md").read_text(
            encoding="utf-8"
        )

        self.assertNotIn("After the implementation PR has merged", plan)
        self.assertIn("After #674 has merged", plan)
        self.assertIn("staged #675 branch", plan)


if __name__ == "__main__":
    unittest.main()

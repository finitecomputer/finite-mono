import argparse
import importlib.util
import json
import pathlib
import re
import sys
import unittest
from importlib.machinery import SourceFileLoader


ROOT = pathlib.Path(__file__).resolve().parents[2]
SELECT_HARNESSES = ROOT / "scripts" / "ci" / "select-harnesses"

# select-harnesses imports its sibling changed_paths module; running the
# script directly puts scripts/ci on sys.path, so do the same here.
sys.path.insert(0, str(SELECT_HARNESSES.parent))

loader = SourceFileLoader("select_harnesses", str(SELECT_HARNESSES))
spec = importlib.util.spec_from_loader(loader.name, loader)
select_harnesses = importlib.util.module_from_spec(spec)
assert spec.loader is not None
sys.modules[spec.name] = select_harnesses
spec.loader.exec_module(select_harnesses)


def selection_for(*paths: str) -> dict[str, str]:
    selection = select_harnesses.select_for_paths(list(paths))
    return selection.values()


def selected(*paths: str) -> set[str]:
    return {key for key, value in selection_for(*paths).items() if value == "true"}


def ci_workflow_text() -> str:
    return (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")


def ci_job_block(job_id: str) -> str:
    text = ci_workflow_text()
    start = text.index(f"\n  {job_id}:\n")
    match = re.search(r"\n  [a-zA-Z0-9_-]+:\n", text[start + 1 :])
    if match is None:
        return text[start:]
    return text[start : start + 1 + match.start()]


class CiHarnessSelectionTests(unittest.TestCase):
    def test_ci_pull_request_trigger_is_scoped_to_main(self) -> None:
        workflow = ci_workflow_text()

        self.assertIn("pull_request:\n    branches:\n      - main", workflow)
        self.assertNotIn("branches: [production]", workflow)

    def test_ci_authenticates_nix_github_fetches(self) -> None:
        workflow = ci_workflow_text()

        self.assertIn(
            "NIX_CONFIG: |\n"
            "    access-tokens = github.com=${{ github.token }}",
            workflow,
        )

    def test_hermes_flake_input_avoids_github_api_tarball_fetcher(self) -> None:
        flake_nix = (ROOT / "flake.nix").read_text(encoding="utf-8")
        flake_lock = json.loads((ROOT / "flake.lock").read_text(encoding="utf-8"))
        hermes_agent = flake_lock["nodes"]["hermes-agent"]
        locked = hermes_agent["locked"]
        original = hermes_agent["original"]

        self.assertEqual(locked["type"], "tarball")
        self.assertRegex(
            locked["url"],
            r"^https://github\.com/NousResearch/hermes-agent/archive/[0-9a-f]{40}\.tar\.gz$",
        )
        self.assertEqual(original, {"type": "tarball", "url": locked["url"]})
        self.assertNotIn("github:NousResearch/hermes-agent", flake_nix)
        self.assertNotIn("github:NousResearch/hermes-agent", ci_workflow_text())
        self.assertNotIn(
            "api.github.com/repos/NousResearch/hermes-agent/tarball",
            flake_nix + ci_workflow_text(),
        )

    def test_nix_consuming_ci_jobs_configure_cachix_read_only(self) -> None:
        for job_id in (
            "rust",
            "electron-alpha",
            "skills-check",
            "monitoring-nixos-contract",
            "finite-status-contract",
            "nix-checks",
        ):
            with self.subTest(job_id=job_id):
                block = ci_job_block(job_id)
                self.assertIn(
                    "uses: DeterminateSystems/nix-installer-action@v16",
                    block,
                )
                self.assertIn("Configure Cachix read-only cache", block)
                self.assertIn("name: ${{ env.CACHIX_CACHE_NAME }}", block)
                self.assertIn("skipPush: true", block)

    def test_closure_workflows_configure_cachix_read_only(self) -> None:
        # The latN closure workflows build first-party service packages
        # that regular CI already pushes to Cachix at the same rev; without
        # the read-only step every dispatch rebuilds the Rust services from
        # source (~30-45 min instead of minutes).
        for host in ("lat1", "lat2", "lat3", "lat4"):
            with self.subTest(workflow=host):
                text = (
                    ROOT / f".github/workflows/{host}-nixos-closure.yml"
                ).read_text(encoding="utf-8")
                self.assertIn(
                    "uses: DeterminateSystems/nix-installer-action@v16", text
                )
                self.assertIn("Configure Cachix read-only cache", text)
                self.assertIn("name: ${{ env.CACHIX_CACHE_NAME }}", text)
                self.assertIn("skipPush: true", text)
                self.assertIn("CACHIX_CACHE_NAME: finite", text)

    def test_monitoring_readme_runs_only_monitoring_contract(self) -> None:
        self.assertEqual(
            selected("infra/monitoring/README.md"),
            {"run_monitoring_nixos_contract"},
        )

    def test_monitoring_justfile_runs_only_monitoring_contract(self) -> None:
        self.assertEqual(
            selected("infra/monitoring/justfile"),
            {"run_monitoring_nixos_contract"},
        )

    def test_nixos_justfile_runs_nixos_contracts(self) -> None:
        self.assertEqual(
            selected("infra/nixos/justfile"),
            {"run_nix_checks", "run_nix_service_packages"},
        )

    def test_infra_justfile_runs_platform_contracts(self) -> None:
        self.assertEqual(
            selected("infra/justfile"),
            {"run_finite_status_contract", "run_nix_checks"},
        )

    def test_brain_justfile_runs_brain_contracts(self) -> None:
        self.assertEqual(
            selected("finite-brain/justfile"),
            {
                "run_brain_product_matrix",
                "run_nix_checks",
                "run_nix_service_packages",
            },
        )

    def test_chat_justfile_runs_chat_contracts(self) -> None:
        self.assertEqual(
            selected("finitechat/justfile"),
            {"run_hermes_bridge", "run_rust"},
        )

    def test_identity_justfile_runs_identity_contracts(self) -> None:
        self.assertEqual(
            selected("finite-identity/justfile"),
            {"run_nix_checks"},
        )

    def test_dashboard_justfile_runs_dashboard_and_nix_contracts(self) -> None:
        self.assertEqual(
            selected("finitecomputer-v2/apps/dashboard/justfile"),
            {"run_dashboard", "run_nix_checks"},
        )

    def test_finitecomputer_justfiles_run_nix_contracts(self) -> None:
        self.assertEqual(
            selected("finitecomputer-v2/justfile"),
            {"run_nix_checks"},
        )
        self.assertEqual(
            selected("finitecomputer-v2/deploy/finite-computer/images/justfile"),
            {"run_nix_checks"},
        )

    def test_dashboard_readme_is_docs_only(self) -> None:
        self.assertEqual(
            selected("finitecomputer-v2/apps/dashboard/README.md"),
            set(),
        )

    def test_runbook_markdown_runs_nix_checks(self) -> None:
        self.assertEqual(
            selected("infra/runbooks/deploy-core.md"),
            {"run_nix_checks"},
        )

    def test_production_cd_setup_files_run_nix_checks(self) -> None:
        self.assertEqual(
            selected(
                "scripts/production_cd_setup.py",
                "scripts/verify-production-cd-setup",
                "scripts/tests/test_production_cd_setup.py",
                "scripts/production_deploy.py",
                "scripts/tests/test_production_deploy.py",
            ),
            {"run_nix_checks"},
        )

    def test_ci_workflow_selects_every_active_harness(self) -> None:
        values = selection_for(".github/workflows/ci.yml")

        for key, value in values.items():
            if key == "run_electron_alpha":
                self.assertEqual(value, "false")
            else:
                self.assertEqual(value, "true", key)

    def test_non_ci_workflow_selects_nix_checks(self) -> None:
        self.assertEqual(
            selected(".github/workflows/production-deploy.yml"),
            {"run_nix_checks"},
        )

    def test_unknown_root_file_selects_every_active_harness(self) -> None:
        values = selection_for("pnpm-workspace.yaml")

        for key, value in values.items():
            if key == "run_electron_alpha":
                self.assertEqual(value, "false")
            else:
                self.assertEqual(value, "true", key)

    def test_unknown_root_script_selects_every_active_harness(self) -> None:
        values = selection_for("scripts/new_domain_helper.py")

        for key, value in values.items():
            if key == "run_electron_alpha":
                self.assertEqual(value, "false")
            else:
                self.assertEqual(value, "true", key)

    def test_dashboard_lib_change_skips_browser_e2e(self) -> None:
        self.assertEqual(
            selected("finitecomputer-v2/apps/dashboard/src/lib/hosted-web-chat.ts"),
            {"run_dashboard"},
        )

    def test_dashboard_component_change_runs_browser_e2e(self) -> None:
        self.assertEqual(
            selected(
                "finitecomputer-v2/apps/dashboard/src/components/hosted-web-chat.tsx"
            ),
            {"run_dashboard", "run_dashboard_browser"},
        )

    def test_shared_chat_ui_change_runs_dashboard_browser_e2e(self) -> None:
        self.assertEqual(
            selected("finitechat/packages/finitechat-chat-ui/src/model.ts"),
            {"run_dashboard", "run_dashboard_browser"},
        )

    def test_skill_spec_runs_skills_check(self) -> None:
        self.assertEqual(
            selected("finite-skills/skills/grill-me/SKILL.md"),
            {"run_skills_check"},
        )

    def test_regular_markdown_is_docs_only_even_under_skills(self) -> None:
        self.assertEqual(
            selected("finite-skills/README.md"),
            set(),
        )

    def test_monitoring_script_runs_only_monitoring_contract(self) -> None:
        self.assertEqual(
            selected("scripts/check_monitoring_nixos_contract.py"),
            {"run_monitoring_nixos_contract"},
        )

    def test_monitoring_nixos_module_runs_monitoring_and_nix_contracts(self) -> None:
        self.assertEqual(
            selected("infra/nixos/modules/monitoring-vps.nix"),
            {
                "run_monitoring_nixos_contract",
                "run_nix_checks",
                "run_nix_service_packages",
            },
        )

    def test_monitoring_runtime_metrics_script_runs_focused_contracts(self) -> None:
        self.assertEqual(
            selected("infra/nixos/scripts/finite_runtime_metrics.py"),
            {
                "run_monitoring_nixos_contract",
                "run_finite_status_contract",
                "run_nix_checks",
            },
        )

    def test_runtime_image_contract_paths_run_nix_checks(self) -> None:
        self.assertEqual(
            selected(
                "finitecomputer-v2/deploy/finite-computer/images/scripts/check_runtime_image_contract.py",
                "finitecomputer-v2/deploy/finite-computer/images/tests/test_runtime_image_contract.py",
            ),
            {"run_nix_checks"},
        )
        self.assertEqual(
            selected(
                "scripts/check_runtime_image_contract.py",
                "scripts/tests/test_runtime_image_contract.py",
            ),
            {"run_nix_checks"},
        )

    def test_identity_edge_contract_paths_run_nix_checks(self) -> None:
        self.assertEqual(
            selected(
                "finite-identity/scripts/identity-edge-contract-gate.py",
                "finite-identity/scripts/tests/test_identity_edge_contract_gate.py",
            ),
            {"run_nix_checks"},
        )

    def test_stripe_price_contract_path_runs_nix_checks(self) -> None:
        self.assertEqual(
            selected(
                "finitecomputer-v2/apps/dashboard/scripts/check_stripe_price_contract.py"
            ),
            {"run_nix_checks"},
        )
        self.assertEqual(
            selected("scripts/check_stripe_price_contract.py"),
            {"run_nix_checks"},
        )

    def test_moved_monitoring_paths_do_not_select_every_harness(self) -> None:
        self.assertEqual(
            selected(
                "scripts/check_self_hosted_monitoring_contract.py",
                "scripts/tests/test_finite_runtime_metrics.py",
            ),
            {"run_monitoring_nixos_contract"},
        )
        self.assertEqual(
            selected("scripts/finite_runtime_metrics.py"),
            {
                "run_monitoring_nixos_contract",
                "run_finite_status_contract",
                "run_nix_checks",
            },
        )

    def test_finite_status_script_runs_only_finite_status_contract(self) -> None:
        self.assertEqual(
            selected("scripts/finite_status.py"),
            {"run_finite_status_contract"},
        )

    def test_devfinity_smoke_depends_on_nix_service_packages(self) -> None:
        selection = select_harnesses.HarnessSelection(run_devfinity_smoke=True)

        select_harnesses.apply_harness_dependencies(selection)

        self.assertTrue(selection.run_nix_service_packages)

    def test_brain_product_matrix_depends_on_nix_service_packages(self) -> None:
        selection = select_harnesses.HarnessSelection(run_brain_product_matrix=True)

        select_harnesses.apply_harness_dependencies(selection)

        self.assertTrue(selection.run_nix_service_packages)

    def test_merge_group_selects_every_active_harness(self) -> None:
        args = argparse.Namespace(changed_files=None, event="merge_group")
        selection, _reason, _paths = select_harnesses.select_harnesses(args)
        values = selection.values()

        for key, value in values.items():
            if key == "run_electron_alpha":
                self.assertEqual(value, "false")
            else:
                self.assertEqual(value, "true", key)

    def test_changed_file_workflow_path_selects_nix_checks(self) -> None:
        args = argparse.Namespace(
            changed_files=[".github/workflows/production-deploy.yml"], event=""
        )
        selection, _reason, paths = select_harnesses.select_harnesses(args)

        self.assertEqual(paths, [".github/workflows/production-deploy.yml"])
        self.assertEqual(
            {key for key, value in selection.values().items() if value == "true"},
            {"run_nix_checks"},
        )

    def test_changed_file_dot_slash_prefix_matches_bare_path(self) -> None:
        prefixed = argparse.Namespace(changed_files=["./justfile"], event="")
        bare = argparse.Namespace(changed_files=["justfile"], event="")

        prefixed_selection, _reason, prefixed_paths = select_harnesses.select_harnesses(
            prefixed
        )
        bare_selection, _reason, bare_paths = select_harnesses.select_harnesses(bare)

        self.assertEqual(prefixed_paths, bare_paths)
        self.assertEqual(prefixed_selection.values(), bare_selection.values())


def pull_request_args(*paths: str) -> argparse.Namespace:
    return argparse.Namespace(changed_files=list(paths), event="pull_request")


class CiPackagingGateTests(unittest.TestCase):
    def test_pull_request_defers_nix_service_packages(self) -> None:
        # A plain source change selects every harness as an unknown path, and
        # the devfinity/brain dependency rule forces packaging on; the
        # pull_request gate must still defer packaging.
        selection, reason, _paths = select_harnesses.select_harnesses(
            pull_request_args("finitechat/crates/finitechat-server/src/lib.rs")
        )

        self.assertFalse(selection.run_nix_service_packages)
        self.assertTrue(selection.run_devfinity_smoke)
        self.assertTrue(selection.run_brain_product_matrix)
        self.assertIn("deferred to landing events", reason)

    def test_pull_request_flake_lock_change_keeps_nix_service_packages(self) -> None:
        selection, reason, _paths = select_harnesses.select_harnesses(
            pull_request_args("flake.lock", "Cargo.lock")
        )

        self.assertTrue(selection.run_nix_service_packages)
        self.assertNotIn("deferred", reason)

    def test_pull_request_cargo_lock_change_keeps_nix_service_packages(self) -> None:
        # Direct coverage for the Cargo.lock-only PR: cargoVendorDir and every
        # mkCargoArtifacts derive from the root Cargo.lock, so deferring
        # packaging here would move the cold crane build into devfinity-smoke
        # without a Cachix warm-up.
        selection, reason, _paths = select_harnesses.select_harnesses(
            pull_request_args("Cargo.lock")
        )

        self.assertTrue(selection.run_nix_service_packages)
        self.assertNotIn("deferred", reason)

    def test_pull_request_package_input_changes_keep_nix_service_packages(self) -> None:
        # Every package-derivation input family keeps the PR-time package
        # build: workspace manifests (mkDummySrc scopes cargoArtifacts to
        # them), the single Rust pin, cargo config, the flake graph, and the
        # infra/nixos packaging surface itself.
        for changed in (
            "Cargo.toml",
            "finitechat/crates/finitechat-server/Cargo.toml",
            "rust-toolchain.toml",
            ".cargo/config.toml",
            "flake.nix",
            "flake.lock",
            "infra/nixos/packages.nix",
            "infra/nixos/modules/monitoring-vps.nix",
        ):
            with self.subTest(changed=changed):
                selection, reason, _paths = select_harnesses.select_harnesses(
                    pull_request_args(changed)
                )
                self.assertTrue(selection.run_nix_service_packages, changed)
                self.assertNotIn("deferred", reason)

    def test_pull_request_derivation_irrelevant_change_defers_nix_service_packages(
        self,
    ) -> None:
        # Generated-code sources feed only the thin final package
        # derivations, which devfinity-smoke builds locally against warm
        # cargoArtifacts - they do not justify the PR-time package build.
        selection, reason, _paths = select_harnesses.select_harnesses(
            pull_request_args("finitechat/protos/chat.proto")
        )

        self.assertFalse(selection.run_nix_service_packages)
        self.assertIn("deferred to landing events", reason)

    def test_pull_request_flake_nix_change_keeps_nix_service_packages(self) -> None:
        selection, _reason, _paths = select_harnesses.select_harnesses(
            pull_request_args("flake.nix")
        )

        self.assertTrue(selection.run_nix_service_packages)

    def test_push_event_keeps_nix_service_packages(self) -> None:
        args = argparse.Namespace(
            changed_files=["finitechat/crates/finitechat-server/src/lib.rs"],
            event="push",
        )

        selection, _reason, _paths = select_harnesses.select_harnesses(args)

        self.assertTrue(selection.run_nix_service_packages)

    def test_merge_group_runs_every_harness_including_packaging(self) -> None:
        args = argparse.Namespace(changed_files=None, event="merge_group")

        selection, _reason, _paths = select_harnesses.select_harnesses(args)

        self.assertTrue(selection.run_nix_service_packages)

    def test_unselected_packaging_gate_is_a_no_op(self) -> None:
        # Docs-only changes select nothing; the gate must not invent a note.
        selection, reason, _paths = select_harnesses.select_harnesses(
            pull_request_args("docs/some-note.md")
        )

        self.assertFalse(selection.run_nix_service_packages)
        self.assertNotIn("deferred", reason)


if __name__ == "__main__":
    unittest.main()

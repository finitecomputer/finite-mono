import argparse
import importlib.util
import pathlib
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
    return {
        key
        for key, value in selection_for(*paths).items()
        if value == "true"
    }


class CiHarnessSelectionTests(unittest.TestCase):
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

    def test_ci_workflow_selects_every_active_harness(self) -> None:
        values = selection_for(".depot/workflows/ci.yml")

        for key, value in values.items():
            self.assertEqual(value, "true", key)

    def test_unknown_root_file_selects_every_active_harness(self) -> None:
        values = selection_for("pnpm-workspace.yaml")

        for key, value in values.items():
            self.assertEqual(value, "true", key)

    def test_unknown_root_script_selects_every_active_harness(self) -> None:
        values = selection_for("scripts/new_domain_helper.py")

        for key, value in values.items():
            self.assertEqual(value, "true", key)

    def test_dashboard_lib_change_skips_browser_e2e(self) -> None:
        self.assertEqual(
            selected("finitecomputer-v2/apps/dashboard/src/lib/hosted-web-chat.ts"),
            {"run_dashboard"},
        )

    def test_dashboard_component_change_runs_browser_e2e(self) -> None:
        self.assertEqual(
            selected("finitecomputer-v2/apps/dashboard/src/components/hosted-web-chat.tsx"),
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
        self.assertEqual(
            selected(
                "scripts/identity-edge-contract-gate.py",
                "scripts/tests/test_identity_edge_contract_gate.py",
            ),
            {"run_nix_checks"},
        )

    def test_stripe_price_contract_path_runs_nix_checks(self) -> None:
        self.assertEqual(
            selected("finitecomputer-v2/apps/dashboard/scripts/check_stripe_price_contract.py"),
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
            self.assertEqual(value, "true", key)

    def test_changed_file_dotfile_path_selects_every_active_harness(self) -> None:
        args = argparse.Namespace(changed_files=[".github/workflows/README.md"])
        selection, _reason, paths = select_harnesses.select_harnesses(args)
        values = selection.values()

        self.assertEqual(paths, [".github/workflows/README.md"])
        for key, value in values.items():
            self.assertEqual(value, "true", key)

    def test_changed_file_dot_slash_prefix_matches_bare_path(self) -> None:
        prefixed = argparse.Namespace(changed_files=["./justfile"])
        bare = argparse.Namespace(changed_files=["justfile"])

        prefixed_selection, _reason, prefixed_paths = select_harnesses.select_harnesses(prefixed)
        bare_selection, _reason, bare_paths = select_harnesses.select_harnesses(bare)

        self.assertEqual(prefixed_paths, bare_paths)
        self.assertEqual(prefixed_selection.values(), bare_selection.values())


if __name__ == "__main__":
    unittest.main()

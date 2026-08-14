import argparse
import importlib.util
import pathlib
import sys
import unittest
from importlib.machinery import SourceFileLoader


ROOT = pathlib.Path(__file__).resolve().parents[2]
SELECT_HARNESSES = ROOT / "scripts" / "ci" / "select-harnesses"

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
            selected("infra/monitoring/self-hosted/README.md"),
            {"run_self_hosted_monitoring_contract"},
        )

    def test_dashboard_readme_is_docs_only(self) -> None:
        self.assertEqual(
            selected("finitecomputer-v2/apps/dashboard/README.md"),
            set(),
        )

    def test_ci_workflow_selects_every_active_harness(self) -> None:
        values = selection_for(".github/workflows/ci.yml")

        for key, value in values.items():
            if key == "run_electron_alpha":
                self.assertEqual(value, "false")
            else:
                self.assertEqual(value, "true", key)

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
            selected("infra/monitoring/self-hosted/scripts/check_self_hosted_monitoring_contract.py"),
            {"run_self_hosted_monitoring_contract"},
        )

    def test_monitoring_runtime_metrics_script_runs_focused_contracts(self) -> None:
        self.assertEqual(
            selected("infra/monitoring/self-hosted/scripts/finite_runtime_metrics.py"),
            {
                "run_self_hosted_monitoring_contract",
                "run_finite_status_contract",
                "run_nix_checks",
            },
        )

    def test_moved_monitoring_paths_do_not_select_every_harness(self) -> None:
        self.assertEqual(
            selected(
                "scripts/check_self_hosted_monitoring_contract.py",
                "scripts/tests/test_finite_runtime_metrics.py",
            ),
            {"run_self_hosted_monitoring_contract"},
        )
        self.assertEqual(
            selected("scripts/finite_runtime_metrics.py"),
            {
                "run_self_hosted_monitoring_contract",
                "run_finite_status_contract",
                "run_nix_checks",
            },
        )

    def test_finite_status_script_runs_only_finite_status_contract(self) -> None:
        self.assertEqual(
            selected("scripts/finite_status.py"),
            {"run_finite_status_contract"},
        )

    def test_merge_group_selects_every_active_harness(self) -> None:
        args = argparse.Namespace(changed_files=None, event="merge_group")
        selection, _reason, _paths = select_harnesses.select_harnesses(args)
        values = selection.values()

        for key, value in values.items():
            if key == "run_electron_alpha":
                self.assertEqual(value, "false")
            else:
                self.assertEqual(value, "true", key)


if __name__ == "__main__":
    unittest.main()

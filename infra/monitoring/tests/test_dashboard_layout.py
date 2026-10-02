"""Check hidden Grafana panels as rigorously as visible panels."""

import copy
import importlib.util
from pathlib import Path
import unittest


SPEC = importlib.util.spec_from_file_location(
    "monitoring_contract",
    Path(__file__).resolve().parents[3] / "scripts/check_monitoring_nixos_contract.py",
)
contract = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(contract)


def panel(id, x, y, w=12, h=8, **extra):
    return {
        "id": id,
        "title": str(id),
        "gridPos": {"x": x, "y": y, "w": w, "h": h},
        **extra,
    }


class DashboardLayoutTest(unittest.TestCase):
    def setUp(self):
        self.dashboard = {
            "panels": [
                panel(1, 0, 0),
                panel(2, 12, 0),
                panel(3, 0, 8, 24, 1, type="row", collapsed=True,
                      panels=[panel(4, 0, 9), panel(5, 12, 9)]),
                panel(6, 0, 9, 24, 1, type="row", collapsed=True,
                      panels=[panel(7, 0, 10)]),
            ]
        }

    def test_hidden_rows_can_overlap_later_top_level_positions(self):
        contract.check_dashboard_layout(self.dashboard, "fixture")
        self.assertEqual(
            [p["id"] for p in contract.all_panels(self.dashboard["panels"])],
            [1, 2, 3, 4, 5, 6, 7],
        )

    def test_nested_panels_reject_duplicate_ids_bounds_and_sibling_overlap(self):
        for change, error in [
            ({"id": 1}, "unique"),
            ({"gridPos": {"x": 20, "y": 9, "w": 12, "h": 8}}, "24-column"),
            ({"gridPos": {"x": 6, "y": 9, "w": 12, "h": 8}}, "overlap"),
        ]:
            with self.subTest(change=change):
                dashboard = copy.deepcopy(self.dashboard)
                dashboard["panels"][2]["panels"][0].update(change)
                with self.assertRaisesRegex(AssertionError, error):
                    contract.check_dashboard_layout(dashboard, "fixture")

    def test_actual_overview_retains_critical_signals_and_hidden_query_contracts(self):
        contract.check_mvp_dashboard_contract()


if __name__ == "__main__":
    unittest.main()

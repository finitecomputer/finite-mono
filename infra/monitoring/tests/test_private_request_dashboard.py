import json
from pathlib import Path
import re
import unittest

DASHBOARD = (
    Path(__file__).parents[1]
    / "grafana"
    / "dashboards"
    / "finite-private-requests.json"
)


class LokiInstantQueries(unittest.TestCase):
    def test_retained_totals_use_the_unsplit_range_aggregation(self):
        # Loki 3.5 splits instant sum/count/max/min_over_time queries hourly and
        # merges a pushed-down sum(max_over_time) with max, so a 24h total became
        # the busiest hour. It never splits last_over_time, and exporter replays
        # are identical, so one value per reservation is still deduplicated.
        dashboard = json.loads(DASHBOARD.read_text())
        expressions = [
            target["expr"]
            for panel in dashboard["panels"]
            if panel.get("datasource", {}).get("uid") == "finite-loki"
            for target in panel.get("targets", [])
            if target.get("queryType") == "instant"
        ]
        self.assertEqual(len(expressions), 9)
        for expression in expressions:
            self.assertEqual(
                re.findall(r"\b(\w+_over_time)\(", expression),
                ["last_over_time"],
                expression,
            )

    def test_user_filter_is_limited_to_retained_events(self):
        dashboard = json.loads(DASHBOARD.read_text())
        for panel in dashboard["panels"]:
            for target in panel.get("targets", []):
                expression = target["expr"]
                if panel["datasource"]["uid"] == "finite-loki":
                    self.assertIn('user=~"${usage_user:raw}"', expression)
                    self.assertIn('{{if .user}}{{.user}}{{else}}unknown{{end}}', expression)
                else:
                    self.assertNotIn("usage_user", expression)


if __name__ == "__main__":
    unittest.main()

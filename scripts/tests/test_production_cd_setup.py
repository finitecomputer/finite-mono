from __future__ import annotations

import unittest

from scripts import production_cd_setup


def valid_ruleset() -> dict[str, object]:
    return {
        "name": "production",
        "target": "branch",
        "enforcement": "active",
        "conditions": {"ref_name": {"include": ["refs/heads/production"]}},
        "rules": [
            {"type": "deletion"},
            {"type": "non_fast_forward"},
            {
                "type": "pull_request",
                "parameters": {"required_approving_review_count": 1},
            },
            {
                "type": "required_status_checks",
                "parameters": {
                    "required_status_checks": [
                        {"context": "CI gate"},
                        {"context": "Plan production deploy"},
                    ]
                },
            },
        ],
    }


class ProductionCdSetupTests(unittest.TestCase):
    def test_parse_secret_names_uses_first_column(self) -> None:
        names = production_cd_setup.parse_secret_names(
            "FINITE_PRODUCTION_SSH_KEY\t2026-08-27T00:00:00Z\n"
            "FINITE_PRODUCTION_KNOWN_HOSTS\t2026-08-27T00:00:00Z\n"
        )
        self.assertEqual(
            names,
            {
                "FINITE_PRODUCTION_KNOWN_HOSTS",
                "FINITE_PRODUCTION_SSH_KEY",
            },
        )

    def test_environment_requires_reviewers_and_branch_policy(self) -> None:
        environment = {
            "protection_rules": [
                {"type": "required_reviewers", "reviewers": [{"type": "User"}]}
            ],
            "deployment_branch_policy": {"protected_branches": True},
        }
        self.assertTrue(
            production_cd_setup.environment_has_required_reviewers(environment)
        )
        self.assertTrue(
            production_cd_setup.environment_branch_policy_mentions_production(
                environment
            )
        )

    def test_ruleset_accepts_minimum_production_governance(self) -> None:
        checks = production_cd_setup.evaluate_ruleset(valid_ruleset())
        self.assertTrue(all(check.ok for check in checks), checks)

    def test_ruleset_rejects_missing_required_check(self) -> None:
        ruleset = valid_ruleset()
        status_rule = production_cd_setup.rule_by_type(
            ruleset, "required_status_checks"
        )
        assert status_rule is not None
        status_rule["parameters"]["required_status_checks"] = [
            {"context": "CI gate"}
        ]
        checks = production_cd_setup.evaluate_ruleset(ruleset)
        failing = [check for check in checks if check.name == "production required checks"]
        self.assertEqual(len(failing), 1)
        self.assertFalse(failing[0].ok)
        self.assertIn("Plan production deploy", failing[0].detail)

    def test_ruleset_rejects_zero_approvals(self) -> None:
        ruleset = valid_ruleset()
        pr_rule = production_cd_setup.rule_by_type(ruleset, "pull_request")
        assert pr_rule is not None
        pr_rule["parameters"]["required_approving_review_count"] = 0
        checks = production_cd_setup.evaluate_ruleset(ruleset)
        failing = [check for check in checks if check.name == "production PR approval"]
        self.assertEqual(len(failing), 1)
        self.assertFalse(failing[0].ok)


if __name__ == "__main__":
    unittest.main()

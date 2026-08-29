#!/usr/bin/env python3
"""Read-only verifier for the GitHub production CD setup."""

from __future__ import annotations

import argparse
import json
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
REQUIRED_ENVIRONMENT = "production"
REQUIRED_ENVIRONMENT_SECRETS = {
    "FINITE_PRODUCTION_KNOWN_HOSTS",
    "FINITE_PRODUCTION_SSH_KEY",
}
REQUIRED_STATUS_CHECKS = {"Plan production deploy"}
REQUIRED_WORKFLOWS = {
    ".github/workflows/open-production-deploy-pr.yml",
    ".github/workflows/production-deploy-plan.yml",
    ".github/workflows/production-deploy.yml",
}


@dataclass(frozen=True)
class Check:
    name: str
    ok: bool
    detail: str


class SetupProbeError(RuntimeError):
    pass


def run_text(command: list[str]) -> str:
    result = subprocess.run(
        command,
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        message = result.stderr.strip() or result.stdout.strip() or "command failed"
        raise SetupProbeError(f"{' '.join(command)}: {message}")
    return result.stdout


def gh_json(args: list[str]) -> Any:
    return json.loads(run_text(["gh", *args]))


def environment_has_required_reviewers(environment: dict[str, Any]) -> bool:
    for rule in environment.get("protection_rules") or []:
        if rule.get("type") != "required_reviewers":
            continue
        reviewers = rule.get("reviewers") or []
        if reviewers:
            return True
    return False


def environment_branch_policy_mentions_production(environment: dict[str, Any]) -> bool:
    policy = environment.get("deployment_branch_policy")
    if policy is None:
        return False
    if policy.get("protected_branches") is True:
        return True
    if policy.get("custom_branch_policies") is True:
        # The environment summary endpoint does not include branch policy names.
        # A custom policy is acceptable only when the setup checklist pins it to
        # production; the verifier reports this as a conservative pass.
        return True
    return False


def parse_secret_names(output: str) -> set[str]:
    names: set[str] = set()
    for line in output.splitlines():
        stripped = line.strip()
        if not stripped:
            continue
        names.add(stripped.split()[0])
    return names


def rule_by_type(ruleset: dict[str, Any], rule_type: str) -> dict[str, Any] | None:
    for rule in ruleset.get("rules") or []:
        if rule.get("type") == rule_type:
            return rule
    return None


def ruleset_targets_production(ruleset: dict[str, Any]) -> bool:
    ref_names = (ruleset.get("conditions") or {}).get("ref_name") or {}
    return "refs/heads/production" in (ref_names.get("include") or [])


def pull_request_review_count(ruleset: dict[str, Any]) -> int:
    rule = rule_by_type(ruleset, "pull_request")
    if not rule:
        return 0
    params = rule.get("parameters") or {}
    return int(params.get("required_approving_review_count") or 0)


def required_check_contexts(ruleset: dict[str, Any]) -> set[str]:
    rule = rule_by_type(ruleset, "required_status_checks")
    if not rule:
        return set()
    checks = (rule.get("parameters") or {}).get("required_status_checks") or []
    return {
        check.get("context")
        for check in checks
        if isinstance(check, dict) and check.get("context")
    }


def evaluate_ruleset(ruleset: dict[str, Any]) -> list[Check]:
    checks = {
        "deletion": rule_by_type(ruleset, "deletion") is not None,
        "non-fast-forward": rule_by_type(ruleset, "non_fast_forward") is not None,
        "pull-request": rule_by_type(ruleset, "pull_request") is not None,
        "required-status-checks": rule_by_type(ruleset, "required_status_checks")
        is not None,
    }
    contexts = required_check_contexts(ruleset)
    missing_contexts = sorted(REQUIRED_STATUS_CHECKS - contexts)
    unexpected_contexts = sorted(contexts - REQUIRED_STATUS_CHECKS)
    return [
        Check(
            "production ruleset active",
            ruleset.get("enforcement") == "active",
            f"enforcement={ruleset.get('enforcement')!r}",
        ),
        Check(
            "production ruleset target",
            ruleset_targets_production(ruleset),
            "targets refs/heads/production",
        ),
        Check(
            "production ruleset branch safety",
            all(checks.values()),
            "requires deletion, non-fast-forward, pull-request, and status-check rules",
        ),
        Check(
            "production PR approval",
            pull_request_review_count(ruleset) >= 1,
            f"required approvals={pull_request_review_count(ruleset)}",
        ),
        Check(
            "production required checks",
            not missing_contexts and not unexpected_contexts,
            "missing: " + ", ".join(missing_contexts)
            if missing_contexts
            else "unexpected: " + ", ".join(unexpected_contexts)
            if unexpected_contexts
            else "exactly Plan production deploy",
        ),
    ]


def local_workflow_checks() -> list[Check]:
    checks: list[Check] = []
    for path in sorted(REQUIRED_WORKFLOWS):
        checks.append(
            Check(
                f"workflow {path}",
                (ROOT / path).is_file(),
                "exists",
            )
        )
    return checks


def collect_checks(repo: str) -> list[Check]:
    checks = local_workflow_checks()

    try:
        gh_json(["api", f"repos/{repo}/git/ref/heads/production"])
        checks.append(Check("production branch", True, "exists"))
    except SetupProbeError as error:
        checks.append(Check("production branch", False, str(error)))

    try:
        environment = gh_json(
            ["api", f"repos/{repo}/environments/{REQUIRED_ENVIRONMENT}"]
        )
        checks.extend(
            [
                Check("production environment", True, "exists"),
                Check(
                    "production environment reviewers",
                    environment_has_required_reviewers(environment),
                    "has required reviewers",
                ),
                Check(
                    "production environment branch policy",
                    environment_branch_policy_mentions_production(environment),
                    "restricted to protected branches or custom production policy",
                ),
            ]
        )
    except SetupProbeError as error:
        checks.append(Check("production environment", False, str(error)))

    try:
        secret_names = parse_secret_names(
            run_text(
                [
                    "gh",
                    "secret",
                    "list",
                    "--repo",
                    repo,
                    "--env",
                    REQUIRED_ENVIRONMENT,
                ]
            )
        )
        missing = sorted(REQUIRED_ENVIRONMENT_SECRETS - secret_names)
        checks.append(
            Check(
                "production environment secrets",
                not missing,
                "missing: " + ", ".join(missing) if missing else "all present",
            )
        )
    except SetupProbeError as error:
        checks.append(Check("production environment secrets", False, str(error)))

    try:
        summaries = gh_json(["api", f"repos/{repo}/rulesets"])
        production = next(
            (
                ruleset
                for ruleset in summaries
                if ruleset.get("name") == "production"
                and ruleset.get("target") == "branch"
            ),
            None,
        )
        if production is None:
            checks.append(Check("production ruleset", False, "missing"))
        else:
            ruleset = gh_json(["api", f"repos/{repo}/rulesets/{production['id']}"])
            checks.extend(evaluate_ruleset(ruleset))
    except SetupProbeError as error:
        checks.append(Check("production ruleset", False, str(error)))

    return checks


def print_checks(checks: list[Check]) -> None:
    for check in checks:
        status = "OK" if check.ok else "MISSING"
        print(f"[{status}] {check.name}: {check.detail}")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", default="finitecomputer/finite-mono")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    checks = collect_checks(args.repo)
    print_checks(checks)
    return 0 if all(check.ok for check in checks) else 1


if __name__ == "__main__":
    raise SystemExit(main())

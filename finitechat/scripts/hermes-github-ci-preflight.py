#!/usr/bin/env python3
"""Check GitHub Actions setup for the Hermes S3 publish gate."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

REQUIRED_SECRETS = {
    "FINITE_DOCKER_RESTIC_PASSWORD",
    "FINITE_DOCKER_RESTIC_AWS_ACCESS_KEY_ID",
    "FINITE_DOCKER_RESTIC_AWS_SECRET_ACCESS_KEY",
    "OPENROUTER_API_KEY",
}
OPTIONAL_SECRETS = {
    "FINITE_DOCKER_RESTIC_AWS_REGION",
    "FINITE_DOCKER_RESTIC_AWS_SESSION_TOKEN",
}
REQUIRED_VARIABLES = {
    "FINITE_LATITUDE_STORAGE_BUCKET",
    "FINITE_DOCKER_RESTIC_PREFIX",
}
OPTIONAL_VARIABLES_WITH_WORKFLOW_DEFAULTS = {
    "FINITE_LATITUDE_OBJECT_ENDPOINT": "https://objects.nyc.storage.sh",
    "FINITECHAT_HERMES_MODEL": "anthropic/claude-sonnet-4.6",
    "FINITECHAT_HERMES_PROVIDER": "openrouter",
}


def load_json(path: Path) -> list[dict[str, Any]]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, list):
        raise SystemExit(f"expected JSON list in {path}")
    return [item for item in value if isinstance(item, dict)]


def gh_json(repo: str, kind: str) -> list[dict[str, Any]]:
    if kind == "secret":
        args = ["gh", "secret", "list", "--repo", repo, "--json", "name"]
    elif kind == "variable":
        args = ["gh", "variable", "list", "--repo", repo, "--json", "name,value"]
    else:
        raise ValueError(kind)
    result = subprocess.run(args, capture_output=True, text=True, check=True)
    value = json.loads(result.stdout)
    if not isinstance(value, list):
        raise SystemExit(f"gh returned non-list JSON for {kind} list")
    return [item for item in value if isinstance(item, dict)]


def names(items: list[dict[str, Any]]) -> set[str]:
    return {str(item.get("name")) for item in items if item.get("name")}


def values_by_name(items: list[dict[str, Any]]) -> dict[str, str]:
    result: dict[str, str] = {}
    for item in items:
        name = item.get("name")
        value = item.get("value")
        if isinstance(name, str) and isinstance(value, str):
            result[name] = value
    return result


def validate(
    *,
    repo: str,
    secrets: list[dict[str, Any]],
    variables: list[dict[str, Any]],
) -> tuple[int, dict[str, Any]]:
    secret_names = names(secrets)
    variable_names = names(variables)
    variable_values = values_by_name(variables)
    missing_secrets = sorted(REQUIRED_SECRETS - secret_names)
    missing_variables = sorted(REQUIRED_VARIABLES - variable_names)
    optional_variables = {
        key: {
            "present": key in variable_names,
            "value": variable_values.get(key),
            "workflow_default": default,
        }
        for key, default in OPTIONAL_VARIABLES_WITH_WORKFLOW_DEFAULTS.items()
    }
    status = "ok" if not missing_secrets and not missing_variables else "failed"
    report = {
        "status": status,
        "generated_at_unix": int(time.time()),
        "repo": repo,
        "required_secrets": sorted(REQUIRED_SECRETS),
        "present_required_secrets": sorted(REQUIRED_SECRETS & secret_names),
        "missing_required_secrets": missing_secrets,
        "optional_secrets": sorted(OPTIONAL_SECRETS),
        "present_optional_secrets": sorted(OPTIONAL_SECRETS & secret_names),
        "required_variables": sorted(REQUIRED_VARIABLES),
        "present_required_variables": sorted(REQUIRED_VARIABLES & variable_names),
        "missing_required_variables": missing_variables,
        "optional_variables": optional_variables,
        "next_manual_workflow": {
            "workflow": ".github/workflows/ci.yml",
            "inputs": {
                "docker_smoke": True,
                "publish_runtime_image": True,
                "restic_backend": "s3",
            },
        },
    }
    return (0 if status == "ok" else 2), report


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", default="finitecomputer/finitechat")
    parser.add_argument("--secrets-json", help="test fixture for gh secret list JSON")
    parser.add_argument("--variables-json", help="test fixture for gh variable list JSON")
    parser.add_argument("--report", default="target/hermes-github-ci-preflight.json")
    args = parser.parse_args()

    secrets = (
        load_json(Path(args.secrets_json)) if args.secrets_json else gh_json(args.repo, "secret")
    )
    variables = (
        load_json(Path(args.variables_json))
        if args.variables_json
        else gh_json(args.repo, "variable")
    )
    status, report = validate(repo=args.repo, secrets=secrets, variables=variables)
    report_path = Path(args.report)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(report, indent=2) + "\n"
    report_path.write_text(text, encoding="utf-8")
    print(text, end="")
    return status


if __name__ == "__main__":
    sys.exit(main())

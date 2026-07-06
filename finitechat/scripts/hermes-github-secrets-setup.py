#!/usr/bin/env python3
"""Install GitHub Actions secrets/variables needed by the Hermes S3 gate."""

from __future__ import annotations

import argparse
import configparser
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

DEFAULT_REPO = "finitecomputer/finitechat"
DEFAULT_ENV_FILE = ".env"
DEFAULT_LATITUDE_ENDPOINT = "https://objects.nyc.storage.sh"
DEFAULT_RESTIC_PREFIX = "agent-runtimes/tinfoil-canary-001/restic"
REQUIRED_SECRET_SOURCES = {
    "FINITE_DOCKER_RESTIC_PASSWORD": ["FINITE_DOCKER_RESTIC_PASSWORD"],
    "FINITE_DOCKER_RESTIC_AWS_ACCESS_KEY_ID": [
        "FINITE_DOCKER_RESTIC_AWS_ACCESS_KEY_ID",
        "AWS_ACCESS_KEY_ID",
    ],
    "FINITE_DOCKER_RESTIC_AWS_SECRET_ACCESS_KEY": [
        "FINITE_DOCKER_RESTIC_AWS_SECRET_ACCESS_KEY",
        "AWS_SECRET_ACCESS_KEY",
    ],
}
OPTIONAL_SECRET_SOURCES = {
    "FINITE_DOCKER_RESTIC_AWS_SESSION_TOKEN": [
        "FINITE_DOCKER_RESTIC_AWS_SESSION_TOKEN",
        "AWS_SESSION_TOKEN",
    ],
    "FINITE_DOCKER_RESTIC_AWS_REGION": [
        "FINITE_DOCKER_RESTIC_AWS_REGION",
        "AWS_REGION",
        "AWS_DEFAULT_REGION",
    ],
}
REQUIRED_VARIABLE_SOURCES = {
    "FINITE_LATITUDE_STORAGE_BUCKET": ["FINITE_LATITUDE_STORAGE_BUCKET"],
}
OPTIONAL_VARIABLE_DEFAULTS = {
    "FINITE_LATITUDE_OBJECT_ENDPOINT": {
        "sources": ["FINITE_LATITUDE_OBJECT_ENDPOINT"],
        "default": DEFAULT_LATITUDE_ENDPOINT,
    },
    "FINITE_DOCKER_RESTIC_PREFIX": {
        "sources": ["FINITE_DOCKER_RESTIC_PREFIX"],
        "default": DEFAULT_RESTIC_PREFIX,
    },
}


def parse_env_file(path: Path) -> dict[str, str]:
    if not path.exists():
        return {}
    result: dict[str, str] = {}
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("export "):
            line = line[len("export ") :].strip()
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip()
        if (value.startswith('"') and value.endswith('"')) or (
            value.startswith("'") and value.endswith("'")
        ):
            value = value[1:-1]
        if key:
            result[key] = value
    return result


def aws_config_section(profile: str) -> str:
    if profile == "default":
        return "default"
    return f"profile {profile}"


def parse_aws_shared_config(
    *,
    credentials_file: Path,
    config_file: Path,
    profile: str,
) -> dict[str, str]:
    values: dict[str, str] = {}
    profile = profile or "default"

    credentials = configparser.ConfigParser()
    credentials.read(credentials_file)
    if credentials.has_section(profile):
        section = credentials[profile]
        access_key = section.get("aws_access_key_id")
        secret_key = section.get("aws_secret_access_key")
        session_token = section.get("aws_session_token")
        if access_key:
            values["AWS_ACCESS_KEY_ID"] = access_key
        if secret_key:
            values["AWS_SECRET_ACCESS_KEY"] = secret_key
        if session_token:
            values["AWS_SESSION_TOKEN"] = session_token
        region = section.get("region")
        if region:
            values["AWS_REGION"] = region

    config = configparser.ConfigParser()
    config.read(config_file)
    section_name = aws_config_section(profile)
    if config.has_section(section_name):
        region = config[section_name].get("region")
        if region:
            values["AWS_REGION"] = region
    return values


def merged_env(
    env_file: Path,
    process_env: dict[str, str],
    *,
    aws_shared_values: dict[str, str] | None = None,
) -> dict[str, str]:
    values = dict(aws_shared_values or {})
    values.update(parse_env_file(env_file))
    values.update(process_env)
    return values


def first_value(values: dict[str, str], sources: list[str]) -> tuple[str | None, str | None]:
    for source in sources:
        value = values.get(source)
        if value:
            return value, source
    return None, None


def gh_names(repo: str, kind: str) -> set[str]:
    if kind == "secret":
        command = ["gh", "secret", "list", "--repo", repo, "--json", "name"]
    elif kind == "variable":
        command = ["gh", "variable", "list", "--repo", repo, "--json", "name"]
    else:
        raise ValueError(kind)
    result = subprocess.run(command, capture_output=True, text=True, check=True)
    value = json.loads(result.stdout)
    if not isinstance(value, list):
        return set()
    return {str(item.get("name")) for item in value if isinstance(item, dict) and item.get("name")}


def existing_action(kind: str, name: str, *, required: bool) -> dict[str, Any]:
    return {
        "kind": kind,
        "name": name,
        "source": "github-existing",
        "value": None,
        "required": required,
        "existing": True,
    }


def secret_plan(
    values: dict[str, str], *, existing_secret_names: set[str]
) -> tuple[list[dict[str, Any]], list[str]]:
    actions: list[dict[str, Any]] = []
    missing: list[str] = []
    for secret_name, sources in REQUIRED_SECRET_SOURCES.items():
        value, source = first_value(values, sources)
        if not value or not source:
            if secret_name in existing_secret_names:
                actions.append(existing_action("secret", secret_name, required=True))
                continue
            missing.append(secret_name)
            continue
        actions.append(
            {
                "kind": "secret",
                "name": secret_name,
                "source": source,
                "value": value,
                "required": True,
            }
        )
    for secret_name, sources in OPTIONAL_SECRET_SOURCES.items():
        value, source = first_value(values, sources)
        if value and source:
            actions.append(
                {
                    "kind": "secret",
                    "name": secret_name,
                    "source": source,
                    "value": value,
                    "required": False,
                }
            )
        elif secret_name in existing_secret_names:
            actions.append(existing_action("secret", secret_name, required=False))
    return actions, missing


def variable_plan(
    values: dict[str, str], *, existing_variable_names: set[str]
) -> tuple[list[dict[str, Any]], list[str]]:
    actions: list[dict[str, Any]] = []
    missing: list[str] = []
    for variable_name, sources in REQUIRED_VARIABLE_SOURCES.items():
        value, source = first_value(values, sources)
        if not value or not source:
            if variable_name in existing_variable_names:
                actions.append(existing_action("variable", variable_name, required=True))
                continue
            missing.append(variable_name)
            continue
        actions.append(
            {
                "kind": "variable",
                "name": variable_name,
                "source": source,
                "value": value,
                "required": True,
            }
        )
    for variable_name, config in OPTIONAL_VARIABLE_DEFAULTS.items():
        sources = config["sources"]
        if not isinstance(sources, list):
            continue
        value, source = first_value(values, [str(item) for item in sources])
        if not value and variable_name in existing_variable_names:
            actions.append(existing_action("variable", variable_name, required=False))
            continue
        if not value:
            default = config["default"]
            value = str(default)
            source = "default"
        actions.append(
            {
                "kind": "variable",
                "name": variable_name,
                "source": source,
                "value": value,
                "required": False,
            }
        )
    return actions, missing


def redacted_action(action: dict[str, Any], *, applied: bool) -> dict[str, Any]:
    value = action.get("value")
    value_present = isinstance(value, str) and bool(value)
    return {
        "kind": action["kind"],
        "name": action["name"],
        "source": action["source"],
        "required": action["required"],
        "value_present": value_present,
        "value_len": len(value) if isinstance(value, str) else 0,
        "existing": action.get("existing") is True,
        "applied": applied,
    }


def gh_set(action: dict[str, Any], *, repo: str) -> None:
    kind = str(action["kind"])
    name = str(action["name"])
    value = action.get("value")
    if not isinstance(value, str) or not value:
        return
    if kind == "secret":
        command = ["gh", "secret", "set", name, "--repo", repo, "--body", value]
    elif kind == "variable":
        command = ["gh", "variable", "set", name, "--repo", repo, "--body", value]
    else:
        raise ValueError(kind)
    subprocess.run(command, capture_output=True, text=True, check=True)


def build_report(
    *,
    repo: str,
    env_file: Path,
    values: dict[str, str],
    existing_secret_names: set[str],
    existing_variable_names: set[str],
    apply: bool,
) -> tuple[int, dict[str, Any]]:
    secret_actions, missing_secrets = secret_plan(
        values, existing_secret_names=existing_secret_names
    )
    variable_actions, missing_variables = variable_plan(
        values, existing_variable_names=existing_variable_names
    )
    errors: list[str] = []
    if missing_secrets:
        errors.append("missing required secret values: " + ", ".join(missing_secrets))
    if missing_variables:
        errors.append("missing required variable values: " + ", ".join(missing_variables))
    status = "ready" if not errors else "failed"
    applied_actions: list[dict[str, Any]] = []
    if status == "ready" and apply:
        for action in [*secret_actions, *variable_actions]:
            gh_set(action, repo=repo)
            applied_actions.append(redacted_action(action, applied=not action.get("existing")))
        status = "applied"
    else:
        applied_actions = [
            redacted_action(action, applied=False)
            for action in [*secret_actions, *variable_actions]
        ]
    report = {
        "status": status,
        "generated_at_unix": int(time.time()),
        "repo": repo,
        "env_file": str(env_file),
        "apply": apply,
        "existing_required_secrets": sorted(
            set(REQUIRED_SECRET_SOURCES).intersection(existing_secret_names)
        ),
        "existing_required_variables": sorted(
            set(REQUIRED_VARIABLE_SOURCES).intersection(existing_variable_names)
        ),
        "actions": applied_actions,
        "missing_required_secrets": missing_secrets,
        "missing_required_variables": missing_variables,
        "errors": errors,
        "next_preflight": f"scripts/hermes-github-ci-preflight.py --repo {repo}",
        "next_publish_gate": f"scripts/hermes-github-publish-gate.py --repo {repo}",
    }
    return (0 if status in {"ready", "applied"} else 2), report


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", default=DEFAULT_REPO)
    parser.add_argument("--env-file", default=DEFAULT_ENV_FILE)
    parser.add_argument("--report", default="target/hermes-github-secrets-setup.json")
    parser.add_argument(
        "--aws-profile",
        default=os.environ.get("AWS_PROFILE", "default"),
        help="AWS shared credentials/config profile to read before .env and process env.",
    )
    parser.add_argument(
        "--aws-credentials-file",
        default=str(Path.home() / ".aws" / "credentials"),
        help="AWS shared credentials file used as a low-precedence input source.",
    )
    parser.add_argument(
        "--aws-config-file",
        default=str(Path.home() / ".aws" / "config"),
        help="AWS shared config file used as a low-precedence input source.",
    )
    parser.add_argument(
        "--no-aws-shared-config",
        action="store_true",
        help="Do not read AWS shared credentials/config files.",
    )
    parser.add_argument(
        "--offline",
        action="store_true",
        help="Do not inspect existing GitHub names before planning.",
    )
    parser.add_argument(
        "--apply",
        action="store_true",
        help="Actually write GitHub secrets/variables. Omit for redacted dry run.",
    )
    args = parser.parse_args()

    env_file = Path(args.env_file)
    existing_secret_names: set[str] = set()
    existing_variable_names: set[str] = set()
    if not args.offline:
        existing_secret_names = gh_names(args.repo, "secret")
        existing_variable_names = gh_names(args.repo, "variable")
    aws_shared_values: dict[str, str] = {}
    if not args.no_aws_shared_config:
        aws_shared_values = parse_aws_shared_config(
            credentials_file=Path(args.aws_credentials_file),
            config_file=Path(args.aws_config_file),
            profile=args.aws_profile,
        )
    status, report = build_report(
        repo=args.repo,
        env_file=env_file,
        values=merged_env(
            env_file,
            dict(os.environ),
            aws_shared_values=aws_shared_values,
        ),
        existing_secret_names=existing_secret_names,
        existing_variable_names=existing_variable_names,
        apply=args.apply,
    )
    report["aws_shared_config"] = {
        "enabled": not args.no_aws_shared_config,
        "profile": args.aws_profile,
        "credentials_file_present": Path(args.aws_credentials_file).exists(),
        "config_file_present": Path(args.aws_config_file).exists(),
        "value_names_loaded": sorted(aws_shared_values),
    }
    report_path = Path(args.report)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(report, indent=2) + "\n"
    report_path.write_text(text, encoding="utf-8")
    print(text, end="")
    return status


if __name__ == "__main__":
    sys.exit(main())

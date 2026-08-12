#!/usr/bin/env python3
"""Validate the values-free NixOS secret contract.

This guard checks contract shape and metadata only. It intentionally reports
field names, logical secret names, and paths, but never prints unknown field
values from the evaluated JSON.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_HOSTS = ("finite-lat-1", "finite-lat-3")
KINDS = {"env", "opaque"}
SOPS_FORMATS = {"binary", "dotenv", "ini", "json", "yaml"}
MODE = re.compile(r"^[0-7]{4}$")
ENV_NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
ABSOLUTE_PATH = re.compile(r"^/")
ALLOWED_FIELDS = {
    "consumers",
    "destinationPath",
    "group",
    "kind",
    "mode",
    "owner",
    "path",
    "reloadUnits",
    "requiredEnvNames",
    "restartUnits",
    "scope",
    "sopsFile",
    "sopsFormat",
    "sopsKey",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--host",
        action="append",
        choices=DEFAULT_HOSTS,
        help="NixOS host config to evaluate. Defaults to both production hosts.",
    )
    parser.add_argument(
        "--contract-json",
        type=Path,
        help="Read one already-evaluated finite.secrets.files JSON object instead of running nix eval.",
    )
    return parser.parse_args()


def nix_eval(host: str) -> dict[str, Any]:
    command = [
        "nix",
        "eval",
        "--json",
        f".#nixosConfigurations.{host}.config.finite.secrets.files",
    ]
    result = subprocess.run(
        command,
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    loaded = json.loads(result.stdout)
    if not isinstance(loaded, dict):
        raise ValueError(f"{host}: finite.secrets.files is not an object")
    return loaded


def load_contract(args: argparse.Namespace) -> dict[str, dict[str, Any]]:
    if args.contract_json is not None:
        loaded = json.loads(args.contract_json.read_text(encoding="utf-8"))
        if not isinstance(loaded, dict):
            raise ValueError("contract JSON is not an object")
        return {"synthetic": loaded}

    hosts = args.host or list(DEFAULT_HOSTS)
    return {host: nix_eval(host) for host in hosts}


def require_string(
    failures: list[str], host: str, name: str, item: dict[str, Any], field: str
) -> str | None:
    value = item.get(field)
    if not isinstance(value, str) or value == "":
        failures.append(f"{host}:{name}: {field} must be a non-empty string")
        return None
    return value


def require_string_list(
    failures: list[str], host: str, name: str, item: dict[str, Any], field: str
) -> list[str]:
    value = item.get(field)
    if not isinstance(value, list) or not all(
        isinstance(entry, str) for entry in value
    ):
        failures.append(f"{host}:{name}: {field} must be a list of strings")
        return []
    return value


def check_contract(host: str, contract: dict[str, Any]) -> list[str]:
    failures: list[str] = []
    for name, raw_item in sorted(contract.items()):
        if not isinstance(name, str) or name == "":
            failures.append(f"{host}: contract contains an empty or non-string name")
            continue
        if not isinstance(raw_item, dict):
            failures.append(f"{host}:{name}: entry must be an object")
            continue

        item = raw_item
        unknown = sorted(set(item) - ALLOWED_FIELDS)
        if unknown:
            failures.append(f"{host}:{name}: unknown field(s): {', '.join(unknown)}")

        kind = require_string(failures, host, name, item, "kind")
        if kind is not None and kind not in KINDS:
            failures.append(f"{host}:{name}: kind must be one of {sorted(KINDS)}")

        mode = require_string(failures, host, name, item, "mode")
        if mode is not None and MODE.fullmatch(mode) is None:
            failures.append(f"{host}:{name}: mode must be four octal digits")

        for field in ("owner", "group", "path", "destinationPath", "sopsFile"):
            require_string(failures, host, name, item, field)

        for field in ("path", "destinationPath", "sopsFile"):
            value = item.get(field)
            if isinstance(value, str) and ABSOLUTE_PATH.match(value) is None:
                failures.append(f"{host}:{name}: {field} must be absolute")

        for field in (
            "consumers",
            "reloadUnits",
            "requiredEnvNames",
            "restartUnits",
            "scope",
        ):
            require_string_list(failures, host, name, item, field)

        if kind == "env":
            for env_name in item.get("requiredEnvNames", []):
                if not ENV_NAME.fullmatch(env_name):
                    failures.append(
                        f"{host}:{name}: invalid required env name {env_name!r}"
                    )

        if item.get("path") != item.get("destinationPath"):
            failures.append(f"{host}:{name}: path must resolve to destinationPath")

        sops_format = require_string(failures, host, name, item, "sopsFormat")
        if sops_format is not None and sops_format not in SOPS_FORMATS:
            failures.append(
                f"{host}:{name}: sopsFormat must be one of {sorted(SOPS_FORMATS)}"
            )

        if item.get("sopsKey") is not None and not isinstance(
            item.get("sopsKey"), str
        ):
            failures.append(f"{host}:{name}: sopsKey must be null or a string")

    return failures


def main() -> int:
    args = parse_args()
    try:
        contracts = load_contract(args)
    except (
        OSError,
        ValueError,
        subprocess.CalledProcessError,
        json.JSONDecodeError,
    ) as error:
        print(f"failed to load secret contract: {error}", file=sys.stderr)
        return 2

    failures: list[str] = []
    for host, contract in contracts.items():
        failures.extend(check_contract(host, contract))

    if failures:
        for failure in failures:
            print(failure, file=sys.stderr)
        return 1

    checked = ", ".join(contracts)
    print(f"NixOS secret contract ok for {checked} (values-free; no values emitted)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

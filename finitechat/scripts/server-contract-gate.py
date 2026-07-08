#!/usr/bin/env python3
"""Verify a Finite Chat server matches this checkout's protocol contract."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SERVER_URL = "https://chat.finite.computer"
CONTRACT_SOURCE = REPO_ROOT / "crates/finitechat-http/src/lib.rs"


class GateFailure(RuntimeError):
    pass


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", default=DEFAULT_SERVER_URL)
    parser.add_argument(
        "--expected-source",
        default="",
        help="Expected source_commit. Defaults to this checkout's short HEAD.",
    )
    parser.add_argument(
        "--expected-contract",
        type=int,
        default=0,
        help="Expected server_contract_version. Defaults to this checkout's constant.",
    )
    parser.add_argument(
        "--allow-dirty",
        action="store_true",
        help="Allow source_dirty=true. Intended only for local branch servers.",
    )
    return parser.parse_args()


def checkout_contract_version() -> int:
    text = CONTRACT_SOURCE.read_text(encoding="utf-8")
    match = re.search(r"FINITECHAT_SERVER_CONTRACT_VERSION:\s*u32\s*=\s*(\d+)", text)
    if not match:
        raise GateFailure(f"could not find server contract version in {CONTRACT_SOURCE}")
    return int(match.group(1))


def checkout_short_head() -> str:
    proc = subprocess.run(
        ["git", "rev-parse", "--short=12", "HEAD"],
        cwd=REPO_ROOT,
        text=True,
        capture_output=True,
        check=True,
    )
    return proc.stdout.strip()


def read_health(server_url: str) -> dict[str, Any]:
    url = f"{server_url.rstrip('/')}/health"
    try:
        with urllib.request.urlopen(url, timeout=10) as response:
            payload = response.read()
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        raise GateFailure(f"{url} returned HTTP {exc.code}: {body}") from exc
    except urllib.error.URLError as exc:
        raise GateFailure(f"{url} is unreachable: {exc}") from exc
    try:
        value = json.loads(payload)
    except json.JSONDecodeError as exc:
        raise GateFailure(f"{url} did not return JSON: {payload[:200]!r}") from exc
    if not isinstance(value, dict):
        raise GateFailure(f"{url} returned non-object JSON")
    return value


def main() -> int:
    args = parse_args()
    expected_contract = args.expected_contract or checkout_contract_version()
    expected_source = args.expected_source or checkout_short_head()
    health = read_health(args.server)

    failures: list[str] = []
    if health.get("status") != "ok":
        failures.append(f"status is {health.get('status')!r}, expected 'ok'")
    if health.get("server_contract_version") != expected_contract:
        failures.append(
            "server_contract_version is "
            f"{health.get('server_contract_version')!r}, expected {expected_contract}"
        )
    if health.get("source_commit") != expected_source:
        failures.append(f"source_commit is {health.get('source_commit')!r}, expected {expected_source!r}")
    if health.get("source_dirty") is True and not args.allow_dirty:
        failures.append("source_dirty is true")
    if "source_dirty" not in health and not args.allow_dirty:
        failures.append("source_dirty is missing")

    report = {
        "status": "passed" if not failures else "failed",
        "server": args.server,
        "expected_contract": expected_contract,
        "expected_source": expected_source,
        "health": health,
        "failures": failures,
    }
    print(json.dumps(report, indent=2, sort_keys=True))
    if failures:
        return 1
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except GateFailure as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1) from error

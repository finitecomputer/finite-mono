#!/usr/bin/env python3
"""Verify a Finite Chat server matches the selected build contract."""

from __future__ import annotations

import argparse
import json
import re
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
    identity = parser.add_mutually_exclusive_group()
    identity.add_argument(
        "--expected-fingerprint",
        default="",
        help="Expected Nix-scoped source_fingerprint for production builds.",
    )
    identity.add_argument(
        "--expected-source",
        default="",
        help="Expected legacy source_commit for non-Nix builds.",
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


def health_failures(
    health: dict[str, Any],
    *,
    expected_contract: int,
    expected_fingerprint: str = "",
    expected_source: str = "",
    allow_dirty: bool = False,
) -> list[str]:
    failures: list[str] = []
    if health.get("status") != "ok":
        failures.append(f"status is {health.get('status')!r}, expected 'ok'")
    if health.get("server_contract_version") != expected_contract:
        failures.append(
            "server_contract_version is "
            f"{health.get('server_contract_version')!r}, expected {expected_contract}"
        )
    if expected_fingerprint and health.get("source_fingerprint") != expected_fingerprint:
        failures.append(
            "source_fingerprint is "
            f"{health.get('source_fingerprint')!r}, expected {expected_fingerprint!r}"
        )
    if expected_source and health.get("source_commit") != expected_source:
        failures.append(
            f"source_commit is {health.get('source_commit')!r}, expected {expected_source!r}"
        )
    if health.get("source_dirty") is True and not allow_dirty:
        failures.append("source_dirty is true")
    if "source_dirty" not in health and not allow_dirty:
        failures.append("source_dirty is missing")
    return failures


def main() -> int:
    args = parse_args()
    expected_contract = args.expected_contract or checkout_contract_version()
    if not args.expected_fingerprint and not args.expected_source:
        raise GateFailure(
            "pass --expected-fingerprint for a Nix deployment or "
            "--expected-source for a legacy build"
        )
    health = read_health(args.server)

    failures = health_failures(
        health,
        expected_contract=expected_contract,
        expected_fingerprint=args.expected_fingerprint,
        expected_source=args.expected_source,
        allow_dirty=args.allow_dirty,
    )

    report = {
        "status": "passed" if not failures else "failed",
        "server": args.server,
        "expected_contract": expected_contract,
        "expected_fingerprint": args.expected_fingerprint or None,
        "expected_source": args.expected_source or None,
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

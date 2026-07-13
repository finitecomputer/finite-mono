#!/usr/bin/env python3
"""Fail when Dashboard and Core are configured with different Stripe Prices."""

from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parents[1]
CONFIGS = (
    ROOT / "infra/nixos/modules/dashboard.nix",
    ROOT / "infra/nixos/modules/finite-saas-core.nix",
)
PRICE_PATTERN = re.compile(
    r'STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID\s*=\s*"(price_[A-Za-z0-9]+)";'
)


def main() -> int:
    configured: dict[Path, str] = {}
    for path in CONFIGS:
        matches = PRICE_PATTERN.findall(path.read_text(encoding="utf-8"))
        if len(matches) != 1:
            print(
                f"{path.relative_to(ROOT)} must define exactly one standard Stripe Price id",
                file=sys.stderr,
            )
            return 1
        configured[path] = matches[0]

    prices = set(configured.values())
    if len(prices) != 1:
        for path, price in configured.items():
            print(f"{path.relative_to(ROOT)}: {price}", file=sys.stderr)
        print("Dashboard and Core Stripe Price ids must match", file=sys.stderr)
        return 1

    print(f"stripe_price_contract=ok price_id={prices.pop()}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

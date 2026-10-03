#!/usr/bin/env python3
"""Cache aggregate Core billing state for the existing node textfile collector."""

from __future__ import annotations

import json
from pathlib import Path
import sys

from scripts import finite_status
from finite_runtime_metrics import write_atomic


STATUSES = (
    "subscribed",
    "trial",
    "sponsored",
    "grandfathered",
    "expired_past_due",
    "incomplete",
    "no_subscription",
    "missing_billing_account",
    "unknown",
)
QUERY = Path(__file__).with_suffix(".sql").read_text()
SUCCESS = "finite_billing_collection_success"


def collect(environment: dict[str, str]) -> dict:
    result = finite_status.run_read_only(
        ["psql", "-X", "-qAt", "-v", "ON_ERROR_STOP=1"],
        environment={**environment, "PGCONNECT_TIMEOUT": "5"},
        input_text=QUERY,
        timeout=15,
    )
    if result.returncode:
        # Database diagnostics can contain row values or connection details.
        raise ValueError("billing aggregate query failed")
    return json.loads(result.stdout)


def render(snapshot: dict) -> str:
    counts = snapshot["counts"]
    timestamp = snapshot["collected_at"]
    if (
        not isinstance(counts, dict)
        or set(counts) - set(STATUSES)
        or any(type(value) is not int or value < 0 for value in counts.values())
        or type(timestamp) is not int
        or timestamp <= 0
    ):
        raise ValueError("invalid billing aggregate")
    return (
        "\n".join(
            [
                f"# HELP {SUCCESS} Whether the last billing collection succeeded.",
                f"# TYPE {SUCCESS} gauge",
                f"{SUCCESS} 1",
                "# HELP finite_billing_collected_at_seconds Core snapshot time in Unix seconds.",
                "# TYPE finite_billing_collected_at_seconds gauge",
                f"finite_billing_collected_at_seconds {timestamp}",
                "# HELP finite_billing_accounts Customer organizations by exclusive billing state; not users or cash payments.",
                "# TYPE finite_billing_accounts gauge",
                *[
                    f'finite_billing_accounts{{status="{status}"}} {counts.get(status, 0)}'
                    for status in STATUSES
                ],
            ]
        )
        + "\n"
    )


def publish(output: Path) -> int:
    try:
        contents = render(collect(finite_status.postgres_environment()))
    except (finite_status.CollectionError, ValueError, KeyError, TypeError):
        # Replace old counts on failure. If even writing fails, the dashboard's
        # timestamp gate still expires the previous file after ten minutes.
        write_atomic(output, f"# TYPE {SUCCESS} gauge\n{SUCCESS} 0\n")
        print("Billing collection failed; counts unavailable.", file=sys.stderr)
        return 1
    write_atomic(output, contents)
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(sys.argv[0]).name} OUTPUT")
    raise SystemExit(publish(Path(sys.argv[1])))

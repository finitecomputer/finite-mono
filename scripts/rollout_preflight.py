#!/usr/bin/env python3
"""Read-only rollout preflight census of saas-core runtime control requests.

This tool is STRICTLY READ ONLY. It issues SELECT statements only, wrapped in
a `BEGIN TRANSACTION READ ONLY; ... ROLLBACK;` so PostgreSQL itself rejects
any mutation. It never writes to the database, never acquires row locks, and
never consumes a DSN value other than to pass it to `psql`.

Context (deploy audit finding): saas-core migration 0021 fails Core STARTUP
closed if `runtime_control_requests` contains any status outside the legacy
four {requested, running, succeeded, failed}. The good news is that this is
safe fail-closed behavior; the bad news is discovering it mid-deploy. This
census reports that population before the deploy window. Because the audit
normally runs BEFORE 0021 applies, presence of newer vocabulary means 0021
has already been (partially) applied; below, the report states which scenario
applies.

It also counts non-terminal (`requested`/`running`) `upgrade`-kind requests —
the population that makes
`finitecomputer-v2/crates/finite-saas-core/migrations/runtime_upgrade_rollback_rescue.sql`
refuse to run — and inventories long-lived `running` rows with age for
rollout planning (they re-label to `launching` after 0021 deploys).

The Postgres DSN is read from `FC_CORE_DATABASE_URL`, the same environment
variable saas-core itself requires
(`postgres_store_from_env` in finite-saas-core/src/main.rs).

Exit codes: 0 clean, 1 violations (unknown statuses present), 2 operational
error (missing DSN, psql unavailable, database unreachable, unexpected result
shape).
"""

from __future__ import annotations

import argparse
import csv
import io
import json
import os
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timezone


SCHEMA = "finite.rollout-preflight-census.v1"
DSN_ENV_VAR = "FC_CORE_DATABASE_URL"
LEGACY_STATUSES = ("requested", "running", "succeeded", "failed")
NON_TERMINAL_STATUSES = ("requested", "running")
UPGRADE_KIND = "upgrade"
DEFAULT_RUNNING_AGE_THRESHOLD_HOURS = 24.0
# Parity with saas-core's optional_duration_secs default for
# FC_CORE_POSTGRES_CONNECT_TIMEOUT_SECS.
PSQL_TIMEOUT_SECS = 60
PGAPPNAME = "finite-rollout-preflight"

STATUS_DISTRIBUTION_SQL = """
SELECT kind, status, count(*)
FROM runtime_control_requests
GROUP BY kind, status
ORDER BY kind, status;
"""

_ACTIVE_STATUS_SQL = ", ".join(f"'{status}'" for status in NON_TERMINAL_STATUSES)
ACTIVE_UPGRADE_IDS_SQL = f"""
SELECT id
FROM runtime_control_requests
WHERE kind = '{UPGRADE_KIND}'
  AND status IN ({_ACTIVE_STATUS_SQL})
ORDER BY id;
"""

RUNNING_ROWS_SQL = """
SELECT id,
       kind,
       agent_runtime_id,
       source_host_id,
       to_char(
           updated_at AT TIME ZONE 'UTC',
           'YYYY-MM-DD"T"HH24:MI:SS"Z"'
       ) AS updated_at
FROM runtime_control_requests
WHERE status = 'running'
ORDER BY updated_at, id;
"""

EXPECTED_HEADERS = {
    "status_distribution": ["kind", "status", "count"],
    "active_upgrades": ["id"],
    "running_rows": [
        "id",
        "kind",
        "agent_runtime_id",
        "source_host_id",
        "updated_at",
    ],
}


class CensusError(ValueError):
    """Operational failure (bad DSN, unreachable database, bad result shape)."""


def utc_now() -> datetime:
    return datetime.now(timezone.utc)


def iso_z(moment: datetime) -> str:
    return moment.isoformat(timespec="seconds").replace("+00:00", "Z")


def format_age(seconds: float) -> str:
    whole = max(int(seconds), 0)
    days, remainder = divmod(whole, 86400)
    hours, remainder = divmod(remainder, 3600)
    minutes = remainder // 60
    if days > 0:
        return f"{days}d{hours}h"
    if hours > 0:
        return f"{hours}h{minutes}m"
    return f"{minutes}m"


@dataclass(frozen=True)
class StatusCell:
    kind: str
    status: str
    count: int


@dataclass(frozen=True)
class RunningRequest:
    id: str
    kind: str
    agent_runtime_id: str
    source_host_id: str
    updated_at: str
    age_seconds: float


def read_only_script(statement: str) -> str:
    """Wrap one SELECT in an explicitly read-only transaction."""
    body = statement.strip()
    if not body.endswith(";"):
        body += ";"
    return f"BEGIN TRANSACTION READ ONLY;\n{body}\nROLLBACK;\n"


def psql_query(dsn: str, statement: str) -> list[list[str]]:
    """Run one SELECT through psql inside a READ ONLY transaction.

    The DSN value is never echoed into errors; psql's stderr tail is enough
    for an operator who already knows what they pointed the tool at.
    """
    command = [
        "psql",
        "-X",
        "--quiet",
        "--csv",
        "--set",
        "ON_ERROR_STOP=1",
        "--dbname",
        dsn,
    ]
    env = dict(os.environ)
    env["PGAPPNAME"] = PGAPPNAME
    try:
        result = subprocess.run(
            command,
            input=read_only_script(statement),
            text=True,
            capture_output=True,
            timeout=PSQL_TIMEOUT_SECS,
            check=False,
            env=env,
        )
    except FileNotFoundError as error:
        raise CensusError("psql binary not found on PATH") from error
    except subprocess.TimeoutExpired as error:
        raise CensusError(
            f"database query did not finish within {PSQL_TIMEOUT_SECS}s"
        ) from error
    if result.returncode != 0:
        detail = result.stderr.strip().splitlines()[-1] if result.stderr.strip() else ""
        suffix = f": {detail}" if detail else ""
        raise CensusError(f"psql exited {result.returncode}{suffix}")
    reader = csv.reader(io.StringIO(result.stdout))
    rows = [[cell for cell in row] for row in reader]
    if not rows:
        raise CensusError("psql returned no CSV header row")
    return rows


def checked_header(rows: list[list[str]], name: str) -> list[list[str]]:
    if len(rows[0]) != len(EXPECTED_HEADERS[name]) or [
        cell.lower() for cell in rows[0]
    ] != EXPECTED_HEADERS[name]:
        raise CensusError(
            f"{name} query returned unexpected columns: {rows[0]}"
        )
    return rows[1:]


def parse_status_distribution(
    rows: list[list[str]],
) -> list[StatusCell]:
    cells = []
    for line in checked_header(rows, "status_distribution"):
        kind, status, raw_count = line
        try:
            count = int(raw_count)
        except ValueError as error:
            raise CensusError(f"non-integer count in status census: {line}") from error
        cells.append(StatusCell(kind=kind, status=status, count=count))
    return cells


def parse_active_upgrade_ids(rows: list[list[str]]) -> list[str]:
    return [line[0] for line in checked_header(rows, "active_upgrades")]


def parse_running_rows(rows: list[list[str]], now: datetime) -> list[RunningRequest]:
    parsed = []
    for line in checked_header(rows, "running_rows"):
        request_id, kind, agent_runtime_id, source_host_id, raw_updated_at = line
        try:
            updated_at = datetime.fromisoformat(raw_updated_at.replace("Z", "+00:00"))
        except ValueError as error:
            raise CensusError(
                f"unparsable updated_at timestamp for {request_id}: {raw_updated_at}"
            ) from error
        parsed.append(
            RunningRequest(
                id=request_id,
                kind=kind,
                agent_runtime_id=agent_runtime_id,
                source_host_id=source_host_id,
                updated_at=iso_z(updated_at),
                age_seconds=max((now - updated_at).total_seconds(), 0.0),
            )
        )
    return parsed


def query_census(dsn: str) -> tuple[list[list[str]], list[list[str]], list[list[str]]]:
    """Single seam between the pure census logic and the live database."""
    return (
        psql_query(dsn, STATUS_DISTRIBUTION_SQL),
        psql_query(dsn, ACTIVE_UPGRADE_IDS_SQL),
        psql_query(dsn, RUNNING_ROWS_SQL),
    )


def build_report(
    status_rows: list[list[str]],
    upgrade_rows: list[list[str]],
    running_rows_raw: list[list[str]],
    *,
    now: datetime,
    generated_at: str,
    threshold_hours: float = DEFAULT_RUNNING_AGE_THRESHOLD_HOURS,
) -> dict:
    cells = parse_status_distribution(status_rows)
    active_upgrade_ids = parse_active_upgrade_ids(upgrade_rows)
    running_requests = parse_running_rows(running_rows_raw, now)

    known = set(LEGACY_STATUSES)
    unknown = sorted({cell.status for cell in cells} - known)
    violations = []
    if unknown:
        violations.append(
            "statuses outside the legacy four "
            f"{list(LEGACY_STATUSES)}: {unknown}; migration 0021 aborts apply "
            "closed when runtime_control_requests holds such statuses"
        )

    scenario = (
        "pre-migration-clean: only legacy statuses are present, consistent "
        "with an audit that runs before 0021 apply"
        if not unknown
        else (
            "0021-already-(partially)-applied-or-unexpected-writer: this audit "
            "normally runs before 0021, and every pre-0021 writer pins rows to "
            "the legacy four under the table CHECK constraint, so statuses "
            f"{unknown} mean 0021 has already started applying here"
        )
    )

    threshold_seconds = threshold_hours * 3600
    long_lived = [
        row for row in running_requests if row.age_seconds >= threshold_seconds
    ]

    return {
        "schema": SCHEMA,
        "generated_at": generated_at,
        "dsn_env_var": DSN_ENV_VAR,
        "verdict": "fail" if violations else "pass",
        "scenario": scenario,
        "legacy_statuses": list(LEGACY_STATUSES),
        "status_distribution": [
            {"kind": cell.kind, "status": cell.status, "count": cell.count}
            for cell in cells
        ],
        "unknown_statuses": unknown,
        "violations": violations,
        "active_upgrade_requests": {
            "count": len(active_upgrade_ids),
            "ids": active_upgrade_ids,
            "note": (
                "rollback-rescue refuses to run while upgrade-kind requests "
                "are non-terminal"
            ),
        },
        "running_inventory": {
            "threshold_hours": threshold_hours,
            "total_running": len(running_requests),
            "long_lived_count": len(long_lived),
            "long_lived": [
                {
                    "id": row.id,
                    "kind": row.kind,
                    "agent_runtime_id": row.agent_runtime_id,
                    "source_host_id": row.source_host_id,
                    "updated_at": row.updated_at,
                    "age_seconds": round(row.age_seconds, 3),
                }
                for row in long_lived
            ],
            "note": "these re-label to launching when 0021 deploys",
        },
    }


def render_report(report: dict) -> str:
    distribution_lines = [
        f"- `{entry['kind']}/{entry['status']}`: {entry['count']}"
        for entry in report["status_distribution"]
    ] or ["- (table empty)"]
    unknown_lines = [
        f"- `{status}`" for status in report["unknown_statuses"]
    ] or ["- None"]
    upgrades = report["active_upgrade_requests"]
    upgrade_lines = [
        f"- `{request_id}`" for request_id in upgrades["ids"]
    ] or ["- None"]
    running = report["running_inventory"]
    long_lived_lines = [
        f"- `{entry['id']}` kind=`{entry['kind']}` "
        f"runtime=`{entry['agent_runtime_id']}` host=`{entry['source_host_id']}` "
        f"updated_at=`{entry['updated_at']}` "
        f"age={format_age(entry['age_seconds'])}"
        for entry in running["long_lived"]
    ] or ["- None"]

    return "\n".join(
        [
            "## Rollout Preflight: Runtime Control Request Census",
            "",
            f"- Schema: `{report['schema']}`",
            f"- Generated at: `{report['generated_at']}`",
            f"- DSN source: env `{report['dsn_env_var']}` (read-only access)",
            f"- Verdict: {report['verdict'].upper()}",
            f"- Scenario: {report['scenario']}.",
            "",
            "### Status distribution (kind/status)",
            *distribution_lines,
            "",
            f"### Statuses outside legacy four {report['legacy_statuses']}",
            *unknown_lines,
            "",
            "### Active upgrade-kind requests (WARN: rollback-rescue blocker)",
            *upgrade_lines,
            "- The rollback-rescue script refuses to run while these are "
            f"non-terminal ({upgrades['count']} found).",
            "",
            (
                "### Long-lived `running` inventory "
                f"(updated_at older than {running['threshold_hours']:g}h; "
                f"{running['total_running']} running total)"
            ),
            *long_lived_lines,
            f"- Note: {running['note']}.",
            "",
        ]
    )


def command_preflight(args: argparse.Namespace) -> int:
    dsn = os.environ.get(DSN_ENV_VAR, "")
    if not dsn:
        raise CensusError(f"environment variable {DSN_ENV_VAR} is not set")

    status_rows, upgrade_rows, running_rows_raw = query_census(dsn)
    now = utc_now()
    report = build_report(
        status_rows,
        upgrade_rows,
        running_rows_raw,
        now=now,
        generated_at=iso_z(now),
        threshold_hours=args.running_age_threshold_hours,
    )
    payload = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.json:
        print(payload, end="")
    else:
        sys.stdout.write(render_report(report))
    return 1 if report["verdict"] == "fail" else 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--json",
        action="store_true",
        help="emit the machine-readable census instead of the human summary",
    )
    parser.add_argument(
        "--running-age-threshold-hours",
        type=float,
        default=DEFAULT_RUNNING_AGE_THRESHOLD_HOURS,
        metavar="HOURS",
        help=(
            "`running` rows whose updated_at is at least this old are listed "
            f"in the long-lived inventory (default: "
            f"{DEFAULT_RUNNING_AGE_THRESHOLD_HOURS:g})"
        ),
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        return command_preflight(args)
    except CensusError as error:
        print(f"rollout preflight error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())

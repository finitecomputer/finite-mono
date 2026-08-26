from __future__ import annotations

import json
import os
import unittest
from contextlib import redirect_stderr, redirect_stdout
from datetime import datetime, timedelta, timezone
from io import StringIO
from unittest import mock

from scripts import rollout_preflight


FAKE_NOW = datetime(2026, 8, 26, 12, 0, 0, tzinfo=timezone.utc)
FAKE_DSN = "postgresql://preflight:unused@127.0.0.1/fake-core"


def census_rows(cells: list[tuple[str, str, int]]) -> list[list[str]]:
    return [["kind", "status", "count"]] + [
        [kind, status, str(count)] for kind, status, count in cells
    ]


def upgrade_rows(ids: list[str]) -> list[list[str]]:
    return [["id"]] + [[request_id] for request_id in ids]


def running_rows(
    entries: list[tuple[str, str, str, str, datetime]],
) -> list[list[str]]:
    return [
        ["id", "kind", "agent_runtime_id", "source_host_id", "updated_at"]
    ] + [
        [
            request_id,
            kind,
            agent_runtime_id,
            source_host_id,
            updated_at.isoformat(timespec="seconds").replace("+00:00", "Z"),
        ]
        for request_id, kind, agent_runtime_id, source_host_id, updated_at in entries
    ]


LEGACY_CLEAN_DISTRIBUTION = [
    ("restart", "succeeded", 40),
    ("restart", "failed", 2),
    ("stop", "requested", 1),
    ("destroy", "running", 1),
    ("recover_known_good_chat_runtime", "succeeded", 7),
]


class RolloutPreflightTests(unittest.TestCase):
    def run_preflight(
        self,
        *,
        distribution: list[list[str]],
        upgrades: list[list[str]] | None = None,
        running: list[list[str]] | None = None,
        argv: list[str] | tuple[str, ...] = (),
    ) -> tuple[int, str, str]:
        captured_out, captured_err = StringIO(), StringIO()

        def frozen_clock() -> datetime:
            return FAKE_NOW

        with mock.patch.dict("os.environ", {"FC_CORE_DATABASE_URL": FAKE_DSN}):
            with mock.patch.object(rollout_preflight, "utc_now", frozen_clock):
                with mock.patch.object(
                    rollout_preflight,
                    "query_census",
                    return_value=(
                        distribution,
                        upgrades or upgrade_rows([]),
                        running or running_rows([]),
                    ),
                ):
                    with redirect_stdout(captured_out), redirect_stderr(captured_err):
                        exit_code = rollout_preflight.main(list(argv))
        return exit_code, captured_out.getvalue(), captured_err.getvalue()

    def test_clean_legacy_vocabulary_passes_with_exit_zero(self) -> None:
        exit_code, out, err = self.run_preflight(
            distribution=census_rows(LEGACY_CLEAN_DISTRIBUTION),
        )
        self.assertEqual(exit_code, 0)
        self.assertEqual(err, "")
        self.assertIn("Verdict: PASS", out)
        self.assertIn("- None", out)

    def test_unknown_status_fails_and_names_offending_statuses(self) -> None:
        rows = census_rows([("restart", "succeeded", 3)])
        rows.append(["restart", "launching", 5])
        rows.append(["stop", "queued", 1])
        exit_code, out, err = self.run_preflight(distribution=rows)
        self.assertEqual(exit_code, 1)
        self.assertIn("Verdict: FAIL", out)
        self.assertIn("`launching`", out)
        self.assertIn("`queued`", out)
        self.assertIn("0021", out)

    def test_active_upgrade_requests_warn_without_failing(self) -> None:
        exit_code, out, _ = self.run_preflight(
            distribution=census_rows(LEGACY_CLEAN_DISTRIBUTION),
            upgrades=upgrade_rows(["req-aaa", "req-bbb"]),
        )
        self.assertEqual(exit_code, 0)
        self.assertIn("WARN", out)
        self.assertIn("`req-aaa`", out)
        self.assertIn("`req-bbb`", out)
        self.assertIn("refuses to run while these are non-terminal (2 found)", out)

    def test_long_lived_running_inventory_lists_age_informationally(self) -> None:
        old = FAKE_NOW - timedelta(days=3, hours=4)
        fresh = FAKE_NOW - timedelta(minutes=10)
        exit_code, out, _ = self.run_preflight(
            distribution=census_rows(LEGACY_CLEAN_DISTRIBUTION),
            running=running_rows(
                [
                    (
                        "runtime-old",
                        "upgrade",
                        "runtime-1",
                        "finite-lat-1",
                        old,
                    ),
                    (
                        "runtime-fresh",
                        "restart",
                        "runtime-2",
                        "finite-lat-1",
                        fresh,
                    ),
                ]
            ),
        )
        self.assertEqual(exit_code, 0)
        self.assertIn("`runtime-old`", out)
        self.assertIn("age=3d4h", out)
        self.assertIn("launching when 0021 deploys", out)
        # Below-threshold rows stay out of the informational inventory.
        self.assertNotIn("`runtime-fresh`", out)
        self.assertIn("2 running total", out)

    def test_json_shape_is_stable_for_consumers(self) -> None:
        old = FAKE_NOW - timedelta(hours=30)
        exit_code, out, err = self.run_preflight(
            distribution=census_rows(LEGACY_CLEAN_DISTRIBUTION),
            upgrades=upgrade_rows(["req-aaa"]),
            running=running_rows(
                [("rt-1", "restart", "runtime-9", "host-a", old)]
            ),
            argv=("--json",),
        )
        payload = json.loads(out)
        self.assertEqual(exit_code, 0)
        self.assertEqual(payload["schema"], rollout_preflight.SCHEMA)
        self.assertEqual(
            set(payload),
            {
                "schema",
                "generated_at",
                "dsn_env_var",
                "verdict",
                "scenario",
                "legacy_statuses",
                "status_distribution",
                "unknown_statuses",
                "violations",
                "active_upgrade_requests",
                "running_inventory",
            },
        )
        self.assertEqual(
            set(payload["active_upgrade_requests"]),
            {"count", "ids", "note"},
        )
        self.assertEqual(
            set(payload["running_inventory"]),
            {
                "threshold_hours",
                "total_running",
                "long_lived_count",
                "long_lived",
                "note",
            },
        )
        long_entry = payload["running_inventory"]["long_lived"][0]
        self.assertEqual(
            set(long_entry),
            {
                "id",
                "kind",
                "agent_runtime_id",
                "source_host_id",
                "updated_at",
                "age_seconds",
            },
        )

    def test_missing_dsn_env_var_is_operational_error(self) -> None:
        captured_out, captured_err = StringIO(), StringIO()
        clean_env = {
            key: value
            for key, value in os.environ.items()
            if key != "FC_CORE_DATABASE_URL"
        }
        with mock.patch.dict("os.environ", clean_env, clear=True):
            with redirect_stdout(captured_out), redirect_stderr(captured_err):
                exit_code = rollout_preflight.main([])
        self.assertEqual(exit_code, 2)
        self.assertIn("FC_CORE_DATABASE_URL", captured_err.getvalue())
        self.assertEqual(captured_out.getvalue(), "")

    def test_psql_failure_surfaces_as_operational_error(self) -> None:
        captured_out, captured_err = StringIO(), StringIO()
        with mock.patch.dict("os.environ", {"FC_CORE_DATABASE_URL": FAKE_DSN}):
            with mock.patch.object(
                rollout_preflight,
                "psql_query",
                side_effect=rollout_preflight.CensusError("psql exited 2"),
            ):
                with redirect_stdout(captured_out), redirect_stderr(captured_err):
                    exit_code = rollout_preflight.main([])
        self.assertEqual(exit_code, 2)
        self.assertIn("rollout preflight error", captured_err.getvalue())

    def test_read_only_script_wraps_select_in_read_only_transaction(self) -> None:
        script = rollout_preflight.read_only_script(
            "SELECT 1 FROM runtime_control_requests;"
        )
        self.assertTrue(script.startswith("BEGIN TRANSACTION READ ONLY;\n"))
        self.assertTrue(script.rstrip().endswith("ROLLBACK;"))
        lower_body = script.lower()
        for forbidden in ("insert ", "update ", "delete ", "alter ", "drop "):
            self.assertNotIn(forbidden, lower_body)

    def test_sql_constants_are_select_only_against_expected_table(self) -> None:
        for statement in (
            rollout_preflight.STATUS_DISTRIBUTION_SQL,
            rollout_preflight.ACTIVE_UPGRADE_IDS_SQL,
            rollout_preflight.RUNNING_ROWS_SQL,
        ):
            normalized = " ".join(statement.split()).lower()
            self.assertTrue(normalized.startswith("select "))
            self.assertNotIn("; select", normalized)
            for forbidden in (
                "insert",
                "update ",
                "delete",
                "alter",
                "drop",
                "truncate",
            ):
                self.assertNotIn(forbidden, normalized)


if __name__ == "__main__":
    unittest.main()

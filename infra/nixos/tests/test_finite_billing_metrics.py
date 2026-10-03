"""Collector contract plus classification against an isolated real Postgres."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT), str(ROOT / "infra/nixos/scripts")]
import finite_billing_metrics as billing  # noqa: E402 — source-tree script import


class ExporterTests(unittest.TestCase):
    def test_empty_snapshot_has_all_nine_zeroes_and_only_status_labels(self):
        metrics = billing.render({"collected_at": 123, "counts": {}})
        samples = [
            line
            for line in metrics.splitlines()
            if line.startswith("finite_billing_accounts{")
        ]
        self.assertEqual(len(samples), 9)
        for status in billing.STATUSES:
            self.assertIn(f'finite_billing_accounts{{status="{status}"}} 0', samples)
        self.assertIn("finite_billing_collection_success 1", metrics)

    def test_unexpected_values_cannot_become_labels_or_counts(self):
        for counts in (
            {"user@example.com": 1},
            {"trial": -1},
            {"trial": True},
            {"trial": 1.5},
            [],
        ):
            with self.subTest(counts=counts), self.assertRaises(ValueError):
                billing.render({"collected_at": 123, "counts": counts})
        for timestamp in (None, 0, True, "123"):
            with self.assertRaises(ValueError):
                billing.render({"collected_at": timestamp, "counts": {}})

    def test_read_only_timeouts_and_no_credentials_in_argv(self):
        with patch.object(
            billing.finite_status,
            "run_read_only",
            return_value=SimpleNamespace(
                returncode=0, stdout='{"collected_at":123,"counts":{}}'
            ),
        ) as run:
            billing.collect({"PGPASSWORD": "synthetic-secret"})
        args, kwargs = run.call_args
        self.assertNotIn("synthetic-secret", repr(args))
        self.assertEqual(kwargs["timeout"], 15)
        self.assertEqual(kwargs["environment"]["PGCONNECT_TIMEOUT"], "5")
        self.assertIn("BEGIN TRANSACTION READ ONLY;", kwargs["input_text"])
        self.assertIn("statement_timeout = '5s'", kwargs["input_text"])
        self.assertIn("lock_timeout = '1s'", kwargs["input_text"])

    def test_success_then_failure_removes_counts_without_leaking_diagnostics(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "billing.prom"
            with (
                patch.object(
                    billing.finite_status, "postgres_environment", return_value={}
                ),
                patch.object(
                    billing,
                    "collect",
                    return_value={"collected_at": 123, "counts": {"subscribed": 3}},
                ),
            ):
                self.assertEqual(billing.publish(output), 0)
            self.assertEqual(output.stat().st_mode & 0o777, 0o640)
            self.assertIn('status="subscribed"} 3', output.read_text())
            with (
                patch.object(
                    billing.finite_status,
                    "postgres_environment",
                    side_effect=ValueError("secret"),
                ),
                patch("sys.stderr") as stderr,
            ):
                self.assertEqual(billing.publish(output), 1)
            self.assertNotIn("secret", str(stderr.mock_calls))
            self.assertEqual(
                output.read_text(),
                "# TYPE finite_billing_collection_success gauge\nfinite_billing_collection_success 0\n",
            )
            self.assertEqual(list(Path(directory).glob("*.tmp.*")), [])


class PostgresTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # No production URL/environment is accepted. Unique local socket only.
        cls.directory = tempfile.TemporaryDirectory(prefix="billing-pg-")
        cls.addClassCleanup(cls.directory.cleanup)
        root = Path(cls.directory.name)
        cls.env = {
            key: value for key, value in os.environ.items() if not key.startswith("PG")
        }
        cls.env.update(PGHOST=str(root), PGDATABASE="postgres", PGUSER="billing_test")
        subprocess.run(
            [
                "initdb",
                "-D",
                str(root / "data"),
                "-U",
                "billing_test",
                "--auth=trust",
                "--no-locale",
            ],
            env=cls.env,
            check=True,
            capture_output=True,
        )
        subprocess.run(
            [
                "pg_ctl",
                "-D",
                str(root / "data"),
                "-l",
                str(root / "log"),
                "-o",
                f"-k {root} -c listen_addresses=''",
                "-w",
                "start",
            ],
            env=cls.env,
            check=True,
            capture_output=True,
        )
        cls.addClassCleanup(
            subprocess.run,
            ["pg_ctl", "-D", str(root / "data"), "-m", "immediate", "-w", "stop"],
            env=cls.env,
            check=True,
            capture_output=True,
        )
        migrations = ROOT / "finitecomputer-v2/crates/finite-saas-core/migrations"
        # Apply the actual schema, including its partial trial uniqueness rule.
        for name in (
            "0001_core.sql",
            "0003_launch_codes.sql",
            "0035_trial_campaigns.sql",
        ):
            cls.sql((migrations / name).read_text())

    @classmethod
    def sql(cls, text):
        result = subprocess.run(
            ["psql", "-X", "-qAt", "-v", "ON_ERROR_STOP=1"],
            input=text,
            env=cls.env,
            text=True,
            capture_output=True,
            timeout=30,
        )
        if result.returncode:
            raise AssertionError(result.stderr)
        return result.stdout.strip()

    def setUp(self):
        self.sql("TRUNCATE users CASCADE;")

    def account(
        self,
        name,
        status=None,
        billing_class="standard",
        subscription=True,
        row=True,
        deadline="NULL",
    ):
        status_sql = "NULL" if status is None else "'" + status + "'"
        sub = "'sub_" + name + "'" if subscription and status is not None else "NULL"
        self.sql(f"""
            INSERT INTO users VALUES ('{name}', '{name}@example.invalid', 'pending', NULL, NOW(), NOW());
            INSERT INTO customer_orgs VALUES ('{name}', '{name}', 'Synthetic', '{billing_class}', NOW(), NOW());
        """)
        if row:
            self.sql(f"""INSERT INTO customer_billing_accounts
                (customer_org_id, stripe_subscription_id, subscription_status, current_period_end, created_at, updated_at)
                VALUES ('{name}', {sub}, {status_sql}, {deadline}, NOW(), NOW());""")

    def snapshot(self):
        snapshot = billing.collect(self.env)
        billing.render(snapshot)
        self.assertGreater(snapshot["collected_at"], 0)
        return snapshot["counts"]

    def test_every_stored_status_and_exemption_precedence(self):
        expected = {
            "subscribed": 3,
            "sponsored": 8,
            "grandfathered": 8,
            "trial": 1,
            "expired_past_due": 5,
            "incomplete": 1,
            "no_subscription": 1,
        }
        for kind in ("standard", "sponsored", "grandfathered"):
            for status in (
                None,
                "active",
                "trialing",
                "past_due",
                "unpaid",
                "canceled",
                "paused",
                "incomplete",
                "incomplete_expired",
            ):
                self.account(
                    f"{kind}_{status}",
                    status,
                    kind,
                    deadline="NOW() + interval '1 day'",
                )
        # Scheduled cancellation still has recorded active status.
        self.sql(
            "UPDATE customer_billing_accounts SET cancel_at_period_end=TRUE WHERE subscription_status='active'"
        )
        self.assertEqual(self.snapshot(), expected)
        self.assertEqual(sum(expected.values()), 27)

    def test_empty_and_missing_rows_are_distinct_from_no_subscription(self):
        self.assertEqual(self.snapshot(), {})
        # A user without an organization does not add a billing account.
        self.sql(
            "INSERT INTO users VALUES ('alone', 'alone@example.invalid', 'pending', NULL, NOW(), NOW())"
        )
        self.account("missing", row=False)
        self.account("customer_only")
        self.sql(
            "UPDATE customer_billing_accounts SET stripe_customer_id='cus_synthetic'"
        )
        self.assertEqual(
            self.snapshot(), {"missing_billing_account": 1, "no_subscription": 1}
        )

    def test_missing_unknown_and_partial_subscription_state(self):
        self.account("status_only", "active", subscription=False)
        self.account("id_only")
        self.account("blank", "active")
        self.account("future", "active")
        self.account("class")
        self.account("undated_trial", "trialing")
        # Simulate schema expansion/corrupt restored states in the disposable DB.
        self.sql("""ALTER TABLE customer_orgs DROP CONSTRAINT customer_orgs_billing_class_check;
            ALTER TABLE customer_billing_accounts DROP CONSTRAINT customer_billing_accounts_subscription_status_check;
            UPDATE customer_billing_accounts SET stripe_subscription_id='sub_partial' WHERE customer_org_id='id_only';
            UPDATE customer_billing_accounts SET stripe_subscription_id=' ' WHERE customer_org_id='blank';
            UPDATE customer_billing_accounts SET subscription_status='future_status' WHERE customer_org_id='future';
            UPDATE customer_orgs SET billing_class='future_class' WHERE id='class';""")
        try:
            self.assertEqual(self.snapshot(), {"unknown": 6})
        finally:
            self.sql("""TRUNCATE users CASCADE;
                ALTER TABLE customer_orgs ADD CONSTRAINT customer_orgs_billing_class_check
                    CHECK (billing_class IN ('standard','sponsored','grandfathered'));
                ALTER TABLE customer_billing_accounts ADD CONSTRAINT customer_billing_accounts_subscription_status_check
                    CHECK (subscription_status IN ('active','trialing','past_due','unpaid','canceled','paused','incomplete','incomplete_expired'));""")

    def test_trial_deadline_fallback_expiry_and_reservations(self):
        self.sql("""INSERT INTO trial_campaigns (id,name,code_hash,seat_limit,trial_days,created_by_workos_user_id)
                    VALUES ('campaign','Synthetic','synthetic',100,7,'operator');""")
        cases = [
            ("future", "redeemed", "NOW()", "NULL", "trial"),
            (
                "elapsed",
                "redeemed",
                "NOW() - interval '8 days'",
                "NULL",
                "expired_past_due",
            ),
            (
                "period_wins",
                "redeemed",
                "NOW()",
                "NOW() - interval '1 day'",
                "expired_past_due",
            ),
            ("missing", "redeemed", "NULL", "NULL", "unknown"),
            ("reserved", "reserved", "NOW()", "NULL", "unknown"),
            ("expired", "expired", "NOW()", "NULL", "unknown"),
        ]
        expected = {}
        for name, state, redeemed, period, status in cases:
            self.account(name, "trialing", deadline=period)
            self.sql(f"""INSERT INTO trial_redemptions
                (id,campaign_id,customer_org_id,attempt_id,stripe_session_id,state,checkout_expires_at,redeemed_at)
                VALUES ('{name}','campaign','{name}','{name}','{name}','{state}',NOW(),{redeemed});""")
            expected[status] = expected.get(status, 0) + 1
        # Multiple expired attempts must not multiply the billing account.
        self.sql("""INSERT INTO trial_redemptions
            (id,campaign_id,customer_org_id,attempt_id,stripe_session_id,state,checkout_expires_at)
            VALUES ('old','campaign','future','old','old','expired',NOW());""")
        self.assertEqual(self.snapshot(), expected)


if __name__ == "__main__":
    unittest.main()

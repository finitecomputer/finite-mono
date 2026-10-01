#!/usr/bin/env python3
"""Opt-in destructive test on UNIQUE temporary local databases and synthetic Loki data.

Run under `devfinity run -- python3 ... --loki-url http://127.0.0.1:<port>`.
Loki must be disposable: unique synthetic events remain until its own retention.
Production hosts are rejected. Nothing deploys, authenticates to production, or
chooses an existing database for writes. Normal unit discovery does not run this.
"""

import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
import urllib.parse
import urllib.request
import uuid

import argparse

ROOT = Path(__file__).resolve().parents[3]
parser = argparse.ArgumentParser(
    description="Synthetic diagnostics replay/retention/restore contract (local services only)"
)
parser.add_argument(
    "--loki-url",
    required=True,
    help="Explicit disposable loopback Loki origin, e.g. http://127.0.0.1:3310",
)
args = parser.parse_args()
loki = urllib.parse.urlsplit(args.loki_url)
if (
    loki.hostname not in {"127.0.0.1", "localhost", "::1"}
    or loki.scheme != "http"
    or loki.username
    or loki.password
    or loki.path not in {"", "/"}
    or loki.query
    or loki.fragment
):
    parser.error("Loki must be an explicit disposable HTTP loopback origin")
LOKI = args.loki_url.rstrip("/")
ADMIN = os.environ["FC_CORE_POSTGRES_TEST_URL"]
parts = urllib.parse.urlsplit(ADMIN)
assert parts.hostname in {"127.0.0.1", "localhost", "::1"}, (
    "local test database required"
)
suffix = uuid.uuid4().hex[:12]
source_name = "metrics_verify_" + suffix
restore_name = "metrics_restore_" + suffix
created = []
reporting_role = "metrics_reader_" + suffix
role_created = False


def url(name):
    return urllib.parse.urlunsplit(parts._replace(path="/" + name))


def command(args, db, data=None):
    connection = urllib.parse.urlsplit(db)
    env = {
        **os.environ,
        "PGHOST": connection.hostname,
        "PGPORT": str(connection.port),
        "PGUSER": connection.username,
        "PGDATABASE": connection.path.lstrip("/"),
    }
    env.pop("PGPASSWORD", None)
    if connection.password:
        env["PGPASSWORD"] = urllib.parse.unquote(connection.password)
    result = subprocess.run(
        args, input=data, text=True, env=env, capture_output=True, timeout=60
    )
    if result.returncode:
        raise RuntimeError("local reporting test subprocess failed: " + result.stderr)
    return result.stdout.strip()


def sql(query, db):
    return command(["psql", "-X", "-qAt", "-v", "ON_ERROR_STOP=1"], db, query)


def parsed(query):
    return json.loads(sql(query, url(source_name)) or "null")


def push(streams):
    # Loki rejects entries more than an hour behind a stream's newest entry, so
    # each run gets its own stream for its hours-old synthetic events.
    for entry in streams:
        entry["stream"]["verify_run"] = suffix
    req = urllib.request.Request(
        LOKI + "/loki/api/v1/push",
        data=json.dumps({"streams": streams}).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=10) as response:
        assert response.status == 204


try:
    for name in (source_name, restore_name):
        sql(f"CREATE DATABASE {name};", ADMIN)
        created.append(name)
    files = sorted(
        (ROOT / "finitecomputer-v2/crates/finite-saas-core/migrations").glob(
            "[0-9][0-9][0-9][0-9]_*.sql"
        )
    )
    sql("\n".join(p.read_text() for p in files if p.name < "0037"), url(source_name))
    # Events span 100 minutes: more than one hourly query split, yet inside
    # Loki's default 2h max_chunk_age and 3h query_ingesters_within, so
    # unflushed synthetic chunks stay visible to every query.
    seed = """
    INSERT INTO users (id,normalized_email,link_status,created_at,updated_at)
      VALUES ('verify-user','verify@example.invalid','pending',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
    INSERT INTO finite_private_limit_profiles (id,burst_window_seconds,burst_limit_units,weekly_limit_units,created_at,updated_at)
      VALUES ('verify-profile',60,100000,10000000,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
    INSERT INTO finite_private_grants (id,user_id,limit_profile_id,status,created_at,updated_at)
      VALUES ('verify-grant','verify-user','verify-profile','active',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
    INSERT INTO finite_private_api_keys (id,grant_id,key_hash,status,created_at,updated_at)
      VALUES ('verify-key','verify-grant','synthetic-noncredential-hash','active',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
    INSERT INTO finite_private_reservations
      (id,request_id,api_key_id,grant_id,endpoint,model,estimated_usage_units,reserved_usage_units,settled_usage_units,settlement_kind,status,usage_formula_version,created_at,updated_at)
      SELECT 'verify-res-'||lpad(n::text,4,'0'),'verify-req-'||n,'verify-key','verify-grant','/v1/chat/completions','synthetic-model',68,68,68,'actual','settled','test',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP
      FROM generate_series(1,1002) n;
    INSERT INTO finite_private_request_diagnostics
      (reservation_id,request_id,api_key_id,endpoint,model,prompt_tokens,completion_tokens,first_output_ms,first_answer_ms,duration_ms,termination_reason,measurement_quality,observed_at)
      SELECT id,request_id,api_key_id,endpoint,model,11,19,240,300,900,'complete','observed_usage',
      CASE WHEN n=1002 THEN CURRENT_TIMESTAMP-INTERVAL '8 days'
        WHEN n<=500 THEN CURRENT_TIMESTAMP-INTERVAL '100 minutes'+n*INTERVAL '1 second'
        ELSE CURRENT_TIMESTAMP-INTERVAL '90 minutes'+(n-501)*INTERVAL '10 seconds' END
      FROM finite_private_reservations, LATERAL (SELECT right(id,4)::int AS n) seq;
    """
    sql(seed.replace("verify-key", "verify-key-" + suffix), url(source_name))
    migration = next(p for p in files if p.name.startswith("0037_"))
    # Upgrade an existing ledger twice: no backfill or changed accounting.
    sql(migration.read_text() * 2, url(source_name))
    # Apply any subsequent migrations too as the repository evolves.
    sql("\n".join(p.read_text() for p in files if p.name > migration.name), url(source_name))
    assert sql(
        "SELECT COUNT(usage_user_id) FROM finite_private_reservations;",
        url(source_name),
    ) == "0"
    # Synthetic snapshots cover two owners plus pre-collection unknowns.
    for table, identifier in (
        ("finite_private_reservations", "id"),
        ("finite_private_request_diagnostics", "reservation_id"),
    ):
        sql(
            f"UPDATE {table} SET usage_user_id = CASE right({identifier},4)::int % 3 "
            "WHEN 1 THEN 'verify-user' WHEN 2 THEN 'verify-user-2' ELSE NULL END;",
            url(source_name),
        )
    module_spec = importlib.util.spec_from_file_location(
        "private_export", ROOT / "infra/monitoring/private_requests/export.py"
    )
    module = importlib.util.module_from_spec(module_spec)
    module_spec.loader.exec_module(module)
    module_text = (
        ROOT / "infra/nixos/modules/finite-private-request-diagnostics.nix"
    ).read_text()
    sql(f"CREATE ROLE {reporting_role};", ADMIN)
    role_created = True
    grants = re.findall(r"GRANT [^;]+;", module_text)
    sql("\n".join(grants).replace("TO finite_private_diagnostics", f"TO {reporting_role}"), url(source_name))
    assert sql(
        f"SELECT has_table_privilege('{reporting_role}', 'finite_private_reservations', 'SELECT');",
        url(source_name),
    ) == "f"

    def export_query(statement):
        return parsed(f"SET ROLE {reporting_role};\n" + statement)

    query = (ROOT / "infra/monitoring/private_requests/requests.sql").read_text()
    first = export_query(query)
    assert len(first) == 500
    assert {r["event"]["usageUserId"] for r in first} == {
        "verify-user", "verify-user-2", None
    }
    for row in first:
        event = row["event"]
        assert event["userAttribution"] == (
            "reservation_grant" if event["usageUserId"] else "unknown"
        )
        if event["usageUserId"] is None:
            # Historical Loki records lack both fields. Replaying their new
            # shape must still count the reservation only once as unknown.
            del event["usageUserId"], event["userAttribution"]
    # Simulate remote success followed by crash before acknowledgement.
    push([module.stream(module.SERVICE, first)])
    total = 0
    batches = []
    while rows := export_query(query):
        batches.append(len(rows))
        total += module.export_batch(rows, send=push, query=export_query)
    assert total == 1001 and batches == [500, 500, 1], (total, batches)
    assert (
        int(
            sql(
                "SELECT COUNT(*) FROM finite_private_request_diagnostics WHERE exported_at IS NOT NULL;",
                url(source_name),
            )
        )
        == 1001
    )
    report = {"exported": total, "batches": batches, "replayed_before_ack": 500}
    dashboard = json.loads(
        (
            ROOT / "infra/monitoring/grafana/dashboards/finite-private-requests.json"
        ).read_text()
    )
    # Per-request value of every Loki instant panel. Loki splits instant
    # queries longer than one hour, so each window must reconcile with Core.
    per_request = {
        ("Input / output tokens by Project · retained events", 0): 11,
        ("Input / output tokens by Project · retained events", 1): 19,
        ("Input / output tokens by key · retained events", 0): 11,
        ("Input / output tokens by key · retained events", 1): 19,
        ("Measurement quality · retained requests", 0): 1,
        ("Termination reasons · retained requests", 0): 1,
        ("Input / output tokens by usage user · retained events", 0): 11,
        ("Input / output tokens by usage user · retained events", 1): 19,
        ("Requests by usage user · retained events", 0): 1,
    }
    instant = {
        (panel["title"], index): target["expr"]
        for panel in dashboard["panels"]
        if panel.get("datasource", {}).get("uid") == "finite-loki"
        for index, target in enumerate(panel.get("targets", []))
        if target.get("queryType") == "instant"
    }
    assert instant.keys() == per_request.keys(), sorted(instant)
    evaluated_at = int(time.time())
    for window, seconds in (("1h", 3600), ("24h", 86400), ("168h", 604800)):
        requests = int(
            sql(
                "SELECT COUNT(*) FROM finite_private_request_diagnostics "
                f"WHERE observed_at > to_timestamp({evaluated_at - seconds}) "
                f"AND observed_at <= to_timestamp({evaluated_at}) "
                f"AND observed_at >= to_timestamp({evaluated_at}) - INTERVAL '7 days';",
                url(source_name),
            )
        )
        report[window] = requests
        groups = parsed(
            "SELECT json_object_agg(owner, total) FROM (SELECT "
            "COALESCE(usage_user_id,'unknown') AS owner, COUNT(*) AS total "
            "FROM finite_private_request_diagnostics "
            f"WHERE observed_at > to_timestamp({evaluated_at - seconds}) "
            f"AND observed_at <= to_timestamp({evaluated_at}) "
            "GROUP BY owner) counts;"
        )
        # Every retained-event panel respects exact user filters, including
        # historical missing fields. All-user queries must preserve each group.
        for owner in (".*", "verify-user", "verify-user-2", "unknown", "absent-user"):
            expected_groups = {
                key: count for key, count in groups.items()
                if owner == ".*" or key == owner
            }
            variables = {
                "$__range": window,
                "$model": "synthetic-model",
                "$endpoint": "/v1/chat/completions",
                "${project:raw}": ".*",
                "${runtime:raw}": ".*",
                "${key:raw}": "verify-key-" + suffix,
                "${usage_user:raw}": owner,
            }
            for (title, index), expression in instant.items():
                for variable, value in variables.items():
                    expression = expression.replace(variable, value)
                endpoint = LOKI + "/loki/api/v1/query?" + urllib.parse.urlencode(
                    {"query": expression, "time": evaluated_at}
                )
                with urllib.request.urlopen(endpoint, timeout=30) as response:
                    result = json.load(response)["data"]["result"]
                expected = {
                    key: count * per_request[(title, index)]
                    for key, count in expected_groups.items()
                }
                assert sum(float(s["value"][1]) for s in result) == sum(expected.values()), (
                    window, owner, title, result, expected
                )
                if "by usage user" in title:
                    assert {
                        s["metric"]["user"]: float(s["value"][1]) for s in result
                    } == expected, (window, owner, title, result, expected)
            if window == "24h" and owner != ".*":
                table = next(p for p in dashboard["panels"] if p["id"] == 26)
                expression = table["targets"][0]["expr"]
                for variable, value in variables.items():
                    expression = expression.replace(variable, value)
                endpoint = LOKI + "/loki/api/v1/query_range?" + urllib.parse.urlencode({
                    "query": expression,
                    "start": (evaluated_at - seconds) * 1000000000,
                    "end": evaluated_at * 1000000000,
                    "limit": 1000,
                })
                with urllib.request.urlopen(endpoint, timeout=30) as response:
                    streams = json.load(response)["data"]["result"]
                events = [json.loads(line) for stream in streams for _, line in stream["values"]]
                assert all((e.get("usageUserId") or "unknown") == owner for e in events)
                assert len({e["reservationId"] for e in events}) == sum(expected_groups.values())
    report["user_attribution"] = {
        "aggregate_queries": len(instant) * 5 * 3,
        "filtered_detail_queries": 4,
        "historical_unknown_and_replay": "passed",
        "restored_owner_snapshots": 668,
    }
    assert report["1h"] < report["24h"] == report["168h"] == 1001, report
    with tempfile.TemporaryDirectory(prefix="metrics-restore-") as directory:
        dump = Path(directory) / "core.dump"
        command(
            [
                "pg_dump",
                "--format=custom",
                "--exclude-table-data=public.finite_private_request_diagnostics",
                "--file",
                str(dump),
            ],
            url(source_name),
        )
        command(
            [
                "pg_restore",
                "--no-owner",
                "--exit-on-error",
                "--dbname",
                restore_name,
                str(dump),
            ],
            url(restore_name),
        )
        assert (
            sql(
                "SELECT COUNT(*) FROM finite_private_request_diagnostics;",
                url(restore_name),
            )
            == "0"
        )
        assert (
            sql(
                "SELECT COUNT(*), SUM(settled_usage_units) FROM finite_private_reservations;",
                url(restore_name),
            )
            == "1002|68136"
        )
        assert sql(
            "SELECT COUNT(usage_user_id), COUNT(DISTINCT usage_user_id) FROM finite_private_reservations;",
            url(restore_name),
        ) == "668|2"
        sql(
            "\n".join(p.read_text() for p in files if p.name < "0032"),
            url(restore_name),
        )
        assert (
            sql(
                "SELECT COUNT(*), SUM(settled_usage_units) FROM finite_private_reservations;",
                url(restore_name),
            )
            == "1002|68136"
        )
        report["restore"] = {
            "diagnostics": 0,
            "accounting_rows": 1002,
            "usage_units": 68136,
            "previous_schema_reapply": "passed",
        }
    prune = re.search(
        r"DELETE FROM finite_private_request_diagnostics\s+WHERE observed_at < CURRENT_TIMESTAMP - INTERVAL \'7 days\';",
        module_text,
    )
    assert prune, "explicit prune query missing"
    export_query(prune.group())
    assert (
        sql(
            "SELECT COUNT(*) FROM finite_private_request_diagnostics;", url(source_name)
        )
        == "1001"
    )
    assert (
        sql("SELECT COUNT(*) FROM finite_private_reservations;", url(source_name))
        == "1002"
    )
    report["prune"] = {
        "expired_diagnostics_removed": 1,
        "accounting_rows_preserved": 1002,
    }
    print(json.dumps(report, indent=2))
finally:
    for name in reversed(created):
        sql(f"DROP DATABASE {name} WITH (FORCE);", ADMIN)
    if role_created:
        sql(f"DROP ROLE {reporting_role};", ADMIN)

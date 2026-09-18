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
    sql("\n".join(p.read_text() for p in files), url(source_name))
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
      CASE WHEN id='verify-res-1002' THEN CURRENT_TIMESTAMP-INTERVAL '8 days' ELSE CURRENT_TIMESTAMP END
      FROM finite_private_reservations;
    """
    sql(seed.replace("verify-key", "verify-key-" + suffix), url(source_name))
    module_spec = importlib.util.spec_from_file_location(
        "private_export", ROOT / "infra/monitoring/private_requests/export.py"
    )
    module = importlib.util.module_from_spec(module_spec)
    module_spec.loader.exec_module(module)
    query = (ROOT / "infra/monitoring/private_requests/requests.sql").read_text()
    first = parsed(query)
    assert len(first) == 500
    # Simulate remote success followed by crash before acknowledgement.
    push([module.stream(module.SERVICE, first)])
    total = 0
    batches = []
    while rows := parsed(query):
        batches.append(len(rows))
        total += module.export_batch(rows, send=push, query=parsed)
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
    expressions = [
        ("loki_count", "Measurement quality · retained requests", 0, 1001),
        ("loki_output_tokens", "Input / output tokens by Project · retained events", 1, 19019),
    ]
    for label, title, target, expected in expressions:
        panel = next(panel for panel in dashboard["panels"] if panel["title"] == title)
        expression = panel["targets"][target]["expr"]
        for variable, value in {
            "$__range": "1h",
            "$model": "synthetic-model",
            "$endpoint": "/v1/chat/completions",
            "${project:raw}": ".*",
            "${runtime:raw}": ".*",
            "${key:raw}": "verify-key-" + suffix,
        }.items():
            expression = expression.replace(variable, value)
        endpoint = (
            LOKI + "/loki/api/v1/query?" + urllib.parse.urlencode({"query": expression})
        )
        with urllib.request.urlopen(endpoint, timeout=15) as response:
            result = json.load(response)["data"]["result"]
            value = sum(float(series["value"][1]) for series in result)
        assert value == expected, (label, value, expected)
        report[label] = value
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
    module_text = (
        ROOT / "infra/nixos/modules/finite-private-request-diagnostics.nix"
    ).read_text()
    prune = re.search(
        r"DELETE FROM finite_private_request_diagnostics\s+WHERE observed_at < CURRENT_TIMESTAMP - INTERVAL \'7 days\';",
        module_text,
    )
    assert prune, "explicit prune query missing"
    sql(prune.group(), url(source_name))
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

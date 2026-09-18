#!/usr/bin/env python3
"""Bounded, redacted Core diagnostics -> Loki. Never logs event data or secrets."""

import base64
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import urllib.request

DIRECTORY = Path(__file__).resolve().parent
SERVICE = "finite-private-request-diagnostics"
GUARD_SERVICE = "finite-private-guard-status"


def database(sql):
    # SQL and acknowledgements go through stdin, never process arguments.
    result = subprocess.run(
        [
            "psql",
            "--no-psqlrc",
            "--quiet",
            "--tuples-only",
            "--no-align",
            "--set=ON_ERROR_STOP=1",
            "--dbname=finite_core",
        ],
        input="SET statement_timeout = '5s'; SET lock_timeout = '1s'; SET timezone = 'UTC'; SET standard_conforming_strings = on;\n"
        + sql,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=8,
        check=True,
    )
    return json.loads(result.stdout.strip() or "null")


def push(streams):
    credentials = (
        os.environ["FINITE_LOGS_WRITE_USERNAME"]
        + ":"
        + os.environ["FINITE_LOGS_WRITE_PASSWORD"]
    ).encode()
    request = urllib.request.Request(
        "https://metrics-ingest.finite.computer/loki/api/v1/push",
        data=json.dumps({"streams": streams}, separators=(",", ":")).encode(),
        headers={
            "Content-Type": "application/json",
            "Authorization": "Basic " + base64.b64encode(credentials).decode(),
        },
        method="POST",
    )

    # A redirected collector must not send the credential to another origin.
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, req, fp, code, msg, headers, newurl):
            return None

    with urllib.request.build_opener(NoRedirect).open(request, timeout=8) as response:
        if not 200 <= response.status < 300:
            raise RuntimeError("Loki push rejected")


def stream(service, rows):
    return {
        "stream": {"service": service},
        "values": [
            [
                row["timestampNs"],
                json.dumps(row["event"], sort_keys=True, separators=(",", ":")),
            ]
            for row in rows
        ],
    }


def export_batch(rows, send=push, query=database):
    if not rows:
        return 0
    send([stream(SERVICE, rows)])
    # Stable timestamp + immutable canonical JSON preserves event identity.
    # Log queries deduplicate exact replay; metric queries must explicitly
    # deduplicate by reservationId before aggregating (see dashboard contract).
    identifiers = json.dumps([row["event"]["reservationId"] for row in rows])
    escaped = identifiers.replace("'", "''")
    query(
        "UPDATE finite_private_request_diagnostics SET exported_at = CURRENT_TIMESTAMP "
        "WHERE exported_at IS NULL AND reservation_id IN "
        f"(SELECT jsonb_array_elements_text('{escaped}'::jsonb));"
    )
    return len(rows)


def atomic_json(path, value):
    atomic_write(path, json.dumps(value, separators=(",", ":")))


def atomic_write(path, text):
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, delete=False) as handle:
        temp = Path(handle.name)
        try:
            handle.write(text)
            handle.flush()
            os.fchmod(handle.fileno(), 0o640)
            os.replace(temp, path)
        finally:
            temp.unlink(missing_ok=True)


def run(state_path, metrics_path, query=database, send=push, clock=time.time):
    now = int(clock())
    state = (
        json.loads(state_path.read_text())
        if state_path.exists()
        else {
            "collection_started": now,
            "last_export": 0,
            "exported": 0,
            "failures": 0,
        }
    )
    success = 0
    pending = -1
    oldest = -1
    guards_complete = 0
    try:
        for _ in range(4):
            rows = query((DIRECTORY / "requests.sql").read_text())
            state["exported"] += export_batch(rows, send=send, query=query)
            if len(rows) < 500:
                break
        guard_rows = query((DIRECTORY / "guards.sql").read_text())
        guards_complete = int(len(guard_rows) <= 1000)
        if guard_rows:
            send([stream(GUARD_SERVICE, guard_rows[:1000])])
        backlog = query(
            "SELECT json_build_object('pending', COUNT(*), 'oldest', "
            "COALESCE(EXTRACT(EPOCH FROM CURRENT_TIMESTAMP - MIN(observed_at)),0)) "
            "FROM finite_private_request_diagnostics WHERE exported_at IS NULL "
            "AND observed_at >= CURRENT_TIMESTAMP - INTERVAL '7 days';"
        )
        pending, oldest = backlog["pending"], backlog["oldest"]
        state["last_export"] = int(clock())
        success = 1
    except Exception:
        # Never print exception messages: HTTP bodies, SQL and credentials
        # may occur in library diagnostics. Failure evidence is bounded metrics.
        state["failures"] += 1
    atomic_json(state_path, state)
    values = {
        "collection_started_timestamp_seconds": state["collection_started"],
        "last_export_timestamp_seconds": state["last_export"],
        "exported_total": state["exported"],
        "export_failures_total": state["failures"],
        "exporter_query_success": success,
        "pending_requests": pending,
        "oldest_pending_age_seconds": oldest,
        "guard_snapshot_complete": guards_complete,
    }
    atomic_write(
        metrics_path,
        "".join(
            f"finite_private_request_diagnostics_{name} {value}\n"
            for name, value in values.items()
        ),
    )
    return 0 if success else 1


if __name__ == "__main__":
    sys.exit(run(Path(sys.argv[1]), Path(sys.argv[2])))

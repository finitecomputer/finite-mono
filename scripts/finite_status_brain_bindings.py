"""Explicit private export of stable Core bindings for scoped operator manifests.

Only the requested account IDs are selected. This is separate from contact-free
monitoring and never authorizes a mapping or changes an account binding.
"""
from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Any

MAX_ACCOUNTS = 100
MAX_BYTES = 64 * 1024
IDENTIFIER = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,199}\Z")

QUERY = r"""
BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL statement_timeout = '15s';
WITH requested AS (SELECT value AS id FROM jsonb_array_elements_text(:'account_ids'::jsonb)),
bindings AS (
  SELECT u.id AS "accountId", u.link_status AS "linkStatus",
    CASE WHEN octet_length(u.normalized_email) <= 254 THEN u.normalized_email END AS email,
    CASE WHEN octet_length(u.workos_user_id) <= 200 THEN u.workos_user_id END AS "workosSubject"
  FROM users u JOIN requested r ON r.id=u.id ORDER BY u.id LIMIT 101
)
SELECT json_build_object('checkedAt', core_rfc3339(now()),
  'bindings', COALESCE((SELECT json_agg(bindings) FROM bindings), '[]'::json));
ROLLBACK;
"""


def account_ids(path: Path) -> list[str]:
    with path.open("rb") as source:
        data = source.read(MAX_BYTES + 1)
    values = json.loads(data) if len(data) <= MAX_BYTES else None
    if (not isinstance(values, list) or not 1 <= len(values) <= MAX_ACCOUNTS
            or any(not isinstance(value, str) or not IDENTIFIER.fullmatch(value) for value in values)
            or len(set(values)) != len(values)):
        raise ValueError("invalid account ID input")
    return sorted(values)


def report(requested: list[str], payload: Any) -> dict[str, Any]:
    if not isinstance(payload, dict) or set(payload) != {"checkedAt", "bindings"}:
        raise ValueError("invalid binding result")
    rows = payload["bindings"]
    if not isinstance(rows, list) or len(rows) > MAX_ACCOUNTS:
        raise ValueError("invalid binding rows")
    seen = set()
    for row in rows:
        if (not isinstance(row, dict) or set(row) != {"accountId", "linkStatus", "email", "workosSubject"}
                or not isinstance(row["accountId"], str) or row["accountId"] not in requested
                or row["accountId"] in seen or row["linkStatus"] != "linked"
                or not isinstance(row["workosSubject"], str) or not IDENTIFIER.fullmatch(row["workosSubject"])
                or not isinstance(row["email"], str) or not 1 <= len(row["email"].encode()) <= 254
                or row["email"].strip() != row["email"] or any(ord(c) < 32 or ord(c) == 127 for c in row["email"])):
            raise ValueError("unsupported account binding")
        seen.add(row["accountId"])
    missing = sorted(set(requested) - seen)
    return {"schema_version": "finite.brain-scoped-bindings.v1", "generated_at": payload["checkedAt"],
            "mode": "privateOperatorBindings", "exit_code": 2 if missing else 0,
            "coverage": {"complete": not missing, "requestedAccounts": len(requested), "returnedAccounts": len(rows)},
            "missingAccountIds": missing, "bindings": rows,
            "limitations": ["Current stored Core enrollment bindings, not a new WorkOS authentication.",
                            "Private operator evidence only; no account, identity, access or scoped record is changed."]}

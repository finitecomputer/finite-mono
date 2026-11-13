"""Contact-free monitoring of exact-Brain operator attribution bindings.

The bounded SQL facts are private in-memory inputs, never the returned report.
No email, WorkOS subject, evidence path or annotation is selected. Current
binding checks mirror Core's scoped reader; names never determine a conflict.
"""
from __future__ import annotations

from collections import Counter, defaultdict
from typing import Any

if __package__:
    from .finite_status_brain_roster import npub_to_hex
else:
    from finite_status_brain_roster import npub_to_hex

MAX_FACT_ROWS = 10_000
MAX_FACT_BYTES = 8 * 1024 * 1024

IDENTITY_QUERY = r"""
SELECT to_regclass('brain_scoped_records') IS NOT NULL AS finite_has_scoped_records \gset
\if :finite_has_scoped_records
WITH records AS (
  SELECT r.id, r.public_key_hex AS key, r.kind, r.account_id AS account, r.project_id AS project,
    COALESCE(u.link_status = 'linked' AND u.workos_user_id = r.workos_subject, false) AS account_matches,
    EXISTS (SELECT 1 FROM account_brain_principals a WHERE a.public_key_hex=r.public_key_hex AND a.status='active') AS human_any,
    EXISTS (SELECT 1 FROM account_brain_principals a WHERE a.public_key_hex=r.public_key_hex AND a.status='active'
        AND a.user_id IS DISTINCT FROM r.account_id) AS human_other
  FROM brain_scoped_records r LEFT JOIN users u ON u.id=r.account_id
  WHERE r.retired_at IS NULL ORDER BY r.id LIMIT 10001
), pins AS (
  SELECT r.id, r.health_reporting_npub AS npub, r.project_id AS project, p.owner_user_id AS account,
    p.id IS NOT NULL AS project_present, r.offboarding_phase AS phase,
    EXISTS (SELECT 1 FROM project_runtime_links l WHERE l.agent_runtime_id=r.id AND l.project_id=r.project_id AND l.active) AS active,
    EXISTS (SELECT 1 FROM project_runtime_links l WHERE l.agent_runtime_id=r.id AND l.project_id IS DISTINCT FROM r.project_id AND l.active) AS foreign_link
  FROM agent_runtimes r LEFT JOIN projects p ON p.id=r.project_id
  WHERE r.health_reporting_npub IS NOT NULL ORDER BY r.id LIMIT 10001
)
-- The collector reads one JSON document per psql output line. json_agg of
-- composite rows inserts physical newlines; the outer jsonb conversion
-- serializes the complete document onto one line, including nested arrays.
SELECT jsonb_build_object('scoped_record_facts', json_build_object(
  'schema_present',true,
  'active',(SELECT count(*) FROM brain_scoped_records WHERE retired_at IS NULL),
  'retired',(SELECT count(*) FROM brain_scoped_records WHERE retired_at IS NOT NULL),
  'records',COALESCE((SELECT json_agg(records) FROM records),'[]'::json),
  'pins',COALESCE((SELECT json_agg(pins) FROM pins),'[]'::json)
));
\else
SELECT json_build_object('scoped_record_facts',json_build_object('schema_present',false));
\endif
"""


def binding_conflict(record: dict[str, Any], pins: list[dict[str, Any]]) -> str | None:
    """Only binding conflicts, independent of participation/contact precedence."""
    kind = record["kind"]
    if kind in {"historical_identity", "invitation_key"}:
        return "annotationSuperseded" if record["human_any"] or pins else None
    if kind not in {"managed_agent", "account_chat_key"}:
        raise ValueError("unsupported scoped record kind")
    if not record["account_matches"]:
        return "accountBindingChanged"
    if kind == "account_chat_key":
        return "identityEvidenceConflict" if pins or record["human_other"] else None
    if record["human_any"]:
        return "identityEvidenceConflict"
    if not pins or any(not p["project_present"] or p["foreign_link"] for p in pins):
        return "agentBindingChanged"
    if ({p["project"] for p in pins} != {record["project"]}
            or {p["account"] for p in pins} != {record["account"]}):
        return "agentBindingChanged"
    # Core orders pins by runtime id and chooses the linked incarnation;
    # inactive legacy siblings do not override a supported live incarnation.
    active = next((p for p in sorted(pins, key=lambda p: p["id"]) if p["active"]), None)
    if active is not None:
        valid = active["phase"] in {None, "retirement_requested", "receipt_verified", "compute_removed"}
    else:
        valid = all(p["phase"] in {"link_deactivated", "archived"} for p in pins)
    return None if valid else "agentBindingChanged"


def identity_summary(facts: dict[str, Any] | None) -> dict[str, Any]:
    if facts is None:
        return {"state": "notChecked", "reason": "scoped record facts were not collected"}
    if facts.get("schema_present") is False:
        return {"state": "notConfigured", "schema_present": False}
    if facts.get("schema_present") is not True:
        raise ValueError("invalid scoped record schema coverage")
    records, pins = facts.get("records"), facts.get("pins")
    if not isinstance(records, list) or not isinstance(pins, list):
        raise ValueError("invalid scoped record fact arrays")
    active, retired = facts.get("active"), facts.get("retired")
    if any(type(count) is not int or count < 0 for count in (active, retired)):
        raise ValueError("invalid scoped record counts")
    if len(records) > MAX_FACT_ROWS or len(pins) > MAX_FACT_ROWS or len(records) != active:
        return {"state": "unavailable", "reason": "scoped record fact coverage exceeded its bound",
                "active": active, "retired": retired}
    by_key: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for pin in pins:
        # Invalid npubs cannot equal the canonical npub Core derives for an
        # exact requested hex key; they never silently choose an account.
        key = npub_to_hex(pin["npub"])
        if key is not None:
            by_key[key].append(pin)
    conflicts = []
    for record in records:
        reason = binding_conflict(record, by_key[record["key"]])
        if reason:
            conflicts.append({"recordId": record["id"], "reason": reason})
    return {"state": "checked", "schema_present": True, "active": active, "retired": retired,
            "conflictCount": len(conflicts), "conflictsByReason": dict(sorted(Counter(r["reason"] for r in conflicts).items())),
            "conflicts": conflicts}

"""Strict, offline-only JSON evidence contract. No runtime qualification."""

import json
import re
from dataclasses import dataclass

MAX_INPUT_BYTES = 262144
TOPOLOGY = "single-process-fixture-v1"
CAPABILITY = "synthetic-json-v1"
MARKERS = {
    "FIXTURE_INPUT_READ",
    "FIXTURE_SCHEMA_VALIDATED",
    "FIXTURE_CLEANUP_RESOLVED",
}
OUTCOMES = {"COMPLETE", "UNSUPPORTED", "INCONCLUSIVE", "ERROR"}


class ContractError(ValueError):
    """The document cannot be interpreted under this schema."""


@dataclass(frozen=True)
class Validation:
    status: str
    reasons: tuple[str, ...] = ()


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ContractError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _reject_constant(value):
    raise ContractError(f"non-JSON numeric constant: {value}")


def parse_json(raw):
    """One UTF-8 JSON document; no duplicate keys, NaN, trailing text or PASS."""
    if type(raw) is not bytes or not raw or len(raw) > MAX_INPUT_BYTES:
        raise ContractError("JSON input must be nonempty bounded bytes")
    try:
        return json.loads(
            raw.decode("utf-8"),
            object_pairs_hook=_unique_object,
            parse_constant=_reject_constant,
        )
    except (UnicodeError, ValueError, RecursionError) as exc:
        raise ContractError(f"invalid JSON: {exc}") from exc


def read_json(path):
    with open(path, "rb") as stream:
        return parse_json(stream.read(MAX_INPUT_BYTES + 1))


def _object(value, keys, path):
    if type(value) is not dict or set(value) != set(keys):
        raise ContractError(f"{path}: expected exactly {sorted(keys)}")


def _bool(value, path):
    # Python bool is an int subclass; truthiness and == True are insufficient.
    if type(value) is not bool:
        raise ContractError(f"{path}: expected Boolean")


def _text(value, path):
    if type(value) is not str or not 1 <= len(value) <= 128:
        raise ContractError(f"{path}: expected nonempty bounded string")


def _choice(value, choices, path):
    if type(value) is not str or value not in choices:
        raise ContractError(f"{path}: unknown value")


def _strings(value, path, *, nonempty=False):
    if type(value) is not list or len(value) > 64 or (nonempty and not value):
        raise ContractError(f"{path}: expected bounded string list")
    for item in value:
        _text(item, path)
    if len(set(value)) != len(value):
        raise ContractError(f"{path}: duplicate values")


def _version(value):
    if type(value) is not int or value != 1:
        raise ContractError("schema_version: expected integer 1")


def _identity(value, path):
    _object(value, {"source_revision", "binary_sha256"}, path)
    for field, length in (("source_revision", 40), ("binary_sha256", 64)):
        item = value[field]
        if type(item) is not str or re.fullmatch(f"[0-9a-f]{{{length}}}", item) is None:
            raise ContractError(f"{path}.{field}: expected lowercase hex identity")


def check_manifest(manifest):
    """Trusted expectations are explicit; no default case, identity or nonce."""
    _object(
        manifest,
        {
            "schema_version",
            "kind",
            "run_nonce",
            "identity",
            "topology",
            "required_capabilities",
            "cases",
        },
        "manifest",
    )
    _version(manifest["schema_version"])
    _choice(manifest["kind"], {"OFFLINE_FIXTURE"}, "manifest.kind")
    _text(manifest["run_nonce"], "manifest.run_nonce")
    _identity(manifest["identity"], "manifest.identity")
    _choice(manifest["topology"], {TOPOLOGY}, "manifest.topology")
    # v1 supports only the offline capability. Unknown requirements fail closed.
    if manifest["required_capabilities"] != [CAPABILITY]:
        raise ContractError("manifest.required_capabilities: unsupported requirement")
    cases = manifest["cases"]
    if type(cases) is not list or not 1 <= len(cases) <= 64:
        raise ContractError("manifest.cases: expected 1..64 cases")
    ids = []
    for case in cases:
        _object(case, {"id", "required_progress"}, "manifest.case")
        _text(case["id"], "manifest.case.id")
        _strings(case["required_progress"], "manifest.required_progress", nonempty=True)
        if not set(case["required_progress"]) <= MARKERS:
            raise ContractError("manifest.required_progress: unknown required marker")
        ids.append(case["id"])
    if len(set(ids)) != len(ids):
        raise ContractError("manifest.cases: duplicate IDs")


def _validate(manifest, evidence):
    check_manifest(manifest)
    _object(
        evidence,
        {
            "schema_version",
            "kind",
            "run_nonce",
            "identity",
            "topology",
            "capabilities",
            "outcome",
            "success",
            "complete",
            "phase",
            "cases",
            "cleanup",
        },
        "evidence",
    )
    _version(evidence["schema_version"])
    _choice(evidence["kind"], {"OFFLINE_FIXTURE"}, "evidence.kind")
    _text(evidence["run_nonce"], "evidence.run_nonce")
    if evidence["run_nonce"] != manifest["run_nonce"]:
        raise ContractError("evidence.run_nonce: stale or mismatched run")
    _identity(evidence["identity"], "evidence.identity")
    if evidence["identity"] != manifest["identity"]:
        raise ContractError("evidence.identity: declared source/binary mismatch")
    _text(evidence["topology"], "evidence.topology")
    _object(evidence["capabilities"], {CAPABILITY}, "evidence.capabilities")
    _bool(evidence["capabilities"][CAPABILITY], "evidence.capability")
    _choice(evidence["outcome"], OUTCOMES, "evidence.outcome")
    _choice(evidence["phase"], {"COMPLETE", "PARTIAL"}, "evidence.phase")
    for field in ("success", "complete"):
        _bool(evidence[field], f"evidence.{field}")
    if evidence["outcome"] != "COMPLETE" and evidence["success"]:
        raise ContractError("non-COMPLETE outcome cannot declare success")

    cases = evidence["cases"]
    if type(cases) is not list or len(cases) != len(manifest["cases"]):
        raise ContractError("evidence.cases: missing or extra expected case")
    incomplete = []
    failed = []
    for expected, actual in zip(manifest["cases"], cases):
        _object(actual, {"id", "complete", "success", "progress"}, "evidence.case")
        _text(actual["id"], "evidence.case.id")
        if actual["id"] != expected["id"]:
            raise ContractError(
                "evidence.case.id: omitted, duplicate or reordered case"
            )
        _bool(actual["complete"], "evidence.case.complete")
        _bool(actual["success"], "evidence.case.success")
        _strings(actual["progress"], "evidence.case.progress")
        if not set(actual["progress"]) <= MARKERS:
            raise ContractError("evidence.case.progress: unknown marker")
        # Complete success requires the exact ordered marker sequence. Partial
        # prefixes remain nonpassing; unknown/extra/reordered markers are errors.
        required = expected["required_progress"]
        progress = actual["progress"]
        if progress != required[: len(progress)]:
            raise ContractError("evidence.case.progress: unexpected marker sequence")
        if not actual["complete"] or progress != required:
            incomplete.append(f"case {actual['id']}: incomplete progress")
        if not actual["success"]:
            failed.append(f"case {actual['id']}: did not succeed")

    _object(evidence["cleanup"], {"resolved", "pending"}, "evidence.cleanup")
    _bool(evidence["cleanup"]["resolved"], "evidence.cleanup.resolved")
    _strings(evidence["cleanup"]["pending"], "evidence.cleanup.pending")
    if not evidence["cleanup"]["resolved"] or evidence["cleanup"]["pending"]:
        incomplete.append("cleanup: unresolved")
    if not evidence["complete"] or evidence["phase"] != "COMPLETE":
        incomplete.append("phase: partial or incomplete")

    if evidence["outcome"] == "ERROR":
        return Validation("ERROR", ("fixture reports error",))
    if evidence["topology"] != manifest["topology"]:
        return Validation("UNSUPPORTED", ("unsupported declared fixture topology",))
    if not evidence["capabilities"][CAPABILITY] or evidence["outcome"] == "UNSUPPORTED":
        return Validation("UNSUPPORTED", ("unsupported fixture capability/outcome",))
    if evidence["outcome"] == "INCONCLUSIVE" or incomplete:
        return Validation(
            "INCONCLUSIVE", tuple(incomplete) or ("inconclusive outcome",)
        )
    if not evidence["success"] or failed:
        return Validation("ERROR", tuple(failed) or ("fixture did not succeed",))
    return Validation("FIXTURE_PASS")


def validate_evidence(manifest, raw):
    """Pure validation: declared identities are compared, never measured here."""
    try:
        return _validate(manifest, parse_json(raw))
    except ContractError as exc:
        return Validation("ERROR", (str(exc),))

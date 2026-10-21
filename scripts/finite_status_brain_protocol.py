"""Opt-in, credential-private v3 capability check for an audited Brain roster.

No downgrade: this evidence proves only the Core endpoint. A real admin viewer
must separately prove Brain/client negotiation before an operator apply.
"""
from __future__ import annotations

import http.client
import ipaddress
import json
import re
import socket
import threading
import time
from datetime import datetime
from typing import Any, Callable

if __package__:
    from .finite_status_brain_roster import npub_to_hex, roster_keys
else:
    from finite_status_brain_roster import npub_to_hex, roster_keys

VERSION = "finite-core-brain-descriptions-v3"
PATH = "/api/core/internal/v1/brain-identity-descriptions"
HEADER = "x-finite-brain-description-credential"
ENV_NAMES = {"FC_CORE_BRAIN_IDENTITY_BIND", "FC_CORE_BRAIN_IDENTITY_BRAIN_SERVER",
             "FC_CORE_BRAIN_DESCRIPTION_TOKEN"}
MAX_BYTES = 256 * 1024
BUDGET = 15.0
PAGE_SECONDS = 3.0
HEX = re.compile(r"[0-9a-f]{64}\Z")
CONFLICTS = {"accountBindingChanged", "identityEvidenceConflict", "agentBindingChanged", "annotationSuperseded"}


def timestamp(value: Any) -> bool:
    if not isinstance(value, str) or not 20 <= len(value) <= 40 or not value.endswith("Z"):
        return False
    try:
        return value[10] == "T" and datetime.fromisoformat(value[:-1] + "+00:00").year >= 1
    except ValueError:
        return False


def text(value: Any, limit: int) -> bool:
    return (isinstance(value, str) and 0 < len(value.encode()) <= limit and value == value.strip()
            and not any(ord(char) < 32 or ord(char) == 127 for char in value))


def validate(request: dict, response: Any) -> tuple[int, int]:
    """Strict shape/provenance, then discard every contact and annotation."""
    if (not isinstance(response, dict) or set(response) != {"version", "brainId", "checkedAt", "results"}
            or response["version"] != VERSION or response["brainId"] != request["brainId"]
            or not timestamp(response["checkedAt"]) or not isinstance(response["results"], list)):
        raise ValueError("invalid v3 envelope")
    keys = request["publicKeysHex"]
    rows = response["results"]
    if (len(rows) != len(keys) or any(not isinstance(row, dict) for row in rows)
            or {row.get("publicKeyHex") for row in rows} != set(keys)):
        raise ValueError("incomplete v3 page")
    used = conflicts = 0
    for row in rows:
        allowed = {"publicKeyHex", "state", "kind", "displayName", "accountEmail", "lifecycle",
                   "responsibleAccount", "source", "operatorAnnotation", "operatorRecord"}
        if not set(row) <= allowed:
            raise ValueError("unexpected v3 field")
        state, kind = row.get("state"), row.get("kind")
        source, record, annotation = row.get("source"), row.get("operatorRecord"), row.get("operatorAnnotation")
        owner = row.get("responsibleAccount")
        if state == "resolved":
            if (not isinstance(source, dict) or set(source) != {"kind", "observedAt", "revision"}
                    or source["kind"] not in {"hostedDeviceObservation", "finiteRuntimeRecord", "operatorScopedRecord"}
                    or not timestamp(source["observedAt"]) or not text(source["revision"], 128)):
                raise ValueError("invalid description source")
            if kind == "human":
                if (not text(row.get("accountEmail"), 254)
                        or row.get("displayName") is not None and not text(row["displayName"], 200)
                        or any(row.get(k) is not None for k in ("lifecycle", "responsibleAccount"))):
                    raise ValueError("invalid human description")
            elif kind == "agent":
                if (not text(row.get("displayName"), 200) or row.get("accountEmail") is not None
                        or row.get("lifecycle") not in {"active", "offboarding", "retired"}
                        or not isinstance(owner, dict) or not {"email", "source", "observedAt"} <= set(owner)
                        or not set(owner) <= {"email", "source", "observedAt", "humanPublicKeysHex"}
                        or not text(owner["email"], 254) or owner["source"] != "coreAccountContact"
                        or not timestamp(owner["observedAt"])):
                    raise ValueError("invalid Agent description")
                hints = owner.get("humanPublicKeysHex", [])
                if (not isinstance(hints, list) or any(not isinstance(key, str) or not HEX.fullmatch(key) for key in hints)
                        or not set(hints) <= set(request["participatingPublicKeysHex"])):
                    raise ValueError("invalid human key hint")
            else:
                raise ValueError("invalid description kind")
        elif state not in {"notShared", "unknown", "ambiguous"} or any(row.get(k) is not None for k in (
                "kind", "displayName", "accountEmail", "lifecycle", "responsibleAccount", "source")):
            raise ValueError("invalid bare description")
        sourced = isinstance(source, dict) and source.get("kind") == "operatorScopedRecord"
        if record is None:
            if sourced or annotation is not None:
                raise ValueError("missing operator provenance")
        else:
            if (not isinstance(record, dict) or not {"state", "revision", "observedAt"} <= set(record)
                    or not set(record) <= {"state", "revision", "observedAt", "reason"}
                    or not text(record["revision"], 128) or not timestamp(record["observedAt"])):
                raise ValueError("invalid operator provenance")
            if record["state"] == "used":
                used += 1
                if record.get("reason") is not None or (annotation is not None) == sourced:
                    raise ValueError("ambiguous operator provenance")
                if sourced:
                    if (state != "resolved" or source["revision"] != record["revision"]
                            or source["observedAt"] != record["observedAt"]
                            or (owner or {}).get("humanPublicKeysHex", [])):
                        raise ValueError("invalid operator description")
                elif (state != "notShared" or not isinstance(annotation, dict)
                      or set(annotation) != {"kind", "label", "observedAt"}
                      or annotation["kind"] not in {"historical_identity", "invitation_key"}
                      or not text(annotation["label"], 120)
                      or any(not 32 <= ord(char) <= 126 for char in annotation["label"])
                      or annotation["observedAt"] != record["observedAt"]):
                    raise ValueError("invalid annotation")
            elif record["state"] in {"conflict", "suppressed"}:
                reasons = CONFLICTS if record["state"] == "conflict" else {"normalResultPrecedence", "contactUnavailable"}
                if sourced or annotation is not None or record.get("reason") not in reasons:
                    raise ValueError("invalid withheld provenance")
                conflicts += record["state"] == "conflict"
            else:
                raise ValueError("invalid operator state")
        if row["publicKeyHex"] not in request["participatingPublicKeysHex"]:
            is_used = record is not None and record["state"] == "used"
            missing = state == "unknown" and record is not None and record.get("reason") == "contactUnavailable" and record["state"] == "suppressed"
            if not is_used and state != "notShared" and not missing:
                raise ValueError("nonparticipant disclosure")
    return used, conflicts


def http_post(bind: str, credential: str) -> Callable:
    """Literal loopback only; no DNS, proxy, redirect, or credential in errors."""
    host, separator, port = bind.rpartition(":")
    host = host.removeprefix("[").removesuffix("]")
    if (not separator or not ipaddress.ip_address(host).is_loopback
            or not port.isascii() or not port.isdigit() or not 1 <= int(port) <= 65535
            or not text(credential, 4096) or not credential.isascii()):
        raise ValueError("invalid private endpoint configuration")

    def post(payload: bytes, seconds: float) -> tuple[int, bytes]:
        connection = http.client.HTTPConnection(host, int(port), timeout=seconds)
        deadline = time.monotonic() + seconds
        timer = None
        try:
            connection.connect()
            transport = connection.sock
            def expire():
                try:
                    transport.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError()
            # Socket timeouts alone are per read: a slow header drip could
            # otherwise keep getresponse alive indefinitely.
            timer = threading.Timer(remaining, expire)
            timer.daemon = True
            timer.start()
            connection.request("POST", PATH, body=payload, headers={HEADER: credential, "Content-Type": "application/json"})
            response = connection.getresponse()
            chunks, count = [], 0
            while count <= MAX_BYTES:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError()
                if transport:
                    transport.settimeout(remaining)
                chunk = response.read1(min(8192, MAX_BYTES + 1 - count))
                if not chunk:
                    break
                chunks.append(chunk)
                count += len(chunk)
            if count > MAX_BYTES:
                raise ValueError("response exceeds bound")
            return response.status, b"".join(chunks)
        finally:
            if timer is not None:
                timer.cancel()
                timer.join()
            connection.close()
    return post


def collect(brain: dict, brain_id: str, environment: dict, *, post: Callable | None = None,
            clock: Callable = time.monotonic) -> dict:
    """Summarize exact pages; failure never claims v3 or prints response/errors."""
    base = {"servedVersion": None, "checkedKeys": 0, "operatorUsedKeys": 0, "conflictKeys": 0,
            "viewerVerified": False}
    if not all(environment.get(key) for key in ENV_NAMES):
        return {**base, "state": "notConfigured"}
    started = clock()
    try:
        entries = [row for row in roster_keys(brain["tables"]).values() if row["brainId"] == brain_id]
        entries.sort(key=lambda row: row["npub"])
        admins = [row for row in entries if row["roles"] & {"admin", "owner"}]
        requester = next((npub_to_hex(row["npub"]) for row in admins if npub_to_hex(row["npub"])), None)
        if requester is None or not entries or any(npub_to_hex(row["npub"]) is None for row in entries):
            return {**base, "state": "notChecked", "reason": "no complete canonical roster and audited admin"}
        sender = post or http_post(environment["FC_CORE_BRAIN_IDENTITY_BIND"], environment["FC_CORE_BRAIN_DESCRIPTION_TOKEN"])
        for start in range(0, len(entries), 100):
            page = entries[start:start + 100]
            request = {"version": VERSION, "brainServer": environment["FC_CORE_BRAIN_IDENTITY_BRAIN_SERVER"],
                       "brainId": brain_id, "requestedByPublicKeyHex": requester,
                       "publicKeysHex": [npub_to_hex(row["npub"]) for row in page],
                       "participatingPublicKeysHex": [npub_to_hex(row["npub"]) for row in page if row["participation"]]}
            remaining = BUDGET - (clock() - started)
            if remaining <= 0:
                raise TimeoutError()
            status, payload = sender(json.dumps(request).encode(), min(PAGE_SECONDS, remaining))
            if status != 200 or len(payload) > MAX_BYTES or clock() - started >= BUDGET:
                raise ValueError("endpoint did not serve bounded v3")
            used, conflicts = validate(request, json.loads(payload))
            base["checkedKeys"] += len(page)
            base["operatorUsedKeys"] += used
            base["conflictKeys"] += conflicts
        return {**base, "state": "checked", "servedVersion": VERSION}
    except (ValueError, TypeError, KeyError, OSError, http.client.HTTPException):
        return {**base, "state": "unavailable", "reason": "v3 request failed or returned invalid evidence"}

"""Opt-in operator roster probe: what platform records say about exact Brain keys.

Read-only. Brain facts come from a private scratch copy of the Brain SQLite
store (never the live or snapshot file); Core facts come from one REPEATABLE
READ, read-only transaction. The two reads are not coordinated, so each
carries its own observation time.

Coverage is deliberately narrow: stored Brain roles and membership
provenance, current-version grant presence, exact-key participation facts,
and Core runtime pins, account key associations and raw account/Brain
sharing-scope rows. It is not Folder entitlement, readability or the Brain
admin access report, and it does not evaluate Core's disclosure policy.

Keys that only admitted a member or issued a grant are looked up as
reference keys. That never makes them roster members and never makes them
the owner or controller of the keys they admitted.

Keys Core cannot name get two more sources, both operator evidence only:
the caller-provided owner_chat_account_id on agent creation requests (same
Core transaction), and the account_id/device_id columns of candidate
accounts' Hosted Chat stores (private scratch copies). Neither changes the
Core classification, proves control, or records consent.

Never read: wrapped Folder Key Grants, access-change events, encrypted
content or device-state ciphertext, identity or key files, sealed Hosted
Device bindings, the Identity Directory database, retired authority tables.
"""

from __future__ import annotations

import json
import re
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable

SCHEMA_VERSION = "finite.brain-roster-status.v1"
BRAIN_DATABASE = Path("/var/lib/private/finitebrain/finite-brain.sqlite3")

MAX_BRAINS = 2_000
# Roster plus reference keys; also keeps the psql --set JSON argument far
# below the 128 KiB per-argument limit.
MAX_KEYS = 512
MAX_ROWS = 50_000
MAX_GRANTS_PER_KEY = 100
MAX_REPORT_BYTES = 32 * 1024 * 1024
# Text bounds applied in SQL; longer values arrive as NULL plus a flag.
MAX_KEY_CHARS = 128
MAX_NAME_CHARS = 256
MAX_PATH_CHARS = 1024
MAX_EMAIL_BYTES = 254

BRAIN_ID = re.compile(r"^[A-Za-z0-9_-]{1,128}$")
_BECH32 = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
NPUB_LENGTH = 63


class RosterError(Exception):
    """Evidence could not be collected; the probe fails closed."""


# Hosted Chat client stores: <root>/users/<sha256(WorkOS user id)>/chat/client.sqlite3
# (finitechat-hosted-device user_root/chat_data_dir; DynamicUser state dir).
HOSTED_DATA_ROOT = Path("/var/lib/private/finitechat-hosted-device")
HOSTED_DEVICE_ID = "hosted-web"
MAX_HOSTED_CANDIDATES = 64
MAX_HOSTED_STORE_BYTES = 1 << 30
MAX_HOSTED_TOTAL_BYTES = 8 << 30
HOSTED_BUDGET_SECONDS = 300
STORAGE_ID = re.compile(r"^[0-9a-f]{64}$")

# Identity Directory operator inspect: loopback full router, exact key only.
DIRECTORY_INSPECT_URL = "http://127.0.0.1:8790/api/v1/operator/inspect"
DIRECTORY_TOKEN_FILE = Path("/etc/finite/identity-operator.env")
DIRECTORY_TOKEN_VARIABLE = "FINITE_IDENTITY_OPERATOR_TOKEN"
DIRECTORY_REQUEST_TIMEOUT_SECONDS = 3
DIRECTORY_BUDGET_SECONDS = 90
MAX_DIRECTORY_RESPONSE_BYTES = 64 * 1024
MAX_BINDINGS_PER_KEY = 16
MAX_CANDIDATE_NAMES = 1024
VIP_NAME = re.compile(r"^[a-z0-9][a-z0-9._+-]{0,63}@[a-z0-9][a-z0-9.-]{0,188}$")


class DirectoryUnavailable(Exception):
    """One Directory request failed; the message never carries credentials."""


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args: Any, **kwargs: Any) -> None:
        return None


def http_directory_post(token: str) -> Callable[[bytes], tuple[int, bytes]]:
    """POST one inspect request to the loopback Directory. No ambient proxy,
    no redirects; the token travels only in its header."""
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), _NoRedirect())

    def exchange(body: bytes) -> tuple[int, bytes]:
        request = urllib.request.Request(
            DIRECTORY_INSPECT_URL, data=body, method="POST",
            headers={"content-type": "application/json", "x-finite-operator-token": token},
        )
        try:
            with opener.open(request, timeout=DIRECTORY_REQUEST_TIMEOUT_SECONDS) as response:
                return response.status, response.read(MAX_DIRECTORY_RESPONSE_BYTES + 1)
        except urllib.error.HTTPError as error:
            # Reading or closing the error body can fail too; that failure
            # propagates to post(), which redacts it.
            try:
                return error.code, error.read(MAX_DIRECTORY_RESPONSE_BYTES + 1)
            finally:
                error.close()

    def post(body: bytes) -> tuple[int, bytes]:
        failure = None
        try:
            return exchange(body)
        except Exception as error:  # noqa: BLE001 - redact every transport failure
            # Class name only: transport errors can echo header values.
            failure = type(error).__name__
        # Raised outside the handler so no original exception (which may
        # hold the header value) is kept as context.
        raise DirectoryUnavailable(failure)

    return post


DIRECTORY_TOKEN = re.compile(r"^[\x21-\x7e]{16,512}$")


def valid_directory_token(token: Any) -> bool:
    """A bounded, printable-ASCII token with no spaces or control characters."""
    return isinstance(token, str) and DIRECTORY_TOKEN.fullmatch(token) is not None


def directory_result(npub: str, status: int, body: bytes) -> dict[str, Any]:
    """Select kind/pubkey/vip_emails from one inspect answer. The echoed key
    must equal the requested key exactly; anything else is invalid."""
    invalid = lambda reason: {"state": "invalidResponse", "reason": reason}  # noqa: E731
    if status not in (200, 404):
        return {"state": "unavailable", "reason": f"status {status}"}
    if len(body) > MAX_DIRECTORY_RESPONSE_BYTES:
        return invalid("response over byte bound")
    try:
        payload = json.loads(body)
    except (ValueError, UnicodeDecodeError):
        return invalid("response is not JSON")
    if not isinstance(payload, dict):
        return invalid("response is not an object")
    if status == 404:
        return {"state": "noBinding"} if payload.get("error") == "principal_not_found" else invalid(
            "unexpected 404 body")
    if payload.get("kind") != "native" or payload.get("pubkey") != npub_to_hex(npub):
        return invalid("kind or echoed key does not match the request")
    rows = payload.get("vip_emails")
    if not isinstance(rows, list) or not rows:
        return invalid("vip_emails missing or empty")
    if len(rows) > MAX_BINDINGS_PER_KEY:
        return invalid("too many bindings")
    bindings = []
    for row in rows:
        if not isinstance(row, dict):
            return invalid("binding is not an object")
        name, disabled = row.get("email"), row.get("disabled")
        created_at, disabled_at = row.get("created_at"), row.get("disabled_at")
        if (not isinstance(name, str) or not VIP_NAME.fullmatch(name)
                or not isinstance(disabled, bool) or not isinstance(created_at, int)
                or not (disabled_at is None or isinstance(disabled_at, int))
                or disabled != (disabled_at is not None)):
            return invalid("binding fields are malformed")
        bindings.append({"name": name, "disabled": disabled,
                         "createdAt": created_at, "disabledAt": disabled_at})
    active = [binding for binding in bindings if not binding["disabled"]]
    return {
        "state": "bound" if active else "disabledOnly",
        "multipleActiveNames": len(active) > 1,
        "bindings": bindings,
    }


def collect_directory(
    npubs: list[str],
    post: Callable[[bytes], tuple[int, bytes]],
    clock: Callable[[], float] = time.monotonic,
) -> dict[str, Any]:
    """One exact-key inspect per canonical key, inside one overall budget."""
    started = clock()
    results: dict[str, dict[str, Any]] = {}
    budget_exhausted = False
    for npub in npubs:
        if npub_to_hex(npub) is None:
            continue
        if clock() - started > DIRECTORY_BUDGET_SECONDS:
            budget_exhausted = True
            results[npub] = {"state": "unavailable", "reason": "Directory budget exhausted"}
            continue
        try:
            status, body = post(json.dumps({"identifier": npub}).encode())
        except DirectoryUnavailable as error:
            results[npub] = {"state": "unavailable", "reason": str(error)}
            continue
        results[npub] = directory_result(npub, status, body)
    return {"state": "checked", "checked_at": _now(), "results": results,
            "budget_exhausted": budget_exhausted}


def candidate_names(directory: dict[str, Any] | None) -> list[str]:
    """Every name (active or disabled) the Directory bound to a selected key."""
    names = sorted({binding["name"] for result in (directory or {}).get("results", {}).values()
                    for binding in result.get("bindings", [])})
    if len(names) > MAX_CANDIDATE_NAMES:
        raise RosterError(f"more than {MAX_CANDIDATE_NAMES} Directory names")
    return names


def name_alias_query(names: list[str]) -> str:
    """Stored Brain aliases for exact candidate names, on any key, including
    keys outside the selected roster. Names are VIP_NAME-validated literals."""
    if any(not VIP_NAME.fullmatch(name) for name in names):
        raise RosterError("alias name lookup accepts only validated Directory names")
    listed = ", ".join(f"'{name}'" for name in names) or "NULL"
    return (f"SELECT {_text('npub', MAX_KEY_CHARS)}, preferred_nip05, nip05_verified_at,"
            f" updated_at FROM identity_aliases WHERE preferred_nip05 IN ({listed})"
            f" ORDER BY preferred_nip05, npub LIMIT {MAX_CANDIDATE_NAMES * 4 + 1};")


def npub_to_hex(value: Any) -> str | None:
    """Strict canonical NIP-19 `npub`: exact length, lowercase, valid
    checksum, 32-byte payload. Anything else is reported invalid and never
    guessed, normalised or matched."""
    if not isinstance(value, str) or len(value) != NPUB_LENGTH or not value.startswith("npub1"):
        return None
    data = value[5:]
    if any(char not in _BECH32 for char in data):
        return None
    values = [_BECH32.index(char) for char in data]
    expanded = [ord(char) >> 5 for char in "npub"] + [0] + [ord(char) & 31 for char in "npub"]
    if _bech32_polymod(expanded + values) != 1:
        return None
    accumulator, bits, output = 0, 0, []
    for item in values[:-6]:
        accumulator = (accumulator << 5) | item
        bits += 5
        while bits >= 8:
            bits -= 8
            output.append((accumulator >> bits) & 0xFF)
    if bits >= 5 or (accumulator & ((1 << bits) - 1)) or len(output) != 32:
        return None
    return bytes(output).hex()


def _bech32_polymod(values: list[int]) -> int:
    generator = [0x3B6A57B2, 0x26508E6D, 0x1EA119FA, 0x3D4233DD, 0x2A1462B3]
    checksum = 1
    for value in values:
        top = checksum >> 25
        checksum = (checksum & 0x1FFFFFF) << 5 ^ value
        for index in range(5):
            checksum ^= generator[index] if (top >> index) & 1 else 0
    return checksum


def _text(column: str, limit: int, alias: str | None = None) -> str:
    """SQLite: the value if within `limit` characters, otherwise NULL."""
    name = alias or column.split(".")[-1]
    return f"CASE WHEN length({column}) <= {limit} THEN {column} END AS {name}"


def _where(column: str, brain_id: str | None, joiner: str = "WHERE") -> str:
    # The Brain id is validated against BRAIN_ID before it is embedded.
    return f"{joiner} {column} = '{brain_id}'" if brain_id else ""


def brain_queries(brain_id: str) -> dict[str, str]:
    """Read-only SELECTs for one exact Brain. No wrapped grant, event or
    content column appears."""
    if not isinstance(brain_id, str) or not BRAIN_ID.fullmatch(brain_id):
        raise RosterError("Brain id must be 1-128 characters of [A-Za-z0-9_-]")
    limit = MAX_ROWS + 1
    key = lambda column, alias=None: _text(column, MAX_KEY_CHARS, alias)  # noqa: E731
    over = lambda column: f"length({column}) > {MAX_KEY_CHARS}"  # noqa: E731
    in_brain = f"brain_id = '{brain_id}'"
    return {
        # Any oversized key value makes the whole roster fail closed rather
        # than merging distinct keys or silently dropping a grantor.
        "oversized_keys": "SELECT"
            f" (SELECT COUNT(*) FROM brains WHERE id = '{brain_id}' AND {over('owner_user_id')})"
            f" + (SELECT COUNT(*) FROM brain_members WHERE {in_brain}"
            f" AND ({over('user_id')} OR {over('delegated_by_npub')}))"
            f" + (SELECT COUNT(*) FROM brain_admins WHERE {in_brain} AND {over('user_id')})"
            f" + (SELECT COUNT(*) FROM personal_agents WHERE {in_brain}"
            f" AND ({over('owner_npub')} OR {over('agent_npub')}))"
            f" + (SELECT COUNT(*) FROM folder_access WHERE {in_brain} AND {over('user_id')})"
            f" + (SELECT COUNT(*) FROM folder_key_grants WHERE {in_brain} AND"
            f" ({over('recipient_npub')} OR {over('issuer_npub')} OR {over('delegated_by_npub')}))"
            " AS oversized;",
        "brains": f"SELECT id, kind, {_text('name', MAX_NAME_CHARS)}, {key('owner_user_id')},"
                  f" created_at FROM brains {_where('id', brain_id)}"
                  f" ORDER BY id LIMIT {MAX_BRAINS + 1};",
        "folders": f"SELECT brain_id, id, {_text('path', MAX_PATH_CHARS)}, current_key_version"
                   f" FROM folders {_where('brain_id', brain_id)} ORDER BY brain_id, id LIMIT {limit};",
        "members": f"SELECT brain_id, {key('user_id')}, origin_kind, {key('delegated_by_npub')},"
                   f" {_text('origin_ref', MAX_NAME_CHARS)} FROM brain_members"
                   f" {_where('brain_id', brain_id)} ORDER BY brain_id, user_id LIMIT {limit};",
        "admins": f"SELECT brain_id, {key('user_id')} FROM brain_admins"
                  f" {_where('brain_id', brain_id)} ORDER BY brain_id, user_id LIMIT {limit};",
        "personal_agents": f"SELECT brain_id, {key('owner_npub')}, {key('agent_npub')}"
                           f" FROM personal_agents WHERE status = 'active'"
                           f" {_where('brain_id', brain_id, 'AND')} ORDER BY brain_id LIMIT {limit};",
        "folder_access": f"SELECT brain_id, folder_id, {key('user_id')} FROM folder_access"
                         f" {_where('brain_id', brain_id)} ORDER BY brain_id, folder_id, user_id"
                         f" LIMIT {limit};",
        # Current-version grant metadata only: never wrapped_event_json or
        # access_change_event_json.
        "current_grants": f"SELECT g.brain_id, g.folder_id, g.key_version,"
                          f" {key('g.recipient_npub')}, {key('g.issuer_npub')},"
                          f" {key('g.delegated_by_npub')}, g.origin_kind,"
                          f" {_text('g.origin_ref', MAX_NAME_CHARS)}, g.created_at"
                          " FROM folder_key_grants g JOIN folders f"
                          " ON f.brain_id = g.brain_id AND f.id = g.folder_id"
                          " AND g.key_version = f.current_key_version"
                          f" {_where('g.brain_id', brain_id)}"
                          f" ORDER BY g.brain_id, g.recipient_npub, g.folder_id LIMIT {limit};",
        # Exact-key participation, the same sources the access report uses.
        # Admin-written rows and approval targets are never participation.
        "participation": "SELECT brain_id, npub, kind, MIN(at) AS first_at FROM ("
                         " SELECT brain_id, redeemed_by_npub AS npub, 'inviteTokenRedemption' AS kind,"
                         " redeemed_at AS at FROM brain_invite_tokens WHERE redeemed_at IS NOT NULL"
                         " UNION ALL SELECT brain_id, user_id, 'invitationAcceptance', accepted_at"
                         " FROM brain_invitations WHERE status = 'accepted' AND target_kind = 'npub'"
                         " AND accepted_at IS NOT NULL"
                         " AND (claimed_by_npub IS NULL OR claimed_by_npub = user_id)"
                         " UNION ALL SELECT brain_id, recipient_npub, 'folderInvitationAcceptance',"
                         " accepted_at FROM share_links WHERE status IN ('accepted', 'revoked')"
                         " AND accepted_at IS NOT NULL"
                         " UNION ALL SELECT source_brain_id, destination_admin_npub,"
                         " 'mountOfferAcceptance', accepted_at FROM shared_folder_invitations"
                         " WHERE status IN ('accepted', 'revoked') AND accepted_at IS NOT NULL"
                         " UNION ALL SELECT brain_id, actor_npub, 'authenticatedBrainAction',"
                         f" accepted_at FROM brain_record_index {_where('brain_id', brain_id)}"
                         " UNION ALL SELECT brain_id, signer_npub, 'appliedApproval', applied_at"
                         " FROM brain_approval_nonces)"
                         f" WHERE length(npub) <= {MAX_KEY_CHARS} {_where('brain_id', brain_id, 'AND')}"
                         f" GROUP BY brain_id, npub, kind ORDER BY brain_id, npub, kind LIMIT {limit};",
    }


def alias_query(npubs: list[str]) -> str:
    """Stored aliases for the selected keys only. Every key is a strictly
    decoded canonical npub, so it is safe to embed as a literal."""
    if any(npub_to_hex(npub) is None for npub in npubs):
        raise RosterError("alias lookup accepts only canonical npubs")
    listed = ", ".join(f"'{npub}'" for npub in npubs) or "NULL"
    return (f"SELECT npub, {_text('preferred_nip05', MAX_EMAIL_BYTES)}, nip05_verified_at"
            f" FROM identity_aliases WHERE npub IN ({listed}) AND preferred_nip05 IS NOT NULL"
            f" ORDER BY npub LIMIT {MAX_KEYS + 1};")


CORE_QUERY = """
SELECT to_regclass('account_brain_principals') IS NOT NULL AS finite_has_brain_identity \\gset
WITH keys AS (SELECT value AS npub FROM jsonb_array_elements_text(:'npubs'::jsonb)),
agent_rows AS (
  SELECT r.health_reporting_npub AS npub, r.id AS runtime_id, r.project_id,
    p.id IS NOT NULL AS project_present,
    CASE WHEN octet_length(p.display_name) <= :name_limit THEN p.display_name END AS display_name,
    CASE WHEN octet_length(p.agent_email) <= :email_limit THEN p.agent_email END AS agent_email,
    p.owner_user_id, r.offboarding_phase,
    EXISTS (SELECT 1 FROM project_runtime_links l WHERE l.agent_runtime_id = r.id
            AND l.project_id = r.project_id AND l.active) AS active_link,
    EXISTS (SELECT 1 FROM project_runtime_links l WHERE l.agent_runtime_id = r.id
            AND l.project_id IS DISTINCT FROM r.project_id AND l.active) AS foreign_active_link,
    core_rfc3339(r.health_reported_at) AS health_reported_at
  FROM agent_runtimes r JOIN keys k ON k.npub = r.health_reporting_npub
  LEFT JOIN projects p ON p.id = r.project_id
  ORDER BY r.id LIMIT :row_limit
)
SELECT json_build_object(
  'checked_at', core_rfc3339(now()),
  'brain_identity_schema', :'finite_has_brain_identity'::boolean,
  'agents', COALESCE((SELECT json_agg(json_build_object(
      'npub', a.npub, 'runtime', a.runtime_id, 'project', a.project_id,
      'project_present', a.project_present, 'display_name', a.display_name,
      'agent_email', a.agent_email,
      'account', a.owner_user_id,
      'offboarding_phase', a.offboarding_phase, 'active_link', a.active_link,
      'foreign_active_link', a.foreign_active_link,
      'health_reported_at', a.health_reported_at)) FROM agent_rows a), '[]'::json),
  'accounts', COALESCE((SELECT json_agg(json_build_object(
      'account', u.id,
      'email', CASE WHEN octet_length(u.normalized_email) <= :email_limit THEN u.normalized_email END,
      'link_status', u.link_status))
    FROM users u WHERE u.id IN (SELECT DISTINCT owner_user_id FROM agent_rows)), '[]'::json)
) AS core;
\\if :finite_has_brain_identity
\\set association_filter 'AND NOT EXISTS (SELECT 1 FROM account_brain_principals a WHERE a.public_key_hex = p.hex)'
WITH hexes AS (SELECT value AS hex FROM jsonb_array_elements_text(:'hexes'::jsonb)),
brains AS (SELECT value AS brain_id FROM jsonb_array_elements_text(:'brain_ids'::jsonb)),
keys AS (SELECT value AS npub FROM jsonb_array_elements_text(:'npubs'::jsonb)),
human_rows AS (
  SELECT a.public_key_hex, a.user_id, a.status, core_rfc3339(a.first_observed_at) AS first_observed_at,
    core_rfc3339(a.last_observed_at) AS last_observed_at
  FROM account_brain_principals a JOIN hexes h ON h.hex = a.public_key_hex
  ORDER BY a.public_key_hex LIMIT :row_limit
), implicated AS (
  SELECT user_id FROM human_rows
  UNION
  SELECT p.owner_user_id FROM agent_runtimes r JOIN keys k ON k.npub = r.health_reporting_npub
  JOIN projects p ON p.id = r.project_id
), scope_rows AS (
  SELECT s.user_id, s.brain_server, s.brain_id, s.established_at, s.revoked_at
  FROM account_brain_sharing_scopes s JOIN brains b ON b.brain_id = s.brain_id
  WHERE s.user_id IN (SELECT user_id FROM implicated)
  ORDER BY s.user_id, s.brain_id, s.established_at LIMIT :row_limit
)
SELECT json_build_object(
  'humans', COALESCE((SELECT json_agg(json_build_object(
      'hex', h.public_key_hex, 'account', h.user_id,
      'status', h.status, 'first_observed_at', h.first_observed_at,
      'last_observed_at', h.last_observed_at)) FROM human_rows h), '[]'::json),
  'accounts', COALESCE((SELECT json_agg(json_build_object(
      'account', u.id,
      'email', CASE WHEN octet_length(u.normalized_email) <= :email_limit THEN u.normalized_email END,
      'link_status', u.link_status))
    FROM users u WHERE u.id IN (SELECT DISTINCT user_id FROM human_rows)), '[]'::json),
  'scopes', COALESCE((SELECT json_agg(json_build_object(
      'account', s.user_id,
      'brain_server', CASE WHEN octet_length(s.brain_server) <= :name_limit THEN s.brain_server END,
      'brain_id', s.brain_id, 'established_at', core_rfc3339(s.established_at),
      'revoked_at', core_rfc3339(s.revoked_at))) FROM scope_rows s), '[]'::json),
  'hosted_candidates', COALESCE((SELECT json_agg(json_build_object(
      'account', c.id, 'storage', c.storage, 'link_status', c.link_status)) FROM (
      SELECT u.id, u.link_status, encode(sha256(convert_to(u.workos_user_id, 'UTF8')), 'hex') AS storage
      FROM users u WHERE u.id IN (SELECT user_id FROM human_rows) AND u.workos_user_id IS NOT NULL
      ORDER BY u.id LIMIT :candidate_limit) c), '[]'::json)
) AS identity;
\\else
\\set association_filter ''
\\endif
WITH pairs AS (SELECT value->>'npub' AS npub, value->>'hex' AS hex
               FROM jsonb_array_elements(:'key_pairs'::jsonb)),
remaining AS (
  -- Keys Core cannot name today: no runtime pin and no account association.
  SELECT p.hex FROM pairs p
  WHERE NOT EXISTS (SELECT 1 FROM agent_runtimes r WHERE r.health_reporting_npub = p.npub)
  :association_filter
),
requests AS (
  SELECT r.id, r.owner_chat_account_id AS hex, r.owner_user_id, r.project_id, r.agent_runtime_id,
    r.status, core_rfc3339(r.created_at) AS created_at, core_rfc3339(r.updated_at) AS updated_at
  FROM agent_creation_requests r JOIN remaining k ON k.hex = r.owner_chat_account_id
  ORDER BY r.id LIMIT :row_limit
), candidates AS (
  SELECT owner_user_id AS user_id FROM requests
  UNION
  SELECT p.owner_user_id FROM agent_runtimes r JOIN pairs k ON k.npub = r.health_reporting_npub
  JOIN projects p ON p.id = r.project_id
)
SELECT json_build_object(
  'launch_chat_keys', COALESCE((SELECT json_agg(json_build_object(
      'hex', q.hex, 'request', q.id, 'account', q.owner_user_id, 'project', q.project_id,
      'runtime', q.agent_runtime_id, 'status', q.status, 'created_at', q.created_at,
      'updated_at', q.updated_at)) FROM requests q), '[]'::json),
  'accounts', COALESCE((SELECT json_agg(json_build_object(
      'account', u.id,
      'email', CASE WHEN octet_length(u.normalized_email) <= :email_limit THEN u.normalized_email END,
      'link_status', u.link_status))
    FROM users u WHERE u.id IN (SELECT DISTINCT owner_user_id FROM requests)), '[]'::json),
  'hosted_candidates', COALESCE((SELECT json_agg(json_build_object(
      'account', c.id, 'storage', c.storage, 'link_status', c.link_status)) FROM (
      SELECT u.id, u.link_status, encode(sha256(convert_to(u.workos_user_id, 'UTF8')), 'hex') AS storage
      FROM users u WHERE u.id IN (SELECT user_id FROM candidates) AND u.workos_user_id IS NOT NULL
      ORDER BY u.id LIMIT :candidate_limit) c), '[]'::json)
) AS launch;
"""


def _bounded(name: str, rows: Any, bound: int | None = None) -> list[dict[str, Any]]:
    bound = MAX_ROWS if bound is None else bound
    if not isinstance(rows, list) or any(not isinstance(row, dict) for row in rows):
        raise RosterError(f"{name} returned an unexpected shape")
    if len(rows) > bound:
        raise RosterError(f"{name} exceeded its {bound}-row bound")
    return rows


def _now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def collect_brain(
    database: Path,
    brain_id: str,
    scratch_copy: Callable[[Path], Any],
    query: Callable[[Path, str], list[dict[str, Any]]],
    directory: Callable[[list[str]], dict[str, Any]] | None = None,
) -> dict[str, Any]:
    """Read every roster table from one scratch copy of the Brain store. When
    a Directory lookup is supplied, its candidate names are checked against
    stored aliases in the same copy."""
    queries = brain_queries(brain_id)
    copy_started = _now()
    tables: dict[str, list[dict[str, Any]]] = {}
    with scratch_copy(database) as scratch:
        copied = _now()
        for name, sql in queries.items():
            bound = 1 if name == "oversized_keys" else MAX_BRAINS if name == "brains" else MAX_ROWS
            tables[name] = _bounded(name, query(scratch, sql), bound)
        oversized = tables.pop("oversized_keys")
        if len(oversized) != 1 or oversized[0].get("oversized") != 0:
            raise RosterError(f"keys longer than {MAX_KEY_CHARS} characters are present; "
                              "refusing to merge or drop them")
        if not tables["brains"]:
            raise RosterError(f"Brain {brain_id} was not found in the scratch copy")
        tables["aliases"] = _bounded(
            "aliases", query(scratch, alias_query(selected_npubs(tables))), MAX_KEYS)
        directory_evidence = directory(selected_npubs(tables)) if directory else None
        names = candidate_names(directory_evidence)
        tables["name_aliases"] = _bounded(
            "name aliases", query(scratch, name_alias_query(names)), MAX_CANDIDATE_NAMES * 4
        ) if names else []
    return {
        "source": str(database),
        "copy_started_at": copy_started,
        "copied_at": copied,
        "tables": tables,
        "directory": directory_evidence,
    }


def roster_keys(tables: dict[str, list[dict[str, Any]]]) -> dict[tuple[str, str], dict[str, Any]]:
    """One entry per (brain, exact key) holding a stored role or current grant."""
    keys: dict[tuple[str, str], dict[str, Any]] = {}
    known_brains = {row["id"] for row in tables["brains"]}

    def entry(brain: str, npub: Any) -> dict[str, Any] | None:
        if brain not in known_brains:
            return None
        if not isinstance(npub, str) or not npub:
            raise RosterError("a stored role or grant has a missing key; refusing to merge it")
        return keys.setdefault((brain, npub), {
            "brainId": brain, "npub": npub, "roles": set(), "membership": None,
            "grants": [], "participation": [],
        })

    for brain in tables["brains"]:
        if brain.get("kind") == "personal" and (row := entry(brain["id"], brain.get("owner_user_id"))):
            row["roles"].add("owner")
    for member in tables["members"]:
        if row := entry(member["brain_id"], member.get("user_id")):
            row["roles"].add("member")
            row["membership"] = {
                "origin": member.get("origin_kind"),
                "admittedBy": member.get("delegated_by_npub"),
                "originRef": member.get("origin_ref"),
            }
    for admin in tables["admins"]:
        if row := entry(admin["brain_id"], admin.get("user_id")):
            row["roles"].add("admin")
    for agent in tables["personal_agents"]:
        if row := entry(agent["brain_id"], agent.get("agent_npub")):
            row["roles"].add("personalAgent")
    for access in tables["folder_access"]:
        if row := entry(access["brain_id"], access.get("user_id")):
            row["roles"].add("explicitFolderAccess")
    paths = {(folder["brain_id"], folder["id"]): folder.get("path") for folder in tables["folders"]}
    for grant in tables["current_grants"]:
        if row := entry(grant["brain_id"], grant.get("recipient_npub")):
            row["grants"].append({
                "folderId": grant["folder_id"],
                "path": paths.get((grant["brain_id"], grant["folder_id"])),
                "keyVersion": grant["key_version"],
                "grantIssuedBy": grant.get("issuer_npub"),
                "grantDelegatedBy": grant.get("delegated_by_npub"),
                "origin": grant.get("origin_kind"),
                "originRef": grant.get("origin_ref"),
                "issuedAt": grant.get("created_at"),
            })
    for fact in tables["participation"]:
        row = keys.get((fact["brain_id"], fact["npub"]))
        if row is not None:
            row["participation"].append({"kind": fact["kind"], "firstAt": fact["first_at"]})
    for row in keys.values():
        if not row["roles"]:
            row["roles"].add("currentGrantOnly")
    if len(keys) > MAX_KEYS:
        raise RosterError(f"more than {MAX_KEYS} roster keys")
    return keys


def reference_keys(
    tables: dict[str, list[dict[str, Any]]], roster: dict[tuple[str, str], dict[str, Any]]
) -> dict[tuple[str, str], set[str]]:
    """Keys that only admitted members or issued/delegated current grants.
    Reference-only: never roster members, never owners of what they admitted."""
    references: dict[tuple[str, str], set[str]] = {}
    for member in tables["members"]:
        if member.get("delegated_by_npub"):
            references.setdefault((member["brain_id"], member["delegated_by_npub"]), set()).add(
                "admittedMembers")
    for grant in tables["current_grants"]:
        for column, label in (("issuer_npub", "issuedCurrentGrants"),
                              ("delegated_by_npub", "delegatedCurrentGrants")):
            if grant.get(column):
                references.setdefault((grant["brain_id"], grant[column]), set()).add(label)
    return {key: labels for key, labels in references.items() if key not in roster}


def core_identity(
    npub: str,
    core: dict[str, Any],
    accounts: dict[str, dict[str, Any]],
) -> dict[str, Any]:
    """Every Core record that names this exact key. Conflicts never pick."""
    hex_key = npub_to_hex(npub)
    if hex_key is None:
        return {"state": "keyInvalid"}

    def account(ref: Any) -> dict[str, Any]:
        found = accounts.get(ref or "")
        return dict(found) if found else {"account": ref, "email": None, "link_status": None}

    pins = [row for row in core.get("agents", []) if row.get("npub") == npub]
    associations = [row for row in core.get("humans", []) if row.get("hex") == hex_key]
    agent_pins = [{
        "runtimeId": row.get("runtime"),
        "projectId": row.get("project"),
        "projectPresent": row.get("project_present"),
        "displayName": row.get("display_name"),
        "agentEmail": row.get("agent_email"),
        "currentOwnerAccount": account(row.get("account")) if row.get("account") else None,
        "offboardingPhase": row.get("offboarding_phase"),
        "activeLink": row.get("active_link"),
        "foreignActiveLink": row.get("foreign_active_link"),
        "healthReportedAt": row.get("health_reported_at"),
    } for row in pins]
    human_associations = [{
        "account": account(row.get("account")),
        "status": row.get("status"),
        "firstObservedAt": row.get("first_observed_at"),
        "lastObservedAt": row.get("last_observed_at"),
    } for row in associations]
    identity: dict[str, Any] = {"agentPins": agent_pins, "humanAssociations": human_associations}
    active_humans = [row for row in associations if row.get("status") == "active"]
    if not pins and not associations:
        identity["state"] = "noCoreRecord"
    elif pins and active_humans:
        identity["state"] = "conflict"
        identity["reason"] = "agent pin and human association on one key"
    elif pins:
        projects = {row.get("project") for row in pins}
        owners = {row.get("account") for row in pins}
        if any(not row.get("project_present") or row.get("foreign_active_link") for row in pins):
            identity["state"] = "inconsistent"
        elif len(projects) > 1 or len(owners) > 1:
            identity["state"] = "conflict"
            identity["reason"] = "pin on several projects or owner accounts"
        else:
            identity["state"] = "agentPin"
            # A probe-derived summary of the raw fields in agentPins; not
            # Core's description policy.
            identity["probeDerivedLifecycle"] = _lifecycle(pins)
    elif active_humans:
        identity["state"] = "humanAssociation"
    else:
        identity["state"] = "retiredAssociationOnly"
    return identity


def _lifecycle(pins: list[dict[str, Any]]) -> str:
    live = [row for row in pins if row.get("active_link")]
    if live:
        phase = live[0].get("offboarding_phase")
        if phase is None:
            return "live"
        if phase in ("retirement_requested", "receipt_verified", "compute_removed"):
            return "offboarding"
        return "notDeterminable"
    if all(row.get("offboarding_phase") in ("link_deactivated", "archived") for row in pins):
        return "retired"
    return "notDeterminable"


def sharing_scopes(identity: dict[str, Any], brain_id: str, scopes: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Raw scope rows for every account this key's Core records name, on this
    exact Brain id. Evidence only; no disclosure decision is derived."""
    implicated = {pin["currentOwnerAccount"]["account"] for pin in identity.get("agentPins", [])
                  if pin.get("currentOwnerAccount")}
    implicated |= {row["account"]["account"] for row in identity.get("humanAssociations", [])}
    return [scope for scope in scopes
            if scope.get("account") in implicated and scope.get("brain_id") == brain_id]


def unknown_hexes(brain: dict[str, Any], core: dict[str, Any]) -> list[str]:
    """Canonical keys for which Core holds no pin and no association."""
    accounts = {row["account"]: row for row in core.get("accounts", [])}
    tables = brain["tables"]
    roster = roster_keys(tables)
    references = reference_keys(tables, roster)
    npubs = {npub for _, npub in roster} | {npub for _, npub in references}
    return sorted(npub_to_hex(npub) for npub in npubs if npub_to_hex(npub)
                  and core_identity(npub, core, accounts)["state"] == "noCoreRecord")


def hosted_candidates(core: dict[str, Any], unknown: list[str]) -> list[dict[str, Any]]:
    """Accounts whose Hosted store may hold an unknown key: historical launch
    submitters for those keys, plus accounts Core links to roster keys now."""
    wanted = {row["account"] for row in core.get("launch_chat_keys", []) if row.get("hex") in unknown}
    wanted |= {row.get("account") for row in core.get("agents", []) if row.get("account")}
    wanted |= {row.get("account") for row in core.get("humans", []) if row.get("account")}
    return [row for row in core.get("hosted_candidates", []) if row.get("account") in wanted]


def hosted_store_query(hexes: list[str]) -> str:
    """Only plaintext key columns of the device-state table; never ciphertext."""
    if any(not re.fullmatch(r"[0-9a-f]{64}", hex_key) for hex_key in hexes):
        raise RosterError("Hosted store lookup accepts only canonical hex keys")
    listed = ", ".join(f"'{hex_key}'" for hex_key in hexes) or "NULL"
    return (f"SELECT account_id, device_id FROM client_device_states WHERE device_id ="
            f" '{HOSTED_DEVICE_ID}' AND account_id IN ({listed}) ORDER BY account_id"
            f" LIMIT {MAX_KEYS + 1};")


def collect_hosted(
    candidates: list[dict[str, Any]],
    unknown: list[str],
    scratch_copy: Callable[[Path], Any],
    query: Callable[[Path, str], list[dict[str, Any]]],
    root: Path | None = None,
    clock: Callable[[], float] = time.monotonic,
    overflow: bool = False,
) -> dict[str, Any]:
    """Look for unknown keys in candidate accounts' Hosted Chat stores.

    The store path follows the Hosted Device (users/<sha256 of the WorkOS
    user id>/chat/client.sqlite3); Core computes that digest so no WorkOS id
    reaches this process. Each store is read through one private scratch copy,
    selecting only the plaintext account_id and device_id key columns."""
    root = HOSTED_DATA_ROOT if root is None else root
    if not unknown:
        return {"state": "notNeeded", "accounts": []}
    if overflow:
        return {"state": "unavailable", "accounts": [],
                "reason": f"more than {MAX_HOSTED_CANDIDATES} candidate accounts"}
    if not root.is_dir():
        return {"state": "unavailable", "accounts": [],
                "reason": "Hosted data root is not present on this host"}
    started, copied_bytes, accounts = clock(), 0, []
    sql = hosted_store_query(unknown)
    for candidate in sorted(candidates, key=lambda row: str(row.get("account"))):
        entry: dict[str, Any] = {"account": candidate.get("account")}
        accounts.append(entry)
        storage = str(candidate.get("storage"))
        if not STORAGE_ID.fullmatch(storage):
            entry.update(state="unavailable", reason="malformed storage id")
            continue
        store = root / "users" / storage / "chat" / "client.sqlite3"
        try:
            if not store.exists():
                entry.update(state="missingStore")
                continue
            size = sum(Path(f"{store}{suffix}").stat().st_size for suffix in ("", "-wal", "-shm")
                       if Path(f"{store}{suffix}").exists())
        except OSError as error:
            entry.update(state="unavailable", reason=type(error).__name__)
            continue
        if clock() - started > HOSTED_BUDGET_SECONDS:
            entry.update(state="unavailable", reason="Hosted time budget exhausted")
        elif size > MAX_HOSTED_STORE_BYTES or copied_bytes + size > MAX_HOSTED_TOTAL_BYTES:
            entry.update(state="unavailable", reason="Hosted store over byte budget")
        else:
            copied_bytes += size
            try:
                with scratch_copy(store) as scratch:
                    rows = _bounded("hosted store", query(scratch, sql), MAX_KEYS)
            except Exception as error:  # noqa: BLE001 - one store failing is per-source evidence
                entry.update(state="unavailable", reason=type(error).__name__)
                continue
            entry.update(state="checked", matches=sorted(
                {row.get("account_id") for row in rows
                 if row.get("device_id") == HOSTED_DEVICE_ID and row.get("account_id") in unknown}))
    return {"state": "checked", "checked_at": _now(), "accounts": accounts}


def build_report(brain: dict[str, Any], core: dict[str, Any], brain_id: str) -> dict[str, Any]:
    tables = brain["tables"]
    roster = roster_keys(tables)
    references = reference_keys(tables, roster)
    accounts = {row["account"]: row for row in core.get("accounts", [])}
    aliases = {row["npub"]: row for row in tables["aliases"] if row.get("npub")}
    scopes = core.get("scopes", [])

    def stored_alias(npub: str) -> dict[str, Any] | None:
        alias = aliases.get(npub)
        return None if alias is None else {
            "name": alias.get("preferred_nip05"),
            "storedAt": alias.get("nip05_verified_at"),
            "note": "global stored alias; any lookup may have written it; not rechecked",
        }

    directory = brain.get("directory")
    directory_results = (directory or {}).get("results", {})
    hosted = core.get("hosted")
    all_accounts = {row["account"]: row for row in core.get("accounts", [])}

    def account_view(ref: Any) -> dict[str, Any]:
        found = all_accounts.get(ref)
        return dict(found) if found else {"account": ref, "email": None, "link_status": None}

    def unresolved_evidence(npub: str, identity: dict[str, Any]) -> dict[str, Any]:
        """Extra operator evidence only for keys Core cannot name today."""
        if identity["state"] != "noCoreRecord":
            return {}
        hex_key = npub_to_hex(npub)
        launches = [{
            "requestId": row.get("request"), "status": row.get("status"),
            "createdAt": row.get("created_at"), "updatedAt": row.get("updated_at"),
            "projectId": row.get("project"), "agentRuntimeId": row.get("runtime"),
            "historicalOwnerAccount": account_view(row.get("account")),
        } for row in core.get("launch_chat_keys", []) if row.get("hex") == hex_key]
        stores = [] if hosted is None else [{
            "account": account_view(entry["account"]), "deviceId": HOSTED_DEVICE_ID,
        } for entry in hosted.get("accounts", []) if hex_key in entry.get("matches", [])]
        return {
            "submittedLaunchChatKey": {
                "note": "caller-provided owner_chat_account_id at agent creation; not ownership proof",
                "requests": launches,
            },
            "hostedChatStores": {
                "note": "key found as a hosted-web device in this account's Hosted Chat store; "
                        "operator evidence, not a signature, control proof or consent",
                "state": "notChecked" if hosted is None else hosted.get("state"),
                "matches": stores,
            },
        }

    def directory_view(npub: str) -> dict[str, Any]:
        if directory is None:
            return {"state": "notChecked"}
        if npub_to_hex(npub) is None:
            return {"state": "keyInvalid"}
        return directory_results.get(npub, {"state": "notChecked"})

    by_brain: dict[str, dict[str, list[dict[str, Any]]]] = {}
    for (brain_key, npub), row in sorted(roster.items()):
        identity = core_identity(npub, core, accounts)
        grants = row["grants"]
        by_brain.setdefault(brain_key, {"roster": [], "references": []})["roster"].append({
            "npub": npub,
            "keyFormat": "valid" if npub_to_hex(npub) else "invalid",
            "storedRoles": sorted(row["roles"]),
            "membership": row["membership"],
            "currentGrants": {
                "count": len(grants),
                "grants": grants[:MAX_GRANTS_PER_KEY],
                "truncated": len(grants) > MAX_GRANTS_PER_KEY,
            },
            "brainParticipation": row["participation"],
            "storedNip05": stored_alias(npub),
            "coreIdentity": identity,
            "identityDirectory": directory_view(npub),
            "sharingScopes": sharing_scopes(identity, brain_key, scopes),
            **unresolved_evidence(npub, identity),
        })
    for (brain_key, npub), labels in sorted(references.items()):
        identity = core_identity(npub, core, accounts)
        by_brain.setdefault(brain_key, {"roster": [], "references": []})["references"].append({
            "npub": npub,
            "keyFormat": "valid" if npub_to_hex(npub) else "invalid",
            "referencedAs": sorted(labels),
            "note": "reference only: admitted or granted others; holds no stored role here",
            "storedNip05": stored_alias(npub),
            "coreIdentity": identity,
            "identityDirectory": directory_view(npub),
            **unresolved_evidence(npub, identity),
        })
    brains = []
    for record in tables["brains"]:
        section = by_brain.get(record["id"], {"roster": [], "references": []})
        brains.append({
            "brainId": record["id"],
            "kind": record.get("kind"),
            "name": record.get("name"),
            "folders": sum(1 for folder in tables["folders"] if folder["brain_id"] == record["id"]),
            "rosterKeys": len(section["roster"]),
            "roster": section["roster"],
            "referenceKeys": section["references"],
        })
    report_directory = {"state": "notChecked"} if directory is None else {
        "state": directory.get("state"),
        "reason": directory.get("reason"),
        "checkedAt": directory.get("checked_at"),
        "keysChecked": len(directory_results),
        "budgetExhausted": directory.get("budget_exhausted", False),
    }
    report = {
        "schema_version": SCHEMA_VERSION,
        "exit_code": 0,
        "mode": "operatorRoster",
        "filter": {"brainId": brain_id},
        "evidence": {
            "brainDatabase": brain["source"],
            "brainScratchCopyStartedAt": brain["copy_started_at"],
            "brainScratchCopiedAt": brain["copied_at"],
            "coreCheckedAt": core.get("checked_at"),
            "coreBrainIdentitySchema": core.get("brain_identity_schema"),
            "identityDirectory": report_directory,
            "hostedChatStores": {"state": "notChecked"} if hosted is None else {
                "state": hosted.get("state"),
                "checkedAt": hosted.get("checked_at"),
                "accounts": [{"account": entry.get("account"), "state": entry.get("state"),
                              "reason": entry.get("reason")} for entry in hosted.get("accounts", [])],
            },
        },
        "coverage": {
            "included": [
                "stored Brain roles and membership provenance",
                "current-version Folder Key Grant presence and issuer metadata",
                "exact-key participation facts",
                "Core runtime pins, account key associations and raw sharing-scope rows",
                "Identity Directory finite.vip bindings for each exact key (operator inspect)",
                "stored Brain alias rows for those exact names, on any key",
            ],
            "notIncluded": [
                "Folder entitlement, readability, and retained-but-unentitled grant detection; "
                "use `fbrain access list --brain <id>` for authoritative access",
                "participants known only through a Mount and holding no stored role or grant here",
                "Core's disclosure policy; no decision about what admins may see is made",
                "sealed Hosted Device bindings, identity files, ciphertext and older key versions",
            ],
        },
        "limitations": [
            "Operator evidence only. Contact and account facts here were not shared with the "
            "Brain's admins; sharingScopes shows the raw consent rows that exist, nothing more.",
            "The Brain scratch copy is uncoordinated with live writes, and Brain and Core are "
            "read at different times; neither is reconciled with the other.",
            "admittedBy, grantIssuedBy and grantDelegatedBy name who did the admitting or "
            "wrapping, never who owns or controls the recipient key.",
            "noCoreRecord means Core holds no pin or association for this exact key. Migrated "
            "agents received new keys, so old agent keys usually have no current Core pin; "
            "check the Identity Directory next.",
            "An account with link_status other than linked has an email Core never verified "
            "through WorkOS.",
            "brainParticipation firstAt is the textual minimum of stored times; mixed fractional "
            "precision can misorder events within the same second.",
            "Agent pins are runner-observed records, not key-signed proof. currentOwnerAccount is "
            "the project's owner now, not at admission time.",
            "identityDirectory lists finite.vip name bindings for the exact key, including "
            "disabled ones. A name is not account ownership, contact or consent.",
            "aliasNameEvidence rows are Brain's stored alias cache, read from the same scratch "
            "copy; the Directory is read at a different time and is the binding authority.",
            "For noCoreRecord keys only: submittedLaunchChatKey is the caller-provided launch-time "
            "owner_chat_account_id (historical, not ownership proof), and hostedChatStores says "
            "a candidate account's Hosted Chat store holds that key as its hosted-web device "
            "(scratch copy, key columns only). Neither changes coreIdentity or implies consent.",
        ],
        "brains": brains,
        "aliasNameEvidence": alias_name_evidence(directory_results, tables.get("name_aliases", [])),
    }
    if len(json.dumps(report).encode()) > MAX_REPORT_BYTES:
        raise RosterError("roster report exceeded its byte bound")
    return report


def selected_npubs(tables: dict[str, list[dict[str, Any]]]) -> list[str]:
    """Canonical roster and reference keys, bounded to MAX_KEYS together."""
    roster = roster_keys(tables)
    references = reference_keys(tables, roster)
    npubs = {npub for _, npub in roster} | {npub for _, npub in references}
    if len(npubs) > MAX_KEYS:
        raise RosterError(f"more than {MAX_KEYS} roster and reference keys")
    return sorted(npub for npub in npubs if npub_to_hex(npub))


def alias_name_evidence(
    directory_results: dict[str, dict[str, Any]], alias_rows: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    """Per Directory name: which selected keys it is bound to, and every
    stored Brain alias row for that exact name. Nothing is selected by order."""
    bound: dict[str, list[dict[str, Any]]] = {}
    for npub, result in sorted(directory_results.items()):
        for binding in result.get("bindings", []):
            bound.setdefault(binding["name"], []).append(
                {"npub": npub, "disabled": binding["disabled"]})
    evidence = []
    for name in sorted(bound):
        active_keys = {row["npub"] for row in bound[name] if not row["disabled"]}
        rows = [row for row in alias_rows if row.get("preferred_nip05") == name]
        evidence.append({
            "name": name,
            "directoryKeys": bound[name],
            "brainAliasRows": [{
                "npub": row.get("npub"),
                "storedAt": row.get("nip05_verified_at"),
                "updatedAt": row.get("updated_at"),
                "matchesActiveDirectoryKey": row.get("npub") in active_keys,
            } for row in rows],
        })
    return evidence


def core_inputs(brain: dict[str, Any]) -> dict[str, str]:
    tables = brain["tables"]
    valid = selected_npubs(tables)
    return {
        "key_pairs": json.dumps([{"npub": npub, "hex": npub_to_hex(npub)} for npub in valid]),
        "candidate_limit": str(MAX_HOSTED_CANDIDATES + 1),
        "npubs": json.dumps(valid),
        "hexes": json.dumps(sorted({npub_to_hex(npub) for npub in valid})),
        "brain_ids": json.dumps(sorted({row["id"] for row in tables["brains"]})),
        "row_limit": str(MAX_ROWS + 1),
        "name_limit": str(MAX_NAME_CHARS),
        "email_limit": str(MAX_EMAIL_BYTES),
    }


def parse_core_output(stdout: str) -> dict[str, Any]:
    """Merge the one or two JSON rows the Core transaction prints."""
    if len(stdout.encode()) > MAX_REPORT_BYTES:
        raise RosterError("Core roster output exceeded its byte bound")
    merged: dict[str, Any] = {"humans": [], "scopes": [], "accounts": [],
                              "launch_chat_keys": [], "hosted_candidates": []}
    rows = [line for line in stdout.splitlines() if line.strip()]
    if not rows:
        raise RosterError("Core roster query returned nothing")
    for line in rows:
        try:
            part = json.loads(line)
        except json.JSONDecodeError as error:
            raise RosterError("Core roster query returned invalid JSON") from error
        if not isinstance(part, dict):
            raise RosterError("Core roster query returned an unexpected shape")
        for name in ("agents", "humans", "scopes", "launch_chat_keys"):
            if name in part:
                merged[name] = _bounded(f"core {name}", part[name] or [])
        merged["accounts"].extend(_bounded("core accounts", part.get("accounts") or []))
        listed = _bounded("core hosted candidates", part.get("hosted_candidates") or [],
                          MAX_HOSTED_CANDIDATES + 1)
        merged["hosted_overflow"] = merged.get("hosted_overflow", False) or (
            len(listed) > MAX_HOSTED_CANDIDATES)
        merged["hosted_candidates"].extend(listed)
        for name in ("checked_at", "brain_identity_schema"):
            if name in part:
                merged[name] = part[name]
    if "agents" not in merged:
        raise RosterError("Core roster query did not return agent evidence")
    candidates = {row.get("account"): row for row in merged["hosted_candidates"]}
    merged["hosted_overflow"] = merged.get("hosted_overflow", False) or (
        len(candidates) > MAX_HOSTED_CANDIDATES)
    merged["hosted_candidates"] = list(candidates.values())
    return merged

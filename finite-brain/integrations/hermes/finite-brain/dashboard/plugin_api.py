"""Brain-owned metadata projection, Personal Brain agent consent, and the
Admin roster changes the Agent signs (invite, revoke invitation, remove).

Native Hermes authenticates before routing.
"""

import asyncio
import json
import re
import secrets
from collections import OrderedDict
from datetime import datetime
from types import SimpleNamespace

from fastapi import APIRouter, Request
from fastapi.responses import JSONResponse
from hermes_cli.finite_dashboard_reads import (
    READ_SECONDS,
    AccessUnverified,
    ChangeFailed,
    InvalidInventory,
    ProductUnavailable,
    inventory_response,
    read_json,
    record,
    records,
    run_change,
    text,
)

router = APIRouter()
# Bound complete list and metadata fanout; exceeding either fails the read,
# rather than misrepresenting a truncated list as an empty/complete inventory.
MAX_BRAINS = 100
MAX_METADATA_READS = 32
MAX_FOLDERS = 500
MAX_IDENTITIES = 1024
ROLES = {"owner", "personal_agent", "admin", "member", "guest", "invited"}
NPUB = re.compile(r"npub1[023456789acdefghjklmnpqrstuvwxyz]{58}")
PERSONAL_BRAIN_ID = re.compile(r"personal-[0-9a-f]{16}")
# A Brain ID that can never look like a flag.
BRAIN_ID = re.compile(r"[A-Za-z0-9_][A-Za-z0-9_-]{0,127}")
CONSENT_VERSION = "finite-brain-personal-agent-consent-v1"
MAX_CONSENT_TAGS = 32
MAX_CONSENT_CONTENT = 8192
RFC3339 = re.compile(
    r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z|[+-]\d{2}:\d{2})"
)


async def read_overview():
    listing = record(
        await read_json("fbrain", "brain", "list", "--json", "--existing-identity")
    )
    brains = []
    seen = set()
    for item in records(listing.get("brains"), MAX_BRAINS):
        brain_id = text(item.get("brainId"), 128)
        # Match BrainId. The equals-form option keeps even flag-shaped IDs as data.
        if not re.fullmatch(r"[A-Za-z0-9_-]+", brain_id) or brain_id in seen:
            raise InvalidInventory()
        seen.add(brain_id)
        if (
            item.get("kind") not in ("personal", "organization")
            or text(item.get("role"), 64) not in ROLES
        ):
            raise InvalidInventory()
        brains.append(
            {
                "id": brain_id,
                "name": text(item.get("name")),
                "kind": item["kind"],
                "role": item["role"],
                "folders": None,
            }
        )
    accessible = [brain for brain in brains if brain["role"] != "invited"]
    if len(accessible) > MAX_METADATA_READS:
        raise InvalidInventory()

    async def folders(brain):
        try:
            metadata = record(
                await read_json(
                    "fbrain",
                    "brain",
                    "metadata",
                    f"--brain={brain['id']}",
                    "--json",
                    "--existing-identity",
                )
            )
            if metadata.get("brainId") != brain["id"]:
                raise InvalidInventory()
            projected = []
            ids = set()
            for folder in records(metadata.get("folders"), MAX_FOLDERS):
                folder_id = text(folder.get("id"), 256)
                if folder_id in ids:
                    raise InvalidInventory()
                ids.add(folder_id)
                projected.append({"id": folder_id, "name": text(folder.get("name"))})
            brain["folders"] = projected
        except (AccessUnverified, InvalidInventory, ProductUnavailable, OSError):
            # The fresh list proves this row; failed metadata proves no folders.
            # Never substitute zero, retain old folders, or include mounted folders.
            brain["folders"] = None

    async with asyncio.TaskGroup() as group:
        for brain in accessible:
            group.create_task(folders(brain))
    return {"version": 1, "brains": brains}


def hex_text(value, length):
    if not isinstance(value, str) or not re.fullmatch(f"[0-9a-f]{{{length}}}", value):
        raise InvalidInventory()
    return value


def integer(value):
    if type(value) is not int:
        raise InvalidInventory()
    return value


def consent_event(value):
    event = record(value)
    hex_text(event.get("id"), 64)
    hex_text(event.get("pubkey"), 64)
    hex_text(event.get("sig"), 128)
    text(event.get("content"), MAX_CONSENT_CONTENT)
    integer(event.get("created_at"))
    integer(event.get("kind"))
    tags = event.get("tags")
    if not isinstance(tags, list) or len(tags) > MAX_CONSENT_TAGS:
        raise InvalidInventory()
    for tag in tags:
        if not isinstance(tag, list) or not all(isinstance(item, str) for item in tag):
            raise InvalidInventory()
    # The Brain server verifies the signature; pass the event through unchanged.
    return event


async def read_personal_agent_consent(owner_npub):
    # The owner comes from the URL path: refuse anything but an npub before
    # running the CLI. A validated npub can never look like a flag, so it is
    # passed as its own argument (this command takes no equals form).
    if not NPUB.fullmatch(owner_npub):
        raise InvalidInventory()
    output = record(
        await read_json(
            "fbrain", "brain", "personal-agent-consent", "--owner", owner_npub, "--json"
        )
    )
    agent_npub = output.get("agentNpub")
    brain_id = output.get("brainId")
    if (
        output.get("version") != CONSENT_VERSION
        or output.get("ownerNpub") != owner_npub
        or not isinstance(agent_npub, str)
        or not NPUB.fullmatch(agent_npub)
        or agent_npub == owner_npub
        or not isinstance(brain_id, str)
        or not PERSONAL_BRAIN_ID.fullmatch(brain_id)
    ):
        raise InvalidInventory()
    return {
        "version": 1,
        "agentNpub": agent_npub,
        "ownerNpub": owner_npub,
        "brainId": brain_id,
        "consent": consent_event(output.get("consent")),
    }


@router.get("/overview")
async def overview(request: Request):
    return await inventory_response(request, read_overview)


def optional_text(value):
    return None if value is None else text(value)


def rfc3339(value):
    try:
        if RFC3339.fullmatch(text(value, 64)):
            datetime.fromisoformat(value)
            return value
    except ValueError:
        pass
    raise InvalidInventory()


def operator_annotation(row, description):
    """Withhold malformed annotations without turning them into contact/alias data."""
    annotation = row.get("operatorAnnotation")
    diagnostic = row.get("operatorRecord")
    if not isinstance(annotation, dict) or set(annotation) != {
        "kind",
        "label",
        "observedAt",
    }:
        return None
    if not isinstance(diagnostic, dict) or diagnostic.get("state") != "used":
        return None
    if description.get("state") != "notShared" or any(
        description.get(field) is not None
        for field in (
            "kind",
            "displayName",
            "accountEmail",
            "responsibleAccount",
            "source",
            "lifecycle",
        )
    ):
        return None
    label, observed = annotation.get("label"), annotation.get("observedAt")
    if annotation.get("kind") not in {"historical_identity", "invitation_key"}:
        return None
    if (
        not isinstance(label, str)
        or not 1 <= len(label) <= 120
        or label.strip() != label
    ):
        return None
    if any(not 0x20 <= ord(char) <= 0x7E for char in label):
        return None
    if (
        not isinstance(observed, str)
        or len(observed) > 40
        or not observed.endswith("Z")
    ):
        return None
    if diagnostic.get("observedAt") != observed:
        return None
    try:
        rfc3339(observed)
    except InvalidInventory:
        return None
    return {"kind": annotation["kind"], "label": label, "observedAt": observed}


async def read_identities(brain_id):
    # The browser supplies only a Brain ID. The Agent's signer and the fixed
    # commands stay Agent-owned; Brain enforces admin authority on the report.
    # `access list` mints a missing identity, so first prove with the
    # overview's no-mint read that this Agent already belongs to the Brain.
    membership = await brain_membership(brain_id)
    if membership.get("role") == "invited":
        raise AccessUnverified()
    report = await access_report(brain_id)
    identities = []
    for row in records(report.get("identities"), MAX_IDENTITIES):
        # Name, email and owner exist only when Core resolved this key.
        description = record(row.get("description"))
        annotation = operator_annotation(row, description)
        if description.get("state") != "resolved":
            description = {}
        owner = (
            description.get("responsibleAccount")
            if description.get("kind") == "agent"
            else None
        )
        lifecycle = (
            description.get("lifecycle") if description.get("kind") == "agent" else None
        )
        if lifecycle is not None and lifecycle not in {
            "active",
            "offboarding",
            "retired",
        }:
            raise InvalidInventory()
        # The public name Brain stored for this exact key, whatever Core
        # resolved: any stored name for a key that acted in this Brain, else
        # only its finite.vip Directory name.
        stored = row.get("storedNip05")
        stored = None if stored is None else record(stored)
        identities.append(
            {
                "npub": text(row.get("npub"), 128),
                "role": text(row.get("brainRole"), 64),
                "kind": optional_text(description.get("kind")),
                "lifecycle": lifecycle,
                "operatorAnnotation": annotation,
                "name": optional_text(description.get("displayName")),
                "email": optional_text(description.get("accountEmail")),
                "ownerEmail": None
                if owner is None
                else text(record(owner).get("email")),
                "publicName": None if stored is None else text(stored.get("name")),
                "publicNameStoredAt": None
                if stored is None or stored.get("storedAt") is None
                else rfc3339(stored.get("storedAt")),
                "folders": [
                    {
                        "id": text(folder.get("folderId"), 256),
                        "state": text(folder.get("state"), 64),
                    }
                    for folder in records(row.get("folders"), MAX_FOLDERS)
                ],
            }
        )
    return {
        "version": 1,
        "brainId": brain_id,
        "identities": identities,
        "changes": roster_changes(membership, report),
    }


async def brain_membership(brain_id):
    """The Agent's own row from the no-mint Brain list, or AccessUnverified."""
    if not BRAIN_ID.fullmatch(brain_id):
        raise AccessUnverified()
    listing = record(
        await read_json("fbrain", "brain", "list", "--json", "--existing-identity")
    )
    for item in records(listing.get("brains"), MAX_BRAINS):
        if item.get("brainId") == brain_id:
            return item
    raise AccessUnverified()


async def access_report(brain_id):
    # This command takes no equals form; BRAIN_ID never starts with "-".
    report = record(
        await read_json("fbrain", "access", "list", "--brain", brain_id, "--json")
    )
    if (
        report.get("version") != "finite-brain-access-report-v1"
        or report.get("brainId") != brain_id
    ):
        raise InvalidInventory()
    return report


def acting_npub(report):
    acting = report.get("actingKey")
    npub = acting.get("npub") if isinstance(acting, dict) else None
    return npub if isinstance(npub, str) and NPUB.fullmatch(npub) else None


def roster_changes(membership, report):
    """Advertise the changes this Agent may offer. Older dashboards ignore it;
    an older plugin omits it, so new dashboards show no buttons."""
    acting = report.get("actingKey")
    admin = (
        membership.get("kind") == "organization"
        and membership.get("role") == "admin"
        and report.get("brainKind") == "organization"
        and isinstance(acting, dict)
        and acting.get("brainRole") == "admin"
        and acting_npub(report) is not None
    )
    return {
        "actingNpub": acting_npub(report),
        "actions": list(CHANGE_ACTIONS) if admin else [],
    }


@router.get("/identities/{brain_id}")
async def identities(request: Request, brain_id: str):
    return await inventory_response(request, lambda: read_identities(brain_id))


@router.get("/personal-agent-consent/{owner_npub}")
async def personal_agent_consent(request: Request, owner_npub: str):
    """The Agent's short-lived consent to be this owner's Personal Brain agent.

    The dashboard pairs it with the owner's own signed creation request, and
    the Brain server checks both signatures against its own records.
    """
    return await inventory_response(
        request, lambda: read_personal_agent_consent(owner_npub)
    )


# Admin roster changes. The browser supplies only a Brain ID, an invite
# target, a removal npub or an invitation ID; each passes a strict pattern
# before any command runs. These commands read flags only as `--flag value`
# (an equals form is silently ignored), so every value is checked never to
# start with "-". The Agent signs; Brain still enforces every rule.
CHANGE_ACTIONS = ("invite", "revokeInvitation", "remove")
# A public NIP-05 name such as name@finite.vip, lowercase and bounded.
NIP05 = re.compile(
    r"[a-z0-9_][a-z0-9._-]{0,63}@[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?"
    r"(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+"
)
MAX_NIP05 = 254
INVITATION_ID = re.compile(r"invitation-[0-9a-f]{32}")
JOB_ID = re.compile(r"[0-9a-f]{32}")
MAX_INVITATIONS = 500
MAX_INVITATION_IDENTITIES = 32
MAX_CHANGE_BODY = 1024
# Each removal step may wait for a Folder Key rotation slot and upload up to
# 64 MiB; the CLI bounds that at four minutes. This only reaps a stuck CLI.
REMOVAL_STEP_SECONDS = 300
MAX_REMOVAL_JOBS = 16
REMOVABLE_ROLES = {"member", "admin"}


class Refused(Exception):
    """A checked precondition failed before anything changed."""

    def __init__(self, status, code):
        super().__init__(code)
        self.status = status
        self.code = code


class ChangeSlot:
    """One roster change at a time for this Agent, including a removal job."""

    def __init__(self):
        self.held = False
        self.jobs = OrderedDict()
        self.tasks = set()


_slot = ChangeSlot()


def invite_target(value):
    if isinstance(value, str) and (
        NPUB.fullmatch(value) or (len(value) <= MAX_NIP05 and NIP05.fullmatch(value))
    ):
        return value
    raise Refused(400, "invalidInput")


def removal_target(value):
    if isinstance(value, str) and NPUB.fullmatch(value):
        return value
    raise Refused(400, "invalidInput")


def change_fields(request, raw, fields):
    """Exactly the named string fields from a small JSON body, nothing else."""
    if not fields:
        if raw:
            raise Refused(400, "invalidInput")
        return {}
    kind = request.headers.get("content-type", "").split(";", 1)[0].strip().lower()
    if kind != "application/json":
        raise Refused(400, "invalidInput")
    try:
        body = json.loads(raw)
    except (ValueError, UnicodeDecodeError, RecursionError):
        raise Refused(400, "invalidInput") from None
    if not isinstance(body, dict) or set(body) != set(fields):
        raise Refused(400, "invalidInput")
    return body


async def change_body(request):
    length = request.headers.get("content-length")
    if request.query_params or request.headers.get("transfer-encoding"):
        raise Refused(400, "invalidInput")
    if length is None:
        return b""
    if not length.isdigit() or int(length) > MAX_CHANGE_BODY:
        raise Refused(400, "invalidInput")
    raw = await request.body()
    if len(raw) > MAX_CHANGE_BODY:
        raise Refused(400, "invalidInput")
    return raw


async def require_admin(brain_id):
    """Prove with the no-mint list that this Agent is an Admin of this
    Organization Brain before any command that could mint an identity."""
    membership = await brain_membership(brain_id)
    if membership.get("kind") != "organization" or membership.get("role") != "admin":
        raise AccessUnverified()


async def change_response(request, brain_id, fields, check, prepare):
    """Run one change under the Agent's change slot.

    `check` validates the request values before anything runs. `prepare`
    runs the read checks and returns (status, payload, follow_up); a
    follow_up keeps the slot until its background job finishes.
    `step.sent` records whether the change command itself has started.
    """
    headers = {"Cache-Control": "no-store"}
    step = SimpleNamespace(sent=False)

    def refuse(status, code):
        return JSONResponse({"error": code}, status, headers)

    try:
        if not BRAIN_ID.fullmatch(brain_id):
            raise Refused(400, "invalidInput")
        values = check(change_fields(request, await change_body(request), fields))
    except Refused as error:
        return refuse(error.status, error.code)
    if _slot.held:
        return refuse(409, "busy")
    _slot.held = True
    handed_off = False
    try:
        async with asyncio.timeout(READ_SECONDS):
            status, payload, follow_up = await prepare(values, step)
        if follow_up is not None:
            task = asyncio.create_task(follow_up())
            _slot.tasks.add(task)
            task.add_done_callback(_slot.tasks.discard)
            handed_off = True
        return JSONResponse(payload, status, headers)
    except Refused as error:
        return refuse(error.status, error.code)
    except AccessUnverified:
        return refuse(403, "notAdmin")
    except ChangeFailed:
        return refuse(502, "failed")
    except TimeoutError:
        # A change command that started may have finished on the server.
        return refuse(
            504 if step.sent else 503, "unknown" if step.sent else "unavailable"
        )
    except (OSError, InvalidInventory, ProductUnavailable):
        return refuse(
            502 if step.sent else 503, "failed" if step.sent else "unavailable"
        )
    finally:
        if not handed_off:
            _slot.held = False


def pending_invitation(item, brain_id):
    """One of this Brain's pending Brain Invitations, or InvalidInventory.

    `folderOnly` appears only in a Brain's own invitation list, never in the
    Agent's incoming inbox, so a misread inbox can never pass as this list.
    """
    item = record(item)
    if item.get("brainId") != brain_id or item.get("folderOnly") is not False:
        raise InvalidInventory()
    invitation_id = item.get("id")
    if not isinstance(invitation_id, str) or not INVITATION_ID.fullmatch(invitation_id):
        raise InvalidInventory()
    target = item.get("userId")
    if target is not None and (
        not isinstance(target, str) or not NPUB.fullmatch(target)
    ):
        raise InvalidInventory()
    name = None
    try:
        identities = records(item.get("identities", []), MAX_INVITATION_IDENTITIES)
    except InvalidInventory:
        # The name is optional; an unexpected identity list hides only the name.
        identities = []
    for identity in identities:
        nip05 = identity.get("nip05")
        if identity.get("npub") == target and isinstance(nip05, str):
            candidate = nip05.lower()
            if len(candidate) <= MAX_NIP05 and NIP05.fullmatch(candidate):
                name = candidate
    return {
        "id": invitation_id,
        "targetNpub": target,
        "targetName": name,
        "expiresAt": rfc3339(item.get("expiresAt")),
        "expired": item.get("expired") is True,
    }


async def read_invitations(brain_id):
    await require_admin(brain_id)
    listing = record(
        await read_json(
            "fbrain", "invite", "brain", "list", "--brain", brain_id, "--json"
        )
    )
    invitations = []
    for item in records(listing.get("invitations"), MAX_INVITATIONS):
        if item.get("status") == "pending":
            invitations.append(pending_invitation(item, brain_id))
    if len({item["id"] for item in invitations}) != len(invitations):
        raise InvalidInventory()
    return {"version": 1, "brainId": brain_id, "invitations": invitations}


@router.get("/invitations/{brain_id}")
async def invitations(request: Request, brain_id: str):
    return await inventory_response(request, lambda: read_invitations(brain_id))


@router.post("/invitations/{brain_id}")
async def invite(request: Request, brain_id: str):
    async def prepare(values, step):
        await require_admin(brain_id)
        step.sent = True
        receipt = await run_change(
            "fbrain", "invite", "brain", "create",
            "--brain", brain_id, "--target", values["target"], "--json",
        )  # fmt: skip
        try:
            invitation = pending_invitation(receipt, brain_id)
        except InvalidInventory:
            # It was created; the refreshed list shows it.
            invitation = None
        return 200, {"version": 1, "invitation": invitation}, None

    return await change_response(
        request,
        brain_id,
        ("target",),
        lambda body: {"target": invite_target(body["target"])},
        prepare,
    )


@router.delete("/invitations/{brain_id}/{invitation_id}")
async def revoke_invitation(request: Request, brain_id: str, invitation_id: str):
    async def prepare(_values, step):
        pending = await read_invitations(brain_id)
        if not any(item["id"] == invitation_id for item in pending["invitations"]):
            raise Refused(422, "notPending")
        step.sent = True
        await run_change(
            "fbrain", "invite", "brain", "revoke", "--id", invitation_id, "--json"
        )
        return 200, {"version": 1}, None

    def check(_body):
        if not INVITATION_ID.fullmatch(invitation_id):
            raise Refused(400, "invalidInput")
        return {}

    return await change_response(request, brain_id, (), check, prepare)


def remember_job(job_id, brain_id, state):
    _slot.jobs[job_id] = {"brainId": brain_id, "state": state}
    while len(_slot.jobs) > MAX_REMOVAL_JOBS:
        _slot.jobs.popitem(last=False)


async def removal_job(job_id, brain_id, npub, demote):
    """Demote an Admin first (Brain refuses removing one), then remove. Never
    retried. A job lost to a Hermes restart reads as unknown."""
    demoted = False
    state = "unknown"
    try:
        if demote:
            async with asyncio.timeout(REMOVAL_STEP_SECONDS):
                await run_change(
                    "fbrain", "admin", "role", "revoke", "admin",
                    "--brain", brain_id, "--target", npub, "--json",
                )  # fmt: skip
            demoted = True
        async with asyncio.timeout(REMOVAL_STEP_SECONDS):
            await run_change(
                "fbrain", "admin", "member", "remove",
                "--brain", brain_id, "--target", npub, "--json",
            )  # fmt: skip
        state = "removed"
    except (ChangeFailed, OSError):
        state = "demotedNotRemoved" if demoted else "failed"
    except TimeoutError:
        state = "unknown"
    finally:
        remember_job(job_id, brain_id, state)
        _slot.held = False


@router.post("/removals/{brain_id}")
async def start_removal(request: Request, brain_id: str):
    async def prepare(values, _step):
        npub = values["npub"]
        await require_admin(brain_id)
        report = await access_report(brain_id)
        acting = acting_npub(report)
        if acting is None:
            raise InvalidInventory()
        if npub == acting:
            raise Refused(422, "ownKey")
        roles = [
            row.get("brainRole")
            for row in records(report.get("identities"), MAX_IDENTITIES)
            if row.get("npub") == npub
        ]
        if len(roles) != 1 or roles[0] not in REMOVABLE_ROLES:
            raise Refused(422, "notRemovable")
        job_id = secrets.token_hex(16)
        remember_job(job_id, brain_id, "running")
        return (
            202,
            {"version": 1, "job": {"id": job_id, "state": "running"}},
            lambda: removal_job(job_id, brain_id, npub, roles[0] == "admin"),
        )

    return await change_response(
        request,
        brain_id,
        ("npub",),
        lambda body: {"npub": removal_target(body["npub"])},
        prepare,
    )


@router.get("/removals/{brain_id}/{job_id}")
async def removal_status(request: Request, brain_id: str, job_id: str):
    headers = {"Cache-Control": "no-store"}
    if request.query_params or request.headers.get("content-length", "0") != "0":
        return JSONResponse({"error": "invalidInput"}, 400, headers)
    job = _slot.jobs.get(job_id) if JOB_ID.fullmatch(job_id) else None
    state = job["state"] if job and job["brainId"] == brain_id else "unknown"
    return JSONResponse(
        {
            "version": 1,
            "job": {"id": job_id if JOB_ID.fullmatch(job_id) else None, "state": state},
        },
        headers=headers,
    )

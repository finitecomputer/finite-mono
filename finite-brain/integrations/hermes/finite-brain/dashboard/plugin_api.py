"""Brain-owned metadata projection. Native Hermes authenticates before routing."""

import asyncio
import re

from fastapi import APIRouter, Request
from hermes_cli.finite_dashboard_reads import (
    AccessUnverified,
    InvalidInventory,
    ProductUnavailable,
    inventory_response,
    read_json,
    record,
    records,
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


@router.get("/overview")
async def overview(request: Request):
    return await inventory_response(request, read_overview)


def optional_text(value):
    return None if value is None else text(value)


async def read_identities(brain_id):
    # The browser supplies only a Brain ID. The Agent's signer and the fixed
    # commands stay Agent-owned; Brain enforces admin authority on the report.
    if not re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_-]{0,127}", brain_id):
        raise AccessUnverified()
    # `access list` mints a missing identity, so first prove with the
    # overview's no-mint read that this Agent already belongs to the Brain.
    listing = record(
        await read_json("fbrain", "brain", "list", "--json", "--existing-identity")
    )
    if not any(
        item.get("brainId") == brain_id and item.get("role") != "invited"
        for item in records(listing.get("brains"), MAX_BRAINS)
    ):
        raise AccessUnverified()
    report = record(
        await read_json("fbrain", "access", "list", "--brain", brain_id, "--json")
    )
    if (
        report.get("version") != "finite-brain-access-report-v1"
        or report.get("brainId") != brain_id
    ):
        raise InvalidInventory()
    identities = []
    for row in records(report.get("identities"), MAX_IDENTITIES):
        # Name, email and owner exist only when Core resolved this key.
        description = record(row.get("description"))
        if description.get("state") != "resolved":
            description = {}
        owner = description.get("responsibleAccount")
        identities.append(
            {
                "npub": text(row.get("npub"), 128),
                "role": text(row.get("brainRole"), 64),
                "kind": optional_text(description.get("kind")),
                "name": optional_text(description.get("displayName")),
                "email": optional_text(description.get("accountEmail")),
                "ownerEmail": None
                if owner is None
                else text(record(owner).get("email")),
                "folders": [
                    {
                        "id": text(folder.get("folderId"), 256),
                        "state": text(folder.get("state"), 64),
                    }
                    for folder in records(row.get("folders"), MAX_FOLDERS)
                ],
            }
        )
    return {"version": 1, "brainId": brain_id, "identities": identities}


@router.get("/identities/{brain_id}")
async def identities(request: Request, brain_id: str):
    return await inventory_response(request, lambda: read_identities(brain_id))

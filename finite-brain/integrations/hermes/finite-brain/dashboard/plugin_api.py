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

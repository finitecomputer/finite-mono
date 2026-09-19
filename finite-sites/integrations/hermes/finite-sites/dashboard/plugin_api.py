"""Sites-owned projection of the agent's Project Repositories and their Sites."""

from urllib.parse import urlsplit

from fastapi import APIRouter, Request
from hermes_cli.finite_dashboard_reads import (
    InvalidInventory,
    inventory_response,
    read_json,
    record,
    records,
    text,
)

router = APIRouter()
MAX_PROJECTS = 500  # Complete response or explicit failure, never silent truncation.


def https_url(value):
    value = text(value, 2048)
    try:
        url = urlsplit(value)
        if url.scheme != "https" or not url.hostname or url.username or url.password:
            raise InvalidInventory()
    except ValueError as error:
        raise InvalidInventory() from error
    return value


async def read_overview():
    listing = record(
        await read_json(
            "fsite",
            "project",
            "list",
            "--output",
            "json",
            "--existing-identity",
        )
    )
    sites = []
    source_only = 0
    ids = set()
    for project in records(listing.get("projects"), MAX_PROJECTS):
        project_id = text(project.get("project_id"), 256)
        if project_id in ids:
            raise InvalidInventory()
        ids.add(project_id)
        if project.get("site") is None:
            source_only += 1
            continue
        site = record(project["site"])
        visibility = site.get("visibility")
        if visibility not in ("private", "shared", "public"):
            raise InvalidInventory()
        version = site.get("active_version")
        if version is not None and (type(version) is not int or version < 1):
            raise InvalidInventory()
        role = text(project.get("role"), 64)
        sites.append(
            {
                "id": project_id,
                "name": text(site.get("name")),
                "url": https_url(site.get("url")),
                "visibility": visibility,
                "status": text(site.get("status"), 64),
                "published": version is not None,
                "canEdit": role in ("owner", "editor"),
                "repositoryUrl": https_url(project.get("git_remote_url")),
            }
        )
    return {"version": 1, "sites": sites, "sourceOnlyProjects": source_only}


@router.get("/overview")
async def overview(request: Request):
    return await inventory_response(request, read_overview)

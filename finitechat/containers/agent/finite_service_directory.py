"""Read-only accessor for the agent's verified service-directory cache.

`finite-agentd` fetches Core's signed ``finite_service_directory.v1``
document, verifies its ed25519 signature, and atomically caches it at
``/data/agent/service-directory.json``. This module only READS that verified
cache — no crypto, no network. Anything that acts on channel heads (skills or
payload sync) must go through agentd's fresh fetch-and-verify path instead.

Usage:
    from finite_service_directory import service_base_urls
    urls = service_base_urls()          # {"sites": "http://...", ...}

CLI (used by smoke tests):
    python3 finite_service_directory.py          # all base URLs as JSON
    python3 finite_service_directory.py sites    # one base URL as text
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

SCHEMA = "finite_service_directory.v1"
DEFAULT_CACHE_PATH = Path("/data/agent/service-directory.json")


class ServiceDirectoryError(RuntimeError):
    """The cache is missing, unreadable, or not a directory document."""


def _load(path: Path) -> dict:
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as error:
        raise ServiceDirectoryError(
            f"service directory cache is missing: {path}"
        ) from error
    except (OSError, json.JSONDecodeError) as error:
        raise ServiceDirectoryError(
            f"service directory cache is unreadable: {path}: {error}"
        ) from error
    if not isinstance(document, dict) or document.get("schema") != SCHEMA:
        raise ServiceDirectoryError(
            f"service directory cache does not carry schema {SCHEMA!r}: {path}"
        )
    return document


def service_base_urls(path: Path | str | None = None) -> dict[str, str]:
    """Every advertised service's base URL, keyed by service name."""
    document = _load(Path(path) if path is not None else DEFAULT_CACHE_PATH)
    services = document.get("services")
    if not isinstance(services, dict):
        raise ServiceDirectoryError("service directory cache has no services map")
    urls: dict[str, str] = {}
    for name, entry in services.items():
        base_url = entry.get("base_url") if isinstance(entry, dict) else None
        if not isinstance(base_url, str) or not base_url:
            raise ServiceDirectoryError(
                f"service directory entry {name!r} has no base_url"
            )
        urls[name] = base_url
    return urls


def service_base_url(name: str, path: Path | str | None = None) -> str:
    """One service's base URL; raises if the service is not advertised."""
    urls = service_base_urls(path)
    try:
        return urls[name]
    except KeyError as error:
        raise ServiceDirectoryError(
            f"service directory does not advertise {name!r}"
        ) from error


def main(argv: list[str]) -> int:
    try:
        if len(argv) > 1:
            print(service_base_url(argv[1]))
        else:
            print(json.dumps(service_base_urls(), indent=2, sort_keys=True))
    except ServiceDirectoryError as error:
        print(f"finite_service_directory: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))

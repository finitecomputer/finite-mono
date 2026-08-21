#!/usr/bin/env python3
"""Pin the Finite Identity Directory's service-owned public route surface.

Two halves:

- Static (CI): the checked-in MANIFEST is exactly the Directory's public API
  surface — every manifest route must be mounted by `public_router` in the
  identity service, and `public_router` must mount nothing outside the
  manifest plus the public GET routes. Every direct Identity call in the
  fsite CLI must name a manifest route. This fails when a route is added to
  the public surface without a manifest entry, when a manifest route is not
  actually public, or when a product keeps calling a retired route.
- Live (deploy verification): probe each manifest route against a target
  (production by default) and fail if any answers 404, which is how a route
  missing from the public surface presents to product callers.

The edge never keeps a route list; this gate watches the service-owned list.
"""

from __future__ import annotations

import argparse
import re
import sys
import urllib.error
import urllib.request
from collections.abc import Callable
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_TARGET = "https://identity.finite.vip"
CLI_API_SOURCE = REPO_ROOT / "finite-sites/crates/fsite-cli/src/api.rs"
AUTHORITY_SOURCE = REPO_ROOT / "finite-identity/src/authority.rs"

# The Directory's public API surface, in full. A route is publicly reachable
# only by being mounted here and in `public_router`; adding or retiring a
# public route means changing this list in the same commit.
MANIFEST = [
    "/api/v1/email-challenges",
    "/api/v1/nip05-resolution",
    "/api/v1/vip-email-bindings/redeem",
]

# Public GET routes served alongside the manifest API: liveness and the
# NIP-05 well-known document.
PUBLIC_GET_ROUTES = [
    "/health",
    "/.well-known/nostr.json",
]

# Routes that must never appear on the public surface: server-to-server,
# operator, and internal contracts stay loopback-only.
PRIVATE_PATTERNS = [
    re.compile(r"^/internal/"),
    re.compile(r"^/api/v1/operator/"),
]

# Direct Identity Directory calls in the CLI look like
# `format!("{}/api/v1/...", self.base_url)`; the Sites API client in the same
# file formats `"{}{}"` instead, so this pattern only matches identity calls.
CLI_DIRECT_CALL = re.compile(
    r'format!\("\{\}(/api/v1/[a-z0-9\-/]+)",\s*self\.base_url\)'
)
ROUTE_REGISTRATION = re.compile(r'\.route\(\s*"([^"]+)"')


def extract_cli_identity_routes(cli_api: str) -> set[str]:
    """Routes the fsite CLI calls against the Identity Directory directly."""
    return set(CLI_DIRECT_CALL.findall(cli_api))


def function_body(source: str, declaration: str) -> str:
    """Return the text of one top-level Rust function (brace at column 0)."""
    start = source.index(declaration)
    end = source.index("\n}\n", start)
    return source[start:end]


def extract_public_router_paths(authority: str) -> set[str]:
    """Routes mounted by `public_router`, including the shared public_routes."""
    shared = function_body(authority, "fn public_routes()")
    mounted = function_body(authority, "pub fn public_router")
    return set(ROUTE_REGISTRATION.findall(shared + mounted))


def static_failures(
    manifest: list[str],
    cli_routes: set[str],
    public_paths: set[str],
) -> list[str]:
    failures = []
    missing_from_manifest = sorted(cli_routes - set(manifest))
    if missing_from_manifest:
        failures.append(
            "CLI calls Identity Directory routes missing from MANIFEST: "
            f"{missing_from_manifest} (the Directory serves only its manifest; "
            "move the call to a kept route, or restore the route and add it to "
            "MANIFEST in finite-identity/scripts/identity-edge-contract-gate.py)"
        )
    not_public = sorted(set(manifest) - public_paths)
    if not_public:
        failures.append(
            f"MANIFEST routes missing from public_router: {not_public} "
            "(product callers get 404 through the edge)"
        )
    unmanifested = sorted(
        public_paths - set(manifest) - set(PUBLIC_GET_ROUTES)
    )
    if unmanifested:
        failures.append(
            f"public_router mounts routes outside the Directory surface: "
            f"{unmanifested} (the public surface is exactly MANIFEST plus "
            "/health and /.well-known/nostr.json)"
        )
    leaked = sorted(
        path
        for path in public_paths
        if any(pattern.search(path) for pattern in PRIVATE_PATTERNS)
    )
    if leaked:
        failures.append(f"private routes mounted on the public surface: {leaked}")
    return failures


def http_post_status(url: str) -> int:
    request = urllib.request.Request(
        url,
        data=b"{}",
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=10):
            return 200
    except urllib.error.HTTPError as error:
        return error.code


def probe_failures(
    target: str,
    routes: list[str],
    post_status: Callable[[str], int] = http_post_status,
) -> list[str]:
    failures = []
    for route in routes:
        url = f"{target.rstrip('/')}{route}"
        status = post_status(url)
        if status == 404:
            failures.append(
                f"{route} returned 404 through {target}: the route is not on "
                "the public surface (public_router) or the deploy is stale"
            )
    return failures


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", default=DEFAULT_TARGET)
    parser.add_argument(
        "--static",
        action="store_true",
        help="Only run the source/manifest checks; skip the live probe.",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    cli_routes = extract_cli_identity_routes(CLI_API_SOURCE.read_text(encoding="utf-8"))
    public_paths = extract_public_router_paths(
        AUTHORITY_SOURCE.read_text(encoding="utf-8")
    )
    failures = static_failures(MANIFEST, cli_routes, public_paths)
    if not args.static:
        failures += probe_failures(args.target, MANIFEST)
    if failures:
        for failure in failures:
            print(f"FAIL {failure}", file=sys.stderr)
        raise SystemExit(1)
    scope = "static" if args.static else f"static + live({args.target})"
    print(f"identity edge contract ({scope}): ok")


if __name__ == "__main__":
    main()

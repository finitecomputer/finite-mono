#!/usr/bin/env python3
"""Read or change Finite's live Numes admission cap without restarting inference."""

import argparse
import json
import os
from pathlib import Path
import sys
import urllib.error
import urllib.parse
import urllib.request


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "limit", type=int, nargs="?", help="0 pauses admission; omit to read"
    )
    args = parser.parse_args()
    if args.limit is not None and not 0 <= args.limit <= 1024:
        parser.error("limit must be between 0 and 1024")
    # Read credentials from a protected file or the environment, never argv.
    key_path = os.environ.get("NUMES_CONTROL_KEY_FILE")
    try:
        key = (
            Path(key_path).read_text().strip()
            if key_path
            else os.environ["NUMES_CONTROL_API_KEY"]
        )
    except (OSError, KeyError):
        parser.error("set NUMES_CONTROL_KEY_FILE or NUMES_CONTROL_API_KEY")
    endpoint = os.environ.get(
        "FINITE_PRIVATE_ENDPOINT",
        "https://finite-private.finite.containers.tinfoil.dev",
    ).rstrip("/")
    parsed = urllib.parse.urlsplit(endpoint)
    if (
        not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
        or parsed.query
        or parsed.fragment
        or not (
            parsed.scheme == "https"
            or (parsed.scheme == "http" and parsed.hostname == "127.0.0.1")
        )
    ):
        parser.error("use HTTPS or an attestation-verifying loopback proxy")
    data = (
        None
        if args.limit is None
        else json.dumps({"max_concurrent": args.limit}).encode()
    )
    request = urllib.request.Request(
        endpoint + "/admin/numes/capacity",
        data=data,
        headers={"Authorization": "Bearer " + key, "Content-Type": "application/json"},
    )
    try:
        with urllib.request.build_opener(NoRedirect).open(
            request, timeout=15
        ) as response:
            result = json.load(response)
        print(json.dumps(result, indent=2, sort_keys=True))
    except urllib.error.HTTPError as error:
        print(f"Numes capacity request failed: HTTP {error.code}", file=sys.stderr)
        return 1
    except (OSError, ValueError):
        print(
            "Numes capacity request failed; check connectivity and configuration.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

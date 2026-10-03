#!/usr/bin/env python3
"""Read-only host model-pack gate for the pinned DeepSeek test and GLM rollback."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import sys
import urllib.error
import urllib.parse
import urllib.request

API = "https://api.tinfoil.sh"
HOST = "control.inf9.tinfoil.sh"
PACKS = {
    "deepseek": {
        "repo": "deepseek-ai/DeepSeek-V4.1-Flash",
        "commit": "dba1be0a40aa45a94ad051997016db3960a90277",
        "schema": 2,
        "root_hash": "159977c3efb94d1eba09025f71af1d1d55d6a58c58e4165b4587ec83166fa385",
        "offset": 510313381888,
        "verity_uuid": "27d8d4f7-446a-5f31-ae99-2339b36c2dde",
    },
    "glm": {
        "repo": "zai-org/GLM-5.3-Flash",
        "commit": "04c4e9e95c5da8862dced7e5056455116f83a7e0",
        "schema": 1,
        "root_hash": "54b2859aba423589c0d0d3bf48af25016c03d3b77db366a9061661e2df85ed79",
        "offset": 328366215168,
        "verity_uuid": "20a7ed6c-fd6a-54cb-98b4-8bfea86f829c",
    },
}


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None  # Never forward the admin bearer to another destination.


def api_key() -> str:
    configured = Path(os.environ.get("TINFOIL_CONFIG", str(Path.home() / ".tinfoil/config.json")))
    cfg = json.loads(configured.read_text()) if configured.exists() else {}
    base = os.environ.get("TINFOIL_CONTROLPLANE_URL", cfg.get("controlplane_url") or API)
    if base.rstrip("/") != API:
        raise ValueError("this production gate only supports https://api.tinfoil.sh")
    key = os.environ.get("TINFOIL_API_KEY") or cfg.get("api_key")
    if not isinstance(key, str) or not key.strip():
        raise ValueError("Tinfoil admin authentication is not configured")
    return key.strip()


def fetch_job(key: str, job_id: str) -> dict:
    if not job_id or not all(c.isascii() and (c.isalnum() or c == "-") for c in job_id):
        raise ValueError("invalid model wrap job id")
    url = f"{API}/api/models/wrap/{HOST}/{urllib.parse.quote(job_id, safe='')}"
    request = urllib.request.Request(url, headers={
        "Authorization": f"Bearer {key}", "Accept": "application/json",
        "User-Agent": "finite-model-preflight/1.0",
    })
    with urllib.request.build_opener(NoRedirect).open(request, timeout=30) as response:
        return json.load(response)


def check_pack(job: dict, name: str, job_id: str) -> dict:
    expected = {"job_id": job_id, "host": HOST, "status": "complete", **PACKS[name]}
    observed = {key: job.get(key) for key in expected}
    # The original schema-1 jobs predate the schema field. Exact MPK comparison
    # remains mandatory; a missing schema can never qualify the schema-2 pack.
    if name == "glm" and observed["schema"] is None:
        observed["schema"] = 1
    mismatches = [key for key, value in expected.items() if observed[key] != value]
    if job.get("error"):
        mismatches.append("error")
    return {"model": name, "passed": not mismatches, "mismatches": mismatches,
            "observed": observed}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--deepseek-job", required=True)
    parser.add_argument("--glm-job", default="zlgwkqrylpqzwjzp")
    args = parser.parse_args()
    key = api_key()
    results = [check_pack(fetch_job(key, job_id), name, job_id)
               for name, job_id in (("deepseek", args.deepseek_job), ("glm", args.glm_job))]
    passed = all(result["passed"] for result in results)
    print(json.dumps({"schema": "finite-private-v41-modelpacks-v1",
                      "checked_at": datetime.now(timezone.utc).isoformat(),
                      "host": HOST, "passed": passed, "packs": results}, indent=2))
    return 0 if passed else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except urllib.error.HTTPError as error:
        print(f"model-pack gate unavailable: HTTP {error.code}", file=sys.stderr)
        raise SystemExit(2)
    except (OSError, ValueError) as error:
        print(f"model-pack gate failed: {error}", file=sys.stderr)
        raise SystemExit(2)

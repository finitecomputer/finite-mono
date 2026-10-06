#!/usr/bin/env python3
"""Gate the pinned DeepSeek relaunch on host packs, live identity, and start time."""

from __future__ import annotations

import argparse
from datetime import date, datetime, timedelta, timezone
import json
import os
import subprocess
import sys
import urllib.error

try:
    from .check_finite_private_v41_modelpacks import HOST, api_key, check_pack, fetch_job
    from .finite_private_v41_benchmark import CONTAINER_ID, ROOT, window
except ImportError:
    from check_finite_private_v41_modelpacks import HOST, api_key, check_pack, fetch_job
    from finite_private_v41_benchmark import CONTAINER_ID, ROOT, window

CANDIDATE = "v2026-09-16-deepseek-v4-1-flash-test-1"
ROLLBACK = "v2026-08-28-glm-5-3-flash-5"


def check_start_time(now: datetime, day: date) -> None:
    start, _ = window(day)
    if not start <= now < start + timedelta(minutes=10):
        raise ValueError("candidate relaunch is restricted to 03:00–03:10 Central on the selected date")


def check_container(state: dict) -> None:
    expected = {
        "id": CONTAINER_ID, "name": "finite-private", "host_name": HOST,
        "repo": "finitecomputer/confidential-finite-private", "status": "ready",
        "current_tag": ROLLBACK, "gpus": 8, "host_gpu_type": "H200",
        "auto_update": False, "debug": False, "disable_cc_mode": False,
        "update_tag": "", "update_status": "",
    }
    mismatches = [field for field, value in expected.items() if state.get(field) != value]
    if set(state.get("secrets", [])) != {
        "VLLM_API_KEY", "VLLM_INTERNAL_API_KEY", "FINITE_USAGE_API_SERVICE_KEY"
    }:
        mismatches.append("secrets")
    if mismatches:
        raise ValueError("serving container preconditions failed: " + ", ".join(mismatches))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--window-date", type=date.fromisoformat, required=True)
    parser.add_argument("--deepseek-job", required=True)
    parser.add_argument("--glm-job", default="zlgwkqrylpqzwjzp")
    parser.add_argument("--execute", action="store_true",
                        help="Relaunch after all runbook checks have passed; default is read-only")
    args = parser.parse_args()
    if args.execute:
        check_start_time(datetime.now(timezone.utc), args.window_date)
    key = api_key()
    packs = [check_pack(fetch_job(key, job_id), name, job_id)
             for name, job_id in (("deepseek", args.deepseek_job), ("glm", args.glm_job))]
    print(json.dumps({"execute": args.execute, "candidate": CANDIDATE,
                      "window_date": args.window_date.isoformat(), "packs": packs}), flush=True)
    if not all(pack["passed"] for pack in packs):
        raise ValueError("host model-pack gate failed; serving container was not changed")
    inventory = subprocess.run(
        ["tinfoil", "container", "get", CONTAINER_ID, "--output", "json"],
        capture_output=True, text=True, check=True, timeout=30,
    )
    check_container(json.loads(inventory.stdout))
    if not args.execute:
        print("Read-only launch preflight passed; no relaunch performed.")
        return 0
    # Recheck after remote calls so slow preflight cannot start a late swap.
    check_start_time(datetime.now(timezone.utc), args.window_date)
    env = {**os.environ, "FINITE_PRIVATE_CONTAINER": CONTAINER_ID,
           "FINITE_PRIVATE_RELAUNCH_APPROVED": CANDIDATE}
    return subprocess.run(
        ["bash", str(ROOT / "infra/runbooks/finite-private-ops.sh"), "relaunch", CANDIDATE],
        env=env, timeout=60, check=False,
    ).returncode


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except urllib.error.HTTPError as error:
        print(f"launch preflight unavailable: HTTP {error.code}", file=sys.stderr)
        raise SystemExit(2)
    except subprocess.TimeoutExpired:
        print("Command timed out: re-read the exact container UUID before any retry; "
              "a relaunch may have been accepted.", file=sys.stderr)
        raise SystemExit(2)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(2)

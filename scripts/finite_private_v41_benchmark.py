#!/usr/bin/env python3
"""Bounded A/B measurement driver; dry-run by default, never changes deployment."""

from __future__ import annotations

import argparse
from datetime import date, datetime, time, timezone
import json
import os
from pathlib import Path
import subprocess
import sys
from zoneinfo import ZoneInfo

CONTAINER_ID = "acc651a6-9de6-4da5-9fdc-bb9888245962"
ENDPOINT = "https://finite-private.finite.containers.tinfoil.dev"
MODELS = {"glm": "glm-5-3-flash", "deepseek": "deepseek-v4-1-flash"}
ROOT = Path(__file__).resolve().parent.parent


def utc(value: str) -> datetime:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError("deadline must include a UTC offset")
    return parsed.astimezone(timezone.utc)


def window(day: date) -> tuple[datetime, datetime]:
    central = ZoneInfo("America/Chicago")
    return tuple(datetime.combine(day, time(hour), central).astimezone(timezone.utc)
                 for hour in (3, 6))


def measurement_cutoff(day: date) -> datetime:
    return datetime.combine(day, time(5, 15), ZoneInfo("America/Chicago")).astimezone(timezone.utc)


def check_window(now: datetime, deadline: datetime, day: date) -> None:
    start, _ = window(day)
    if not start < deadline <= measurement_cutoff(day):
        raise ValueError("measurement must end by 05:15 Central to reserve GLM recovery time")
    if not start <= now < deadline:
        raise ValueError("outside the authorized measurement interval")


def command(model: str, tag: str, tier: int) -> list[str]:
    return [
        sys.executable, str(ROOT / "scripts/check_finite_private_glm53_capacity.py"),
        "--url", ENDPOINT, "--model", model, "--expected-model", model,
        "--concurrency", str(tier), "--required-concurrency", str(tier),
        "--output-tokens", "1024", "--repetitions", "3", "--warmup", "1",
        "--thinking", "on", "--reasoning-effort", "high", "--timeout", "90",
        "--minimum-p10-output-tok-s", "10", "--minimum-p50-output-tok-s", "20",
        "--minimum-aggregate-output-tok-s", "0", "--maximum-p95-ttft-s", "10",
        "--tag", tag,
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", choices=MODELS, required=True)
    parser.add_argument("--tag", required=True, help="Exact deployed release tag")
    parser.add_argument("--window-date", required=True, type=date.fromisoformat,
                        help="Authorized maintenance date in America/Chicago, YYYY-MM-DD")
    parser.add_argument("--deadline-utc", required=True, type=utc)
    parser.add_argument("--evidence-dir", type=Path, required=True)
    parser.add_argument("--execute", action="store_true", help="Send measured inference traffic")
    args = parser.parse_args()
    start, _ = window(args.window_date)
    if not start < args.deadline_utc <= measurement_cutoff(args.window_date):
        parser.error("deadline must be after 03:00 and no later than 05:15 Central")
    model = MODELS[args.model]
    tiers = (1, 8, 16, 32, 64, 128)
    if not args.execute:
        print(json.dumps({"execute": False, "model": model,
                          "not_before": start.isoformat(),
                          "deadline": args.deadline_utc.isoformat(),
                          "commands": [command(model, args.tag, c) for c in tiers]}, indent=2))
        return 0
    check_window(datetime.now(timezone.utc), args.deadline_utc, args.window_date)
    if not os.environ.get("FINITE_PRIVATE_CANARY_API_KEY"):
        parser.error("FINITE_PRIVATE_CANARY_API_KEY is required")
    args.evidence_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
    for tier in tiers:
        now = datetime.now(timezone.utc)
        check_window(now, args.deadline_utc, args.window_date)
        remaining = (args.deadline_utc - now).total_seconds()
        if remaining < 120:
            print("Insufficient time for another tier; stop measurement.", file=sys.stderr)
            return 2
        # Check control-plane identity before every tier. Inference stream identity
        # is checked separately, so a stale route cannot masquerade as the candidate.
        inventory = subprocess.run(
            ["tinfoil", "container", "get", CONTAINER_ID, "--output", "json"],
            check=True, capture_output=True, text=True, timeout=min(20, remaining),
        )
        state = json.loads(inventory.stdout)
        if (state.get("id") != CONTAINER_ID or state.get("name") != "finite-private"
                or state.get("current_tag") != args.tag or state.get("status") != "ready"
                or state.get("host_name") != "control.inf9.tinfoil.sh"
                or state.get("repo") != "finitecomputer/confidential-finite-private"
                or state.get("gpus") != 8 or state.get("host_gpu_type") != "H200"):
            raise ValueError("live deployment does not match the selected test release")
        remaining = (args.deadline_utc - datetime.now(timezone.utc)).total_seconds()
        if remaining < 120:
            return 2
        path = args.evidence_dir / f"{args.model}-c{tier}.jsonl"
        # Refuse overwriting evidence from an earlier attempt.
        with path.open("x") as output:
            os.chmod(path, 0o600)
            try:
                result = subprocess.run(command(model, args.tag, tier), stdout=output,
                                        timeout=min(300, remaining), check=False)
            except subprocess.TimeoutExpired:
                print("Measurement deadline reached; restore GLM.", file=sys.stderr)
                return 2
        print(f"{model} concurrency={tier} exit={result.returncode} evidence={path}", flush=True)
        if result.returncode:
            print("Stop escalation at the first failing tier; retain completed results.", file=sys.stderr)
            return 1
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(2)

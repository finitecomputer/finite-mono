#!/usr/bin/env python3
"""Tag and optionally push the runtime image proven by the Docker smoke."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path
from typing import Any


def run(args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, capture_output=True, text=True, check=True)


def docker_image_id(image: str) -> str:
    result = run(["docker", "image", "inspect", "--format", "{{.Id}}", image])
    return result.stdout.strip()


def docker_repo_digests(image: str) -> list[str]:
    result = run(["docker", "image", "inspect", "--format", "{{json .RepoDigests}}", image])
    value = result.stdout.strip()
    if not value or value == "null":
        return []
    return json.loads(value)


def load_passed_smoke_report(path: Path) -> dict[str, Any]:
    report = json.loads(path.read_text(encoding="utf-8"))
    if report.get("status") != "passed":
        raise SystemExit(f"smoke report is not passed: {path}")
    facts = report.get("facts")
    if not isinstance(facts, dict):
        raise SystemExit(f"smoke report is missing facts: {path}")
    if not facts.get("image_id") or not facts.get("image"):
        raise SystemExit(f"smoke report is missing image/image_id facts: {path}")
    return report


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--report", required=True, help="Docker smoke report JSON")
    parser.add_argument(
        "--image-ref", required=True, help="target image ref, e.g. ghcr.io/org/name:tag"
    )
    parser.add_argument("--publish-report", required=True, help="output JSON report path")
    parser.add_argument(
        "--require-restic-backend",
        help="fail unless the smoke report facts.restic_backend equals this value",
    )
    parser.add_argument("--push", action="store_true", help="push the tagged image")
    args = parser.parse_args()

    smoke_report_path = Path(args.report)
    publish_report_path = Path(args.publish_report)
    target_ref = args.image_ref.strip().lower()
    if not target_ref:
        raise SystemExit("--image-ref must not be empty")

    smoke = load_passed_smoke_report(smoke_report_path)
    facts = smoke["facts"]
    if args.require_restic_backend:
        actual_backend = facts.get("restic_backend")
        if actual_backend != args.require_restic_backend:
            raise SystemExit(
                "smoke report restic backend mismatch: "
                f"expected {args.require_restic_backend}, got {actual_backend}"
            )
    proven_image = str(facts["image"])
    proven_image_id = str(facts["image_id"])
    inspected_id = docker_image_id(proven_image_id)
    if inspected_id != proven_image_id:
        raise SystemExit(
            f"local Docker image mismatch: expected {proven_image_id}, inspected {inspected_id}"
        )

    report: dict[str, Any] = {
        "status": "dry_run",
        "generated_at_unix": int(time.time()),
        "source_report": str(smoke_report_path),
        "source_image": proven_image,
        "source_image_id": proven_image_id,
        "target_image_ref": target_ref,
        "pushed": False,
        "repo_digests": [],
        "proof": {
            "smoke_status": smoke["status"],
            "hermes_agent_version_actual": facts.get("hermes_agent_version_actual"),
            "restic_version": facts.get("restic_version"),
            "agent_npub_after_restore": facts.get("agent_npub_after_restore"),
            "restic_backend": facts.get("restic_backend"),
            "real_gateway_runtime": facts.get("real_gateway_runtime"),
            "gateway_admission_before_restore": facts.get("gateway_admission_before_restore"),
            "gateway_admission_after_restore": facts.get("gateway_admission_after_restore"),
        },
    }

    if args.push:
        run(["docker", "tag", proven_image_id, target_ref])
        push = run(["docker", "push", target_ref])
        report["status"] = "published"
        report["pushed"] = True
        report["push_output_tail"] = "\n".join(push.stdout.splitlines()[-8:])
        report["repo_digests"] = docker_repo_digests(target_ref)

    publish_report_path.parent.mkdir(parents=True, exist_ok=True)
    publish_report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())

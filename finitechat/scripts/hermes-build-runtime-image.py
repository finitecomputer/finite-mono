#!/usr/bin/env python3
"""Build the Hermes runtime image used by Docker smoke and Tinfoil publish."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT))
DEFAULT_HERMES_AGENT_VERSION = "0.17.0"


def run(args: list[str], *, timeout: int = 3600) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, capture_output=True, text=True, check=True, timeout=timeout)


def docker_image_metadata(image: str) -> dict[str, Any]:
    result = run(["docker", "image", "inspect", image], timeout=60)
    inspected = json.loads(result.stdout)[0]
    return {
        "id": inspected["Id"],
        "repo_tags": inspected.get("RepoTags") or [],
        "repo_digests": inspected.get("RepoDigests") or [],
        "created": inspected.get("Created"),
        "size_bytes": inspected.get("Size"),
    }


def stage_context(target: Path) -> None:
    from tests.container.test_agent_container_e2e import stage_build_context

    stage_build_context(target)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--image-ref", required=True, help="Local image tag to build")
    parser.add_argument(
        "--hermes-agent-version",
        default=DEFAULT_HERMES_AGENT_VERSION,
        help="hermes-agent package version to install in the runtime image",
    )
    parser.add_argument("--report", help="Optional build report JSON path")
    args = parser.parse_args()

    image_ref = args.image_ref.strip()
    if not image_ref:
        raise SystemExit("--image-ref must not be empty")

    started = time.monotonic()
    temp_root = REPO_ROOT / "target"
    temp_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=temp_root) as tmp_value:
        context = Path(tmp_value) / "ctx"
        context.mkdir()
        stage_context(context)
        dockerfile = context / "finitechat" / "containers" / "agent" / "Dockerfile"
        run(
            [
                "docker",
                "build",
                "--build-arg",
                f"HERMES_AGENT_VERSION={args.hermes_agent_version}",
                "--tag",
                image_ref,
                "--file",
                str(dockerfile),
                str(context),
            ],
            timeout=3600,
        )

    report = {
        "status": "built",
        "generated_at_unix": int(time.time()),
        "elapsed_ms": int((time.monotonic() - started) * 1000),
        "image": image_ref,
        "hermes_agent_version": args.hermes_agent_version,
        "image_metadata": docker_image_metadata(image_ref),
    }
    if args.report:
        report_path = Path(args.report)
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())

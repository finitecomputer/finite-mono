#!/usr/bin/env python3
"""Build the Finite Computer v2 Agent Runtime image."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_IMAGE_REF = "finitecomputer-v2-agent-runtime:local"
DEFAULT_HERMES_AGENT_VERSION = "0.18.0"

BUILD_EXCLUDES = [
    ".DS_Store",
    ".git",
    ".env",
    ".env.*",
    ".local-state",
    ".next",
    ".state",
    ".venv",
    "DerivedData",
    "ios",
    "node_modules",
    "secrets",
    "target",
    "tmp",
]


def run(
    args: list[str],
    *,
    cwd: Path | None = None,
    timeout: int = 3600,
    capture: bool = True,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        cwd=cwd,
        capture_output=capture,
        text=True,
        check=True,
        timeout=timeout,
    )


def repo_arg(default_name: str, env_name: str) -> Path:
    value = os.environ.get(env_name)
    if value:
        return Path(value).expanduser().resolve()
    return (REPO_ROOT.parent / default_name).resolve()


def git_value(repo: Path, *args: str) -> str | None:
    try:
        return run(["git", "-C", str(repo), *args], timeout=60).stdout.strip()
    except subprocess.CalledProcessError:
        return None


def repo_metadata(name: str, repo: Path) -> dict[str, Any]:
    status = git_value(repo, "status", "--short") or ""
    return {
        "name": name,
        "path": str(repo),
        "head": git_value(repo, "rev-parse", "HEAD"),
        "branch": git_value(repo, "branch", "--show-current"),
        "dirty": bool(status.strip()),
    }


def stage_repo(source: Path, dest: Path) -> None:
    if not source.is_dir():
        raise SystemExit(f"repo not found: {source}")

    dest.parent.mkdir(parents=True, exist_ok=True)
    command = ["rsync", "-a", "--delete"]
    for item in BUILD_EXCLUDES:
        command.extend(["--exclude", item])
    command.extend([f"{source}/", f"{dest}/"])
    run(command, timeout=900, capture=False)


def docker_image_metadata(image: str) -> dict[str, Any]:
    inspected = json.loads(run(["docker", "image", "inspect", image], timeout=60).stdout)[0]
    return {
        "id": inspected["Id"],
        "repo_tags": inspected.get("RepoTags") or [],
        "repo_digests": inspected.get("RepoDigests") or [],
        "created": inspected.get("Created"),
        "size_bytes": inspected.get("Size"),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--image-ref",
        default=os.environ.get("FC_RUNTIME_IMAGE_REF", DEFAULT_IMAGE_REF),
        help=f"image reference to build, default: {DEFAULT_IMAGE_REF}",
    )
    parser.add_argument(
        "--hermes-agent-version",
        default=os.environ.get("FC_RUNTIME_HERMES_AGENT_VERSION", DEFAULT_HERMES_AGENT_VERSION),
        help=f"hermes-agent package version, default: {DEFAULT_HERMES_AGENT_VERSION}",
    )
    parser.add_argument(
        "--finitechat-repo",
        type=Path,
        default=repo_arg("finite-chat-darkmatter", "FINITECHAT_REPO"),
        help="path to the finitechat checkout",
    )
    parser.add_argument(
        "--finite-sites-repo",
        type=Path,
        default=repo_arg("finite-sites", "FINITE_SITES_REPO"),
        help="path to the finite-sites checkout",
    )
    parser.add_argument(
        "--finite-brain-repo",
        type=Path,
        default=repo_arg("finite-brain", "FINITE_BRAIN_REPO"),
        help="path to the finite-brain checkout",
    )
    parser.add_argument(
        "--context-dir",
        type=Path,
        help="optional persistent staged Docker context",
    )
    parser.add_argument("--platform", help="optional docker build platform, e.g. linux/amd64")
    parser.add_argument("--no-cache", action="store_true", help="pass --no-cache to docker build")
    parser.add_argument("--push", action="store_true", help="push image after a successful build")
    parser.add_argument("--report", type=Path, help="optional build report JSON path")
    return parser.parse_args()


def build_image(args: argparse.Namespace, context: Path, repos: dict[str, Path]) -> dict[str, Any]:
    for name, repo in repos.items():
        stage_repo(repo, context / name)

    dockerfile = context / "finitecomputer-v2/deploy/finite-computer/images/runtime.Dockerfile"
    build = [
        "docker",
        "build",
        "--file",
        str(dockerfile),
        "--tag",
        args.image_ref,
        "--build-arg",
        f"HERMES_AGENT_VERSION={args.hermes_agent_version}",
        "--build-arg",
        f"FINITECOMPUTER_V2_REV={git_value(REPO_ROOT, 'rev-parse', 'HEAD') or 'unknown'}",
        "--build-arg",
        f"FINITECHAT_REV={git_value(repos['finitechat'], 'rev-parse', 'HEAD') or 'unknown'}",
        "--build-arg",
        f"FINITE_SITES_REV={git_value(repos['finite-sites'], 'rev-parse', 'HEAD') or 'unknown'}",
        "--build-arg",
        f"FINITE_BRAIN_REV={git_value(repos['finite-brain'], 'rev-parse', 'HEAD') or 'unknown'}",
    ]
    if args.platform:
        build.extend(["--platform", args.platform])
    if args.no_cache:
        build.append("--no-cache")
    build.append(str(context))
    run(build, timeout=7200, capture=False)

    if args.push:
        run(["docker", "push", args.image_ref], timeout=3600, capture=False)

    return docker_image_metadata(args.image_ref)


def main() -> int:
    args = parse_args()
    image_ref = args.image_ref.strip()
    if not image_ref:
        raise SystemExit("--image-ref must not be empty")
    args.image_ref = image_ref

    repos = {
        "finitecomputer-v2": REPO_ROOT,
        "finitechat": args.finitechat_repo.expanduser().resolve(),
        "finite-sites": args.finite_sites_repo.expanduser().resolve(),
        "finite-brain": args.finite_brain_repo.expanduser().resolve(),
    }
    repo_facts = {name: repo_metadata(name, repo) for name, repo in repos.items()}

    started = time.monotonic()
    if args.context_dir:
        context = args.context_dir.expanduser().resolve()
        context.mkdir(parents=True, exist_ok=True)
        image_metadata = build_image(args, context, repos)
    else:
        temp_parent = REPO_ROOT / "target/runtime-image"
        temp_parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=temp_parent) as tmp_value:
            context = Path(tmp_value) / "ctx"
            context.mkdir()
            image_metadata = build_image(args, context, repos)

    report = {
        "status": "built",
        "generated_at_unix": int(time.time()),
        "elapsed_ms": int((time.monotonic() - started) * 1000),
        "image": args.image_ref,
        "hermes_agent_version": args.hermes_agent_version,
        "pushed": bool(args.push),
        "platform": args.platform,
        "sources": repo_facts,
        "image_metadata": image_metadata,
    }

    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())

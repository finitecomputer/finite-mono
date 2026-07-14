#!/usr/bin/env python3
"""Build the Finite Computer v2 Agent Runtime image."""

from __future__ import annotations

import argparse
import json
import os
import platform as host_platform
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

MONOREPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_IMAGE_REF = "finitecomputer-v2-agent-runtime:local"
DEFAULT_HERMES_AGENT_VERSION = "0.18.2"
DEFAULT_IMAGE_ENGINE = "docker"
IMAGE_ENGINES = ("docker", "apple-container")

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
    repo_digests = inspected.get("RepoDigests") or []
    digest = inspected["Id"]
    if repo_digests and "@" in repo_digests[0]:
        digest = repo_digests[0].split("@", maxsplit=1)[1]
    return {
        "engine": "docker",
        "id": inspected["Id"],
        "reference": image,
        "digest": digest,
        "media_type": None,
        "repo_tags": inspected.get("RepoTags") or [],
        "repo_digests": repo_digests,
        "created": inspected.get("Created"),
        "size_bytes": inspected.get("Size"),
        "platforms": [
            {
                "os": inspected.get("Os"),
                "architecture": inspected.get("Architecture"),
                "variant": inspected.get("Variant"),
            }
        ],
    }


def apple_image_metadata(image: str) -> dict[str, Any]:
    payload = json.loads(
        run(["container", "image", "inspect", image], timeout=60).stdout
    )
    if not isinstance(payload, list) or not payload or not isinstance(payload[0], dict):
        raise SystemExit(f"unexpected Apple Container image inspect output for {image}")

    inspected = payload[0]
    configuration = inspected.get("configuration")
    if not isinstance(configuration, dict):
        configuration = {}
    descriptor = configuration.get("descriptor")
    if not isinstance(descriptor, dict):
        descriptor = {}

    platforms: list[dict[str, Any]] = []
    size_bytes = 0
    for item in inspected.get("variants") or []:
        if not isinstance(item, dict):
            continue
        item_platform = item.get("platform")
        if not isinstance(item_platform, dict):
            item_platform = {}
        variant_size = item.get("size")
        if isinstance(variant_size, int):
            size_bytes += variant_size
        platforms.append(
            {
                "os": item_platform.get("os"),
                "architecture": item_platform.get("architecture"),
                "variant": item_platform.get("variant"),
                "digest": item.get("digest"),
                "size_bytes": variant_size if isinstance(variant_size, int) else None,
            }
        )

    image_id = inspected.get("id")
    if isinstance(image_id, str) and image_id and ":" not in image_id:
        image_id = f"sha256:{image_id}"

    # Deliberately omit the inspected OCI config, labels, history, and environment.
    # Runtime image reports are build provenance, not a channel for image contents or
    # values that could have been supplied as secrets.
    return {
        "engine": "apple-container",
        "id": image_id,
        "reference": configuration.get("name") or image,
        "digest": descriptor.get("digest"),
        "media_type": descriptor.get("mediaType"),
        "created": configuration.get("creationDate"),
        "size_bytes": size_bytes or None,
        "platforms": platforms,
    }


def native_linux_platform() -> str:
    machine = host_platform.machine().lower()
    architecture = {
        "aarch64": "arm64",
        "arm64": "arm64",
        "amd64": "amd64",
        "x86_64": "amd64",
    }.get(machine)
    if architecture is None:
        raise SystemExit(
            f"unsupported native architecture for container image build: {machine}"
        )
    return f"linux/{architecture}"


def effective_build_platform(engine: str, requested: str | None) -> str | None:
    if requested:
        return requested
    if engine == "apple-container":
        return native_linux_platform()
    return None


def target_architecture(platform: str) -> str:
    parts = platform.split("/")
    if len(parts) < 2 or parts[0] != "linux" or parts[1] not in {"amd64", "arm64"}:
        raise SystemExit(f"unsupported runtime image platform: {platform}")
    return parts[1]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--engine",
        choices=IMAGE_ENGINES,
        default=os.environ.get("FC_RUNTIME_IMAGE_ENGINE", DEFAULT_IMAGE_ENGINE),
        help=(
            "image engine to use; docker remains the release/CI default, "
            "apple-container uses Apple's container CLI"
        ),
    )
    parser.add_argument(
        "--image-ref",
        default=os.environ.get("FC_RUNTIME_IMAGE_REF", DEFAULT_IMAGE_REF),
        help=f"image reference to build, default: {DEFAULT_IMAGE_REF}",
    )
    parser.add_argument(
        "--hermes-agent-version",
        default=os.environ.get(
            "FC_RUNTIME_HERMES_AGENT_VERSION", DEFAULT_HERMES_AGENT_VERSION
        ),
        help=f"hermes-agent package version, default: {DEFAULT_HERMES_AGENT_VERSION}",
    )
    parser.add_argument(
        "--context-dir",
        type=Path,
        help="optional persistent staged image build context",
    )
    parser.add_argument("--platform", help="optional image build platform, e.g. linux/amd64")
    parser.add_argument("--no-cache", action="store_true", help="disable the engine build cache")
    parser.add_argument("--push", action="store_true", help="push image after a successful build")
    parser.add_argument("--report", type=Path, help="optional build report JSON path")
    return parser.parse_args()


def build_image(
    args: argparse.Namespace,
    context: Path,
    *,
    mono_sha: str,
    platform: str | None,
) -> dict[str, Any]:
    stage_repo(MONOREPO_ROOT, context)

    dockerfile = context / "finitecomputer-v2/deploy/finite-computer/images/runtime.Dockerfile"
    if args.engine == "docker":
        build = ["docker", "build"]
    else:
        build = ["container", "build"]
    build.extend(
        [
            "--file",
            str(dockerfile),
            "--tag",
            args.image_ref,
            "--build-arg",
            f"HERMES_AGENT_VERSION={args.hermes_agent_version}",
            "--build-arg",
            f"FINITE_MONO_REV={mono_sha}",
        ]
    )
    if platform:
        build.extend(["--platform", platform])
        # Docker's legacy builder accepts --platform but does not populate the
        # BuildKit TARGETARCH argument. Pass the already-validated architecture
        # explicitly so release, smoke, and Apple builds select the same tools.
        build.extend(["--build-arg", f"TARGETARCH={target_architecture(platform)}"])
    if args.no_cache:
        build.append("--no-cache")
    build.append(str(context))
    run(build, timeout=7200, capture=False)

    if args.push:
        if args.engine == "docker":
            push = ["docker", "push", args.image_ref]
        else:
            push = ["container", "image", "push", args.image_ref]
        run(push, timeout=3600, capture=False)

    if args.engine == "docker":
        return docker_image_metadata(args.image_ref)
    return apple_image_metadata(args.image_ref)


def main() -> int:
    args = parse_args()
    image_ref = args.image_ref.strip()
    if not image_ref:
        raise SystemExit("--image-ref must not be empty")
    args.image_ref = image_ref
    if args.hermes_agent_version != DEFAULT_HERMES_AGENT_VERSION:
        raise SystemExit(
            "--hermes-agent-version is release-pinned to "
            f"{DEFAULT_HERMES_AGENT_VERSION}, got {args.hermes_agent_version}"
        )

    source_facts = repo_metadata("finite-mono", MONOREPO_ROOT)
    mono_sha = source_facts.pop("head", None)
    if not isinstance(mono_sha, str) or not mono_sha:
        raise SystemExit("finite-mono source revision is unavailable")

    platform = effective_build_platform(args.engine, args.platform)
    started = time.monotonic()
    if args.context_dir:
        context = args.context_dir.expanduser().resolve()
        context.mkdir(parents=True, exist_ok=True)
        image_metadata = build_image(args, context, mono_sha=mono_sha, platform=platform)
    else:
        temp_parent = MONOREPO_ROOT / "target/runtime-image"
        temp_parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=temp_parent) as tmp_value:
            context = Path(tmp_value) / "ctx"
            context.mkdir()
            image_metadata = build_image(args, context, mono_sha=mono_sha, platform=platform)

    report = {
        "status": "built",
        "generated_at_unix": int(time.time()),
        "elapsed_ms": int((time.monotonic() - started) * 1000),
        "image": args.image_ref,
        "engine": args.engine,
        "mono_sha": mono_sha,
        "hermes_agent_version": args.hermes_agent_version,
        "pushed": bool(args.push),
        "platform": platform,
        "source": source_facts,
        "image_metadata": image_metadata,
    }

    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())

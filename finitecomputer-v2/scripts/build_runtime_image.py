#!/usr/bin/env python3
"""Build the Finite Computer v2 Agent Runtime image."""

from __future__ import annotations

import argparse
import json
import os
import platform as host_platform
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

MONOREPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_IMAGE_REF = "finitecomputer-v2-agent-runtime:local"
DEFAULT_HERMES_AGENT_VERSION = "0.20.0"
DEFAULT_IMAGE_ENGINE = "docker"
IMAGE_ENGINES = ("docker", "apple-container")
PAYLOAD_STAGE = "finite-payload"
SEED_ARTIFACT_ID_PREFIX = "finite-agent-payload-"

# Runs inside the finite-payload stage image with the seed work directory
# mounted at /out: pack + sign the seed payload bundle with finite-release.
# The signing key never enters an image layer; it only transits the mount.
PACK_SEED_SCRIPT = """\
import json
import subprocess
from pathlib import Path

out = Path("/out")
config = json.loads((out / "seed-config.json").read_text(encoding="utf-8"))


def pack(version_label: str, artifact_id: str, dest: Path) -> dict:
    command = [
        "finite-release",
        "pack-payload",
        "--source",
        "/payload",
        "--artifact-id",
        artifact_id,
        "--version-label",
        version_label,
        "--min-shell-version",
        config["minShellVersion"],
        "--signing-key",
        str(out / "release.key"),
        "--out-dir",
        str(dest),
    ]
    if config.get("sourceGitSha"):
        command += ["--source-git-sha", config["sourceGitSha"]]
    proc = subprocess.run(command, capture_output=True, text=True, check=True)
    return json.loads(proc.stdout.strip().splitlines()[-1])


label = config.get("versionLabel")
if not label:
    # Content-addressed default for the local harness: a probe pack learns
    # the payload tree digest, the real pack bakes it into the label.
    probe = pack("probe", "probe", out / "probe")
    label = "devfinity-" + probe["treeDigest"][:12]
artifact_id = config["artifactIdPrefix"] + label
result = pack(label, artifact_id, out / "dist")
(out / "seed-report.json").write_text(
    json.dumps(result, indent=2) + "\\n", encoding="utf-8"
)
"""

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
    return native_linux_platform()


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
    parser.add_argument(
        "--seed-signing-key",
        type=Path,
        help=(
            "release signing key (hex seed file) used to sign the in-image "
            "seed payload bundle on the build host; required for a full image "
            "build (images carry no key, but the seed must verify at boot)"
        ),
    )
    parser.add_argument(
        "--seed-version-label",
        help=(
            "seed payload version label (CI passes the image version input); "
            "defaults to the content-addressed devfinity-<tree digest prefix>"
        ),
    )
    parser.add_argument(
        "--emit-payload",
        type=Path,
        help=(
            "instead of building the full image, build only the payload stage "
            "and write the UNSIGNED payload rootfs to <dir>/payload (devfinity "
            "publish-payload signs it separately)"
        ),
    )
    return parser.parse_args()


def engine_cli(engine: str) -> str:
    return "docker" if engine == "docker" else "container"


def build_command(
    args: argparse.Namespace,
    context: Path,
    tag: str,
    *,
    mono_sha: str,
    platform: str | None,
    target: str | None = None,
) -> list[str]:
    dockerfile = context / "finitecomputer-v2/deploy/finite-computer/images/runtime.Dockerfile"
    build = [
        engine_cli(args.engine),
        "build",
        "--file",
        str(dockerfile),
        "--tag",
        tag,
        "--build-arg",
        f"HERMES_AGENT_VERSION={args.hermes_agent_version}",
        "--build-arg",
        f"FINITE_MONO_REV={mono_sha}",
    ]
    if target:
        build.extend(["--target", target])
    if platform:
        build.extend(["--platform", platform])
        # Docker's legacy builder accepts --platform but does not populate the
        # BuildKit TARGETARCH argument. Pass the already-validated architecture
        # explicitly so release, smoke, and Apple builds select the same tools.
        build.extend(["--build-arg", f"TARGETARCH={target_architecture(platform)}"])
    if args.no_cache:
        build.append("--no-cache")
    build.append(str(context))
    return build


def payload_stage_ref(image_ref: str) -> str:
    """A deterministic sibling tag for the payload stage image."""
    name, separator, tag = image_ref.rpartition(":")
    if separator and "/" not in tag:
        return f"{name}:{tag}-payload"
    return f"{image_ref}:payload"


def run_in_payload_image(
    engine: str, payload_ref: str, host_dir: Path, command: list[str]
) -> None:
    run(
        [
            engine_cli(engine),
            "run",
            "--rm",
            "--volume",
            f"{host_dir}:/out",
            payload_ref,
            *command,
        ],
        timeout=3600,
        capture=False,
    )


def shell_version() -> str:
    """finite-shell's crate version: the seed's min_shell_version."""
    manifest = (MONOREPO_ROOT / "finite-shell/Cargo.toml").read_text(encoding="utf-8")
    for line in manifest.splitlines():
        if line.strip().startswith("version"):
            _, _, value = line.partition("=")
            return value.strip().strip('"')
    raise SystemExit("finite-shell/Cargo.toml has no version")


def pack_seed(
    args: argparse.Namespace,
    context: Path,
    payload_ref: str,
    *,
    mono_sha: str,
) -> dict[str, Any]:
    """Pack + sign the seed payload on the build host (via the payload stage
    image) and place the bundle into the build context as seed-payload/."""
    signing_key: Path = args.seed_signing_key.expanduser().resolve()
    if not signing_key.is_file():
        raise SystemExit(f"--seed-signing-key {signing_key} is not a file")
    seed_dir = context / "seed-payload"
    if seed_dir.exists():
        shutil.rmtree(seed_dir)
    seed_dir.mkdir(parents=True)

    work_parent = MONOREPO_ROOT / "target/runtime-image"
    work_parent.mkdir(parents=True, exist_ok=True)
    workdir = Path(tempfile.mkdtemp(prefix="seed-pack-", dir=work_parent))
    try:
        (workdir / "seed-config.json").write_text(
            json.dumps(
                {
                    "versionLabel": args.seed_version_label,
                    "minShellVersion": shell_version(),
                    "sourceGitSha": mono_sha,
                    "artifactIdPrefix": SEED_ARTIFACT_ID_PREFIX,
                }
            )
            + "\n",
            encoding="utf-8",
        )
        key_copy = workdir / "release.key"
        key_copy.write_text(signing_key.read_text(encoding="utf-8"), encoding="utf-8")
        key_copy.chmod(0o600)
        (workdir / "pack_seed.py").write_text(PACK_SEED_SCRIPT, encoding="utf-8")

        run_in_payload_image(
            args.engine, payload_ref, workdir, ["python3", "/out/pack_seed.py"]
        )

        seed_report = json.loads(
            (workdir / "seed-report.json").read_text(encoding="utf-8")
        )
        artifact_id = seed_report["artifactId"]
        dist = workdir / "dist"
        shutil.copyfile(dist / f"{artifact_id}.tar.gz", seed_dir / "payload.tar.gz")
        shutil.copyfile(
            dist / f"{artifact_id}.tar.gz.manifest.json",
            seed_dir / "payload.tar.gz.manifest.json",
        )
        return seed_report
    finally:
        shutil.rmtree(workdir, ignore_errors=True)


def emit_payload(args: argparse.Namespace, payload_ref: str) -> Path:
    """Copy the UNSIGNED payload rootfs out of the payload stage image."""
    emit_dir = args.emit_payload.expanduser().resolve()
    emit_dir.mkdir(parents=True, exist_ok=True)
    destination = emit_dir / "payload"
    if destination.exists():
        shutil.rmtree(destination)
    run_in_payload_image(
        args.engine,
        payload_ref,
        emit_dir,
        ["sh", "-ec", "cp -a /payload /out/payload"],
    )
    if not (destination / "bin/finite-agentd").is_file():
        raise SystemExit(f"emitted payload at {destination} is missing bin/finite-agentd")
    return destination


def build_image(
    args: argparse.Namespace,
    context: Path,
    *,
    mono_sha: str,
    platform: str | None,
) -> tuple[dict[str, Any] | None, dict[str, Any] | None]:
    stage_repo(MONOREPO_ROOT, context)

    # Phase 1: the payload stage (also the extraction vehicle for
    # --emit-payload and the seed pack, which runs finite-release inside it).
    payload_ref = payload_stage_ref(args.image_ref)
    run(
        build_command(
            args,
            context,
            payload_ref,
            mono_sha=mono_sha,
            platform=platform,
            target=PAYLOAD_STAGE,
        ),
        timeout=7200,
        capture=False,
    )

    if args.emit_payload:
        emit_payload(args, payload_ref)
        return None, None

    # Phase 2: sign the seed on the build host, then build the shell image
    # with the signed seed in its context.
    seed_report = pack_seed(args, context, payload_ref, mono_sha=mono_sha)
    run(
        build_command(args, context, args.image_ref, mono_sha=mono_sha, platform=platform),
        timeout=7200,
        capture=False,
    )

    if args.push:
        if args.engine == "docker":
            push = ["docker", "push", args.image_ref]
        else:
            push = ["container", "image", "push", args.image_ref]
        run(push, timeout=3600, capture=False)

    if args.engine == "docker":
        return docker_image_metadata(args.image_ref), seed_report
    return apple_image_metadata(args.image_ref), seed_report


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

    if args.emit_payload is None and args.seed_signing_key is None:
        raise SystemExit(
            "--seed-signing-key is required for a full image build: the image "
            "carries no key, but its seed payload must verify at boot"
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
        image_metadata, seed_report = build_image(
            args, context, mono_sha=mono_sha, platform=platform
        )
    else:
        temp_parent = MONOREPO_ROOT / "target/runtime-image"
        temp_parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=temp_parent) as tmp_value:
            context = Path(tmp_value) / "ctx"
            context.mkdir()
            image_metadata, seed_report = build_image(
                args, context, mono_sha=mono_sha, platform=platform
            )

    if args.emit_payload is not None:
        emitted = {
            "status": "payload_emitted",
            "engine": args.engine,
            "mono_sha": mono_sha,
            "payloadRootfs": str(args.emit_payload.expanduser().resolve() / "payload"),
            "elapsed_ms": int((time.monotonic() - started) * 1000),
        }
        print(json.dumps(emitted, indent=2))
        return 0

    assert seed_report is not None
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
        "payloadVersionLabel": seed_report.get("versionLabel"),
        "payloadTreeDigest": seed_report.get("treeDigest"),
        "payloadSha256": seed_report.get("tarballSha256"),
    }

    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())

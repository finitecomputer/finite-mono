#!/usr/bin/env python3
"""Provider-thin delivery contracts used by local tools and CI workflows."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import re
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path
from typing import Callable, Sequence


RELEASE_SCHEMA = "finite-release/v1"
COMPONENTS = frozenset({"finitechat", "fbrain", "fsite"})
COMPONENT_MANIFESTS = {
    "finitechat": Path("finitechat/crates/finitechat-cli/Cargo.toml"),
    "fbrain": Path("finite-brain/crates/finite-brain-cli/Cargo.toml"),
    "fsite": Path("finite-sites/crates/fsite-cli/Cargo.toml"),
}
DIGEST_RE = re.compile(r"sha256:[0-9a-f]{64}\Z")
SOURCE_SHA_RE = re.compile(r"[0-9a-f]{40}\Z")
VERSION_RE = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?\Z")


class DeliveryError(ValueError):
    """A delivery contract was not satisfied."""


CommandRunner = Callable[..., subprocess.CompletedProcess[str]]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _validate_checksum_pairs(paths: Sequence[Path]) -> None:
    paths_by_name = {path.name: path for path in paths}
    archives = [path for path in paths if not path.name.endswith(".sha256")]
    checksums = [path for path in paths if path.name.endswith(".sha256")]

    for archive_path in archives:
        checksum_name = f"{archive_path.name}.sha256"
        if checksum_name not in paths_by_name:
            raise DeliveryError(f"release asset is missing checksum: {archive_path.name}")

    for checksum_path in checksums:
        archive_name = checksum_path.name.removesuffix(".sha256")
        archive_path = paths_by_name.get(archive_name)
        if archive_path is None:
            raise DeliveryError(f"checksum has no matching asset: {checksum_path.name}")
        fields = checksum_path.read_text(encoding="utf-8").strip().split()
        if len(fields) < 2 or fields[-1].removeprefix("*") != archive_path.name:
            raise DeliveryError(f"invalid checksum record: {checksum_path.name}")
        if fields[0].lower() != sha256_file(archive_path):
            raise DeliveryError(f"checksum mismatch: {archive_path.name}")


def build_release_metadata(
    *,
    component: str,
    version: str,
    source_sha: str,
    run_id: str,
    assets_dir: Path,
) -> dict[str, object]:
    if component not in COMPONENTS:
        raise DeliveryError(f"unsupported release component: {component}")
    if not VERSION_RE.fullmatch(version):
        raise DeliveryError(f"invalid release version: {version}")
    if not SOURCE_SHA_RE.fullmatch(source_sha):
        raise DeliveryError("source SHA must be a lowercase 40-character git SHA")
    if not run_id.strip():
        raise DeliveryError("build run identifier is required")
    if not assets_dir.is_dir():
        raise DeliveryError(f"asset directory does not exist: {assets_dir}")

    paths = sorted(path for path in assets_dir.iterdir() if path.is_file())
    if not paths:
        raise DeliveryError("release has no assets")
    if component == "finitechat" and any("electron" in path.name for path in paths):
        raise DeliveryError("Electron assets are outside the CLI release path")

    _validate_checksum_pairs(paths)

    return {
        "schema": RELEASE_SCHEMA,
        "component": component,
        "version": version,
        "tag": f"{component}/v{version}",
        "source_sha": source_sha,
        "build_run": run_id,
        "assets": [
            {
                "name": path.name,
                "sha256": sha256_file(path),
                "size": path.stat().st_size,
            }
            for path in paths
        ],
    }


def verify_component_version(component: str, version: str, repository_root: Path) -> None:
    if component not in COMPONENT_MANIFESTS:
        raise DeliveryError(f"unsupported release component: {component}")
    if not VERSION_RE.fullmatch(version):
        raise DeliveryError(f"invalid release version: {version}")
    manifest_path = repository_root / COMPONENT_MANIFESTS[component]
    try:
        manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
        package_version = str(manifest["package"]["version"])
    except (OSError, KeyError, tomllib.TOMLDecodeError) as error:
        raise DeliveryError(f"cannot read component manifest {manifest_path}: {error}") from error
    if package_version != version:
        raise DeliveryError(
            f"release version {version} does not match {component} package version "
            f"{package_version}"
        )


def verify_remote_release_assets(
    *, local_assets: Sequence[Path], downloaded_dir: Path
) -> None:
    for local_path in local_assets:
        remote_path = downloaded_dir / local_path.name
        if not remote_path.is_file():
            raise DeliveryError(f"release is missing remote asset: {local_path.name}")
        if sha256_file(local_path) != sha256_file(remote_path):
            raise DeliveryError(
                f"refusing to replace versioned asset with different bytes: {local_path.name}"
            )


def _validated_versioned_release(
    *,
    component: str,
    version: str,
    expected_source_sha: str | None,
    downloaded_dir: Path,
) -> tuple[dict[str, object], bytes, list[Path]]:
    metadata_path = downloaded_dir / "release.json"
    try:
        metadata_bytes = metadata_path.read_bytes()
        metadata = json.loads(metadata_bytes)
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise DeliveryError(f"versioned release metadata is invalid: {error}") from error
    if not isinstance(metadata, dict):
        raise DeliveryError("versioned release metadata is not an object")
    expected_facts = {
        "schema": RELEASE_SCHEMA,
        "component": component,
        "version": version,
        "tag": f"{component}/v{version}",
    }
    for key, value in expected_facts.items():
        if metadata.get(key) != value:
            raise DeliveryError(f"versioned release metadata has the wrong {key}")
    source_sha = metadata.get("source_sha")
    if not isinstance(source_sha, str) or not SOURCE_SHA_RE.fullmatch(source_sha):
        raise DeliveryError("versioned release metadata has an invalid source SHA")
    if expected_source_sha is not None and source_sha != expected_source_sha:
        raise DeliveryError("versioned release metadata has a different source SHA")
    build_run = metadata.get("build_run")
    if not isinstance(build_run, str) or not build_run.strip():
        raise DeliveryError("versioned release metadata has no build run")
    asset_records = metadata.get("assets")
    if not isinstance(asset_records, list) or not asset_records:
        raise DeliveryError("versioned release metadata has no assets")

    local_assets = sorted(
        path for path in downloaded_dir.iterdir() if path.is_file() and path.name != "release.json"
    )
    records_by_name: dict[str, dict[str, object]] = {}
    for record in asset_records:
        if not isinstance(record, dict) or not isinstance(record.get("name"), str):
            raise DeliveryError("versioned release metadata has an invalid asset record")
        name = str(record["name"])
        if name in records_by_name:
            raise DeliveryError(f"versioned release metadata repeats asset: {name}")
        records_by_name[name] = record
    if set(records_by_name) != {path.name for path in local_assets}:
        raise DeliveryError("versioned release assets do not match release metadata")
    for path in local_assets:
        record = records_by_name[path.name]
        if record.get("sha256") != sha256_file(path) or record.get("size") != path.stat().st_size:
            raise DeliveryError(f"versioned release asset does not match metadata: {path.name}")
    _validate_checksum_pairs(local_assets)
    return metadata, metadata_bytes, [*local_assets, metadata_path]


def canonical_release_metadata(
    existing_bytes: bytes, candidate: dict[str, object]
) -> bytes:
    """Reuse immutable provenance when a matching release is retried."""
    try:
        existing = json.loads(existing_bytes)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise DeliveryError(f"existing release metadata is invalid: {error}") from error
    if not isinstance(existing, dict):
        raise DeliveryError("existing release metadata is not an object")

    existing_facts = {key: value for key, value in existing.items() if key != "build_run"}
    candidate_facts = {
        key: value for key, value in candidate.items() if key != "build_run"
    }
    if existing_facts != candidate_facts:
        raise DeliveryError("release metadata already exists with different facts")
    return existing_bytes


def _run(
    command: Sequence[str],
    *,
    command_runner: CommandRunner,
    allow_not_found: bool = False,
) -> subprocess.CompletedProcess[str]:
    result = command_runner(
        list(command),
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode == 0:
        return result
    if allow_not_found and (
        "404" in result.stderr or "not found" in result.stderr.lower()
    ):
        return result
    detail = result.stderr.strip() or result.stdout.strip() or "unknown error"
    raise DeliveryError(f"command failed ({' '.join(command[:3])}): {detail}")


def _release_assets(
    *,
    repository: str,
    tag: str,
    command_runner: CommandRunner,
) -> set[str] | None:
    result = _run(
        ["gh", "release", "view", tag, "--repo", repository, "--json", "assets"],
        command_runner=command_runner,
        allow_not_found=True,
    )
    if result.returncode != 0:
        return None
    payload = json.loads(result.stdout)
    return {asset["name"] for asset in payload["assets"]}


def _download_release_assets(
    *,
    repository: str,
    tag: str,
    names: Sequence[str],
    destination: Path,
    command_runner: CommandRunner,
) -> None:
    destination.mkdir(parents=True, exist_ok=True)
    for name in names:
        _run(
            [
                "gh",
                "release",
                "download",
                tag,
                "--repo",
                repository,
                "--pattern",
                name,
                "--dir",
                str(destination),
            ],
            command_runner=command_runner,
        )


def _ensure_metadata_commit(
    *,
    repository: str,
    component: str,
    version: str,
    metadata: dict[str, object],
    metadata_bytes: bytes,
    command_runner: CommandRunner,
) -> tuple[str, bytes]:
    metadata_path = f"releases/{component}/v{version}.json"
    endpoint = f"repos/{repository}/contents/{metadata_path}"
    existing = _run(
        ["gh", "api", endpoint],
        command_runner=command_runner,
        allow_not_found=True,
    )
    if existing.returncode == 0:
        payload = json.loads(existing.stdout)
        existing_bytes = base64.b64decode(payload["content"])
        metadata_bytes = canonical_release_metadata(existing_bytes, metadata)
        commits = _run(
            [
                "gh",
                "api",
                "--method",
                "GET",
                f"repos/{repository}/commits",
                "-f",
                f"path={metadata_path}",
                "-F",
                "per_page=1",
            ],
            command_runner=command_runner,
        )
        return json.loads(commits.stdout)[0]["sha"], metadata_bytes

    created = _run(
        [
            "gh",
            "api",
            "--method",
            "PUT",
            endpoint,
            "-f",
            f"message=Record {component}/v{version}",
            "-f",
            f"content={base64.b64encode(metadata_bytes).decode('ascii')}",
            "-f",
            "branch=main",
        ],
        command_runner=command_runner,
    )
    return json.loads(created.stdout)["commit"]["sha"], metadata_bytes


def _ensure_tag(
    *,
    repository: str,
    tag: str,
    commit_sha: str,
    movable: bool,
    command_runner: CommandRunner,
) -> None:
    endpoint = f"repos/{repository}/git/ref/tags/{tag}"
    existing = _run(
        ["gh", "api", endpoint],
        command_runner=command_runner,
        allow_not_found=True,
    )
    if existing.returncode == 0:
        current_sha = json.loads(existing.stdout)["object"]["sha"]
        if current_sha == commit_sha:
            return
        if not movable:
            raise DeliveryError(f"version tag already points at a different commit: {tag}")
        _run(
            [
                "gh",
                "api",
                "--method",
                "PATCH",
                f"repos/{repository}/git/refs/tags/{tag}",
                "-f",
                f"sha={commit_sha}",
                "-F",
                "force=true",
            ],
            command_runner=command_runner,
        )
        return
    _run(
        [
            "gh",
            "api",
            "--method",
            "POST",
            f"repos/{repository}/git/refs",
            "-f",
            f"ref=refs/tags/{tag}",
            "-f",
            f"sha={commit_sha}",
        ],
        command_runner=command_runner,
    )


def _publish_alias(
    *,
    repository: str,
    component: str,
    version_tag: str,
    alias_tag: str,
    source_sha: str,
    run_id: str,
    local_assets: Sequence[Path],
    command_runner: CommandRunner,
) -> None:
    local_by_name = {path.name: path for path in local_assets}
    alias_names = _release_assets(
        repository=repository,
        tag=alias_tag,
        command_runner=command_runner,
    )
    notes = f"Rolling alias for {version_tag} (source {source_sha}; build {run_id})."
    if alias_names is None:
        _run(
            [
                "gh",
                "release",
                "create",
                alias_tag,
                *[str(path) for path in local_assets],
                "--repo",
                repository,
                "--title",
                f"{component} (latest)",
                "--notes",
                notes,
                "--latest=false",
            ],
            command_runner=command_runner,
        )
        return

    for obsolete in sorted(alias_names - local_by_name.keys()):
        _run(
            [
                "gh",
                "release",
                "delete-asset",
                alias_tag,
                obsolete,
                "--repo",
                repository,
                "--yes",
            ],
            command_runner=command_runner,
        )
    _run(
        [
            "gh",
            "release",
            "upload",
            alias_tag,
            *[str(path) for path in local_assets],
            "--repo",
            repository,
            "--clobber",
        ],
        command_runner=command_runner,
    )
    _run(
        [
            "gh",
            "release",
            "edit",
            alias_tag,
            "--repo",
            repository,
            "--notes",
            notes,
        ],
        command_runner=command_runner,
    )


def _versioned_release_is_complete(
    *,
    component: str,
    version: str,
    expected_source_sha: str,
    repository: str,
    remote_names: set[str],
    command_runner: CommandRunner,
) -> bool:
    with tempfile.TemporaryDirectory(prefix="finite-release-retry-check-") as directory:
        downloaded_dir = Path(directory)
        _download_release_assets(
            repository=repository,
            tag=f"{component}/v{version}",
            names=sorted(remote_names),
            destination=downloaded_dir,
            command_runner=command_runner,
        )
        try:
            _validated_versioned_release(
                component=component,
                version=version,
                expected_source_sha=expected_source_sha,
                downloaded_dir=downloaded_dir,
            )
        except DeliveryError:
            return False
    return True


def promote_release_alias(
    *,
    component: str,
    version: str,
    expected_source_sha: str | None,
    repository: str,
    command_runner: CommandRunner = subprocess.run,
) -> None:
    if component not in COMPONENTS:
        raise DeliveryError(f"unsupported release component: {component}")
    if not VERSION_RE.fullmatch(version):
        raise DeliveryError(f"invalid release version: {version}")
    if expected_source_sha is not None and not SOURCE_SHA_RE.fullmatch(expected_source_sha):
        raise DeliveryError("expected source SHA must be a lowercase 40-character git SHA")

    version_tag = f"{component}/v{version}"
    alias_tag = f"{component}-latest"
    remote_names = _release_assets(
        repository=repository,
        tag=version_tag,
        command_runner=command_runner,
    )
    if remote_names is None:
        raise DeliveryError(f"versioned release does not exist: {version_tag}")
    with tempfile.TemporaryDirectory(prefix="finite-alias-promotion-") as directory:
        downloaded_dir = Path(directory)
        _download_release_assets(
            repository=repository,
            tag=version_tag,
            names=sorted(remote_names),
            destination=downloaded_dir,
            command_runner=command_runner,
        )
        metadata, metadata_bytes, local_assets = _validated_versioned_release(
            component=component,
            version=version,
            expected_source_sha=expected_source_sha,
            downloaded_dir=downloaded_dir,
        )
        metadata_commit, _ = _ensure_metadata_commit(
            repository=repository,
            component=component,
            version=version,
            metadata=metadata,
            metadata_bytes=metadata_bytes,
            command_runner=command_runner,
        )
        _ensure_tag(
            repository=repository,
            tag=version_tag,
            commit_sha=metadata_commit,
            movable=False,
            command_runner=command_runner,
        )
        _ensure_tag(
            repository=repository,
            tag=alias_tag,
            commit_sha=metadata_commit,
            movable=True,
            command_runner=command_runner,
        )
        _publish_alias(
            repository=repository,
            component=component,
            version_tag=version_tag,
            alias_tag=alias_tag,
            source_sha=str(metadata["source_sha"]),
            run_id=str(metadata["build_run"]),
            local_assets=local_assets,
            command_runner=command_runner,
        )


def publish_release(
    *,
    component: str,
    version: str,
    source_sha: str,
    run_id: str,
    assets_dir: Path,
    repository: str,
    refresh_alias: bool = True,
    command_runner: CommandRunner = subprocess.run,
) -> None:
    metadata = build_release_metadata(
        component=component,
        version=version,
        source_sha=source_sha,
        run_id=run_id,
        assets_dir=assets_dir,
    )
    metadata_bytes = (json.dumps(metadata, indent=2, sort_keys=True) + "\n").encode()
    version_tag = f"{component}/v{version}"
    alias_tag = f"{component}-latest"

    # A rerun after immutable publication must reuse the original bytes. Build
    # output may differ in irrelevant archive metadata, so never compare a new
    # archive to an already-complete versioned release merely to repair its
    # rolling alias.
    existing_remote_names = _release_assets(
        repository=repository,
        tag=version_tag,
        command_runner=command_runner,
    )
    if existing_remote_names is not None and _versioned_release_is_complete(
        component=component,
        version=version,
        expected_source_sha=source_sha,
        repository=repository,
        remote_names=existing_remote_names,
        command_runner=command_runner,
    ):
        if refresh_alias:
            promote_release_alias(
                component=component,
                version=version,
                expected_source_sha=source_sha,
                repository=repository,
                command_runner=command_runner,
            )
        return

    with tempfile.TemporaryDirectory(prefix="finite-release-") as directory:
        temporary = Path(directory)
        metadata_asset = temporary / "release.json"
        local_assets = sorted(path for path in assets_dir.iterdir() if path.is_file())

        metadata_commit, metadata_bytes = _ensure_metadata_commit(
            repository=repository,
            component=component,
            version=version,
            metadata=metadata,
            metadata_bytes=metadata_bytes,
            command_runner=command_runner,
        )
        metadata_asset.write_bytes(metadata_bytes)
        local_assets.append(metadata_asset)
        local_by_name = {path.name: path for path in local_assets}
        _ensure_tag(
            repository=repository,
            tag=version_tag,
            commit_sha=metadata_commit,
            movable=False,
            command_runner=command_runner,
        )

        remote_names = _release_assets(
            repository=repository,
            tag=version_tag,
            command_runner=command_runner,
        )
        if remote_names is None:
            _run(
                [
                    "gh",
                    "release",
                    "create",
                    version_tag,
                    *[str(path) for path in local_assets],
                    "--repo",
                    repository,
                    "--title",
                    version_tag,
                    "--notes",
                    f"Source commit: {source_sha}\nBuild run: {run_id}",
                    "--latest=false",
                ],
                command_runner=command_runner,
            )
        else:
            unexpected = remote_names - local_by_name.keys()
            if unexpected:
                raise DeliveryError(
                    "versioned release contains unexpected assets: "
                    + ", ".join(sorted(unexpected))
                )
            if remote_names:
                existing_dir = temporary / "existing"
                _download_release_assets(
                    repository=repository,
                    tag=version_tag,
                    names=sorted(remote_names),
                    destination=existing_dir,
                    command_runner=command_runner,
                )
                verify_remote_release_assets(
                    local_assets=[local_by_name[name] for name in sorted(remote_names)],
                    downloaded_dir=existing_dir,
                )
            missing = local_by_name.keys() - remote_names
            if missing:
                _run(
                    [
                        "gh",
                        "release",
                        "upload",
                        version_tag,
                        *[str(local_by_name[name]) for name in sorted(missing)],
                        "--repo",
                        repository,
                    ],
                    command_runner=command_runner,
                )

        verified_dir = temporary / "verified"
        _download_release_assets(
            repository=repository,
            tag=version_tag,
            names=sorted(local_by_name),
            destination=verified_dir,
            command_runner=command_runner,
        )
        verify_remote_release_assets(
            local_assets=local_assets,
            downloaded_dir=verified_dir,
        )

        if refresh_alias:
            _ensure_tag(
                repository=repository,
                tag=alias_tag,
                commit_sha=metadata_commit,
                movable=True,
                command_runner=command_runner,
            )
            _publish_alias(
                repository=repository,
                component=component,
                version_tag=version_tag,
                alias_tag=alias_tag,
                source_sha=source_sha,
                run_id=run_id,
                local_assets=local_assets,
                command_runner=command_runner,
            )


def verify_image_promotion(source_digest: str, destination_digest: str) -> None:
    if not DIGEST_RE.fullmatch(source_digest) or not DIGEST_RE.fullmatch(
        destination_digest
    ):
        raise DeliveryError("image digest must be sha256 followed by 64 lowercase hex characters")
    if source_digest != destination_digest:
        raise DeliveryError(
            f"image digest changed during promotion: {source_digest} != {destination_digest}"
        )


def verify_image_index(
    manifest_path: Path, *, platform: str, require_attestation: bool
) -> None:
    try:
        os_name, architecture = platform.split("/", maxsplit=1)
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (ValueError, OSError, json.JSONDecodeError) as error:
        raise DeliveryError(f"cannot read image index: {error}") from error
    descriptors = manifest.get("manifests") if isinstance(manifest, dict) else None
    if not isinstance(descriptors, list):
        raise DeliveryError("published image is not an OCI image index")

    has_platform = False
    has_attestation = False
    for descriptor in descriptors:
        if not isinstance(descriptor, dict):
            continue
        descriptor_platform = descriptor.get("platform")
        if not isinstance(descriptor_platform, dict):
            descriptor_platform = {}
        if (
            descriptor_platform.get("os") == os_name
            and descriptor_platform.get("architecture") == architecture
        ):
            has_platform = True
        annotations = descriptor.get("annotations")
        annotation_text = json.dumps(annotations or {}).lower()
        if (
            descriptor_platform.get("os") == "unknown"
            and descriptor_platform.get("architecture") == "unknown"
            and "attestation" in annotation_text
        ):
            has_attestation = True
    if not has_platform:
        raise DeliveryError(f"published image index is missing platform: {platform}")
    if require_attestation and not has_attestation:
        raise DeliveryError("published image index is missing an attestation manifest")


def require_production_disabled(manifest_path: Path) -> None:
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise DeliveryError(f"cannot read production manifest: {error}") from error
    if manifest.get("environment") != "production":
        raise DeliveryError("expected the production deployment manifest")
    if manifest.get("mutation_enabled") is not False:
        raise DeliveryError("production mutation must remain disabled during migration")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    metadata = commands.add_parser("release-metadata")
    metadata.add_argument("--component", required=True, choices=sorted(COMPONENTS))
    metadata.add_argument("--version", required=True)
    metadata.add_argument("--source-sha", required=True)
    metadata.add_argument("--run-id", required=True)
    metadata.add_argument("--assets-dir", type=Path, required=True)
    metadata.add_argument("--output", type=Path, required=True)

    component_version = commands.add_parser("verify-component-version")
    component_version.add_argument(
        "--component", required=True, choices=sorted(COMPONENTS)
    )
    component_version.add_argument("--version", required=True)
    component_version.add_argument("--repository-root", type=Path, default=Path("."))

    image = commands.add_parser("verify-image-promotion")
    image.add_argument("--source-digest", required=True)
    image.add_argument("--destination-digest", required=True)

    production = commands.add_parser("require-production-disabled")
    production.add_argument("--manifest", type=Path, required=True)

    publish = commands.add_parser("publish-release")
    publish.add_argument("--component", required=True, choices=sorted(COMPONENTS))
    publish.add_argument("--version", required=True)
    publish.add_argument("--source-sha", required=True)
    publish.add_argument("--run-id", required=True)
    publish.add_argument("--assets-dir", type=Path, required=True)
    publish.add_argument(
        "--repository", default="finitecomputer/finite-releases"
    )

    promote_alias = commands.add_parser("promote-release-alias")
    promote_alias.add_argument("--component", required=True, choices=sorted(COMPONENTS))
    promote_alias.add_argument("--version", required=True)
    promote_alias.add_argument("--expected-source-sha")
    promote_alias.add_argument(
        "--repository", default="finitecomputer/finite-releases"
    )

    image_index = commands.add_parser("verify-image-index")
    image_index.add_argument("--manifest", type=Path, required=True)
    image_index.add_argument("--platform", required=True)
    image_index.add_argument("--require-attestation", action="store_true")
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        if args.command == "release-metadata":
            metadata = build_release_metadata(
                component=args.component,
                version=args.version,
                source_sha=args.source_sha,
                run_id=args.run_id,
                assets_dir=args.assets_dir,
            )
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(
                json.dumps(metadata, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )
        elif args.command == "verify-component-version":
            verify_component_version(
                args.component,
                args.version,
                args.repository_root,
            )
        elif args.command == "verify-image-promotion":
            verify_image_promotion(args.source_digest, args.destination_digest)
        elif args.command == "require-production-disabled":
            require_production_disabled(args.manifest)
        elif args.command == "publish-release":
            publish_release(
                component=args.component,
                version=args.version,
                source_sha=args.source_sha,
                run_id=args.run_id,
                assets_dir=args.assets_dir,
                repository=args.repository,
            )
        elif args.command == "promote-release-alias":
            promote_release_alias(
                component=args.component,
                version=args.version,
                expected_source_sha=args.expected_source_sha,
                repository=args.repository,
            )
        elif args.command == "verify-image-index":
            verify_image_index(
                args.manifest,
                platform=args.platform,
                require_attestation=args.require_attestation,
            )
        else:  # pragma: no cover - argparse prevents this branch
            raise DeliveryError(f"unsupported command: {args.command}")
    except DeliveryError as error:
        print(f"delivery contract failed: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

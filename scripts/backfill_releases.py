#!/usr/bin/env python3
"""Copy immutable CLI releases from finite-mono to finite-releases."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import tempfile
from pathlib import Path
from typing import Sequence

try:
    from scripts import delivery
except ImportError:  # Executed directly as scripts/backfill_releases.py.
    import delivery  # type: ignore[no-redef]


TAG_RE = re.compile(
    r"(?P<component>finitechat|fbrain|fsite)/v"
    r"(?P<major>\d+)\.(?P<minor>\d+)\.(?P<patch>\d+)\Z"
)


def versioned(releases: Sequence[dict[str, object]]) -> list[dict[str, object]]:
    selected: list[tuple[tuple[object, ...], dict[str, object]]] = []
    for release in releases:
        tag = str(release.get("tag_name", ""))
        match = TAG_RE.fullmatch(tag)
        if match is None:
            continue
        key: tuple[object, ...] = (
            match.group("component"),
            int(match.group("major")),
            int(match.group("minor")),
            int(match.group("patch")),
        )
        selected.append((key, release))
    return [release for _key, release in sorted(selected, key=lambda item: item[0])]


def release_facts(tag: str) -> tuple[str, str, tuple[int, int, int]]:
    match = TAG_RE.fullmatch(tag)
    if match is None:
        raise delivery.DeliveryError(f"not a component semver tag: {tag}")
    version_key = (
        int(match.group("major")),
        int(match.group("minor")),
        int(match.group("patch")),
    )
    version = f"{version_key[0]}.{version_key[1]}.{version_key[2]}"
    return match.group("component"), version, version_key


def cli_assets(assets: Sequence[dict[str, object]]) -> list[dict[str, object]]:
    return [
        asset
        for asset in assets
        if "electron" not in str(asset.get("name", "")).lower()
        and str(asset.get("name", "")) != "latest-mac.yml"
    ]


def run_json(command: Sequence[str]) -> object:
    result = subprocess.run(command, check=False, capture_output=True, text=True)
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip()
        raise delivery.DeliveryError(f"command failed ({' '.join(command[:3])}): {detail}")
    return json.loads(result.stdout)


def source_commit(repository: str, tag: str) -> str:
    payload = run_json(["gh", "api", f"repos/{repository}/commits/{tag}"])
    assert isinstance(payload, dict)
    sha = str(payload["sha"])
    if not delivery.SOURCE_SHA_RE.fullmatch(sha):
        raise delivery.DeliveryError(f"tag did not resolve to a source commit: {tag}")
    return sha


def download_assets(
    *, source_repository: str, tag: str, assets: Sequence[dict[str, object]], output: Path
) -> None:
    output.mkdir(parents=True, exist_ok=True)
    for asset in assets:
        name = str(asset["name"])
        result = subprocess.run(
            [
                "gh",
                "release",
                "download",
                tag,
                "--repo",
                source_repository,
                "--pattern",
                name,
                "--dir",
                str(output),
            ],
            check=False,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            raise delivery.DeliveryError(
                f"could not download {tag}/{name}: {result.stderr.strip()}"
            )
        expected = str(asset.get("digest", ""))
        actual = f"sha256:{delivery.sha256_file(output / name)}"
        if expected != actual:
            raise delivery.DeliveryError(
                f"source release digest mismatch for {tag}/{name}: {expected} != {actual}"
            )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", default="finitecomputer/finite-mono")
    parser.add_argument("--destination", default="finitecomputer/finite-releases")
    parser.add_argument("--component", choices=sorted(delivery.COMPONENTS))
    parser.add_argument("--tag", action="append")
    parser.add_argument("--dry-run", action="store_true")
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    payload = run_json(["gh", "api", f"repos/{args.source}/releases?per_page=100"])
    assert isinstance(payload, list)
    candidates = versioned(payload)
    if args.component:
        candidates = [
            release
            for release in candidates
            if str(release["tag_name"]).startswith(f"{args.component}/")
        ]
    if args.tag:
        requested_tags = set(args.tag)
        candidates = [
            release for release in candidates if release["tag_name"] in requested_tags
        ]
    if not candidates:
        raise delivery.DeliveryError("no matching versioned releases")

    newest: dict[str, tuple[tuple[int, int, int], str, str | None]] = {}
    for release in candidates:
        tag = str(release["tag_name"])
        component, version, version_key = release_facts(tag)
        assets_value = release.get("assets", [])
        assert isinstance(assets_value, list)
        assets = cli_assets(assets_value)
        if not assets:
            raise delivery.DeliveryError(f"release has no CLI assets: {tag}")
        print(f"{tag}: {len(assets)} CLI assets")
        if args.dry_run:
            previous = newest.get(component)
            if previous is None or previous[0] < version_key:
                newest[component] = (version_key, version, None)
            continue

        source_sha = source_commit(args.source, tag)
        with tempfile.TemporaryDirectory(prefix="finite-backfill-") as directory:
            assets_dir = Path(directory)
            download_assets(
                source_repository=args.source,
                tag=tag,
                assets=assets,
                output=assets_dir,
            )
            delivery.publish_release(
                component=component,
                version=version,
                source_sha=source_sha,
                run_id=f"github-release-backfill:{release['id']}",
                assets_dir=assets_dir,
                repository=args.destination,
                refresh_alias=False,
            )
        previous = newest.get(component)
        if previous is None or previous[0] < version_key:
            newest[component] = (version_key, version, source_sha)

    for component, (_version_key, version, source_sha) in sorted(newest.items()):
        print(f"{component}-latest -> {component}/v{version}")
        if args.dry_run:
            continue
        assert source_sha is not None
        delivery.promote_release_alias(
            component=component,
            version=version,
            expected_source_sha=source_sha,
            repository=args.destination,
        )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except delivery.DeliveryError as error:
        raise SystemExit(f"release backfill failed: {error}") from error

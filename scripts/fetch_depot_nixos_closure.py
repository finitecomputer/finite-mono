#!/usr/bin/env python3
"""Dispatch, download, and validate an Origin/Depot NixOS closure artifact."""

from __future__ import annotations

import argparse
import json
import os
import re
import stat
import subprocess
import sys
import tempfile
import time
import zipfile
from pathlib import Path, PurePosixPath

DEPOT_ORG = "scthc5h66g"
DEPOT_REPOSITORY = "finite-co/finite-mono"
ORIGIN_CLONE_URL = "https://origin.cursor.com/finite-co/finite-mono.git"
POLL_SECONDS = 10
TIMEOUT_SECONDS = 2 * 60 * 60
HOSTS = {
    "lat1": {
        "workflow": "lat1-nixos-closure.yml",
        "artifact_prefix": "lat1-nixos-closure-",
        "schema": "finite.lat1.nixos-closure.v1",
        "validator": "scripts/deploy-lat1-closure-cache",
    },
    "lat3": {
        "workflow": "lat3-nixos-closure.yml",
        "artifact_prefix": "lat3-nixos-closure-",
        "schema": "finite.lat3.nixos-closure.v1",
        "validator": "scripts/deploy-lat3-closure-cache",
    },
}


class ClosureFetchError(RuntimeError):
    """A fail-closed operator error."""


def command_json(command: list[str]) -> dict[str, object]:
    result = subprocess.run(command, text=True, capture_output=True, check=False)
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or "no output"
        raise ClosureFetchError(f"command failed: {' '.join(command)}\n{detail}")
    try:
        document = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise ClosureFetchError(
            f"command returned invalid JSON: {' '.join(command)}"
        ) from error
    if not isinstance(document, dict):
        raise ClosureFetchError(f"command returned a non-object: {' '.join(command)}")
    return document


def run_checked(command: list[str]) -> None:
    result = subprocess.run(command, text=True, check=False)
    if result.returncode != 0:
        raise ClosureFetchError(f"command failed: {' '.join(command)}")


def require_origin_main_revision(revision: str) -> Path:
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ClosureFetchError("revision must be exactly 40 lowercase hex characters")
    root_result = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        text=True,
        capture_output=True,
        check=False,
    )
    if root_result.returncode != 0:
        raise ClosureFetchError("run this command from the finite-mono checkout")
    root = Path(root_result.stdout.strip()).resolve()
    remote_result = subprocess.run(
        ["git", "-C", str(root), "remote", "get-url", "origin"],
        text=True,
        capture_output=True,
        check=False,
    )
    remote = remote_result.stdout.strip().removesuffix(".git").rstrip("/")
    accepted_remotes = {
        ORIGIN_CLONE_URL.removesuffix(".git"),
        "git@origin.cursor.com:finite-co/finite-mono",
    }
    if remote_result.returncode != 0 or remote not in accepted_remotes:
        raise ClosureFetchError(
            f"origin remote is {remote or '<missing>'}, expected Cursor Origin"
        )
    run_checked(["git", "-C", str(root), "fetch", "origin", "--prune"])
    ancestry = subprocess.run(
        [
            "git",
            "-C",
            str(root),
            "merge-base",
            "--is-ancestor",
            revision,
            "origin/main",
        ],
        check=False,
    )
    if ancestry.returncode != 0:
        raise ClosureFetchError(f"{revision} is not on authoritative origin/main")
    return root


def dispatch(workflow: str, revision: str) -> str:
    document = command_json(
        [
            "depot",
            "ci",
            "dispatch",
            "--org",
            DEPOT_ORG,
            "--repo",
            DEPOT_REPOSITORY,
            "--workflow",
            workflow,
            "--ref",
            "main",
            "--input",
            f"rev={revision}",
            "--output",
            "json",
        ]
    )
    run_id = document.get("run_id")
    if not isinstance(run_id, str) or not run_id:
        raise ClosureFetchError("Depot dispatch did not return a run_id")
    return run_id


def wait_for_success(run_id: str) -> None:
    deadline = time.monotonic() + TIMEOUT_SECONDS
    while True:
        document = command_json(
            [
                "depot",
                "ci",
                "status",
                run_id,
                "--org",
                DEPOT_ORG,
                "--output",
                "json",
            ]
        )
        if document.get("run_id") != run_id:
            raise ClosureFetchError("Depot status returned a different run_id")
        status_value = document.get("status")
        if status_value == "finished":
            return
        if status_value in {"failed", "cancelled"}:
            raise ClosureFetchError(f"Depot run {run_id} ended as {status_value}")
        if not isinstance(status_value, str) or not status_value:
            raise ClosureFetchError(f"Depot run {run_id} returned no status")
        if time.monotonic() >= deadline:
            raise ClosureFetchError(f"Depot run {run_id} exceeded the two-hour timeout")
        print(f"Depot run {run_id}: {status_value}; waiting", flush=True)
        time.sleep(POLL_SECONDS)


def select_artifact(
    document: dict[str, object], run_id: str, workflow: str, expected_name: str
) -> str:
    artifacts = document.get("artifacts")
    if not isinstance(artifacts, list):
        raise ClosureFetchError("Depot artifact response has no artifacts list")
    matches = [
        item
        for item in artifacts
        if isinstance(item, dict)
        and item.get("run_id") == run_id
        and item.get("workflow_path") == workflow
        and item.get("name") == expected_name
        and isinstance(item.get("artifact_id"), str)
        and item.get("artifact_id")
    ]
    if len(matches) != 1:
        raise ClosureFetchError(
            f"expected exactly one {expected_name} artifact, found {len(matches)}"
        )
    return str(matches[0]["artifact_id"])


def artifact_id(run_id: str, workflow: str, expected_name: str) -> str:
    document = command_json(
        [
            "depot",
            "ci",
            "artifacts",
            "list",
            run_id,
            "--org",
            DEPOT_ORG,
            "--output",
            "json",
        ]
    )
    return select_artifact(document, run_id, workflow, expected_name)


def download(artifact: str, archive: Path) -> None:
    run_checked(
        [
            "depot",
            "ci",
            "artifacts",
            "download",
            artifact,
            "--org",
            DEPOT_ORG,
            "--output-file",
            str(archive),
        ]
    )


def safe_extract(archive: Path, destination: Path) -> None:
    if not zipfile.is_zipfile(archive):
        raise ClosureFetchError("Depot closure artifact is not a ZIP archive")
    with zipfile.ZipFile(archive) as bundle:
        for member in bundle.infolist():
            path = PurePosixPath(member.filename)
            mode = member.external_attr >> 16
            if path.is_absolute() or ".." in path.parts or "\\" in member.filename:
                raise ClosureFetchError(f"unsafe artifact path: {member.filename!r}")
            if stat.S_ISLNK(mode):
                raise ClosureFetchError(
                    f"artifact contains a symlink: {member.filename!r}"
                )
        bundle.extractall(destination)


def validate_manifest(
    artifact_dir: Path, revision: str, schema: str, expected_name: str
) -> None:
    manifest_path = artifact_dir / "manifest.json"
    try:
        document = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ClosureFetchError(
            "closure artifact has no valid manifest.json"
        ) from error
    if not isinstance(document, dict):
        raise ClosureFetchError("closure artifact manifest is not an object")
    expected = {
        "schema": schema,
        "repository": DEPOT_REPOSITORY,
        "rev": revision,
        "cache": "nix-cache",
    }
    for key, value in expected.items():
        if document.get(key) != value:
            raise ClosureFetchError(
                f"{expected_name} manifest {key} is {document.get(key)!r}, expected {value!r}"
            )
    if not (artifact_dir / "nix-cache" / "nix-cache-info").is_file():
        raise ClosureFetchError("closure artifact has no complete file binary cache")


def fetch(host: str, revision: str, output_dir: Path) -> tuple[str, str]:
    config = HOSTS[host]
    root = require_origin_main_revision(revision)
    output_dir = output_dir.resolve()
    if output_dir.exists():
        raise ClosureFetchError(f"output directory already exists: {output_dir}")
    output_dir.parent.mkdir(parents=True, exist_ok=True)
    workflow = str(config["workflow"])
    expected_name = f"{config['artifact_prefix']}{revision}"
    print(f"Dispatching {workflow} for Origin revision {revision}", flush=True)
    run_id = dispatch(workflow, revision)
    print(f"Depot run: {run_id}", flush=True)
    wait_for_success(run_id)
    selected_artifact = artifact_id(run_id, workflow, expected_name)
    print(f"Depot artifact: {selected_artifact}", flush=True)

    with tempfile.TemporaryDirectory(
        prefix=f".{output_dir.name}.", dir=output_dir.parent
    ) as temporary:
        temporary_root = Path(temporary)
        archive = temporary_root / "artifact.zip"
        extracted = temporary_root / "extracted"
        extracted.mkdir()
        download(selected_artifact, archive)
        safe_extract(archive, extracted)
        validate_manifest(extracted, revision, str(config["schema"]), expected_name)
        run_checked(
            [
                str(root / str(config["validator"])),
                "--validate-only",
                str(extracted),
            ]
        )
        os.replace(extracted, output_dir)
    return run_id, selected_artifact


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Fetch an exact Origin/Depot NixOS closure without mutating production"
    )
    parser.add_argument("host", choices=sorted(HOSTS))
    parser.add_argument("revision")
    parser.add_argument("output_dir", type=Path)
    args = parser.parse_args()
    try:
        run_id, selected_artifact = fetch(args.host, args.revision, args.output_dir)
    except ClosureFetchError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(f"Validated artifact at {args.output_dir}")
    print(f"Evidence: run_id={run_id} artifact_id={selected_artifact}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

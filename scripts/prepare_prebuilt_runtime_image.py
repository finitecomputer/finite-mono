#!/usr/bin/env python3
"""Verify an existing canonical Linux image for Devfinity; never build or publish."""
import argparse
import json
import re
import subprocess
from pathlib import Path


def validate_reference(reference):
    if not re.fullmatch(r"ghcr\.io/finitecomputer/agent-runtime@sha256:[0-9a-f]{64}", reference):
        raise ValueError("expected canonical digest-pinned runtime image")
    return reference


def verified_report(reference, metadata, hermes_version):
    validate_reference(reference)
    if metadata.get("Os") != "linux" or metadata.get("Architecture") != "amd64":
        raise ValueError("prebuilt Docker qualification requires Linux AMD64")
    if reference not in metadata.get("RepoDigests", []):
        raise ValueError("Docker did not verify the requested registry digest")
    labels = metadata.get("Config", {}).get("Labels", {}) or {}
    if labels.get("computer.finite.runtime.hermes-agent-version") != hermes_version:
        raise ValueError("prebuilt Hermes version differs from this flake")
    revision = labels.get("org.opencontainers.image.revision", "")
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("image source revision is missing")
    return {
        "status": "verified_prebuilt", "engine": "docker", "built_locally": False,
        "mono_sha": revision, "image": reference, "platform": "linux/amd64",
        "image_metadata": {"id": metadata["Id"], "digest": reference.split("@", 1)[1],
                           "hermes_nix_runtime": {"version": hermes_version}},
    }


def command(args):
    result = subprocess.run(args, capture_output=True, text=True, timeout=600)
    if result.returncode:
        # Tool output can contain registry/auth configuration. Keep it private.
        raise RuntimeError(f"{args[0]} verification command failed")
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True, type=validate_reference)
    parser.add_argument("--report", required=True, type=Path)
    args = parser.parse_args()
    version = command(["nix", "eval", "--raw", ".#packages.x86_64-linux.hermes-agent.version"]).strip()
    command(["docker", "pull", "--platform", "linux/amd64", args.image])
    metadata = json.loads(command(["docker", "image", "inspect", args.image]))
    if len(metadata) != 1:
        raise ValueError("expected exactly one inspected image")
    report = verified_report(args.image, metadata[0], version)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()

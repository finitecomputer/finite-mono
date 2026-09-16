#!/usr/bin/env python3
"""Qualify the bundled Sites guidance against the actual Runtime CLI (offline)."""

import argparse
import json
from pathlib import Path
import re
import subprocess
import tomllib


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fsite", required=True)
    parser.add_argument("--skills", required=True, type=Path)
    args = parser.parse_args()
    root = args.skills / "software-development"
    skill = (root / "finite-sites-publishing-finite/SKILL.md").read_text()

    def cli(*command):
        return subprocess.check_output([args.fsite, *command], text=True, timeout=30)

    expected_version = re.search(r"`fsite` (\d+\.\d+\.\d+)", skill).group(1)
    if cli("--version").strip() != f"fsite {expected_version}":
        raise RuntimeError("Sites skill and Runtime CLI versions differ")

    workflows = re.findall(r"^fsite describe workflow (\S+) --output json$", skill, re.M)
    if not workflows:
        raise RuntimeError("Sites skill has no discoverable workflows")
    for workflow in workflows:
        if json.loads(cli("describe", "workflow", workflow, "--output", "json"))["name"] != workflow:
            raise RuntimeError(f"CLI did not describe {workflow}")

    schema = json.loads(cli("describe", "workflow", "project-config", "--output", "json"))["schema"]
    examples = 0
    for name in ("finite-sites-publishing-finite", "git-finite", "website-building-finite"):
        for path in (root / name).rglob("*.md"):
            for block in re.findall(r"```toml\n(.*?)```", path.read_text(), re.S):
                config = tomllib.loads(block)
                if "project" not in config:
                    continue
                examples += 1
                fields = {f"{table}.{key}" for table, values in config.items() for key in values}
                if not fields <= schema.keys():
                    raise RuntimeError(f"{path}: config fields absent from CLI schema: {fields - schema.keys()}")
    if examples < 3:
        raise RuntimeError("Expected publishing, bare repository, and website config examples")

    # Old binaries used to accept these workflows. Reject them on the exact
    # binary being promoted, not just through a prose substring check.
    for retired in ("publish-stateful-app", "publish-document", "share-output"):
        result = subprocess.run(
            [args.fsite, "describe", "workflow", retired, "--output", "json"],
            capture_output=True, timeout=30,
        )
        if result.returncode == 0:
            raise RuntimeError(f"CLI still advertises retired workflow {retired}")
    print(f"Sites {expected_version}: {len(workflows)} workflows and {examples} config examples passed")


if __name__ == "__main__":
    main()

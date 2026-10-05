#!/usr/bin/env python3
"""Push a built output to the Finite Sites Project Repository using scoped Git auth."""

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--site", type=Path, required=True)
    parser.add_argument("--remote", required=True)
    args = parser.parse_args()
    if not (args.site / "finite-mono.cc.json").is_file():
        parser.error("site must contain a completed code-map build")
    if not args.remote.startswith("https://") or "@" in args.remote:
        parser.error("remote must be an HTTPS URL without embedded credentials")
    with tempfile.TemporaryDirectory(prefix="finite-code-map-publish-") as scratch:
        temp = Path(scratch)
        env = os.environ.copy()
        env["GIT_TERMINAL_PROMPT"] = "0"
        if env.get("FINITE_CODE_MAP_GIT_PASSWORD"):
            if not env.get("FINITE_CODE_MAP_GIT_USERNAME"):
                parser.error("scoped credential username is missing")
            askpass = temp / "askpass"
            askpass.write_text("""#!/bin/sh
case "$1" in
  *Username*) printf '%s\\n' "$FINITE_CODE_MAP_GIT_USERNAME" ;;
  *Password*) printf '%s\\n' "$FINITE_CODE_MAP_GIT_PASSWORD" ;;
  *) exit 1 ;;
esac
""")
            askpass.chmod(0o700)
            env["GIT_ASKPASS"] = str(askpass)
            env["GIT_CONFIG_COUNT"] = "1"
            env["GIT_CONFIG_KEY_0"] = "credential.helper"
            env["GIT_CONFIG_VALUE_0"] = ""

        def git(*cmd, cwd=None):
            return subprocess.run(["git", *cmd], cwd=cwd, env=env, check=True)

        deploy = temp / "project"
        git("clone", args.remote, str(deploy))
        config = ROOT / "infra/code-map/finite.toml"
        existing = deploy / "finite.toml"
        if existing.exists() and existing.read_bytes() != config.read_bytes():
            raise RuntimeError(
                "Remote finite.toml differs; inspect the Project before publishing"
            )
        git("checkout", "-B", "main", cwd=deploy)
        site = deploy / "site"
        if site.is_symlink():
            raise RuntimeError("Remote site directory must not be a symlink")
        if site.exists():
            shutil.rmtree(site)
        shutil.copytree(args.site, site)
        shutil.copyfile(config, existing)
        # Keep the producer scripts alongside committed deploy bytes. The monorepo
        # remains their canonical source; the served path contains only site/.
        producer = deploy / "infra/code-map"
        producer.mkdir(parents=True, exist_ok=True)
        for name in (
            "build.py",
            "metrics.py",
            "publish.py",
            "README.md",
            "finite.toml",
        ):
            shutil.copyfile(ROOT / "infra/code-map" / name, producer / name)
        git("add", "finite.toml", "site", "infra/code-map", cwd=deploy)
        changed = subprocess.run(
            ["git", "diff", "--cached", "--quiet"], cwd=deploy, env=env
        )
        if changed.returncode == 0:
            print("Code-map output is already current")
            return
        if changed.returncode != 1:
            raise RuntimeError("Cannot inspect deployment changes")
        import json

        commit = json.loads((site / "build.json").read_text())["source_commit"]
        git(
            "-c",
            "user.name=Finite code map",
            "-c",
            "user.email=code-map@users.noreply.github.com",
            "commit",
            "-m",
            f"Publish code map for {commit}",
            cwd=deploy,
        )
        git("push", "origin", "HEAD:refs/heads/main", cwd=deploy)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Shared changed-path plumbing for the CI selector scripts.

Both scripts/ci/select-harnesses and scripts/ci/affected-rust-packages turn a
changed path set into a CI scope. This module owns the pieces they must never
diverge on: path normalization, the --changed-file override argument, and the
git-diff invocation. Classification stays local to each script.
"""

from __future__ import annotations

import argparse
import os
import subprocess


def add_changed_file_argument(parser: argparse.ArgumentParser) -> None:
    parser.add_argument(
        "--changed-file",
        action="append",
        dest="changed_files",
        help="Override git diff input; useful for tests and local dry-runs.",
    )


def git_diff_changed_files(diff_range: str) -> list[str]:
    completed = subprocess.run(
        ["git", "diff", "--name-only", "--no-renames", diff_range, "--"],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    )
    return completed.stdout.splitlines()


def normalized_paths(paths: list[str]) -> list[str]:
    normalized = [normalize_path(path) for path in paths]
    return [path for path in normalized if path]


def normalize_path(path: str) -> str:
    if not path:
        return ""
    if os.path.isabs(path):
        try:
            path = os.path.relpath(path, os.getcwd())
        except ValueError:
            return path
    # Only strip a literal "./" prefix; lstrip("./") would also eat leading
    # dots and turn ".github/workflows/ci.yml" into "github/workflows/ci.yml".
    return path.replace(os.sep, "/").removeprefix("./")

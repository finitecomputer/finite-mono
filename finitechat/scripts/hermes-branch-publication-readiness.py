#!/usr/bin/env python3
"""Report whether the hardening branch is safe to commit and push for CI."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

DEFAULT_REPORT = "target/hermes-branch-publication-readiness.json"
DEFAULT_COMMIT_MESSAGE = "Harden Hermes finitechat sidecar runtime"
BLOCKED_EXACT = {
    ".env",
    ".DS_Store",
}
BLOCKED_PREFIXES = (
    ".ruff_cache/",
    "target/",
)
BLOCKED_SUFFIXES = (
    ".pyc",
    ".pyo",
    ".sqlite",
    ".sqlite3",
    ".sqlite3-shm",
    ".sqlite3-wal",
    ".db",
    ".key",
    ".pem",
)
ALLOWED_ENV_FILES = {
    ".env.example",
}


def run(args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, capture_output=True, text=True, check=True)


def current_branch() -> str:
    return run(["git", "branch", "--show-current"]).stdout.strip()


def git_status_lines(*, include_ignored: bool = False) -> list[str]:
    args = ["git", "status", "--porcelain=v1", "--untracked-files=all"]
    if include_ignored:
        args.append("--ignored")
    return [line for line in run(args).stdout.splitlines() if line.strip()]


def parse_status_line(line: str) -> dict[str, str]:
    status = line[:2]
    path = line[3:].strip()
    if " -> " in path:
        path = path.split(" -> ", 1)[1]
    return {"status": status, "path": path}


def is_blocked_path(path: str) -> bool:
    if path in ALLOWED_ENV_FILES:
        return False
    if path in BLOCKED_EXACT:
        return True
    if path.startswith(".env."):
        return True
    if any(path.startswith(prefix) for prefix in BLOCKED_PREFIXES):
        return True
    if "__pycache__/" in path:
        return True
    return any(path.endswith(suffix) for suffix in BLOCKED_SUFFIXES)


def shell_quote(path: str) -> str:
    return "'" + path.replace("'", "'\"'\"'") + "'"


def classify_status(lines: list[str]) -> dict[str, Any]:
    candidate_paths: list[str] = []
    blocked_paths: list[dict[str, str]] = []
    ignored_paths: list[str] = []
    for line in lines:
        parsed = parse_status_line(line)
        status = parsed["status"]
        path = parsed["path"]
        if status == "!!":
            ignored_paths.append(path)
            continue
        if is_blocked_path(path):
            blocked_paths.append({"status": status, "path": path})
            continue
        candidate_paths.append(path)
    return {
        "candidate_paths": sorted(set(candidate_paths)),
        "blocked_paths": blocked_paths,
        "ignored_count": len(ignored_paths),
        "ignored_sample": ignored_paths[:20],
    }


def build_report(
    *,
    branch: str,
    status_lines: list[str],
    include_ignored: bool,
    commit_message: str,
) -> tuple[int, dict[str, Any]]:
    classified = classify_status(status_lines)
    candidate_paths = classified["candidate_paths"]
    blocked_paths = classified["blocked_paths"]
    errors: list[str] = []
    notes: list[str] = []
    if blocked_paths:
        errors.append("blocked generated or sensitive paths are present in git status")
    if errors:
        status = "blocked"
    elif candidate_paths:
        status = "ready"
    else:
        status = "clean"
        notes.append("no local source changes are available to publish")
    git_add = "git add " + " ".join(shell_quote(path) for path in candidate_paths)
    report = {
        "status": status,
        "generated_at_unix": int(time.time()),
        "branch": branch,
        "include_ignored": include_ignored,
        "candidate_path_count": len(candidate_paths),
        "candidate_paths": candidate_paths,
        "blocked_paths": blocked_paths,
        "ignored_count": classified["ignored_count"],
        "ignored_sample": classified["ignored_sample"],
        "errors": errors,
        "notes": notes,
        "suggested_commands": {
            "stage": git_add if candidate_paths else None,
            "commit": f"git commit -m {shell_quote(commit_message)}" if candidate_paths else None,
            "push": f"git push -u origin {shell_quote(branch)}" if branch else None,
        },
    }
    return (0 if status in {"ready", "clean"} else 2), report


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--branch", help="branch to push; defaults to current branch")
    parser.add_argument("--commit-message", default=DEFAULT_COMMIT_MESSAGE)
    parser.add_argument("--include-ignored", action="store_true")
    parser.add_argument("--report", default=DEFAULT_REPORT)
    args = parser.parse_args()

    branch = args.branch or current_branch()
    status, report = build_report(
        branch=branch,
        status_lines=git_status_lines(include_ignored=args.include_ignored),
        include_ignored=args.include_ignored,
        commit_message=args.commit_message,
    )
    report_path = Path(args.report)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(report, indent=2) + "\n"
    report_path.write_text(text, encoding="utf-8")
    print(text, end="")
    return status


if __name__ == "__main__":
    sys.exit(main())

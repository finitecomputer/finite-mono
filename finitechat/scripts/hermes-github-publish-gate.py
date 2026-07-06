#!/usr/bin/env python3
"""Run the GitHub Actions S3 smoke and proven-image publish gate."""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

DEFAULT_REPO = "finitecomputer/finitechat"
DEFAULT_WORKFLOW = "ci.yml"
DEFAULT_CONFIG_REPO = "finitecomputer/tinfoil-agent-runtime-canary"
DEFAULT_RELEASE_TAG = "v0.1.0"
DEFAULT_ARTIFACT_DIR = "target/hermes-github-publish-gate/artifacts"
DEFAULT_REPORT = "target/hermes-github-publish-gate/report.json"
CANONICAL_ARTIFACTS = [
    "target/hermes-hardening-audit.json",
    "target/hermes-docker-smoke/report.json",
    "target/hermes-docker-smoke/restic-preflight.json",
    "target/hermes-docker-smoke/image-publish.json",
    "target/hermes-docker-smoke/tinfoil-handoff.json",
    "target/hermes-docker-smoke/tinfoil-canary/tinfoil-canary-summary.json",
]


def command_text(args: list[str]) -> str:
    return " ".join(args)


def run(args: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, capture_output=True, text=True, check=check)


def run_json(args: list[str]) -> Any:
    result = run(args)
    return json.loads(result.stdout)


def current_branch() -> str:
    result = run(["git", "branch", "--show-current"])
    branch = result.stdout.strip()
    if not branch:
        raise SystemExit("could not infer current git branch; pass --ref explicitly")
    return branch


def local_status_lines() -> list[str]:
    result = run(["git", "status", "--short"], check=False)
    if result.returncode != 0:
        return ["<git status failed>"]
    return [line for line in result.stdout.splitlines() if line.strip()]


def branch_from_ref(ref: str) -> str:
    return ref.removeprefix("refs/heads/")


def remote_ref_status(*, repo: str, branch: str) -> dict[str, Any]:
    result = run(
        [
            "gh",
            "api",
            f"repos/{repo}/git/ref/heads/{branch}",
            "--jq",
            ".object.sha",
        ],
        check=False,
    )
    if result.returncode == 0 and result.stdout.strip():
        return {"status": "present", "sha": result.stdout.strip()}
    return {
        "status": "missing",
        "stderr_tail": "\n".join(result.stderr.splitlines()[-5:]),
    }


def readiness_errors(
    *,
    local_status: list[str],
    remote_ref: dict[str, Any],
    allow_dirty: bool,
) -> list[str]:
    errors: list[str] = []
    if local_status and not allow_dirty:
        errors.append(
            "local worktree has uncommitted changes; push the intended branch or pass --allow-dirty"
        )
    if remote_ref.get("status") != "present":
        errors.append("remote workflow ref is missing; push the branch before dispatch")
    return errors


def workflow_fields(args: argparse.Namespace) -> list[tuple[str, str]]:
    fields = [
        ("docker_smoke", "true"),
        ("publish_runtime_image", "true"),
        ("restic_backend", "s3"),
        ("tinfoil_config_repo", args.tinfoil_config_repo),
        ("tinfoil_release_tag", args.tinfoil_release_tag),
    ]
    optional = {
        "restic_repository": args.restic_repository,
        "latitude_storage_bucket": args.latitude_storage_bucket,
        "latitude_object_endpoint": args.latitude_object_endpoint,
        "restic_prefix": args.restic_prefix,
    }
    for key, value in optional.items():
        if value:
            fields.append((key, value))
    return fields


def workflow_run_command(args: argparse.Namespace, ref: str) -> list[str]:
    command = [
        "gh",
        "workflow",
        "run",
        args.workflow,
        "--repo",
        args.repo,
        "--ref",
        ref,
    ]
    for key, value in workflow_fields(args):
        command.extend(["-f", f"{key}={value}"])
    return command


def preflight_command(args: argparse.Namespace) -> list[str]:
    return [
        "scripts/hermes-github-ci-preflight.py",
        "--repo",
        args.repo,
        "--report",
        args.preflight_report,
    ]


def find_downloaded_file(artifact_dir: Path, relative_path: str) -> Path | None:
    direct = artifact_dir / relative_path
    if direct.is_file():
        return direct
    suffixes = [Path(relative_path).parts]
    if relative_path.startswith("target/"):
        suffixes.append(Path(relative_path).parts[1:])
    for path in artifact_dir.rglob(Path(relative_path).name):
        if not path.is_file():
            continue
        for suffix_parts in suffixes:
            if path.parts[-len(suffix_parts) :] == suffix_parts:
                return path
    return None


def ingest_artifacts(artifact_dir: Path, *, repo_root: Path) -> dict[str, Any]:
    copied: list[dict[str, str]] = []
    missing: list[str] = []
    for relative_path in CANONICAL_ARTIFACTS:
        source = find_downloaded_file(artifact_dir, relative_path)
        destination = repo_root / relative_path
        if source is None:
            missing.append(relative_path)
            continue
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(source.read_bytes())
        copied.append({"source": str(source), "destination": str(destination)})
    return {
        "status": "ok" if not missing else "missing_artifacts",
        "artifact_dir": str(artifact_dir),
        "repo_root": str(repo_root),
        "copied": copied,
        "missing": missing,
    }


def list_workflow_runs(*, repo: str, workflow: str, branch: str) -> list[dict[str, Any]]:
    value = run_json(
        [
            "gh",
            "run",
            "list",
            "--repo",
            repo,
            "--workflow",
            workflow,
            "--branch",
            branch,
            "--event",
            "workflow_dispatch",
            "--json",
            "databaseId,headBranch,status,conclusion,createdAt,url,name",
            "--limit",
            "10",
        ]
    )
    if not isinstance(value, list):
        return []
    return [item for item in value if isinstance(item, dict)]


def wait_for_new_dispatched_run(
    *,
    repo: str,
    workflow: str,
    branch: str,
    excluded_run_ids: set[str],
    timeout_seconds: int,
    poll_seconds: int,
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout_seconds
    last_runs: list[dict[str, Any]] = []
    while time.monotonic() < deadline:
        last_runs = list_workflow_runs(repo=repo, workflow=workflow, branch=branch)
        for item in last_runs:
            run_id = str(item.get("databaseId") or "")
            if run_id and run_id not in excluded_run_ids:
                return item
        time.sleep(poll_seconds)
    raise SystemExit(
        "timed out waiting for dispatched workflow run; "
        f"last runs: {json.dumps(last_runs, sort_keys=True)}"
    )


def run_gate(args: argparse.Namespace) -> tuple[int, dict[str, Any]]:
    ref = args.ref or current_branch()
    branch = args.branch or branch_from_ref(ref)
    status_lines = local_status_lines()
    report: dict[str, Any] = {
        "status": "dry_run" if args.dry_run else "running",
        "generated_at_unix": int(time.time()),
        "repo": args.repo,
        "workflow": args.workflow,
        "ref": ref,
        "branch": branch,
        "local_status": {
            "dirty": bool(status_lines),
            "line_count": len(status_lines),
            "sample": status_lines[:20],
        },
        "preflight_report": args.preflight_report,
        "artifact_dir": args.artifact_dir,
        "commands": {},
    }
    preflight = preflight_command(args)
    dispatch = workflow_run_command(args, ref)
    report["commands"]["preflight"] = command_text(preflight)
    report["commands"]["dispatch"] = command_text(dispatch)

    if args.dry_run:
        return 0, report

    remote_ref = remote_ref_status(repo=args.repo, branch=branch)
    report["remote_ref"] = remote_ref
    ready_errors = readiness_errors(
        local_status=status_lines,
        remote_ref=remote_ref,
        allow_dirty=args.allow_dirty,
    )
    if ready_errors:
        report["status"] = "not_ready"
        report["errors"] = ready_errors
        return 2, report

    preflight_result = run(preflight, check=False)
    report["preflight_exit_code"] = preflight_result.returncode
    if preflight_result.returncode != 0:
        report["status"] = "preflight_failed"
        report["preflight_stdout"] = preflight_result.stdout
        report["preflight_stderr"] = preflight_result.stderr
        return 2, report

    existing_runs = list_workflow_runs(repo=args.repo, workflow=args.workflow, branch=branch)
    excluded_run_ids = {
        str(item.get("databaseId")) for item in existing_runs if item.get("databaseId")
    }
    report["existing_workflow_run_ids"] = sorted(excluded_run_ids)
    run(dispatch)
    workflow_run = wait_for_new_dispatched_run(
        repo=args.repo,
        workflow=args.workflow,
        branch=branch,
        excluded_run_ids=excluded_run_ids,
        timeout_seconds=args.dispatch_timeout_seconds,
        poll_seconds=args.poll_seconds,
    )
    run_id = str(workflow_run["databaseId"])
    run_url = str(workflow_run.get("url") or "")
    report["run"] = workflow_run
    report["run_id"] = run_id
    report["run_url"] = run_url

    watch_command = [
        "gh",
        "run",
        "watch",
        run_id,
        "--repo",
        args.repo,
        "--interval",
        str(args.poll_seconds),
        "--exit-status",
    ]
    report["commands"]["watch"] = command_text(watch_command)
    watch_result = run(watch_command, check=False)
    report["watch_exit_code"] = watch_result.returncode
    report["watch_stdout_tail"] = "\n".join(watch_result.stdout.splitlines()[-40:])
    report["watch_stderr_tail"] = "\n".join(watch_result.stderr.splitlines()[-20:])

    artifact_dir = Path(args.artifact_dir)
    if artifact_dir.exists():
        shutil.rmtree(artifact_dir)
    artifact_dir.mkdir(parents=True, exist_ok=True)
    download_command = [
        "gh",
        "run",
        "download",
        run_id,
        "--repo",
        args.repo,
        "--dir",
        str(artifact_dir),
    ]
    report["commands"]["download"] = command_text(download_command)
    download_result = run(download_command, check=False)
    report["download_exit_code"] = download_result.returncode
    report["download_stdout_tail"] = "\n".join(download_result.stdout.splitlines()[-20:])
    report["download_stderr_tail"] = "\n".join(download_result.stderr.splitlines()[-20:])
    report["downloaded_files"] = sorted(
        str(path.relative_to(artifact_dir)) for path in artifact_dir.rglob("*") if path.is_file()
    )

    if watch_result.returncode == 0 and download_result.returncode == 0:
        ingest = ingest_artifacts(artifact_dir, repo_root=Path(args.ingest_root))
        report["artifact_ingest"] = ingest
        if ingest["status"] != "ok":
            report["status"] = "artifact_ingest_failed"
            return 2, report
        audit_command = [
            "scripts/hermes-hardening-audit.py",
            "--report",
            "target/hermes-hardening-audit.json",
        ]
        report["commands"]["local_audit_after_ingest"] = command_text(audit_command)
        audit_result = run(audit_command, check=False)
        report["local_audit_exit_code"] = audit_result.returncode
        report["local_audit_stdout_tail"] = "\n".join(audit_result.stdout.splitlines()[-40:])
        report["local_audit_stderr_tail"] = "\n".join(audit_result.stderr.splitlines()[-20:])
        report["status"] = "passed"
        return 0, report
    report["status"] = "failed"
    return 2, report


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", default=DEFAULT_REPO)
    parser.add_argument("--workflow", default=DEFAULT_WORKFLOW)
    parser.add_argument("--ref", help="git ref to dispatch; defaults to current branch")
    parser.add_argument("--branch", help="branch name used to locate the dispatched run")
    parser.add_argument("--restic-repository")
    parser.add_argument("--latitude-storage-bucket")
    parser.add_argument("--latitude-object-endpoint")
    parser.add_argument("--restic-prefix")
    parser.add_argument("--tinfoil-config-repo", default=DEFAULT_CONFIG_REPO)
    parser.add_argument("--tinfoil-release-tag", default=DEFAULT_RELEASE_TAG)
    parser.add_argument("--preflight-report", default="target/hermes-github-ci-preflight.json")
    parser.add_argument("--artifact-dir", default=DEFAULT_ARTIFACT_DIR)
    parser.add_argument("--ingest-root", default=".")
    parser.add_argument("--report", default=DEFAULT_REPORT)
    parser.add_argument("--dispatch-timeout-seconds", type=int, default=120)
    parser.add_argument("--poll-seconds", type=int, default=30)
    parser.add_argument(
        "--allow-dirty",
        action="store_true",
        help="Allow dispatch when the local worktree has uncommitted changes.",
    )
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    status, report = run_gate(args)
    report_path = Path(args.report)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(report, indent=2) + "\n"
    report_path.write_text(text, encoding="utf-8")
    print(text, end="")
    return status


if __name__ == "__main__":
    sys.exit(main())

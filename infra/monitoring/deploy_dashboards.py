#!/usr/bin/env python3
"""Validate and update existing file-provisioned dashboards without a restart."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time
import urllib.request


DASHBOARDS = Path("/var/lib/finite-monitoring/grafana/dashboards")
PROVIDER = Path("/etc/finite/monitoring/grafana/provisioning/dashboards/finite.yml")
BACKUPS = Path("/var/backups/finite-monitoring-dashboards")
LOCK = Path("/run/lock/finite-monitoring-dashboards.lock")
MONITORING = "ubuntu@152.236.5.27"
APP_PLANE = "root@64.34.80.19"
STATUS_COMMAND = "/run/current-system/sw/bin/python3 /var/lib/finite-monitoring-ci/scripts/finite-status --json"
DASHBOARD_COMMAND = "/usr/bin/sudo -n /usr/bin/python3 /var/lib/finite-monitoring-ci/deploy_dashboards.py"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def validate(bundle):
    require(re.fullmatch(r"[0-9a-f]{40}", bundle["revision"]), "invalid revision")
    require(bundle.get("schema_version") == 1, "unsupported deployment bundle")
    require(
        re.fullmatch(r"[0-9a-f]{64}", bundle["helper_sha256"]), "invalid helper hash"
    )
    require(
        re.fullmatch(r"[0-9a-f]{64}", bundle["provider_sha256"]),
        "invalid provider hash",
    )
    require(bool(bundle["files"]), "production dashboard list is empty")
    uids = set()
    for name, entry in bundle["files"].items():
        require(
            re.fullmatch(r"finite-[a-z0-9-]+\.json", name), "invalid dashboard filename"
        )
        document = json.loads(entry["content"])
        uid = entry["uid"]
        require(re.fullmatch(r"[a-zA-Z0-9_-]{1,40}", uid), "invalid dashboard UID")
        require(uid not in uids, "duplicate dashboard UID")
        uids.add(uid)
        require(document.get("uid") == uid, f"UID mismatch: {name}")
        require(bool(document.get("title")), f"missing title: {name}")
        require(
            isinstance(document.get("panels"), list) and document["panels"],
            f"missing panels: {name}",
        )
        require("draft" not in document.get("tags", []), f"draft dashboard: {name}")


def bundle_from_repo():
    root = Path(__file__).resolve().parents[2]
    monitoring = root / "infra/monitoring"
    manifest = json.loads((monitoring / "grafana/production.json").read_text())
    bundle = {
        "schema_version": 1,
        "helper_sha256": digest(Path(__file__).read_bytes()),
        "revision": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=root, text=True
        ).strip(),
        "provider_sha256": digest(
            (
                monitoring / "ubuntu/grafana/provisioning/dashboards/finite.yml"
            ).read_bytes()
        ),
        "files": {},
    }
    for name, uid in manifest.items():
        require(re.fullmatch(r"finite-[a-z0-9-]+\.json", name), "invalid manifest path")
        path = monitoring / "grafana/dashboards" / name
        require(not path.is_symlink(), f"symlink dashboard: {name}")
        bundle["files"][name] = {"uid": uid, "content": path.read_text()}
    validate(bundle)
    return bundle


def grafana_dashboard(uid):
    password = Path("/etc/finite/monitoring/grafana-admin-password").read_text().strip()
    token = base64.b64encode(f"admin:{password}".encode()).decode()
    request = urllib.request.Request(
        f"http://127.0.0.1:3000/api/dashboards/uid/{uid}",
        headers={"Authorization": f"Basic {token}"},
    )
    with urllib.request.urlopen(request, timeout=10) as response:
        return json.load(response)


def owned_by_file(response, name):
    meta = response.get("meta", {})
    return meta.get("provisioned") is True and meta.get("provisionedExternalId") in (
        name,
        str(DASHBOARDS / name),
    )


def matches(expected, actual):
    # Grafana assigns database IDs and versions; every source-owned field must
    # otherwise match. Server-added top-level fields are harmless.
    return all(
        actual.get(key) == value
        for key, value in expected.items()
        if key not in ("id", "version")
    )


def wait_for_reload(files, fetch, timeout=100):
    deadline = time.monotonic() + timeout
    while True:
        pending = []
        for name, content in files.items():
            expected = json.loads(content)
            try:
                response = fetch(expected["uid"])
                if owned_by_file(response, name) and matches(
                    expected, response["dashboard"]
                ):
                    continue
            except (OSError, ValueError, KeyError):
                pass
            pending.append(name)
        if not pending:
            return
        if time.monotonic() >= deadline:
            raise RuntimeError(f"Grafana did not load: {', '.join(pending)}")
        time.sleep(5)


def atomic_write(path, content):
    fd, temporary = tempfile.mkstemp(
        prefix=".dashboard-", suffix=".tmp", dir=path.parent
    )
    try:
        with os.fdopen(fd, "wb") as output:
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
            os.fchmod(output.fileno(), 0o644)
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def preflight(
    bundle, *, directory=DASHBOARDS, provider=PROVIDER, fetch=grafana_dashboard
):
    validate(bundle)
    require(
        digest(Path(__file__).read_bytes()) == bundle["helper_sha256"],
        "installed deployment helper differs; operator must run install_ci_access.py from this revision",
    )
    require(
        directory.is_dir() and not directory.is_symlink(), "invalid dashboard directory"
    )
    require(
        digest(provider.read_bytes()) == bundle["provider_sha256"],
        "live file provider differs from repo",
    )
    previous = {}
    for name, entry in bundle["files"].items():
        path = directory / name
        require(
            path.is_file() and not path.is_symlink(),
            f"expected existing regular file: {name}",
        )
        previous[name] = path.read_bytes()
        require(
            json.loads(previous[name]).get("uid") == entry["uid"],
            f"existing file UID differs: {name}",
        )
        response = fetch(entry["uid"])
        require(
            owned_by_file(response, name), f"Grafana file ownership differs: {name}"
        )
        require(
            matches(json.loads(previous[name]), response["dashboard"]),
            f"live dashboard differs from its file: {name}",
        )
    return previous


def apply(
    bundle,
    *,
    directory=DASHBOARDS,
    provider=PROVIDER,
    backups=BACKUPS,
    fetch=grafana_dashboard,
    verify=wait_for_reload,
    write=atomic_write,
):
    previous = preflight(bundle, directory=directory, provider=provider, fetch=fetch)
    candidate = {
        name: entry["content"].encode() for name, entry in bundle["files"].items()
    }
    if candidate == previous:
        print("Production dashboards already match; no files changed.", flush=True)
        return
    backups.mkdir(mode=0o700, parents=True, exist_ok=True)
    backup = Path(tempfile.mkdtemp(prefix=f"{bundle['revision']}.", dir=backups))
    for name, content in previous.items():
        (backup / name).write_bytes(content)
    (backup / "manifest.json").write_text(
        json.dumps(
            {
                "revision": bundle["revision"],
                "previous_sha256": {
                    name: digest(content) for name, content in previous.items()
                },
                "candidate_sha256": {
                    name: digest(content) for name, content in candidate.items()
                },
            },
            indent=2,
        )
        + "\n"
    )
    print(f"Previous dashboard files backed up at {backup}", flush=True)
    try:
        for name, content in candidate.items():
            write(directory / name, content)
        verify(candidate, fetch)
    except Exception:
        # Restore every file, including one whose replacement may have succeeded
        # before the write raised. Never delete a dashboard or call a write API.
        for name, content in previous.items():
            atomic_write(directory / name, content)
        verify(previous, fetch)
        print("Previous files restored and verified in Grafana.", flush=True)
        raise
    print(
        json.dumps(
            {
                "revision": bundle["revision"],
                "backup": str(backup),
                "deployed_sha256": {
                    name: digest(content) for name, content in candidate.items()
                },
            },
            indent=2,
        ),
        flush=True,
    )


def ssh(target, command, **kwargs):
    return subprocess.run(
        [
            "ssh",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "ConnectTimeout=15",
            target,
            command,
        ],
        **kwargs,
    )


def status(output):
    # The key is forced to the installed canonical, read-only status command.
    result = ssh(APP_PLANE, STATUS_COMMAND, text=True, capture_output=True)
    require(result.returncode in (0, 1, 2), "finite-status SSH execution failed")
    report = json.loads(result.stdout)
    require(
        report.get("schema_version") == "finite.status.v1",
        "invalid finite-status report",
    )
    require(
        report.get("exit_code") == result.returncode, "finite-status exit code mismatch"
    )
    # Actions artifacts are accessible with this public repository. Never
    # publish the full report's agent names, project IDs, addresses, or errors.
    summary = {
        "schema_version": "finite.status.summary.v1",
        "source_schema_version": report["schema_version"],
        "generated_at": report["generated_at"],
        "overall_status": report["overall_status"],
        "exit_code": report["exit_code"],
        "sections": {
            name: {"status": report["sections"][name]["status"]}
            for name in (
                "fleet_convergence",
                "host_health",
                "recovery_boundary",
                "rollout_state",
                "chat_plane",
            )
        },
    }
    Path(output).write_text(json.dumps(summary, indent=2) + "\n")
    print(
        f"finite-status: {report['overall_status']} (exit {result.returncode}); evidence: {output}"
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "command",
        choices=("validate", "preview", "preflight", "deploy", "apply", "status"),
    )
    parser.add_argument("--output", default="finite-status.json")
    options = parser.parse_args()
    if options.command == "status":
        status(options.output)
    elif options.command == "preflight":
        bundle = json.load(sys.stdin)
        previous = preflight(bundle)
        print(
            json.dumps(
                {
                    name: {
                        "previous_sha256": digest(content),
                        "candidate_sha256": digest(
                            bundle["files"][name]["content"].encode()
                        ),
                        "changed": content != bundle["files"][name]["content"].encode(),
                    }
                    for name, content in previous.items()
                },
                indent=2,
            )
        )
    elif options.command == "apply":
        import fcntl

        require(os.geteuid() == 0, "remote apply requires root")
        with LOCK.open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            apply(json.load(sys.stdin))
    else:
        bundle = bundle_from_repo()
        if options.command == "validate":
            print("Production dashboards validated: " + ", ".join(bundle["files"]))
            return
        if options.command == "preview":
            command = DASHBOARD_COMMAND + " preflight"
            ssh(MONITORING, command, input=json.dumps(bundle), text=True, check=True)
            return
        root = Path(__file__).resolve().parents[2]
        require(
            not subprocess.check_output(["git", "status", "--porcelain"], cwd=root),
            "deploy requires a clean checkout",
        )
        subprocess.run(["git", "fetch", "origin", "main"], cwd=root, check=True)
        subprocess.run(
            ["git", "merge-base", "--is-ancestor", bundle["revision"], "origin/main"],
            cwd=root,
            check=True,
        )
        for path in ("infra/monitoring", ".github/workflows/monitoring-dashboards.yml"):
            result = subprocess.run(
                ["git", "diff", "--quiet", "HEAD", "origin/main", "--", path], cwd=root
            )
            require(
                result.returncode == 0,
                "newer dashboard deployment changes are on main; run from current main",
            )
        command = DASHBOARD_COMMAND + " apply"
        ssh(MONITORING, command, input=json.dumps(bundle), text=True, check=True)


if __name__ == "__main__":
    main()

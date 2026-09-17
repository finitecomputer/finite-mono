#!/usr/bin/env python3
"""Operator-only initial Tinfoil dashboard provisioning; never used by CI keys."""

import argparse
import fcntl
import json
import os
from pathlib import Path
import sys
import subprocess
import tempfile
import urllib.error

import deploy_dashboards as dashboards

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from finite_tinfoil_status import collect  # noqa: E402

NAME = "finite-tinfoil-gpu.json"
UID = "finite-tinfoil-gpu"


def preflight(
    bundle,
    directory=dashboards.DASHBOARDS,
    provider=dashboards.PROVIDER,
    fetch=dashboards.grafana_dashboard,
    evidence=collect,
):
    dashboards.validate(bundle)
    dashboards.require(
        bundle["helper_sha256"]
        == dashboards.digest(Path(dashboards.__file__).read_bytes()),
        "helper differs from reviewed bundle",
    )
    dashboards.require(
        set(bundle["files"]) == {NAME}, "only Tinfoil may be initially provisioned"
    )
    dashboards.require(bundle["files"][NAME]["uid"] == UID, "unexpected Tinfoil UID")
    dashboards.require(
        directory.is_dir() and not directory.is_symlink(), "invalid dashboard directory"
    )
    dashboards.require(
        dashboards.digest(provider.read_bytes()) == bundle["provider_sha256"],
        "provider drift",
    )
    path = directory / NAME
    dashboards.require(
        not path.exists() and not path.is_symlink(),
        "dashboard already exists; use normal deployment",
    )
    # Fail closed on another file owning this UID, even if Grafana hasn't loaded it.
    for existing in directory.glob("*.json"):
        dashboards.require(not existing.is_symlink(), "symlink in dashboard provider")
        dashboards.require(
            json.loads(existing.read_bytes()).get("uid") != UID,
            "UID already belongs to another file",
        )
    try:
        fetch(UID)
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise
    else:
        raise ValueError(
            "Grafana UID already exists; ownership requires operator reconciliation"
        )
    report = evidence()
    dashboards.require(
        report["overall_status"] == "green",
        "Tinfoil metrics are not ready; run scripts/finite-status --tinfoil --json",
    )
    return report


def provision(
    bundle,
    *,
    directory=dashboards.DASHBOARDS,
    provider=dashboards.PROVIDER,
    backups=dashboards.BACKUPS,
    fetch=dashboards.grafana_dashboard,
    evidence=collect,
    verify=dashboards.wait_for_reload,
):
    before = preflight(bundle, directory, provider, fetch, evidence)
    content = bundle["files"][NAME]["content"].encode()
    backups.mkdir(mode=0o700, parents=True, exist_ok=True)
    backup = Path(
        tempfile.mkdtemp(prefix=f"tinfoil-initial-{bundle['revision']}.", dir=backups)
    )
    (backup / "manifest.json").write_text(
        json.dumps(
            {
                "revision": bundle["revision"],
                "previous_file": "absent",
                "previous_uid": "absent",
                "candidate_sha256": dashboards.digest(content),
                "before": before,
            },
            indent=2,
        )
        + "\n"
    )
    (backup / NAME).write_bytes(content)
    print(f"Initial provisioning receipt: {backup}", flush=True)
    try:
        dashboards.atomic_write(directory / NAME, content)
        verify({NAME: content}, fetch)
        after = evidence()
        (backup / "after.json").write_text(json.dumps(after, indent=2) + "\n")
        dashboards.require(
            after["overall_status"] == "green", "metrics failed after provisioning"
        )
    except Exception:
        # disableDeletion=true deliberately retains Grafana DB history. Removing
        # our exact candidate file restores file state; never delete another UID.
        if (
            (directory / NAME).is_file()
            and not (directory / NAME).is_symlink()
            and (directory / NAME).read_bytes() == content
        ):
            (directory / NAME).unlink()
        print(
            f"Provisioning failed. File removed if unchanged; Grafana may retain {UID} because disableDeletion=true. Receipt: {backup}",
            file=sys.stderr,
        )
        raise
    print(f"Verified /d/{UID}; use normal dashboard deployment for subsequent updates.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--apply",
        action="store_true",
        help="perform initial provisioning after preflight",
    )
    parser.add_argument("--bundle", type=Path, help="reviewed bundle generated locally")
    parser.add_argument(
        "--export", type=Path, help="export a bundle from a clean merged checkout"
    )
    options = parser.parse_args()
    if options.export:
        dashboards.require(
            not options.apply and not options.bundle, "export cannot apply"
        )
        dashboards.require(
            not subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT),
            "export requires clean checkout",
        )
        subprocess.run(["git", "fetch", "origin", "main"], cwd=ROOT, check=True)
        subprocess.run(
            ["git", "merge-base", "--is-ancestor", "HEAD", "origin/main"],
            cwd=ROOT,
            check=True,
        )
        subprocess.run(
            [
                "git",
                "diff",
                "--exit-code",
                "HEAD",
                "origin/main",
                "--",
                "infra/monitoring",
                "scripts/finite_tinfoil_status.py",
            ],
            cwd=ROOT,
            check=True,
        )
        bundle = dashboards.bundle_from_repo()
        bundle["files"] = {NAME: bundle["files"][NAME]}
        options.export.write_text(json.dumps(bundle) + "\n")
        return
    dashboards.require(options.bundle is not None, "supply --bundle or --export")
    bundle = json.loads(options.bundle.read_text())
    # Share the same lock as full-stack deploy, CI updates and helper installation.
    with dashboards.LOCK.open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if options.apply:
            dashboards.require(
                os.geteuid() == 0,
                "initial provisioning must run as root on monitoring host",
            )
            provision(bundle)
        else:
            report = preflight(bundle)
            print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()

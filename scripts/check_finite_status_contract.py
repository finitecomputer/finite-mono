#!/usr/bin/env python3
"""Keep finite-status operational names aligned with their Nix authorities."""

from __future__ import annotations

from pathlib import Path

import finite_status


ROOT = Path(__file__).resolve().parents[1]


def require_all(path: Path, values: list[str]) -> None:
    contents = path.read_text(encoding="utf-8")
    missing = [value for value in values if value not in contents]
    if missing:
        raise SystemExit(
            f"{path.relative_to(ROOT)} is missing finite-status contract values: "
            f"{missing}"
        )


def main() -> None:
    contract = finite_status.CONTRACT
    health = contract["healthcheck"]
    require_all(
        ROOT / "infra" / "nixos" / "modules" / "monitoring.nix",
        [
            health["unit"].removesuffix(".service"),
            *health["services"],
            *health["probes"].values(),
        ],
    )

    runner = contract["runner"]
    require_all(
        ROOT / "infra" / "nixos" / "modules" / "finite-saas-runner.nix",
        [
            runner["service"].removesuffix(".service"),
            runner["timer"].removesuffix(".timer"),
            runner["environment_file"],
        ],
    )
    require_all(
        ROOT / "infra" / "hosts" / "lat1" / "systemd" / "runner.env.example",
        [runner["namespace"], runner["drain_variable"], runner["artifact_variable"]],
    )
    require_all(
        ROOT / "infra" / "nixos" / "hosts" / "finite-lat-1" / "disko.nix",
        contract["hosts"]["finite-lat-1"]["disks"],
    )

    recovery = contract["recovery"]
    require_all(
        ROOT / "infra" / "nixos" / "modules" / "backups.nix",
        [
            recovery["snapshot_root"],
            recovery["manifest_name"],
            recovery["borg_job_unit"].removeprefix("borgbackup-job-").removesuffix(".service"),
            recovery["borg_health_unit"].removesuffix(".service"),
            recovery["borg_success_stamp"].removeprefix("/var/lib/finitecomputer/backups/"),
            str(recovery["maximum_age_seconds"]),
        ],
    )
    require_all(
        ROOT / "infra" / "nixos" / "modules" / "finite-litestream.nix",
        [
            recovery["litestream_service_unit"].removesuffix(".service"),
            recovery["litestream_health_unit"].removesuffix(".service"),
            recovery["litestream_success_stamp"].removeprefix("/var/lib/finite-litestream/"),
        ],
    )

    expected_distribution = """select ar.source_host_id, ra.version_label, count(*)
  from agent_runtimes ar
  join runtime_artifacts ra on ra.id = ar.runtime_artifact_id
  group by 1,2 order by 1,2;"""
    if finite_status.DISTRIBUTION_QUERY != expected_distribution:
        raise SystemExit("finite-status drifted from the verified fleet distribution query")
    for query in (
        finite_status.ARTIFACTS_QUERY,
        finite_status.DISTRIBUTION_QUERY,
        finite_status.RUNTIME_DETAILS_QUERY,
    ):
        mutating_tokens = ("INSERT ", "UPDATE ", "DELETE ", "ALTER ", "DROP ")
        if any(token in query.upper() for token in mutating_tokens):
            raise SystemExit("finite-status includes a mutating SQL statement")

    print("finite status contract: ok")


if __name__ == "__main__":
    main()

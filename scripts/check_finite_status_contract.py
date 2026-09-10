#!/usr/bin/env python3
"""Keep finite-status operational names aligned with their Nix authorities."""

from __future__ import annotations

import re
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

    monitoring = contract["monitoring_host"]
    monitoring_units = {
        path.name
        for path in (ROOT / "infra" / "monitoring" / "ubuntu" / "systemd").iterdir()
    }
    missing_monitoring_units = set(monitoring["services"]) - monitoring_units
    if missing_monitoring_units:
        raise SystemExit(
            f"finite-status monitoring units are not repo-owned: {sorted(missing_monitoring_units)}"
        )
    commercial = monitoring["commercial_register"]
    commercial_units = {
        path.name
        for path in (ROOT / "infra" / "commercial-register" / "ubuntu" / "systemd").iterdir()
    }
    expected_commercial_units = set(commercial["services"]) | set(
        commercial["oneshot_services"]
    )
    if not expected_commercial_units <= commercial_units:
        raise SystemExit(
            "finite-status commercial-register units are not repo-owned: "
            f"{sorted(expected_commercial_units - commercial_units)}"
        )
    require_all(
        ROOT / "infra" / "commercial-register" / "ubuntu" / "compose.yaml",
        list(commercial["containers"].values()),
    )
    require_all(
        ROOT / "infra" / "commercial-register" / "ubuntu" / "scripts" / "backup",
        [
            commercial["snapshot_root"],
            commercial["snapshot_format"],
            commercial["borg_success_stamp"],
        ],
    )

    recovery = contract["recovery"]
    require_all(
        ROOT / "infra" / "nixos" / "modules" / "backups.nix",
        [
            recovery["snapshot_root"],
            recovery["manifest_name"],
            recovery["borg_job_unit"]
            .removeprefix("borgbackup-job-")
            .removesuffix(".service"),
            recovery["borg_health_unit"].removesuffix(".service"),
            recovery["borg_success_stamp"].removeprefix(
                "/var/lib/finitecomputer/backups/"
            ),
            str(recovery["snapshot_maximum_age_seconds"]),
            str(recovery["borg_maximum_age_seconds"]),
        ],
    )
    require_all(
        ROOT / "infra" / "nixos" / "modules" / "finite-litestream.nix",
        [
            "finite-litestream-",
            recovery["litestream_health_unit"].removesuffix(".service"),
            recovery["litestream_success_stamp"].removeprefix(
                "/var/lib/finite-litestream/"
            ),
        ],
    )
    # Per-db replicator unit names are finite-litestream-<db.name>; the db
    # names are authored in the lat1 host config.
    require_all(
        ROOT / "infra" / "nixos" / "hosts" / "finite-lat-1" / "default.nix",
        [
            unit.removeprefix("finite-litestream-").removesuffix(".service")
            for unit in recovery["litestream_service_units"]
        ],
    )

    expected_distribution = """select ar.source_host_id, ra.version_label, count(*)
  from agent_runtimes ar
  join runtime_artifacts ra on ra.id = ar.runtime_artifact_id
  group by 1,2 order by 1,2;"""
    if finite_status.DISTRIBUTION_QUERY != expected_distribution:
        raise SystemExit(
            "finite-status drifted from the verified fleet distribution query"
        )
    for query in (
        finite_status.ARTIFACTS_QUERY,
        finite_status.DISTRIBUTION_QUERY,
        finite_status.RUNTIME_DETAILS_QUERY,
    ):
        mutating_tokens = ("INSERT ", "UPDATE ", "DELETE ", "ALTER ", "DROP ")
        if any(token in query.upper() for token in mutating_tokens):
            raise SystemExit("finite-status includes a mutating SQL statement")

    chat = contract["chat_plane"]
    # The freeze detector's red line is derived from the server's own snapshot
    # cadence constant; it must never drift from the Rust authority.
    server_source = (
        ROOT / "finitechat" / "crates" / "finitechat-server" / "src" / "lib.rs"
    ).read_text(encoding="utf-8")
    match = re.search(
        r"pub\(crate\) const SNAPSHOT_INTERVAL_OPS: u64 = ([0-9_]+);", server_source
    )
    if match is None:
        raise SystemExit(
            "finitechat-server no longer declares SNAPSHOT_INTERVAL_OPS; the"
            " chat-plane freeze detector needs its new authority"
        )
    if int(match.group(1).replace("_", "")) != chat["snapshot_interval_ops"]:
        raise SystemExit(
            "finite-status snapshot_interval_ops drifted from finitechat-server's"
            " SNAPSHOT_INTERVAL_OPS"
        )
    require_all(
        ROOT / "infra" / "nixos" / "modules" / "finitechat-server.nix",
        [chat["server_database"].replace("/var/lib/private/", "/var/lib/")],
    )
    # The sync-rate probe reads the edge's access log: the chat vhost must
    # keep a `log` directive so the evidence exists at all.
    require_all(
        ROOT / "infra" / "nixos" / "modules" / "caddy.nix",
        ['virtualHosts."chat.finite.computer"', "log"],
    )
    # Egress attribution map must match the deployed host addresses.
    host_ips = {
        "finite-lat-1": None,  # retired (ADR 0007); kept for historical rows
        "finite-lat-2": ROOT
        / "infra"
        / "nixos"
        / "hosts"
        / "finite-lat-2"
        / "default.nix",
        "finite-lat-3": ROOT
        / "infra"
        / "nixos"
        / "hosts"
        / "finite-lat-3"
        / "default.nix",
        "finite-lat-4": ROOT
        / "infra"
        / "nixos"
        / "hosts"
        / "finite-lat-4"
        / "default.nix",
    }
    reverse_egress = {ip: host for ip, host in chat["egress_ips"].items()}
    for hostname, path in host_ips.items():
        if path is None:
            continue
        ips = [ip for ip, host in reverse_egress.items() if host == hostname]
        if not ips:
            raise SystemExit(f"chat-plane egress map has no address for {hostname}")
        require_all(path, ips)
    # Role gating must follow the deployed host surfaces: the app host runs
    # the chat server and no Runner, runner hosts import the Runner module.
    require_all(
        ROOT / "infra" / "nixos" / "hosts" / "finite-lat-2" / "default.nix",
        ["finitechat-server.nix"],
    )
    if "runner" in contract["hosts"]["finite-lat-2"]["roles"]:
        raise SystemExit(
            "finite-lat-2 is the app-plane host (ADR 0007); no runner role"
        )
    for runner_host in ("finite-lat-3", "finite-lat-4"):
        if "runner" not in contract["hosts"][runner_host]["roles"]:
            raise SystemExit(f"{runner_host} must keep the runner role")
        require_all(
            ROOT / "infra" / "nixos" / "hosts" / runner_host / "default.nix",
            ["finite-saas-runner.nix"],
        )

    print("finite status contract: ok")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Build a read-only, evidence-labelled view of Finite platform state."""

from __future__ import annotations

import argparse
import csv
import hashlib
import io
import json
import os
import re
import shlex
import socket
import subprocess
import sys
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.parse import unquote, urlparse


ROOT = Path(__file__).resolve().parents[1]

# Every operational name lives here. The contract test keeps entries that are
# owned by Nix aligned with their declaring modules.
CONTRACT: dict[str, Any] = {
    "database": {
        "name": "finite_core",
        "environment_file": "/etc/finite/core.env",
        "url_variable": "FC_CORE_DATABASE_URL",
    },
    "healthcheck": {
        "unit": "finite-healthcheck.service",
        "services": [
            "finite-saas-core.service",
            "podman-finite-saas-dashboard.service",
            "finitechat-server.service",
            "finitechat-hosted-device.service",
            "finite-brain-app.service",
            "finite-saas-sites.service",
            "podman-searxng.service",
            "podman-firecrawl-api.service",
            "prometheus-node-exporter.service",
        ],
        "probes": {
            "finite-saas-core": "http://127.0.0.1:4200/healthz",
            "dashboard": "http://127.0.0.1:3000/healthz",
            "finitechat-server": "http://127.0.0.1:8788/health",
            "hosted-web-device": "http://127.0.0.1:38918/healthz",
            "finite-brain": "http://127.0.0.1:3015/health",
            "finitesitesd": "http://127.0.0.1:8787/api/v1/healthz",
            "searxng": "http://127.0.0.1:8080/healthz",
            "firecrawl": "http://127.0.0.1:3002/v0/health/readiness",
            "node-exporter": "http://127.0.0.1:9100/metrics",
        },
    },
    "runner": {
        "service": "finite-saas-runner.service",
        "timer": "finite-saas-runner.timer",
        "environment_file": "/etc/finite/runner.env",
        "namespace": "finite",
        "drain_variable": "FC_RUNNER_DRAIN",
        "artifact_variable": "FC_RUNNER_RUNTIME_ARTIFACT_ID",
    },
    "recovery": {
        "snapshot_root": "/data/recovery-snapshots/hosted-web-chat",
        "latest_name": "latest",
        "manifest_name": "manifest.sha256",
        "borg_job_unit": "borgbackup-job-finite-hosted-web-chat-offsite.service",
        "borg_health_unit": "finite-hosted-web-chat-offsite-health.service",
        "borg_success_stamp": "/var/lib/finitecomputer/backups/hosted-web-chat-last-success",
        "maximum_age_seconds": 180_000,
    },
    "rollout": {
        "state_root": ".local-state/runtime-rollouts",
        "plan_name": "plan.json",
        "events_name": "events.jsonl",
    },
    "hosts": {
        "finite-lat-1": {
            "mounts": ["/", "/data"],
            "storage": "single-disk",
            "mdstat_path": "/proc/mdstat",
            "disks": [
                "/dev/disk/by-id/nvme-Micron_7450_MTFDKBA480TFR_24474C59E53F",
                "/dev/disk/by-id/nvme-SAMSUNG_MZQL21T9HCJR-00A07_S64GNC0Y510146",
            ],
            "recovery": True,
        },
        "finite-lat-3": {
            "mounts": ["/", "/data", "/boot-a", "/boot-b"],
            "storage": "raid",
            "storage_health_unit": "finite-storage-health.service",
            "recovery": False,
        },
    },
    "thresholds": {
        "filesystem_red_percent": 90.0,
        "heartbeat_fresh_seconds": 300,
    },
}

ARTIFACTS_QUERY = """select id, version_label, promoted_at, retired_at
  from runtime_artifacts order by created_at desc;"""

# This query is deliberately byte-for-byte equivalent to the operator-verified
# query in the task. Do not replace it with inferred counts from richer joins.
DISTRIBUTION_QUERY = """select ar.source_host_id, ra.version_label, count(*)
  from agent_runtimes ar
  join runtime_artifacts ra on ra.id = ar.runtime_artifact_id
  group by 1,2 order by 1,2;"""

RUNTIME_DETAILS_QUERY = """select ar.source_host_id,
       ar.id as agent_runtime_id,
       ar.project_id,
       coalesce(p.display_name, ar.id) as agent_name,
       coalesce(ra.version_label, 'unknown') as version_label,
       case
         when exists (
           select 1 from project_runtime_links prl
            where prl.agent_runtime_id = ar.id and prl.active
         ) then 'active'
         when exists (
           select 1 from project_runtime_links prl
            where prl.agent_runtime_id = ar.id
         ) then 'inactive'
         else 'unlinked'
       end as link_state,
       rss.last_heartbeat_at
  from agent_runtimes ar
  left join runtime_artifacts ra on ra.id = ar.runtime_artifact_id
  left join projects p on p.id = ar.project_id
  left join runtime_status_snapshots rss on rss.agent_runtime_id = ar.id
  order by ar.source_host_id, ar.id;"""

SYSTEMD_PROPERTIES = (
    "LoadState",
    "ActiveState",
    "SubState",
    "Result",
    "ExecMainStatus",
    "InvocationID",
    "ActiveEnterTimestamp",
    "InactiveEnterTimestamp",
)


class CollectionError(RuntimeError):
    """A read-only evidence source could not be observed."""


def utc_now() -> datetime:
    return datetime.now(timezone.utc)


def parse_time(value: str | None) -> datetime | None:
    if not value:
        return None
    normalized = value[:-1] + "+00:00" if value.endswith("Z") else value
    try:
        parsed = datetime.fromisoformat(normalized)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def isoformat(value: datetime) -> str:
    return value.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")


def run_read_only(
    command: list[str],
    *,
    environment: dict[str, str] | None = None,
    input_text: str | None = None,
    timeout: int = 30,
) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            command,
            check=False,
            capture_output=True,
            text=True,
            timeout=timeout,
            env=environment,
            input=input_text,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise CollectionError(f"{command[0]} unavailable: {error}") from error


def read_environment_values(path: Path, keys: set[str]) -> dict[str, str]:
    values: dict[str, str] = {}
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise CollectionError(f"cannot read {path}: {error}") from error
    for line in lines:
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or "=" not in stripped:
            continue
        key, raw_value = stripped.split("=", 1)
        if key not in keys:
            continue
        try:
            parsed = shlex.split(raw_value, comments=True, posix=True)
        except ValueError as error:
            raise CollectionError(f"cannot parse {key} in {path}: {error}") from error
        if len(parsed) != 1:
            raise CollectionError(f"cannot parse {key} in {path}")
        values[key] = parsed[0]
    return values


def postgres_environment() -> dict[str, str]:
    database = CONTRACT["database"]
    values = read_environment_values(
        Path(database["environment_file"]), {database["url_variable"]}
    )
    raw_url = values.get(database["url_variable"])
    if not raw_url:
        raise CollectionError(
            f"{database['url_variable']} is absent from {database['environment_file']}"
        )
    parsed = urlparse(raw_url)
    if parsed.scheme not in {"postgres", "postgresql"} or not parsed.hostname:
        raise CollectionError("Core database URL is not a PostgreSQL URL")
    environment = dict(os.environ)
    environment.update(
        {
            "PGHOST": parsed.hostname,
            "PGDATABASE": parsed.path.lstrip("/") or database["name"],
        }
    )
    if parsed.port:
        environment["PGPORT"] = str(parsed.port)
    if parsed.username:
        environment["PGUSER"] = unquote(parsed.username)
    if parsed.password:
        environment["PGPASSWORD"] = unquote(parsed.password)
    return environment


def psql_query_sets(environment: dict[str, str]) -> dict[str, list[dict[str, Any]]]:
    definitions = [
        (
            "artifacts",
            ARTIFACTS_QUERY,
            ["id", "version_label", "promoted_at", "retired_at"],
        ),
        (
            "distribution",
            DISTRIBUTION_QUERY,
            ["source_host_id", "version_label", "count"],
        ),
        (
            "runtimes",
            RUNTIME_DETAILS_QUERY,
            [
                "source_host_id",
                "agent_runtime_id",
                "project_id",
                "agent_name",
                "version_label",
                "link_state",
                "last_heartbeat_at",
            ],
        ),
    ]
    markers = {
        f"__FINITE_STATUS_{name.upper()}__": (name, columns)
        for name, _, columns in definitions
    }
    sql_lines = ["BEGIN TRANSACTION READ ONLY;", "SET LOCAL statement_timeout = '10s';"]
    for name, query, _ in definitions:
        sql_lines.extend([f"\\echo __FINITE_STATUS_{name.upper()}__", query])
    sql_lines.append("COMMIT;")
    result = run_read_only(
        [
            "psql",
            "--no-psqlrc",
            "--csv",
            "--tuples-only",
            "--quiet",
            "--set",
            "ON_ERROR_STOP=1",
            "--dbname",
            CONTRACT["database"]["name"],
        ],
        environment=environment,
        input_text="\n".join(sql_lines) + "\n",
    )
    if result.returncode != 0:
        message = result.stderr.strip().splitlines()
        detail = message[-1] if message else f"exit {result.returncode}"
        raise CollectionError(f"read-only Core query failed: {detail}")
    query_sets = {name: [] for name, _, _ in definitions}
    active_name: str | None = None
    active_columns: list[str] = []
    for values in csv.reader(io.StringIO(result.stdout)):
        if len(values) == 1 and values[0] in markers:
            active_name, active_columns = markers[values[0]]
            continue
        if not values:
            continue
        if active_name is None or len(values) != len(active_columns):
            raise CollectionError("Core query returned an unexpected transaction shape")
        query_sets[active_name].append(dict(zip(active_columns, values, strict=True)))
    if active_name != "runtimes":
        raise CollectionError("Core query transaction did not reach Runtime details")
    return query_sets


def collect_core() -> dict[str, Any]:
    environment = postgres_environment()
    return psql_query_sets(environment)


def systemd_properties(unit: str) -> dict[str, str]:
    result = run_read_only(
        [
            "systemctl",
            "show",
            "--no-pager",
            *[f"--property={name}" for name in SYSTEMD_PROPERTIES],
            unit,
        ]
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or f"exit {result.returncode}"
        raise CollectionError(f"cannot observe {unit}: {detail}")
    properties: dict[str, str] = {}
    for line in result.stdout.splitlines():
        if "=" in line:
            key, value = line.split("=", 1)
            properties[key] = value
    return properties


def collect_healthcheck_journal(properties: dict[str, str]) -> dict[str, str]:
    invocation = properties.get("InvocationID")
    if not invocation:
        latest = run_read_only(
            [
                "journalctl",
                "--no-pager",
                "--reverse",
                "--lines=20",
                "--output=json",
                f"--unit={CONTRACT['healthcheck']['unit']}",
            ]
        )
        if latest.returncode == 0:
            for line in latest.stdout.splitlines():
                try:
                    journal_entry = json.loads(line)
                except json.JSONDecodeError:
                    continue
                candidate = journal_entry.get("_SYSTEMD_INVOCATION_ID")
                if candidate:
                    invocation = candidate
                    break
    if not invocation:
        raise CollectionError("finite-healthcheck has no recorded journal invocation")
    result = run_read_only(
        [
            "journalctl",
            "--no-pager",
            "--output=cat",
            f"_SYSTEMD_INVOCATION_ID={invocation}",
        ]
    )
    if result.returncode != 0:
        raise CollectionError("finite-healthcheck journal invocation is unavailable")
    probes: dict[str, str] = {}
    expected = CONTRACT["healthcheck"]["probes"]
    pattern = re.compile(r"^(OK|WAIT|FAIL)\s+([A-Za-z0-9-]+)(?:\s|$)")
    for line in result.stdout.splitlines():
        match = pattern.match(line.strip())
        if match and match.group(2) in expected:
            probes[match.group(2)] = match.group(1)
    return probes


def line_count(command: list[str]) -> int:
    result = run_read_only(command)
    if result.returncode != 0:
        raise CollectionError(
            f"{' '.join(command[:3])} failed: {result.stderr.strip() or result.returncode}"
        )
    return sum(bool(line.strip()) for line in result.stdout.splitlines())


def collect_host_health(hostname: str) -> dict[str, Any]:
    profile = CONTRACT["hosts"].get(hostname)
    if profile is None:
        return {"hostname": hostname, "error": "host has no finite-status profile"}

    raw: dict[str, Any] = {"hostname": hostname, "errors": [], "units": {}}
    units = list(CONTRACT["healthcheck"]["services"])
    units.extend(
        [
            CONTRACT["healthcheck"]["unit"],
            CONTRACT["runner"]["service"],
            CONTRACT["runner"]["timer"],
        ]
    )
    if profile.get("storage_health_unit"):
        units.append(profile["storage_health_unit"])
    for unit in dict.fromkeys(units):
        try:
            raw["units"][unit] = systemd_properties(unit)
        except CollectionError as error:
            raw["units"][unit] = {"error": str(error)}

    healthcheck = raw["units"].get(CONTRACT["healthcheck"]["unit"], {})
    try:
        raw["probes"] = collect_healthcheck_journal(healthcheck)
    except CollectionError as error:
        raw["probes"] = {}
        raw["errors"].append(str(error))

    raw["filesystems"] = []
    for mount in profile["mounts"]:
        try:
            stats = os.statvfs(mount)
            total = stats.f_blocks * stats.f_frsize
            available = stats.f_bavail * stats.f_frsize
            used_percent = 0.0 if total == 0 else (total - available) * 100.0 / total
            raw["filesystems"].append(
                {
                    "mount": mount,
                    "total_bytes": total,
                    "available_bytes": available,
                    "used_percent": round(used_percent, 1),
                }
            )
        except OSError as error:
            raw["filesystems"].append({"mount": mount, "error": str(error)})

    raw["storage"] = {"mode": profile["storage"]}
    if profile["storage"] == "single-disk":
        raw["storage"]["disks"] = [
            {"path": path, "present": Path(path).exists()}
            for path in profile["disks"]
        ]
        try:
            mdstat = Path(profile["mdstat_path"]).read_text(encoding="utf-8")
            raw["storage"]["md_arrays"] = re.findall(
                r"^(md\S+)\s*:", mdstat, flags=re.MULTILINE
            )
        except OSError as error:
            raw["storage"]["error"] = str(error)

    namespace = CONTRACT["runner"]["namespace"]
    raw["containers"] = {}
    container_commands = {
        "podman_running": ["podman", "ps", "--quiet"],
        "podman_total": ["podman", "ps", "--all", "--quiet"],
        "kata_running": ["nerdctl", "--namespace", namespace, "ps", "--quiet"],
        "kata_total": [
            "nerdctl",
            "--namespace",
            namespace,
            "ps",
            "--all",
            "--quiet",
        ],
    }
    for name, command in container_commands.items():
        try:
            raw["containers"][name] = line_count(command)
        except CollectionError as error:
            raw["containers"][name] = None
            raw["errors"].append(str(error))

    runner = CONTRACT["runner"]
    try:
        raw["runner_environment"] = read_environment_values(
            Path(runner["environment_file"]),
            {runner["drain_variable"], runner["artifact_variable"]},
        )
    except CollectionError as error:
        raw["runner_environment"] = {}
        raw["errors"].append(str(error))
    return raw


def safe_snapshot_directory(root: Path) -> Path:
    latest = root / CONTRACT["recovery"]["latest_name"]
    if not latest.is_symlink():
        raise CollectionError(f"{latest} is not a symlink")
    try:
        resolved_root = root.resolve(strict=True)
        snapshot = latest.resolve(strict=True)
        snapshot.relative_to(resolved_root)
    except (OSError, ValueError) as error:
        raise CollectionError(f"latest snapshot escapes its recovery root: {error}") from error
    if not snapshot.is_dir():
        raise CollectionError("latest recovery snapshot is not a directory")
    return snapshot


def verify_manifest(snapshot: Path) -> tuple[int, list[str]]:
    try:
        snapshot = snapshot.resolve(strict=True)
    except OSError as error:
        raise CollectionError(f"cannot resolve snapshot directory: {error}") from error
    manifest = snapshot / CONTRACT["recovery"]["manifest_name"]
    try:
        lines = manifest.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise CollectionError(f"cannot read {manifest}: {error}") from error
    if not lines:
        raise CollectionError("snapshot checksum manifest is empty")
    failures: list[str] = []
    checked = 0
    seen_paths: set[str] = set()
    for line in lines:
        match = re.fullmatch(r"([0-9a-fA-F]{64}) ([ *])(.+)", line)
        if not match:
            failures.append("invalid manifest line")
            continue
        expected, _, relative_name = match.groups()
        if relative_name in seen_paths:
            failures.append(f"duplicate path: {relative_name}")
            continue
        seen_paths.add(relative_name)
        relative = Path(relative_name)
        if relative.is_absolute() or ".." in relative.parts:
            failures.append(f"unsafe path: {relative_name}")
            continue
        candidate = snapshot / relative
        try:
            if candidate.is_symlink():
                raise OSError("manifest entry is a symlink")
            resolved = candidate.resolve(strict=True)
            resolved.relative_to(snapshot)
            if not resolved.is_file():
                raise OSError("not a regular file")
            digest = hashlib.sha256()
            with resolved.open("rb") as stream:
                for block in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(block)
        except (OSError, ValueError) as error:
            failures.append(f"{relative_name}: {error}")
            continue
        checked += 1
        if digest.hexdigest().lower() != expected.lower():
            failures.append(f"{relative_name}: checksum mismatch")
    return checked, failures


def collect_recovery(hostname: str) -> dict[str, Any]:
    profile = CONTRACT["hosts"].get(hostname)
    if profile and not profile["recovery"]:
        return {"applicable": False}
    recovery = CONTRACT["recovery"]
    raw: dict[str, Any] = {"applicable": True, "errors": []}
    try:
        snapshot = safe_snapshot_directory(Path(recovery["snapshot_root"]))
        checked, failures = verify_manifest(snapshot)
        created = datetime.strptime(snapshot.name, "%Y%m%dT%H%M%SZ").replace(
            tzinfo=timezone.utc
        )
        raw["snapshot"] = {
            "name": snapshot.name,
            "created_at": isoformat(created),
            "manifest_entries": checked,
            "manifest_valid": not failures,
            "manifest_failures": failures,
        }
    except (CollectionError, ValueError) as error:
        raw["snapshot"] = {"error": str(error)}

    stamp = Path(recovery["borg_success_stamp"])
    try:
        raw["borg_last_success_epoch"] = int(stamp.read_text(encoding="utf-8").strip())
    except (OSError, ValueError) as error:
        raw["borg_last_success_error"] = f"cannot read {stamp}: {error}"
    for key in ("borg_job_unit", "borg_health_unit"):
        unit = recovery[key]
        try:
            raw[key] = systemd_properties(unit)
        except CollectionError as error:
            raw[key] = {"error": str(error)}
    return raw


def collect_rollout() -> dict[str, Any]:
    rollout = CONTRACT["rollout"]
    root = Path.cwd() / rollout["state_root"]
    if not root.is_dir():
        return {"exists": False, "root": str(root)}
    candidates: list[tuple[int, Path]] = []
    try:
        for child in root.iterdir():
            events = child / rollout["events_name"]
            if child.is_dir() and events.is_file():
                candidates.append((events.stat().st_mtime_ns, child))
    except OSError as error:
        return {"exists": True, "error": str(error), "root": str(root)}
    if not candidates:
        return {"exists": False, "root": str(root)}
    evidence = max(candidates, key=lambda item: (item[0], item[1].name))[1]
    try:
        plan = json.loads((evidence / rollout["plan_name"]).read_text(encoding="utf-8"))
        events = [
            json.loads(line)
            for line in (evidence / rollout["events_name"])
            .read_text(encoding="utf-8")
            .splitlines()
            if line.strip()
        ]
    except (OSError, json.JSONDecodeError) as error:
        return {
            "exists": True,
            "plan_hash": evidence.name,
            "root": str(root),
            "error": str(error),
        }
    return {
        "exists": True,
        "plan_hash": evidence.name,
        "root": str(root),
        "plan": plan,
        "events": events,
    }


def expand_fixture(raw: dict[str, Any]) -> dict[str, Any]:
    groups = raw.get("core", {}).pop("runtime_groups", [])
    for group in groups:
        count = int(group["count"])
        for index in range(1, count + 1):
            raw["core"].setdefault("runtimes", []).append(
                {
                    "source_host_id": group["source_host_id"],
                    "agent_runtime_id": f"{group['id_prefix']}-{index:02d}",
                    "project_id": f"{group['project_prefix']}-{index:02d}",
                    "agent_name": f"{group['name_prefix']} {index:02d}",
                    "version_label": group["version_label"],
                    "link_state": group["link_state"],
                    "last_heartbeat_at": group.get("last_heartbeat_at", ""),
                }
            )
    return raw


def load_fixture(path: Path) -> dict[str, Any]:
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise CollectionError(f"cannot load fixture {path}: {error}") from error
    return expand_fixture(raw)


def heartbeat_signal(value: str | None, now: datetime) -> dict[str, Any]:
    heartbeat = parse_time(value)
    if heartbeat is None:
        return {"last_heartbeat_at": value or None, "freshness": "missing", "age_seconds": None}
    age = int((now - heartbeat).total_seconds())
    if age < 0:
        freshness = "future"
    elif age <= CONTRACT["thresholds"]["heartbeat_fresh_seconds"]:
        freshness = "fresh"
    else:
        freshness = "stale"
    return {
        "last_heartbeat_at": isoformat(heartbeat),
        "freshness": freshness,
        "age_seconds": age,
    }


def combine_status(statuses: list[str]) -> str:
    if "red" in statuses:
        return "red"
    if "unknown" in statuses:
        return "unknown"
    return "green"


def build_fleet(
    core: dict[str, Any] | None,
    now: datetime,
    error: str | None = None,
) -> dict[str, Any]:
    if core is None:
        return {"status": "unknown", "error": error or "Core evidence unavailable"}
    artifacts = core.get("artifacts", [])
    target = next(
        (
            artifact
            for artifact in artifacts
            if artifact.get("promoted_at") and not artifact.get("retired_at")
        ),
        None,
    )
    if target is None:
        return {"status": "unknown", "error": "no promoted, non-retired Runtime artifact"}

    target_version = target["version_label"]
    hosts: dict[str, Any] = {}
    for row in core.get("runtimes", []):
        host = hosts.setdefault(
            row["source_host_id"],
            {"active": [], "inactive": [], "unlinked": []},
        )
        entry = {
            "agent_runtime_id": row["agent_runtime_id"],
            "project_id": row["project_id"],
            "agent_name": row["agent_name"],
            "version_label": row["version_label"],
            "heartbeat": heartbeat_signal(row.get("last_heartbeat_at"), now),
        }
        host.setdefault(row.get("link_state", "unlinked"), host["unlinked"]).append(entry)

    host_reports: list[dict[str, Any]] = []
    section_statuses: list[str] = []
    for hostname in sorted(hosts):
        groups = hosts[hostname]
        active = groups["active"]
        stragglers = [row for row in active if row["version_label"] != target_version]
        on_target = len(active) - len(stragglers)
        status = "red" if stragglers else ("unknown" if groups["unlinked"] else "green")
        section_statuses.append(status)
        stale = sum(
            row["heartbeat"]["freshness"] != "fresh"
            for row in active
        )
        non_fresh_heartbeats = [
            row for row in active if row["heartbeat"]["freshness"] != "fresh"
        ]
        host_reports.append(
            {
                "source_host_id": hostname,
                "status": status,
                "active_total": len(active),
                "on_target": on_target,
                "straggler_count": len(stragglers),
                "stragglers": stragglers,
                "intentionally_inactive_count": len(groups["inactive"]),
                "intentionally_inactive": groups["inactive"],
                "unlinked_count": len(groups["unlinked"]),
                "unlinked": groups["unlinked"],
                "non_fresh_heartbeat_signals": stale,
                "non_fresh_heartbeats": non_fresh_heartbeats,
            }
        )

    distribution = [
        {
            "source_host_id": row["source_host_id"],
            "version_label": row["version_label"],
            "count": int(row["count"]),
        }
        for row in core.get("distribution", [])
    ]
    expected_distribution = Counter(
        (row["source_host_id"], row["version_label"])
        for row in core.get("runtimes", [])
    )
    observed_distribution = Counter(
        {
            (row["source_host_id"], row["version_label"]): int(row["count"])
            for row in core.get("distribution", [])
        }
    )
    distribution_consistent = expected_distribution == observed_distribution
    if not distribution_consistent:
        section_statuses.append("unknown")
    if not host_reports:
        section_statuses.append("unknown")
    return {
        "status": combine_status(section_statuses),
        "evidence": "Core-recorded artifact/link state; not verified live compute",
        "status_basis": "active-link artifact convergence only",
        "heartbeat_note": "timestamps are staleness signals, not lifecycle proof",
        "target_artifact": {
            "id": target["id"],
            "version_label": target_version,
            "promoted_at": target["promoted_at"],
            "retired_at": target.get("retired_at") or None,
        },
        "recorded_distribution": distribution,
        "distribution_consistent_with_detail_snapshot": distribution_consistent,
        "hosts": host_reports,
    }


def unit_status(properties: dict[str, Any], *, active_required: bool) -> str:
    if properties.get("error") or properties.get("LoadState") in {None, "not-found"}:
        return "unknown"
    if active_required:
        return "green" if properties.get("ActiveState") == "active" else "red"
    result = properties.get("Result")
    if result == "success" and str(properties.get("ExecMainStatus", "0")) == "0":
        return "green"
    if result and result != "success":
        return "red"
    return "unknown"


def build_host_health(raw: dict[str, Any] | None, target_id: str | None) -> dict[str, Any]:
    if raw is None or raw.get("error"):
        return {
            "status": "unknown",
            "hostname": None if raw is None else raw.get("hostname"),
            "error": "host evidence unavailable" if raw is None else raw["error"],
        }
    statuses: list[str] = []
    units = []
    for unit in CONTRACT["healthcheck"]["services"]:
        properties = raw.get("units", {}).get(unit, {"error": "not observed"})
        status = unit_status(properties, active_required=True)
        statuses.append(status)
        units.append(
            {
                "unit": unit,
                "status": status,
                "active_state": properties.get("ActiveState"),
                "sub_state": properties.get("SubState"),
                "error": properties.get("error"),
            }
        )

    health_unit = CONTRACT["healthcheck"]["unit"]
    health_props = raw.get("units", {}).get(health_unit, {"error": "not observed"})
    health_status = unit_status(health_props, active_required=False)
    statuses.append(health_status)
    probes = []
    for name, endpoint in CONTRACT["healthcheck"]["probes"].items():
        observed = raw.get("probes", {}).get(name)
        status = "green" if observed == "OK" else ("red" if observed == "FAIL" else "unknown")
        statuses.append(status)
        probes.append(
            {
                "name": name,
                "endpoint": endpoint,
                "recorded_result": observed,
                "status": status,
            }
        )

    filesystems = []
    for filesystem in raw.get("filesystems", []):
        if filesystem.get("error"):
            status = "unknown"
        elif (
            float(filesystem["used_percent"])
            >= CONTRACT["thresholds"]["filesystem_red_percent"]
        ):
            status = "red"
        else:
            status = "green"
        statuses.append(status)
        filesystems.append({**filesystem, "status": status})

    storage = dict(raw.get("storage", {}))
    if storage.get("error"):
        storage["status"] = "unknown"
    elif storage.get("mode") == "single-disk":
        disks = storage.get("disks")
        if not disks:
            storage["status"] = "unknown"
        elif storage.get("md_arrays") or not all(disk.get("present") for disk in disks):
            storage["status"] = "red"
        else:
            storage["status"] = "green"
    elif storage.get("mode") == "raid":
        properties = raw.get("units", {}).get(
            CONTRACT["hosts"]["finite-lat-3"]["storage_health_unit"], {}
        )
        storage["status"] = unit_status(properties, active_required=False)
    else:
        storage["status"] = "unknown"
    statuses.append(storage["status"])

    raw_containers = raw.get("containers", {})
    containers = dict(raw_containers)
    containers["kata_vm_count"] = containers.get("kata_running")
    containers["status"] = (
        "green"
        if raw_containers
        and all(value is not None for value in raw_containers.values())
        else "unknown"
    )
    statuses.append(containers["status"])

    runner_contract = CONTRACT["runner"]
    timer_props = raw.get("units", {}).get(runner_contract["timer"], {})
    timer_status = unit_status(timer_props, active_required=True)
    runner_env = raw.get("runner_environment", {})
    drain = runner_env.get(runner_contract["drain_variable"])
    if drain is None:
        drain_status = "unknown"
    else:
        drain_status = "green" if drain.lower() == "false" else "red"
    pin = runner_env.get(runner_contract["artifact_variable"])
    if not pin or not target_id:
        pin_status = "unknown"
    else:
        pin_status = "green" if pin == target_id else "red"
    statuses.extend([timer_status, drain_status, pin_status])
    runner = {
        "timer_unit": runner_contract["timer"],
        "timer_status": timer_status,
        "drain": drain,
        "drain_status": drain_status,
        "artifact_pin": pin,
        "target_artifact_id": target_id,
        "pin_status": pin_status,
    }
    return {
        "status": combine_status(statuses),
        "hostname": raw.get("hostname"),
        "healthcheck": {
            "unit": health_unit,
            "status": health_status,
            "recorded_result": health_props.get("Result"),
            "invocation_id": health_props.get("InvocationID"),
        },
        "services": units,
        "http_probes": probes,
        "filesystems": filesystems,
        "storage": storage,
        "containers": containers,
        "runner": runner,
        "collection_errors": raw.get("errors", []),
    }


def build_recovery(raw: dict[str, Any] | None, now: datetime) -> dict[str, Any]:
    if raw is None:
        return {"status": "unknown", "error": "recovery evidence unavailable"}
    if not raw.get("applicable", True):
        return {"status": "green", "applicable": False, "state": "not-applicable"}
    statuses: list[str] = []
    snapshot = dict(raw.get("snapshot", {}))
    created = parse_time(snapshot.get("created_at"))
    if snapshot.get("error") or created is None:
        snapshot["status"] = "unknown"
        snapshot["age_seconds"] = None
    else:
        snapshot["age_seconds"] = int((now - created).total_seconds())
        snapshot["status"] = (
            "green"
            if snapshot.get("manifest_valid")
            and snapshot["age_seconds"] <= CONTRACT["recovery"]["maximum_age_seconds"]
            else "red"
        )
    statuses.append(snapshot["status"])

    epoch = raw.get("borg_last_success_epoch")
    if isinstance(epoch, int):
        borg_age = int(now.timestamp()) - epoch
        stamp_status = (
            "green"
            if 0 <= borg_age <= CONTRACT["recovery"]["maximum_age_seconds"]
            else "red"
        )
    else:
        borg_age = None
        stamp_status = "unknown"
    job_status = unit_status(raw.get("borg_job_unit", {}), active_required=False)
    health_status = unit_status(raw.get("borg_health_unit", {}), active_required=False)
    statuses.extend([stamp_status, job_status, health_status])
    return {
        "status": combine_status(statuses),
        "applicable": True,
        "snapshot": snapshot,
        "borg": {
            "completion_mechanism": "systemd job result plus Nix postCreate success stamp",
            "job_unit": CONTRACT["recovery"]["borg_job_unit"],
            "job_status": job_status,
            "job_result": raw.get("borg_job_unit", {}).get("Result"),
            "health_unit": CONTRACT["recovery"]["borg_health_unit"],
            "health_status": health_status,
            "last_success_epoch": epoch,
            "age_seconds": borg_age,
            "stamp_status": stamp_status,
            "error": raw.get("borg_last_success_error"),
        },
    }


def build_rollout(raw: dict[str, Any] | None) -> dict[str, Any]:
    if raw is None:
        return {"status": "unknown", "error": "rollout evidence unavailable"}
    if not raw.get("exists"):
        return {
            "status": "green",
            "evidence_present": False,
            "state": "no-local-evidence",
            "root": raw.get("root"),
        }
    if raw.get("error"):
        return {
            "status": "unknown",
            "evidence_present": True,
            "plan_hash": raw.get("plan_hash"),
            "error": raw["error"],
        }
    plan = raw.get("plan", {})
    events = raw.get("events", [])
    start_indexes = [index for index, event in enumerate(events) if event.get("event") == "start"]
    if not start_indexes:
        return {
            "status": "unknown",
            "evidence_present": True,
            "plan_hash": raw.get("plan_hash"),
            "error": "events contain no start record",
        }
    phase_events = events[start_indexes[-1] :]
    phase = phase_events[0].get("phase")
    terminals = [event for event in phase_events if event.get("event") == "final"]
    terminal = terminals[-1] if terminals else None
    completed_ids = {
        event.get("agent_runtime_id")
        for event in phase_events
        if event.get("event") == "entry_postflight" and event.get("status") == "succeeded"
    }
    completed_ids.discard(None)
    planned = len(plan.get("planned", []))
    if terminal is None:
        status = "red"
        terminal_state = "interrupted-or-incomplete"
    elif terminal.get("status") == "interrupted":
        status = "red"
        terminal_state = "interrupted"
    elif terminal.get("status") == "noop":
        status = "green"
        terminal_state = "noop"
    elif terminal.get("status") != "success":
        status = "red"
        terminal_state = "failure"
    elif phase == "execute" and len(completed_ids) != planned:
        status = "red"
        terminal_state = "success-with-incomplete-evidence"
    else:
        status = "green"
        terminal_state = "prepared" if phase == "prepare" else "success"
    return {
        "status": status,
        "evidence_present": True,
        "plan_hash": raw.get("plan_hash"),
        "phase": phase,
        "planned_entries": planned,
        "completed_entries": len(completed_ids),
        "terminal_state": terminal_state,
        "terminal_event": terminal,
        "latest_event_at": phase_events[-1].get("timestamp") if phase_events else None,
    }


def report_exit_code(report: dict[str, Any]) -> int:
    statuses = [section["status"] for section in report["sections"].values()]
    if "red" in statuses:
        return 1
    if "unknown" in statuses:
        return 2
    return 0


def build_report(raw: dict[str, Any], now: datetime) -> dict[str, Any]:
    errors = raw.get("collection_errors", {})
    fleet = build_fleet(raw.get("core"), now, errors.get("core"))
    target_id = fleet.get("target_artifact", {}).get("id")
    sections = {
        "fleet_convergence": fleet,
        "host_health": build_host_health(raw.get("host_health"), target_id),
        "recovery_boundary": build_recovery(raw.get("recovery"), now),
        "rollout_state": build_rollout(raw.get("rollout")),
    }
    report = {
        "schema_version": "finite.status.v1",
        "generated_at": isoformat(now),
        "overall_status": combine_status([section["status"] for section in sections.values()]),
        "exit_code": 0,
        "sections": sections,
    }
    report["exit_code"] = report_exit_code(report)
    return report


def human_age(seconds: int | None) -> str:
    if seconds is None:
        return "unknown"
    if seconds < 0:
        return f"{abs(seconds)}s in the future"
    if seconds < 120:
        return f"{seconds}s"
    if seconds < 7200:
        return f"{seconds // 60}m"
    if seconds < 172800:
        return f"{seconds // 3600}h"
    return f"{seconds // 86400}d"


def badge(status: str) -> str:
    return f"[{status.upper()}]"


def render_human(report: dict[str, Any]) -> str:
    sections = report["sections"]
    lines = [
        f"Finite platform status — {report['generated_at']}",
        f"Overall {badge(report['overall_status'])}; exit {report['exit_code']}",
        "",
    ]

    fleet = sections["fleet_convergence"]
    lines.append(
        f"Fleet convergence {badge(fleet['status'])} — "
        "Core-recorded, NOT verified live"
    )
    if fleet.get("target_artifact"):
        target = fleet["target_artifact"]
        lines.append(
            f"  target: {target['version_label']} ({target['id']}) "
            f"promoted {target['promoted_at']}"
        )
        for host in fleet.get("hosts", []):
            lines.append(
                f"  {host['source_host_id']}: {host['on_target']}/{host['active_total']} active on target; "
                f"{host['straggler_count']} stragglers; "
                f"{host['intentionally_inactive_count']} intentionally inactive excluded; "
                f"{host['non_fresh_heartbeat_signals']} non-fresh heartbeat signals"
            )
            for runtime in host["stragglers"]:
                heartbeat = runtime["heartbeat"]
                lines.append(
                    f"    STRAGGLER {runtime['agent_name']} [{runtime['agent_runtime_id']}]: "
                    f"{runtime['version_label']}; heartbeat {heartbeat['freshness']} "
                    f"({human_age(heartbeat['age_seconds'])} old)"
                )
            for runtime in host["unlinked"]:
                lines.append(
                    f"    UNKNOWN-LINK {runtime['agent_name']} [{runtime['agent_runtime_id']}]: "
                    f"{runtime['version_label']}"
                )
            straggler_ids = {
                runtime["agent_runtime_id"] for runtime in host["stragglers"]
            }
            for runtime in host["non_fresh_heartbeats"]:
                if runtime["agent_runtime_id"] in straggler_ids:
                    continue
                heartbeat = runtime["heartbeat"]
                lines.append(
                    f"    HEARTBEAT {runtime['agent_name']} [{runtime['agent_runtime_id']}]: "
                    f"{heartbeat['freshness']} ({human_age(heartbeat['age_seconds'])} old)"
                )
    else:
        lines.append(f"  {fleet.get('error', 'unavailable')}")
    lines.append("  heartbeat timestamps are staleness signals, not lifecycle proof")
    lines.append("")

    health = sections["host_health"]
    lines.append(
        f"Host health {badge(health['status'])} — "
        f"{health.get('hostname') or 'unknown host'}"
    )
    if health.get("healthcheck"):
        check = health["healthcheck"]
        lines.append(
            f"  {check['unit']}: recorded result={check['recorded_result'] or 'unknown'} {badge(check['status'])}"
        )
        failed_services = [
            unit for unit in health["services"] if unit["status"] != "green"
        ]
        lines.append(
            f"  services: {len(health['services']) - len(failed_services)}/{len(health['services'])} active"
        )
        for unit in failed_services:
            lines.append(
                f"    {badge(unit['status'])} {unit['unit']}: {unit['active_state'] or unit.get('error') or 'unknown'}"
            )
        for probe in health["http_probes"]:
            lines.append(
                f"    {badge(probe['status'])} HTTP {probe['name']}: "
                f"{probe['recorded_result'] or 'not recorded'} ({probe['endpoint']})"
            )
        for filesystem in health["filesystems"]:
            used = filesystem.get("used_percent")
            lines.append(
                f"  filesystem {filesystem['mount']}: {used if used is not None else 'unknown'}% used "
                f"{badge(filesystem['status'])}"
            )
        storage = health["storage"]
        if storage.get("mode") == "single-disk":
            disks = storage.get("disks", [])
            present = sum(bool(disk.get("present")) for disk in disks)
            lines.append(
                f"  storage: single-disk; expected devices {present}/{len(disks)} present; "
                f"MD arrays={len(storage.get('md_arrays', []))} {badge(storage['status'])}"
            )
        else:
            lines.append(
                f"  storage: {storage.get('mode', 'unknown')} recorded health "
                f"{badge(storage['status'])}"
            )
        containers = health["containers"]
        lines.append(
            f"  containers: podman {containers.get('podman_running')}/{containers.get('podman_total')} running; "
            f"Kata VMs {containers.get('kata_running')}/{containers.get('kata_total')} running"
        )
        runner = health["runner"]
        lines.append(
            f"  runner: timer {badge(runner['timer_status'])}; drain={runner['drain'] or 'unknown'} "
            f"{badge(runner['drain_status'])}; pin={runner['artifact_pin'] or 'unknown'} "
            f"{badge(runner['pin_status'])}"
        )
    else:
        lines.append(f"  {health.get('error', 'unavailable')}")
    lines.append("")

    recovery = sections["recovery_boundary"]
    lines.append(f"Recovery boundary {badge(recovery['status'])}")
    if recovery.get("applicable") is False:
        lines.append("  not applicable on this host")
    elif recovery.get("snapshot"):
        snapshot = recovery["snapshot"]
        lines.append(
            f"  snapshot {snapshot.get('name', 'unknown')}: age {human_age(snapshot.get('age_seconds'))}; "
            f"manifest={snapshot.get('manifest_valid', 'unknown')} ({snapshot.get('manifest_entries', 0)} files) "
            f"{badge(snapshot['status'])}"
        )
        borg = recovery["borg"]
        lines.append(
            f"  Borg: job result={borg['job_result'] or 'unknown'} {badge(borg['job_status'])}; "
            f"last success {human_age(borg['age_seconds'])} ago {badge(borg['stamp_status'])}; "
            f"health {badge(borg['health_status'])}"
        )
    else:
        lines.append(f"  {recovery.get('error', 'unavailable')}")
    lines.append("")

    rollout = sections["rollout_state"]
    lines.append(f"Rollout state {badge(rollout['status'])}")
    if rollout.get("error"):
        lines.append(f"  {rollout['error']}")
    elif not rollout.get("evidence_present"):
        lines.append("  no local .local-state/runtime-rollouts evidence")
    elif rollout.get("phase"):
        lines.append(
            f"  {rollout['plan_hash']}: phase={rollout['phase']}; "
            f"planned={rollout['planned_entries']}; completed={rollout['completed_entries']}; "
            f"terminal={rollout['terminal_state']}"
        )
    else:
        lines.append(f"  {rollout.get('error', 'unavailable')}")
    lines.extend(
        [
            "",
            "Exit codes: 0 all green; 1 any red; 2 could not determine (when nothing is red).",
        ]
    )
    return "\n".join(lines)


def collect_live() -> tuple[dict[str, Any], datetime]:
    now = utc_now()
    hostname = socket.gethostname().split(".", 1)[0]
    raw: dict[str, Any] = {"collection_errors": {}}
    try:
        raw["core"] = collect_core()
    except CollectionError as error:
        raw["collection_errors"]["core"] = str(error)
    raw["host_health"] = collect_host_health(hostname)
    raw["recovery"] = collect_recovery(hostname)
    raw["rollout"] = collect_rollout()
    return raw, now


def parse_args(arguments: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Canonical read-only Finite platform and fleet status"
    )
    parser.add_argument("--json", action="store_true", help="emit finite.status.v1 JSON")
    parser.add_argument(
        "--fixture",
        type=Path,
        help="read an offline recorded fixture instead of host/production evidence",
    )
    return parser.parse_args(arguments)


def main(arguments: list[str] | None = None) -> None:
    options = parse_args(sys.argv[1:] if arguments is None else arguments)
    try:
        if options.fixture:
            raw = load_fixture(options.fixture)
            now = parse_time(raw.get("now"))
            if now is None:
                raise CollectionError("fixture has no valid 'now' timestamp")
        else:
            raw, now = collect_live()
        report = build_report(raw, now)
    except CollectionError as error:
        report = {
            "schema_version": "finite.status.v1",
            "generated_at": isoformat(utc_now()),
            "overall_status": "unknown",
            "exit_code": 2,
            "sections": {
                "fleet_convergence": {"status": "unknown", "error": str(error)},
                "host_health": {"status": "unknown", "error": str(error)},
                "recovery_boundary": {"status": "unknown", "error": str(error)},
                "rollout_state": {"status": "unknown", "error": str(error)},
            },
        }
    if options.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print(render_human(report))
    raise SystemExit(report["exit_code"])

#!/usr/bin/env python3
"""Build a read-only, evidence-labelled view of Finite platform state."""

from __future__ import annotations

import argparse
import contextlib
import csv
import glob
import hashlib
import io
import json
import os
import re
import shlex
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from collections import Counter
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Iterator
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
            "prometheus-node-exporter.service",
        ],
        "probes": {
            "finite-saas-core": "http://127.0.0.1:4200/healthz",
            "dashboard": "http://127.0.0.1:3000/healthz",
            "finitechat-server": "http://127.0.0.1:8788/readyz",
            "hosted-web-device": "http://127.0.0.1:38918/healthz",
            "finite-brain": "http://127.0.0.1:3015/health",
            "finitesitesd": "http://127.0.0.1:8787/api/v1/healthz",
            "node-exporter": "http://127.0.0.1:9100/metrics",
        },
    },
    "runner": {
        "service": "finite-saas-runner.service",
        "timer": "finite-saas-runner.timer",
        "shared_environment_file": "/etc/finite/runner-shared.env",
        "environment_file": "/etc/finite/runner.env",
        "namespace": "finite",
        "drain_variable": "FC_RUNNER_DRAIN",
        "artifact_variable": "FC_RUNNER_RUNTIME_ARTIFACT_ID",
        "base_url_variable": "FC_RUNNER_FINITE_PRIVATE_BASE_URL",
        "model_variable": "FC_RUNNER_FINITE_PRIVATE_MODEL",
        "expected_base_url": "https://finite-private.finite.containers.tinfoil.dev/v1",
        "expected_model": "glm-5-3-flash",
        "mixed_version_models": ["glm-5-2", "deepseek-v4-flash-0731", "glm-5.3-flash"],
    },
    "recovery": {
        "snapshot_root": "/data/recovery-snapshots/hosted-web-chat",
        "latest_name": "latest",
        "manifest_name": "manifest.sha256",
        "borg_job_unit": "borgbackup-job-finite-hosted-web-chat-offsite.service",
        "borg_health_unit": "finite-hosted-web-chat-offsite-health.service",
        "borg_success_stamp": "/var/lib/finitecomputer/backups/hosted-web-chat-last-success",
        # The coordinated snapshot is deploy/manual-triggered because it fences
        # live writers; its deployed health unit allows seven days. Borg ships
        # that recovery point daily and has the separate 50-hour freshness gate.
        "snapshot_maximum_age_seconds": 604_800,
        "borg_maximum_age_seconds": 180_000,
        # One replicator instance per enrolled database (finite-litestream.nix);
        # all must be active for the recovery boundary to stay green.
        "litestream_service_units": [
            "finite-litestream-finite-chat-server.service",
            "finite-litestream-finite-brain.service",
        ],
        "litestream_health_unit": "finite-litestream-health.service",
        "litestream_success_stamp": "/var/lib/finite-litestream/health-last-success",
        # The health timer refreshes the stamp every 5 minutes when replication
        # is verified end-to-end; 30 minutes of silence is red.
        "litestream_maximum_age_seconds": 1_800,
    },
    "rollout": {
        "state_root": ".local-state/runtime-rollouts",
        "plan_name": "plan.json",
        "events_name": "events.jsonl",
    },
    # Chat-plane incident probes (2026-08-27..29 outage: a server-side
    # projection freeze plus an agent-side quarantine livelock both sat green
    # under every existing section). All three probes are host-local and
    # read-only; each names its evidence and degrades to unknown, never to a
    # guessed green or red.
    "chat_plane": {
        # App host (ADR 0007): finitechat-server SQLite via DynamicUser
        # StateDirectory (modules/finitechat-server.nix, litestream config in
        # hosts/finite-lat-2/default.nix).
        "server_database": "/var/lib/private/finite-chat/data/server.sqlite3",
        "hosted_device_root": "/var/lib/private/finitechat-hosted-device",
        "hosted_device_store_glob": "users/*/chat/client.sqlite3",
        # finitechat-server writes a durable-state snapshot every
        # SNAPSHOT_INTERVAL_OPS accepted operations
        # (finitechat/crates/finitechat-server/src/lib.rs). The freeze detector
        # goes red past two un-snapshotted intervals: the Aug 27-29 freeze had
        # accumulated ~8,000 un-snapshotted ops before anyone noticed.
        "snapshot_interval_ops": 4_096,
        "snapshot_gap_red_intervals": 2,
        # Quarantine-livelock detector: healthy sync clients sit far below one
        # POST /sync/group per second per egress address (the resident bridge
        # syncs every ~10s); the Aug 29 livelock ran 13-25/s per client. The
        # red line is per client IP so a full runner host of healthy agents
        # (~0.1/s each through one egress IP) stays far below it.
        "sync_path": "/sync/group",
        "edge_unit": "caddy.service",
        "access_log_glob": "/var/log/caddy/access*.log",
        "sync_rate_window_seconds": 5,
        "sync_rate_red_per_second": 10.0,
        # Runner egress addresses (infra/nixos/hosts/*/default.nix) used for
        # best-effort attribution of sync-rate clients to fleet hosts.
        "egress_ips": {
            "64.34.82.77": "finite-lat-1",
            "64.34.80.19": "finite-lat-2",
            "207.188.7.157": "finite-lat-3",
            "152.236.34.15": "finite-lat-4",
        },
        # Agent cursor-vs-head blindness detector: bounded sample (a full
        # sweep of every runtime store is too slow for a status command).
        # `MAX(seq) FROM client_app_events` is a durable-cursor PROXY: only
        # decrypted app events land there, so membership/key-package ops keep
        # a modest standing lag; the red line is a large lag combined with a
        # store that has not been written for the staleness window (the
        # freeze signature: the head advances while the store stands still).
        "cursor_sample_per_source": 3,
        "cursor_rooms_per_store": 5,
        "cursor_lag_red_ops": 256,
        "cursor_store_stale_seconds": 600,
        "runner_work_root_variable": "FC_RUNNER_WORK_ROOT",
        "runner_default_work_root": "/data/finite-saas-runner",
        "agent_store_suffix": "agent/client.sqlite3",
    },
    "hosts": {
        "finite-lat-1": {
            "mounts": ["/", "/data"],
            "storage": "single-disk",
            "disks": [
                "/dev/disk/by-id/nvme-Micron_7450_MTFDKBA480TFR_24474C59E53F",
                "/dev/disk/by-id/nvme-SAMSUNG_MZQL21T9HCJR-00A07_S64GNC0Y510146",
            ],
            # Historical combined host: it ran the app stack AND the
            # existing-Agent Runner lane before the ADR 0007 cutover.
            "roles": ["app", "runner"],
            "recovery": True,
        },
        "finite-lat-3": {
            "mounts": ["/", "/data", "/boot-a", "/boot-b"],
            "storage": "raid",
            "storage_health_unit": "finite-storage-health.service",
            "roles": ["runner"],
            "recovery": False,
        },
        # Emergency replacement app-plane host (ADR 0007, 2026-08-28): the
        # Recovery Authority role moves here with lat1's stack. It runs no
        # Agent Runner, so runner-role gates must never score it.
        "finite-lat-2": {
            "mounts": ["/", "/data", "/boot-a", "/boot-b"],
            "storage": "raid",
            "storage_health_unit": "finite-storage-health.service",
            "roles": ["app"],
            "recovery": True,
        },
        "finite-lat-4": {
            "mounts": ["/", "/data", "/boot-a", "/boot-b"],
            "storage": "raid",
            "storage_health_unit": "finite-storage-health.service",
            "roles": ["runner"],
            "recovery": False,
        },
    },
    "thresholds": {
        "filesystem_red_percent": 90.0,
    },
}

ARTIFACTS_QUERY = """select id, reference, version_label, source_git_sha, finitec_version,
       promoted_at, retired_at
  from runtime_artifacts order by created_at desc;"""

# This query is deliberately byte-for-byte equivalent to the operator-verified
# query in the task. Do not replace it with inferred counts from richer joins.
DISTRIBUTION_QUERY = """select ar.source_host_id, ra.version_label, count(*)
  from agent_runtimes ar
  join runtime_artifacts ra on ra.id = ar.runtime_artifact_id
  group by 1,2 order by 1,2;"""

RUNTIME_DETAILS_QUERY = """select ar.source_host_id,
       ar.runtime_artifact_id,
       ar.id as agent_runtime_id,
       ar.project_id,
       ar.source_machine_id,
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
       control.kind as control_kind,
       control.status as control_status,
       ar.host_facts ->> 'runtime_status' as runtime_status,
       core_rfc3339(ar.health_reported_at) as health_reported_at,
       ar.health_ready,
       ar.health_reason,
       ar.health_report_interval_seconds
  from agent_runtimes ar
  left join runtime_artifacts ra on ra.id = ar.runtime_artifact_id
  left join projects p on p.id = ar.project_id
  left join lateral (
    select request.kind, request.status
      from runtime_control_requests request
     where request.agent_runtime_id = ar.id
       and request.status in ('requested', 'launching', 'compute_up', 'ready')
     order by request.created_at, request.id
     limit 1
  ) control on true
  order by ar.source_host_id, ar.id;"""

# The runner's read-only lifecycle probe. App health (endpoints, versions)
# and lifecycle-control health (can the platform stop/replace this guest) are
# separate facts; this binary answers the second and is consumed per Agent.
LIFECYCLE_PROBE_BINARY = "/run/current-system/sw/bin/finite-saas-runner"
LIFECYCLE_PROBE_SCHEMA = "finite.lifecycle-probe.v1"
LIFECYCLE_VERDICTS = ("operable", "degraded", "inoperable", "unknown")

# Runner-ferried standing readiness (2026-08 audit synthesis, H1 slice 3).
# These constants mirror Core's project_runtime_health exactly: a runtime is
# "ready" only while a fresh report says ready; a fresh ready=false report is
# "not_ready" with its reason; no report or a report older than 3x the poll
# cadence is the named "unknown" state, so a runtime that died overnight never
# displays a frozen last-known ready. Do not drift from the Core projection.
HEALTH_DEFAULT_INTERVAL_SECONDS = 60
HEALTH_MIN_INTERVAL_SECONDS = 5
HEALTH_MAX_INTERVAL_SECONDS = 3600
HEALTH_STALE_MULTIPLIER = 3
HEALTH_STATES = ("ready", "not_ready", "unknown")

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
            [
                "id",
                "reference",
                "version_label",
                "source_git_sha",
                "finitec_version",
                "promoted_at",
                "retired_at",
            ],
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
                "runtime_artifact_id",
                "agent_runtime_id",
                "project_id",
                "source_machine_id",
                "agent_name",
                "version_label",
                "link_state",
                "control_kind",
                "control_status",
                "runtime_status",
                "health_reported_at",
                "health_ready",
                "health_reason",
                "health_report_interval_seconds",
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

    raw: dict[str, Any] = {
        "hostname": hostname,
        "errors": [],
        "units": {},
        # Role-scoped scoring: an app host is never scored against runner
        # invariants and a Runner host never against app-plane services
        # (ADR 0007 split the roles across hosts). Legacy inputs without a
        # recorded role keep the historical combined scoring.
        "roles": profile.get("roles", ["app", "runner"]),
    }
    units: list[str] = []
    if "app" in raw["roles"]:
        units.extend(CONTRACT["healthcheck"]["services"])
        units.append(CONTRACT["healthcheck"]["unit"])
    if "runner" in raw["roles"]:
        units.extend([CONTRACT["runner"]["service"], CONTRACT["runner"]["timer"]])
    if profile.get("storage_health_unit"):
        units.append(profile["storage_health_unit"])
    for unit in dict.fromkeys(units):
        try:
            raw["units"][unit] = systemd_properties(unit)
        except CollectionError as error:
            raw["units"][unit] = {"error": str(error)}

    healthcheck = raw["units"].get(CONTRACT["healthcheck"]["unit"], {})
    if "app" in raw["roles"]:
        try:
            raw["probes"] = collect_healthcheck_journal(healthcheck)
        except CollectionError as error:
            raw["probes"] = {}
            raw["errors"].append(str(error))
    else:
        raw["probes"] = {}

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
        # lat1 disks retain stale MD metadata from a failed RAID attempt;
        # leftover arrays in /proc/mdstat are not a single-disk health signal.
        raw["storage"]["disks"] = [
            {"path": path, "present": Path(path).exists()} for path in profile["disks"]
        ]

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
    raw["runner_environment"] = {}
    # Which environment files were actually readable. Projections need this to
    # tell "the environment was read and this key is genuinely absent" apart
    # from "we never got the environment" (see the FC_RUNNER_RUNTIME_ARTIFACT_ID
    # projection below).
    raw["runner_environment_files_read"] = []
    if "runner" in raw["roles"]:
        runner_keys = {
            runner["drain_variable"],
            runner["artifact_variable"],
            runner["base_url_variable"],
            runner["model_variable"],
        }
        # Match systemd EnvironmentFile ordering: the operator file loads last and
        # may deliberately override the shared role.
        for path_key, result_key in (
            ("shared_environment_file", "runner_shared_environment"),
            ("environment_file", "runner_operator_environment"),
        ):
            try:
                values = read_environment_values(Path(runner[path_key]), runner_keys)
                raw[result_key] = values
                raw["runner_environment"].update(values)
                raw["runner_environment_files_read"].append(result_key)
            except CollectionError as error:
                raw[result_key] = {}
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
        raise CollectionError(
            f"latest snapshot escapes its recovery root: {error}"
        ) from error
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

    litestream_stamp = Path(recovery["litestream_success_stamp"])
    try:
        raw["litestream_last_success_epoch"] = int(
            litestream_stamp.read_text(encoding="utf-8").strip()
        )
    except (OSError, ValueError) as error:
        raw["litestream_last_success_error"] = (
            f"cannot read {litestream_stamp}: {error}"
        )
    litestream_units: dict[str, Any] = {}
    for unit in recovery["litestream_service_units"]:
        try:
            litestream_units[unit] = systemd_properties(unit)
        except CollectionError as error:
            litestream_units[unit] = {"error": str(error)}
    raw["litestream_service_units"] = litestream_units
    health_unit = recovery["litestream_health_unit"]
    try:
        raw["litestream_health_unit"] = systemd_properties(health_unit)
    except CollectionError as error:
        raw["litestream_health_unit"] = {"error": str(error)}
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


def probe_agent_entry(
    verdict: str, reason: str | None, detail: str | None = None
) -> dict[str, Any]:
    entry: dict[str, Any] = {"verdict": verdict, "reason": reason}
    if detail is not None:
        entry["detail"] = detail
    return entry


def collect_lifecycle_probe(
    runtimes: list[dict[str, Any]], hostname: str
) -> dict[str, Any]:
    """Probe lifecycle-control health for this host's active Agents.

    Read-only: the probe binary gates upgrade eligibility and never touches a
    serving Agent. Every collection failure is recorded as an `unknown`
    verdict on that Agent — unknown is a displayed state, never hidden.
    """
    binary = os.environ.get("FINITE_STATUS_LIFECYCLE_PROBE_BIN", LIFECYCLE_PROBE_BINARY)
    candidates = [
        row
        for row in runtimes
        if row.get("source_host_id") == hostname and row.get("link_state") == "active"
    ]
    raw: dict[str, Any] = {
        "binary": binary,
        "schema": LIFECYCLE_PROBE_SCHEMA,
        "available": False,
        "agents": {},
        "errors": [],
    }
    if not candidates:
        return raw
    if not Path(binary).exists():
        raw["errors"].append(f"lifecycle probe binary {binary} is not installed")
        return raw
    raw["available"] = True
    environment = dict(os.environ)
    try:
        # Forward every probe-relevant runner env key so finite-status probes
        # the same roots the rollout wrapper probes; site-specific overrides
        # must not split the two views.
        environment.update(
            read_environment_values(
                Path(CONTRACT["runner"]["environment_file"]),
                {
                    "FC_RUNNER_SOURCE_HOST_ID",
                    "FC_RUNNER_WORK_ROOT",
                    "FC_RUNNER_KATA_NAMESPACE",
                    "FC_RUNNER_KATA_NERDCTL_BIN",
                    "FC_RUNNER_KATA_CTR_BIN",
                    "FC_RUNNER_KATA_SANDBOX_ROOT",
                    "FC_RUNNER_KATA_NETNS_ROOT",
                    "FC_RUNNER_KATA_PROC_ROOT",
                },
            )
        )
    except CollectionError as error:
        raw["errors"].append(str(error))
    for row in candidates:
        runtime_id = row["agent_runtime_id"]
        command = [
            binary,
            "lifecycle-probe",
            "--project-id",
            row["project_id"],
            "--agent-runtime-id",
            runtime_id,
            "--source-machine-id",
            row.get("source_machine_id") or "",
        ]
        try:
            result = run_read_only(command, environment=environment)
        except CollectionError as error:
            raw["agents"][runtime_id] = probe_agent_entry(
                "unknown", "probe_unavailable", str(error)
            )
            continue
        if result.returncode != 0:
            detail = result.stderr.strip() or f"exit {result.returncode}"
            raw["agents"][runtime_id] = probe_agent_entry(
                "unknown", "probe_unavailable", detail
            )
            continue
        try:
            report = json.loads(result.stdout)
        except json.JSONDecodeError as error:
            raw["agents"][runtime_id] = probe_agent_entry(
                "unknown", "probe_invalid", str(error)
            )
            continue
        if (
            report.get("schema") != LIFECYCLE_PROBE_SCHEMA
            or report.get("verdict") not in LIFECYCLE_VERDICTS
        ):
            raw["agents"][runtime_id] = probe_agent_entry(
                "unknown", "probe_invalid", "unrecognized probe report shape"
            )
            continue
        raw["agents"][runtime_id] = probe_agent_entry(
            report["verdict"], report.get("reason")
        )
    return raw


def sanitize_sandbox_name(value: str) -> str:
    """Mirror finite-saas-runner's sandbox-name sanitization (lib.rs)."""
    result = "".join(
        character if re.match(r"[A-Za-z0-9_-]", character) else "-"
        for character in value
    )
    return result[:63].strip("-")


# Scratch-copy ceiling for the SQLite probes: a status command must stay
# fast; a store beyond this size refuses (unknown) instead of stalling.
SCRATCH_COPY_MAX_BYTES = 16 * (1 << 30)


@contextlib.contextmanager
def scratch_copy_sqlite(database: Path) -> Iterator[Path]:
    """Yield a scratch copy of a live SQLite database, then remove it.

    Follows the repo's snapshot-sqlite discipline: the live file (and its
    WAL/SHM sidecars) is never opened directly, only an uncoordinated copy in
    a private scratch directory is queried read-only. A torn copy from a
    concurrent writer surfaces as a query error and degrades to unknown.
    """
    if not database.is_file():
        raise CollectionError(f"{database} is not a regular file")
    scratch = Path(tempfile.mkdtemp(prefix="finite-status-sqlite."))
    try:
        # Sidecar copy order matches scripts/snapshot-sqlite (db, wal, shm).
        # The size ceiling keeps a pathological store from stalling the
        # status command; refusing degrades the probe to unknown.
        total_bytes = 0
        for suffix in ("", "-wal", "-shm"):
            source = Path(f"{database}{suffix}")
            if not source.exists():
                continue
            if not source.is_file() or source.is_symlink():
                raise CollectionError(f"{source} is not a regular, non-symlink file")
            total_bytes += source.stat().st_size
        if total_bytes > SCRATCH_COPY_MAX_BYTES:
            raise CollectionError(
                f"{database} is {total_bytes / (1 << 30):.1f} GiB; over the"
                f" {SCRATCH_COPY_MAX_BYTES / (1 << 30):.0f} GiB status-probe copy ceiling"
            )
        for suffix in ("", "-wal", "-shm"):
            source = Path(f"{database}{suffix}")
            if not source.exists():
                continue
            copied = scratch / f"scratch.sqlite3{suffix}"
            shutil.copyfile(source, copied)
            copied.chmod(0o600)
        yield scratch / "scratch.sqlite3"
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


def sqlite_json_query(
    database: Path, sql: str, timeout: int = 15
) -> list[dict[str, Any]]:
    """Run one read-only SQLite query on a scratch copy; -json rows back."""
    result = run_read_only(
        [
            "sqlite3",
            "-safe",
            "-readonly",
            "-batch",
            "-init",
            "/dev/null",
            "-json",
            str(database),
            sql,
        ],
        timeout=timeout,
    )
    if result.returncode != 0:
        detail = result.stderr.strip().splitlines()
        raise CollectionError(
            f"read-only sqlite query failed: {detail[-1] if detail else result.returncode}"
        )
    payload = result.stdout.strip()
    if not payload:
        return []
    try:
        rows = json.loads(payload)
    except json.JSONDecodeError as error:
        raise CollectionError(
            f"sqlite query returned unparseable rows: {error}"
        ) from error
    if not isinstance(rows, list):
        raise CollectionError("sqlite query returned an unexpected row shape")
    return rows


def sqlite_int_query(database: Path, sql: str, timeout: int = 15) -> int:
    rows = sqlite_json_query(database, sql, timeout=timeout)
    if len(rows) != 1:
        raise CollectionError("sqlite scalar query did not return exactly one row")
    (row,) = rows
    if len(row) != 1:
        raise CollectionError("sqlite scalar query did not return exactly one column")
    try:
        return int(next(iter(row.values())))
    except (TypeError, ValueError) as error:
        raise CollectionError(f"sqlite scalar is not an integer: {error}") from error


def collect_chat_server_state(hostname: str) -> dict[str, Any]:
    """Snapshot-watermark evidence from the app host's chat server database.

    `http_delivery_ops` is the accepted-operation log and
    `http_state_snapshots_v2.last_op_seq` is the replay watermark: a server
    that keeps accepting ops while its watermark stands still is exactly the
    2026-08-27..29 projection freeze, so both numbers are recorded side by
    side. Room heads (`http_room_memberships.projection_json` -> $.last_seq)
    feed the cursor-vs-head probe.
    """
    contract = CONTRACT["chat_plane"]
    database = Path(contract["server_database"])
    raw: dict[str, Any] = {
        "database": str(database),
        "applicable": database.exists(),
        "errors": [],
    }
    if not raw["applicable"]:
        raw["reason"] = "chat server database not present on this host"
        return raw
    try:
        with scratch_copy_sqlite(database) as scratch:
            raw["ops_head"] = sqlite_int_query(
                scratch, "SELECT COALESCE(MAX(seq), 0) AS head FROM http_delivery_ops;"
            )
            raw["snapshot_watermark"] = sqlite_int_query(
                scratch,
                "SELECT COALESCE(last_op_seq, 0) AS watermark"
                " FROM http_state_snapshots_v2 WHERE id = 1;",
            )
            rows = sqlite_json_query(
                scratch,
                "SELECT room_id AS room_id,"
                " CAST(json_extract(projection_json, '$.last_seq') AS INTEGER)"
                " AS last_seq FROM http_room_memberships ORDER BY room_id;",
            )
            raw["room_heads"] = {
                row["room_id"]: int(row["last_seq"])
                for row in rows
                if row.get("last_seq") is not None
            }
    except CollectionError as error:
        raw["errors"].append(str(error))
    return raw


def parse_caddy_access_entry(line: str) -> dict[str, Any] | None:
    """Extract (ts, remote_ip, method, path) from one Caddy JSON log line."""
    try:
        entry = json.loads(line)
    except json.JSONDecodeError:
        return None
    request = entry.get("request")
    if not isinstance(request, dict):
        return None
    ts = entry.get("ts")
    if not isinstance(ts, (int, float)):
        return None
    return {
        "ts": float(ts),
        "remote_ip": request.get("remote_ip"),
        "method": request.get("method"),
        "path": str(request.get("uri") or "").split("?", 1)[0],
    }


def collect_sync_fetch_rate(now: datetime) -> dict[str, Any]:
    """Sample POST /sync/group rates per client over a bounded recent window.

    The quarantine livelock of 2026-08-29 showed as one or two egress
    addresses re-fetching the same group sync 13-25 times per second for
    hours. Evidence is host-local and read-only: the newest Caddy access-log
    file when one exists, else the Caddy unit journal (the deployed edge logs
    access entries to the default logger -> journald).
    """
    contract = CONTRACT["chat_plane"]
    window = int(contract["sync_rate_window_seconds"])
    since = now - timedelta(seconds=window)
    raw: dict[str, Any] = {
        "window_seconds": window,
        "since": isoformat(since),
        "available": False,
        "source": None,
        "clients": [],
        "total": 0,
        "errors": [],
    }

    def tally(entries: list[dict[str, Any]], source: str) -> None:
        counts: Counter[str] = Counter()
        for entry in entries:
            if entry["method"] != "POST" or entry["path"] != contract["sync_path"]:
                continue
            if entry["ts"] < since.timestamp():
                continue
            address = entry["remote_ip"] or "unknown"
            counts[address] += 1
        raw["available"] = True
        raw["source"] = source
        raw["total"] = sum(counts.values())
        raw["clients"] = [
            {
                "ip": address,
                "count": count,
                "rate_per_second": round(count / window, 2),
                "attributed_host": contract["egress_ips"].get(address),
            }
            for address, count in counts.most_common()
        ]

    # Source 1: operator-configured access-log files (newest wins).
    def log_mtime(name: str) -> float:
        try:
            return os.path.getmtime(name)
        except OSError:
            return 0.0

    log_files = sorted(glob.glob(contract["access_log_glob"]), key=log_mtime)
    for name in reversed(log_files):
        try:
            size = os.path.getsize(name)
            with open(name, "rb") as stream:
                if size > 262_144:
                    stream.seek(size - 262_144)
                    stream.readline()  # drop the partial first line
                chunk = stream.read().decode("utf-8", errors="replace")
        except OSError as error:
            raw["errors"].append(f"cannot read {name}: {error}")
            continue
        entries = [
            parsed
            for parsed in (
                parse_caddy_access_entry(line) for line in chunk.splitlines()
            )
            if parsed is not None
        ]
        if entries:
            tally(entries, f"file:{name}")
            return raw

    # Source 2: the Caddy unit journal over the same bounded window.
    try:
        result = run_read_only(
            [
                "journalctl",
                "--no-pager",
                "--output=json",
                f"--unit={contract['edge_unit']}",
                f"--since={isoformat(since)}",
                "--lines=20000",
            ],
            timeout=20,
        )
    except CollectionError as error:
        raw["errors"].append(str(error))
        return raw
    if result.returncode != 0:
        raw["errors"].append(
            f"cannot read {contract['edge_unit']} journal: "
            f"{result.stderr.strip() or result.returncode}"
        )
        return raw
    entries = []
    for line in result.stdout.splitlines():
        try:
            journal_entry = json.loads(line)
        except json.JSONDecodeError:
            continue
        message = journal_entry.get("MESSAGE")
        if not isinstance(message, str):
            continue
        parsed = parse_caddy_access_entry(message)
        if parsed is not None:
            entries.append(parsed)
    if not entries:
        raw["errors"].append(
            "no access-log evidence in the "
            f"{contract['edge_unit']} journal; the chat vhost needs a `log`"
            " directive (infra/nixos/modules/caddy.nix) for this probe"
        )
        return raw
    tally(entries, f"journal:{contract['edge_unit']}")
    return raw


def store_freshness(database: Path) -> float | None:
    """Seconds since the newest write to the store (db or WAL sidecar)."""
    newest: float | None = None
    for suffix in ("", "-wal"):
        try:
            mtime = os.path.getmtime(f"{database}{suffix}")
        except OSError:
            continue
        newest = mtime if newest is None else max(newest, mtime)
    return None if newest is None else max(0.0, time.time() - newest)


def collect_local_client_stores(
    runtimes: list[dict[str, Any]], hostname: str
) -> dict[str, Any]:
    """Bounded sample of this host's chat client stores (the cursor side).

    Two local sources, both read-only: the hosted-device per-user stores on
    the app host, and the Kata /data volumes of this host's active runtimes
    on a Runner host (`<work_root>/kata/<sandbox>/agent/client.sqlite3` is
    bind-mounted as /data/agent, where FINITECHAT_HOME points). The sample is
    bounded per source; a full sweep is too slow for a status command.
    """
    contract = CONTRACT["chat_plane"]
    raw: dict[str, Any] = {"stores": [], "skipped": [], "errors": []}

    hosted_root = Path(contract["hosted_device_root"])
    hosted_stores = sorted(
        hosted_root.glob(contract["hosted_device_store_glob"]),
        key=lambda path: store_freshness(path) or 0,
    )
    for store in hosted_stores[: contract["cursor_sample_per_source"]]:
        raw["stores"].append(
            {
                "kind": "hosted-device",
                "label": store.parent.parent.name,
                "path": str(store),
            }
        )

    work_root = runner_work_root()
    if work_root is None:
        raw["runner_work_root"] = None
    else:
        raw["runner_work_root"] = str(work_root)
        candidates = sorted(
            (
                row
                for row in runtimes
                if row.get("source_host_id") == hostname
                and row.get("link_state") == "active"
                and row.get("source_machine_id")
            ),
            key=lambda row: row["agent_runtime_id"],
        )
        for row in candidates[: contract["cursor_sample_per_source"]]:
            machine = sanitize_sandbox_name(row["source_machine_id"])
            store = (
                work_root / "kata" / machine.lower() / contract["agent_store_suffix"]
            )
            if store.is_file():
                raw["stores"].append(
                    {
                        "kind": "agent",
                        "label": row.get("agent_name") or row["agent_runtime_id"],
                        "agent_runtime_id": row["agent_runtime_id"],
                        "path": str(store),
                    }
                )
            else:
                raw["skipped"].append(
                    {
                        "agent_runtime_id": row["agent_runtime_id"],
                        "reason": f"no client store at {store}",
                    }
                )

    for store in raw["stores"]:
        database = Path(store["path"])
        store["age_seconds"] = store_freshness(database)
        try:
            with scratch_copy_sqlite(database) as scratch:
                rows = sqlite_json_query(
                    scratch,
                    "SELECT room_id AS room_id, MAX(seq) AS max_seq"
                    " FROM client_app_events GROUP BY room_id"
                    " ORDER BY room_id"
                    f" LIMIT {int(contract['cursor_rooms_per_store'])};",
                )
                store["rooms"] = {row["room_id"]: int(row["max_seq"]) for row in rows}
        except CollectionError as error:
            store["error"] = str(error)
    return raw


def runner_work_root() -> Path | None:
    """Resolve this host's Runner work root without trusting a hardcoded box.

    The runner declares FC_RUNNER_WORK_ROOT in its systemd unit environment
    (modules/kata-runner-host.nix); the environment files may override. The
    contract default is the last resort and is pinned to the Nix host configs
    by the contract check.
    """
    contract = CONTRACT["chat_plane"]
    variable = contract["runner_work_root_variable"]
    candidates: list[str] = []
    try:
        properties = systemd_properties_single(
            CONTRACT["runner"]["service"], "Environment"
        )
        for assignment in properties:
            for token in shlex.split(assignment):
                if token.startswith(f"{variable}="):
                    candidates.append(token.split("=", 1)[1])
    except CollectionError:
        pass
    for path_key in ("shared_environment_file", "environment_file"):
        try:
            values = read_environment_values(
                Path(CONTRACT["runner"][path_key]), {variable}
            )
        except CollectionError:
            continue
        if values.get(variable):
            candidates.append(values[variable])
    candidates.append(contract["runner_default_work_root"])
    for candidate in candidates:
        if candidate:
            return Path(candidate)
    return None


def systemd_properties_single(unit: str, name: str) -> list[str]:
    result = run_read_only(
        ["systemctl", "show", "--no-pager", f"--property={name}", unit]
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or f"exit {result.returncode}"
        raise CollectionError(f"cannot observe {unit}: {detail}")
    return [
        line.split("=", 1)[1]
        for line in result.stdout.splitlines()
        if line.startswith(f"{name}=")
    ]


def host_roles(hostname: str) -> list[str] | None:
    """Contract roles for this host; None when the host has no profile.

    A host without a profile is not a fleet host (e.g. a dev machine); its
    probes keep the historical collect-everything behavior and degrade to
    unknown with reasons rather than not-applicable.
    """
    profile = CONTRACT["hosts"].get(hostname)
    if profile is None:
        return None
    return list(profile.get("roles", ["app", "runner"]))


def collect_chat_plane(
    hostname: str, runtimes: list[dict[str, Any]], now: datetime
) -> dict[str, Any]:
    """Collect every chat-plane incident probe for this host.

    Probes are role-gated like host health (ADR 0007): the chat edge and the
    server database live on app-role hosts, so a runner-role host records the
    sync-rate probe as not-applicable instead of failing to read a Caddy
    journal it does not run.
    """
    roles = host_roles(hostname)
    if roles is not None and "app" not in roles:
        sync_rate: dict[str, Any] = {
            "applicable": False,
            "reason": (
                "chat edge is not on this runner-role host "
                f"(roles: {', '.join(roles)})"
            ),
        }
    else:
        sync_rate = collect_sync_fetch_rate(now)
    return {
        "server": collect_chat_server_state(hostname),
        "sync_rate": sync_rate,
        "client_stores": collect_local_client_stores(runtimes, hostname),
    }


def expand_fixture(raw: dict[str, Any]) -> dict[str, Any]:
    groups = raw.get("core", {}).pop("runtime_groups", [])
    for group in groups:
        count = int(group["count"])
        for index in range(1, count + 1):
            row = {
                "source_host_id": group["source_host_id"],
                "agent_runtime_id": f"{group['id_prefix']}-{index:02d}",
                "project_id": f"{group['project_prefix']}-{index:02d}",
                "source_machine_id": f"machine-{group['id_prefix']}-{index:02d}",
                "agent_name": f"{group['name_prefix']} {index:02d}",
                "version_label": group["version_label"],
                "link_state": group["link_state"],
            }
            # Optional canonical-control projection, mirroring the lateral
            # active-control columns of RUNTIME_DETAILS_QUERY.
            if group.get("control_status"):
                row["control_kind"] = group["control_kind"]
                row["control_status"] = group["control_status"]
            # Optional standing-health projection inputs, mirroring the health
            # columns of RUNTIME_DETAILS_QUERY. A group carrying any health
            # field defaults runtime_status to online (production rows always
            # carry host_facts.runtime_status); override it explicitly to model
            # an intentionally offline runtime.
            health_keys = (
                "health_reported_at",
                "health_ready",
                "health_reason",
                "health_report_interval_seconds",
            )
            if group.get("runtime_status"):
                row["runtime_status"] = group["runtime_status"]
            if any(group.get(key) is not None for key in health_keys):
                row.setdefault("runtime_status", "online")
                for key in health_keys:
                    if group.get(key) is not None:
                        row[key] = group[key]
            raw["core"].setdefault("runtimes", []).append(row)
    return raw


def load_fixture(path: Path) -> dict[str, Any]:
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise CollectionError(f"cannot load fixture {path}: {error}") from error
    return expand_fixture(raw)


def combine_status(statuses: list[str]) -> str:
    if "red" in statuses:
        return "red"
    if "unknown" in statuses:
        return "unknown"
    return "green"


def target_runtime_artifact(artifacts: list[dict[str, Any]]) -> dict[str, Any] | None:
    return next(
        (
            artifact
            for artifact in artifacts
            if artifact.get("promoted_at") and not artifact.get("retired_at")
        ),
        None,
    )


def health_ready_value(raw: Any) -> bool | None:
    """The CSV path yields t/f strings; fixtures yield JSON booleans."""
    if raw in (True, "t", "true"):
        return True
    if raw in (False, "f", "false"):
        return False
    return None


def project_runtime_health(row: dict[str, Any], now: datetime) -> dict[str, Any]:
    """Project one runtime's standing readiness from the latest stored report.

    Mirrors Core's `project_runtime_health` (finite-saas-core): reports only
    speak for runtimes Core considers online, freshness is measured from the
    report's Core-recorded time, and the deadline is 3x the reporter's poll
    cadence. Keep the named states (`ready`/`not_ready`/`unknown`) aligned;
    the fixture tests pin both surfaces agreeing.
    """
    reported_at = parse_time(row.get("health_reported_at"))
    ready = health_ready_value(row.get("health_ready"))
    age_seconds = (
        int((now - reported_at).total_seconds()) if reported_at is not None else None
    )
    status = "unknown"
    if (
        row.get("runtime_status") == "online"
        and reported_at is not None
        and ready is not None
    ):
        try:
            interval = int(
                row.get("health_report_interval_seconds")
                or HEALTH_DEFAULT_INTERVAL_SECONDS
            )
        except (TypeError, ValueError):
            interval = HEALTH_DEFAULT_INTERVAL_SECONDS
        interval = min(
            max(interval, HEALTH_MIN_INTERVAL_SECONDS), HEALTH_MAX_INTERVAL_SECONDS
        )
        if (
            age_seconds is not None
            and age_seconds <= interval * HEALTH_STALE_MULTIPLIER
        ):
            status = "ready" if ready else "not_ready"
    return {
        "status": status,
        "reason": row.get("health_reason") or None,
        "reported_at": row.get("health_reported_at") or None,
        "age_seconds": age_seconds,
    }


def build_fleet(
    core: dict[str, Any] | None,
    now: datetime,
    error: str | None = None,
    probe: dict[str, Any] | None = None,
) -> dict[str, Any]:
    if core is None:
        return {"status": "unknown", "error": error or "Core evidence unavailable"}
    artifacts = core.get("artifacts", [])
    target = target_runtime_artifact(artifacts)
    if target is None:
        return {
            "status": "unknown",
            "error": "no promoted, non-retired Runtime artifact",
        }

    target_version = target["version_label"]
    probe_agents = (probe or {}).get("agents", {})
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
            # Core-recorded summary status; the health projection below only
            # answers for `online` runtimes.
            "runtime_status": row.get("runtime_status"),
        }
        # The canonical runtime-control lifecycle state, projected straight
        # from Core's one-active request row; never re-derived locally.
        if row.get("control_status"):
            entry["control"] = {
                "kind": row["control_kind"],
                "status": row["control_status"],
            }
        # Runner-ferried standing readiness: the named ready/not_ready/unknown
        # projection of the latest health report, mirrored from Core.
        entry["health"] = project_runtime_health(row, now)
        lifecycle = probe_agents.get(row["agent_runtime_id"])
        if lifecycle is not None:
            # App health (version above) and lifecycle-control health are
            # separate facts; both are displayed per Agent.
            entry["lifecycle"] = lifecycle
        host.setdefault(row.get("link_state", "unlinked"), host["unlinked"]).append(
            entry
        )

    host_reports: list[dict[str, Any]] = []
    section_statuses: list[str] = []
    for hostname in sorted(hosts):
        groups = hosts[hostname]
        active = groups["active"]
        stragglers = [row for row in active if row["version_label"] != target_version]
        on_target = len(active) - len(stragglers)
        # Standing readiness rolls up only over runtimes expected to report
        # (Core-recorded online, active link): an intentionally offline agent
        # is displayed with its projected state but never counted against the
        # host. A fresh not_ready report turns the host red; stale or missing
        # reports read unknown.
        health_tracked = [
            row for row in active if row.get("runtime_status") == "online"
        ]
        health_ready = [
            row for row in health_tracked if row["health"]["status"] == "ready"
        ]
        health_not_ready = [
            row for row in health_tracked if row["health"]["status"] == "not_ready"
        ]
        health_unknown = [
            row for row in health_tracked if row["health"]["status"] == "unknown"
        ]
        status = (
            "red"
            if stragglers or health_not_ready
            else ("unknown" if groups["unlinked"] or health_unknown else "green")
        )
        section_statuses.append(status)
        lifecycle_probed = [row for row in active if "lifecycle" in row]
        lifecycle_attention = [
            row for row in lifecycle_probed if row["lifecycle"]["verdict"] != "operable"
        ]
        control_active = [row for row in active if "control" in row]
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
                "lifecycle_probed_count": len(lifecycle_probed),
                "lifecycle_attention": lifecycle_attention,
                "control_active": control_active,
                "health_ready_count": len(health_ready),
                "health_tracked_count": len(health_tracked),
                "health_not_ready": health_not_ready,
                "health_unknown": health_unknown,
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
    report = {
        "status": combine_status(section_statuses),
        "evidence": "Core-recorded artifact/link state; not verified live compute",
        "status_basis": "active-link artifact convergence only",
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
    if probe is not None:
        report["lifecycle_probe"] = {
            "available": bool(probe.get("available")),
            "schema": probe.get("schema"),
            "note": "lifecycle verdicts gate upgrade eligibility only; they never affect serving Agents",
        }
    return report


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


def build_host_health(
    raw: dict[str, Any] | None, target_id: str | None
) -> dict[str, Any]:
    if raw is None or raw.get("error"):
        return {
            "status": "unknown",
            "hostname": None if raw is None else raw.get("hostname"),
            "error": "host evidence unavailable" if raw is None else raw["error"],
        }
    statuses: list[str] = []
    roles = raw.get("roles") or ["app", "runner"]
    units = []
    if "app" in roles:
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
    if "app" in roles:
        health_status = unit_status(health_props, active_required=False)
        statuses.append(health_status)
    else:
        health_status = "green"
        health_props = {"not_applicable": True}
    probes = []
    if "app" in roles:
        for name, endpoint in CONTRACT["healthcheck"]["probes"].items():
            observed = raw.get("probes", {}).get(name)
            status = (
                "green"
                if observed == "OK"
                else ("red" if observed == "FAIL" else "unknown")
            )
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
        elif not all(disk.get("present") for disk in disks):
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
    if "runner" not in roles:
        # ADR 0007: the app host runs no Agent Runner. Runner invariants are
        # displayed as not applicable instead of a wall of not-found unknowns.
        runner = {
            "applicable": False,
            "timer_unit": runner_contract["timer"],
        }
    else:
        timer_props = raw.get("units", {}).get(runner_contract["timer"], {})
        timer_status = unit_status(timer_props, active_required=True)
        runner_env = raw.get("runner_environment", {})
        drain = runner_env.get(runner_contract["drain_variable"])
        if drain is None:
            drain_status = "unknown"
        else:
            drain_status = "green" if drain.lower() == "false" else "red"
        pin = runner_env.get(runner_contract["artifact_variable"])
        # Status/state pair, mirroring finite_private_model_status/_state below.
        # An absent FC_RUNNER_RUNTIME_ARTIFACT_ID is a halt-new-agents condition
        # (the Runner fails closed without a pin), so once the environment itself
        # was readable it must render red/"absent" — never the same plain unknown
        # an unprobeable host gets. Legacy inputs predating
        # runner_environment_files_read keep the conservative unknown.
        if not pin:
            if raw.get("runner_environment_files_read"):
                pin_status = "red"
                pin_state = "absent"
            else:
                pin_status = "unknown"
                pin_state = "unresolved"
        elif not target_id:
            pin_status = "unknown"
            pin_state = "unresolved"
        else:
            pin_status = "green" if pin == target_id else "red"
            pin_state = "matched" if pin == target_id else "mismatched"
        finite_private_base_url = runner_env.get(runner_contract["base_url_variable"])
        if finite_private_base_url is None:
            finite_private_base_url_status = "unknown"
        else:
            finite_private_base_url_status = (
                "green"
                if finite_private_base_url == runner_contract["expected_base_url"]
                else "red"
            )
        finite_private_model = runner_env.get(runner_contract["model_variable"])
        shared_model = raw.get("runner_shared_environment", {}).get(
            runner_contract["model_variable"]
        )
        operator_model = raw.get("runner_operator_environment", {}).get(
            runner_contract["model_variable"]
        )
        if finite_private_model is None:
            finite_private_model_status = "unknown"
            finite_private_model_state = "unresolved"
        elif finite_private_model == runner_contract["expected_model"]:
            finite_private_model_status = "green"
            finite_private_model_state = "canonical"
        elif finite_private_model in runner_contract["mixed_version_models"]:
            if shared_model is None:
                finite_private_model_status = "unknown"
                finite_private_model_state = "unresolved-shared-role"
            elif (
                shared_model == runner_contract["expected_model"]
                and operator_model == finite_private_model
            ):
                finite_private_model_status = "red"
                finite_private_model_state = "stale-operator-override"
            elif shared_model in runner_contract["mixed_version_models"]:
                # Before the canonical Runner role is deployed, the historical
                # request name is a deliberately served mixed-version alias. It
                # must not block the independent scheduler promotion.
                finite_private_model_status = "green"
                finite_private_model_state = "mixed-version-compatibility"
            else:
                finite_private_model_status = "red"
                finite_private_model_state = "unexpected-shared-role"
        else:
            finite_private_model_status = "red"
            finite_private_model_state = "unexpected"
        statuses.extend(
            [
                timer_status,
                drain_status,
                pin_status,
                finite_private_base_url_status,
                finite_private_model_status,
            ]
        )
        runner = {
            "timer_unit": runner_contract["timer"],
            "timer_status": timer_status,
            "drain": drain,
            "drain_status": drain_status,
            "artifact_pin": pin,
            "target_artifact_id": target_id,
            "pin_status": pin_status,
            "pin_state": pin_state,
            "finite_private_base_url": finite_private_base_url,
            "finite_private_base_url_status": finite_private_base_url_status,
            "finite_private_model": finite_private_model,
            "finite_private_model_status": finite_private_model_status,
            "finite_private_model_state": finite_private_model_state,
            "finite_private_shared_model": shared_model,
            "finite_private_operator_model": operator_model,
        }
    return {
        "status": combine_status(statuses),
        "hostname": raw.get("hostname"),
        "roles": roles,
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
            and snapshot["age_seconds"]
            <= CONTRACT["recovery"]["snapshot_maximum_age_seconds"]
            else "red"
        )
    statuses.append(snapshot["status"])

    epoch = raw.get("borg_last_success_epoch")
    if isinstance(epoch, int):
        borg_age = int(now.timestamp()) - epoch
        stamp_status = (
            "green"
            if 0 <= borg_age <= CONTRACT["recovery"]["borg_maximum_age_seconds"]
            else "red"
        )
    else:
        borg_age = None
        stamp_status = "unknown"
    job_status = unit_status(raw.get("borg_job_unit", {}), active_required=False)
    health_status = unit_status(raw.get("borg_health_unit", {}), active_required=False)
    statuses.extend([stamp_status, job_status, health_status])

    litestream_epoch = raw.get("litestream_last_success_epoch")
    if isinstance(litestream_epoch, int):
        litestream_age = int(now.timestamp()) - litestream_epoch
        litestream_stamp_status = (
            "green"
            if 0
            <= litestream_age
            <= CONTRACT["recovery"]["litestream_maximum_age_seconds"]
            else "red"
        )
    else:
        litestream_age = None
        litestream_stamp_status = "unknown"
    litestream_service_units = raw.get("litestream_service_units")
    if not isinstance(litestream_service_units, dict) or not litestream_service_units:
        litestream_service_units = {
            unit: {"error": "unit evidence missing"}
            for unit in CONTRACT["recovery"]["litestream_service_units"]
        }
    litestream_service_statuses = {
        unit: unit_status(
            properties if isinstance(properties, dict) else {}, active_required=True
        )
        for unit, properties in litestream_service_units.items()
    }
    litestream_service_status = combine_status(
        list(litestream_service_statuses.values())
    )
    litestream_health_status = unit_status(
        raw.get("litestream_health_unit", {}), active_required=False
    )
    statuses.extend(
        [litestream_stamp_status, litestream_service_status, litestream_health_status]
    )
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
        "litestream": {
            "completion_mechanism": (
                "health timer verifies replicated LTX freshness and writes a success stamp"
            ),
            "service_units": litestream_service_statuses,
            "service_status": litestream_service_status,
            "health_unit": CONTRACT["recovery"]["litestream_health_unit"],
            "health_status": litestream_health_status,
            "last_success_epoch": litestream_epoch,
            "age_seconds": litestream_age,
            "stamp_status": litestream_stamp_status,
            "error": raw.get("litestream_last_success_error"),
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
    start_indexes = [
        index for index, event in enumerate(events) if event.get("event") == "start"
    ]
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
        if event.get("event") == "entry_postflight"
        and event.get("status") == "succeeded"
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


def build_chat_plane(raw: dict[str, Any] | None, now: datetime) -> dict[str, Any]:
    """Score the chat-plane incident probes collected for this host.

    Every probe names its evidence. Where the evidence source is not on this
    host (server database / chat edge on the app host, agent stores on a
    Runner host) the probe reads not-applicable rather than guessing; where
    evidence exists but cannot be compared, it reads unknown — unknown is a
    displayed state, never hidden, and never silently green.
    """
    if raw is None:
        return {"status": "unknown", "error": "chat-plane evidence unavailable"}
    contract = CONTRACT["chat_plane"]
    statuses: list[str] = []

    server = dict(raw.get("server") or {})
    watermark: dict[str, Any]
    if not server.get("applicable"):
        watermark = {
            "status": "green",
            "applicable": False,
            "state": "not-applicable",
            "reason": server.get("reason", "chat server database not on this host"),
        }
    elif server.get("errors"):
        watermark = {
            "status": "unknown",
            "applicable": True,
            "database": server.get("database"),
            "errors": server["errors"],
        }
    else:
        head = int(server.get("ops_head", 0))
        mark = int(server.get("snapshot_watermark", 0))
        gap = head - mark
        interval = int(contract["snapshot_interval_ops"])
        limit = interval * int(contract["snapshot_gap_red_intervals"])
        watermark = {
            "status": "red" if gap > limit else "green",
            "applicable": True,
            "database": server.get("database"),
            "ops_head": head,
            "snapshot_watermark": mark,
            "gap_ops": gap,
            "snapshot_interval_ops": interval,
            "gap_red_ops": limit,
            "gap_intervals": round(gap / interval, 2),
        }
    statuses.append(watermark["status"])

    rate = dict(raw.get("sync_rate") or {})
    sync: dict[str, Any]
    if rate.get("applicable") is False:
        sync = {
            "status": "green",
            "available": False,
            "applicable": False,
            "state": "not-applicable",
            "reason": rate.get(
                "reason", "chat edge is not on this host"
            ),
        }
    elif rate.get("available"):
        hot = [
            client
            for client in rate.get("clients", [])
            if float(client.get("rate_per_second", 0))
            >= float(contract["sync_rate_red_per_second"])
        ]
        sync = {
            "status": "red" if hot else "green",
            "available": True,
            "source": rate.get("source"),
            "window_seconds": rate.get("window_seconds"),
            "since": rate.get("since"),
            "path": contract["sync_path"],
            "red_per_second": contract["sync_rate_red_per_second"],
            "total_requests": rate.get("total", 0),
            "clients": rate.get("clients", []),
            "over_threshold": hot,
        }
    else:
        sync = {
            "status": "unknown",
            "available": False,
            "window_seconds": rate.get("window_seconds"),
            "path": contract["sync_path"],
            "errors": rate.get("errors") or ["sync-rate evidence unavailable"],
        }
    statuses.append(sync["status"])

    stores_raw = dict(raw.get("client_stores") or {})
    room_heads = (
        server.get("room_heads")
        if server.get("applicable") and not server.get("errors")
        else None
    )
    stores: list[dict[str, Any]] = []
    flagged_rooms: list[dict[str, Any]] = []
    lag_red = int(contract["cursor_lag_red_ops"])
    stale_after = int(contract["cursor_store_stale_seconds"])
    for store in stores_raw.get("stores", []):
        rooms: list[dict[str, Any]] = []
        store_flagged: list[dict[str, Any]] = []
        for room_id, cursor in (store.get("rooms") or {}).items():
            room: dict[str, Any] = {
                "room_id": short_room_id(room_id),
                "cursor_max_seq": cursor,
            }
            if room_heads is not None and room_id in room_heads:
                head_seq = int(room_heads[room_id])
                lag = head_seq - int(cursor)
                room["head_seq"] = head_seq
                room["lag_ops"] = lag
                age = store.get("age_seconds")
                frozen = lag > lag_red and age is not None and float(age) > stale_after
                room["flagged"] = frozen
                if frozen:
                    entry = {
                        **room,
                        "kind": store["kind"],
                        "label": store["label"],
                        "store_age_seconds": int(age),
                    }
                    store_flagged.append(entry)
                    flagged_rooms.append(entry)
            rooms.append(room)
        stores.append(
            {
                "kind": store["kind"],
                "label": store["label"],
                "agent_runtime_id": store.get("agent_runtime_id"),
                "age_seconds": (
                    int(store["age_seconds"])
                    if store.get("age_seconds") is not None
                    else None
                ),
                "rooms": rooms,
                "error": store.get("error"),
                "flagged": store_flagged,
            }
        )

    cursors: dict[str, Any]
    if not stores and not stores_raw.get("skipped"):
        cursors = {
            "status": "green",
            "applicable": False,
            "state": "not-applicable",
            "reason": "no chat client stores on this host",
            "evidence_note": CURSOR_EVIDENCE_NOTE,
        }
    elif stores and room_heads is None:
        cursors = {
            "status": "unknown",
            "applicable": True,
            "reason": (
                "server room heads unavailable on this host; run on the app"
                " host for the head-vs-cursor comparison"
            ),
            "evidence_note": CURSOR_EVIDENCE_NOTE,
            "stores": stores,
            "skipped": stores_raw.get("skipped", []),
        }
    else:
        errored = [store for store in stores if store.get("error")]
        skipped = stores_raw.get("skipped", [])
        # Fail-closed: an active runtime whose local store cannot be found is
        # actionable evidence, never a clean green cursor probe.
        cursors = {
            "status": combine_status(
                ["red"]
                if flagged_rooms
                else (["unknown"] if (errored or skipped) else ["green"])
            ),
            "applicable": True,
            "lag_red_ops": lag_red,
            "store_stale_seconds": stale_after,
            "evidence_note": CURSOR_EVIDENCE_NOTE,
            "stores": stores,
            "skipped": stores_raw.get("skipped", []),
            "flagged_rooms": flagged_rooms,
        }
    statuses.append(cursors["status"])

    return {
        "status": combine_status(statuses),
        "server_watermark": watermark,
        "sync_fetch_rate": sync,
        "agent_cursors": cursors,
    }


def short_room_id(room_id: str) -> str:
    """Purpose-scoped display form: ids stay evidence, not chatter."""
    return room_id if len(room_id) <= 12 else f"{room_id[:12]}…"


# `MAX(seq) FROM client_app_events` is a durable-cursor PROXY, not the cursor:
# only decrypted application events land in that table, so membership and
# key-package operations keep a modest standing lag against the server head
# even on a healthy device. The freeze line is therefore lag beyond
# cursor_lag_red_ops AND a store untouched for cursor_store_stale_seconds.
CURSOR_EVIDENCE_NOTE = (
    "client_app_events MAX(seq) can lag the true durable cursor; only a lag"
    " beyond cursor_lag_red_ops combined with a stale store is the freeze"
    " signature"
)


def report_exit_code(report: dict[str, Any]) -> int:
    statuses = [section["status"] for section in report["sections"].values()]
    if "red" in statuses:
        return 1
    if "unknown" in statuses:
        return 2
    return 0


def build_report(raw: dict[str, Any], now: datetime) -> dict[str, Any]:
    errors = raw.get("collection_errors", {})
    fleet = build_fleet(
        raw.get("core"), now, errors.get("core"), raw.get("lifecycle_probe")
    )
    target_id = fleet.get("target_artifact", {}).get("id")
    sections = {
        "fleet_convergence": fleet,
        "host_health": build_host_health(raw.get("host_health"), target_id),
        "recovery_boundary": build_recovery(raw.get("recovery"), now),
        "rollout_state": build_rollout(raw.get("rollout")),
        "chat_plane": build_chat_plane(raw.get("chat_plane"), now),
    }
    report = {
        "schema_version": "finite.status.v1",
        "generated_at": isoformat(now),
        "overall_status": combine_status(
            [section["status"] for section in sections.values()]
        ),
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
        f"Fleet convergence {badge(fleet['status'])} — Core-recorded, NOT verified live"
    )
    if fleet.get("target_artifact"):
        target = fleet["target_artifact"]
        lines.append(
            f"  target: {target['version_label']} ({target['id']}) "
            f"promoted {target['promoted_at']}"
        )
        for host in fleet.get("hosts", []):
            host_line = (
                f"  {host['source_host_id']}: {host['on_target']}/{host['active_total']} active on target; "
                f"{host['straggler_count']} stragglers; "
                f"{host['intentionally_inactive_count']} intentionally inactive excluded"
            )
            probed = host.get("lifecycle_probed_count", 0)
            if probed:
                attention = len(host.get("lifecycle_attention", []))
                host_line += f"; lifecycle {probed - attention}/{probed} operable"
            tracked = host.get("health_tracked_count", 0)
            if tracked:
                host_line += (
                    f"; health {host['health_ready_count']}/{tracked} ready"
                    f" ({len(host.get('health_unknown', []))} unknown)"
                )
            lines.append(host_line)
            for runtime in host["stragglers"]:
                lines.append(
                    f"    STRAGGLER {runtime['agent_name']} [{runtime['agent_runtime_id']}]: "
                    f"{runtime['version_label']}"
                )
            for runtime in host["unlinked"]:
                lines.append(
                    f"    UNKNOWN-LINK {runtime['agent_name']} [{runtime['agent_runtime_id']}]: "
                    f"{runtime['version_label']}"
                )
            for runtime in host.get("lifecycle_attention", []):
                lifecycle = runtime["lifecycle"]
                reason = lifecycle.get("reason") or "no reason recorded"
                lines.append(
                    f"    LIFECYCLE {runtime['agent_name']} [{runtime['agent_runtime_id']}]: "
                    f"{lifecycle['verdict']} ({reason})"
                )
            for runtime in host.get("control_active", []):
                control = runtime["control"]
                lines.append(
                    f"    CONTROL {runtime['agent_name']} [{runtime['agent_runtime_id']}]: "
                    f"{control['kind']} {control['status']}"
                )
            for runtime in host.get("health_not_ready", []):
                health = runtime["health"]
                reason = health.get("reason") or "no reason recorded"
                lines.append(
                    f"    HEALTH {runtime['agent_name']} [{runtime['agent_runtime_id']}]: "
                    f"not_ready ({reason})"
                )
            for runtime in host.get("health_unknown", []):
                health = runtime["health"]
                last = (
                    f"last report {human_age(health['age_seconds'])} ago"
                    if health.get("age_seconds") is not None
                    else "never reported"
                )
                lines.append(
                    f"    HEALTH-UNKNOWN {runtime['agent_name']} "
                    f"[{runtime['agent_runtime_id']}]: no fresh report ({last})"
                )
    else:
        lines.append(f"  {fleet.get('error', 'unavailable')}")
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
                f"  storage: single-disk; expected devices {present}/{len(disks)} present "
                f"{badge(storage['status'])}"
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
        if runner.get("applicable") is False:
            lines.append(
                "  runner: not applicable on this host (app-plane host, ADR 0007)"
            )
        else:
            # Only a CONFIRMED absence reads as unset; an unprobeable environment
            # stays honest about never having seen the value.
            pin_display = runner["artifact_pin"] or (
                "unset" if runner["pin_state"] == "absent" else "unknown"
            )
            lines.append(
                f"  runner: timer {badge(runner['timer_status'])}; drain={runner['drain'] or 'unknown'} "
                f"{badge(runner['drain_status'])}; pin={pin_display} "
                f"{badge(runner['pin_status'])} ({runner['pin_state']})"
            )
            lines.append(
                f"    Finite Private: model={runner['finite_private_model'] or 'unknown'} "
                f"{badge(runner['finite_private_model_status'])} "
                f"({runner['finite_private_model_state']}); "
                f"route={runner['finite_private_base_url'] or 'unknown'} "
                f"{badge(runner['finite_private_base_url_status'])}"
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
    lines.append("")

    chat = sections["chat_plane"]
    lines.append(f"Chat plane {badge(chat['status'])}")
    if not {"server_watermark", "sync_fetch_rate", "agent_cursors"} <= chat.keys():
        # Error-only section: no chat-plane evidence was collected at all.
        lines.append(f"  {chat.get('error', 'chat-plane evidence unavailable')}")
        lines.extend(
            [
                "",
                "Exit codes: 0 all green; 1 any red; 2 could not determine (when nothing is red).",
            ]
        )
        return "\n".join(lines)
    watermark = chat["server_watermark"]
    if watermark.get("applicable"):
        lines.append(
            f"  server watermark: ops head {watermark['ops_head']}; "
            f"snapshot {watermark['snapshot_watermark']}; "
            f"gap {watermark['gap_ops']} ops "
            f"(~{watermark['gap_intervals']} intervals of "
            f"{watermark['snapshot_interval_ops']}) {badge(watermark['status'])}"
        )
    elif watermark.get("state") == "not-applicable":
        lines.append(f"  server watermark: not applicable ({watermark['reason']})")
    else:
        detail = watermark.get("errors") or [watermark.get("error", "unavailable")]
        lines.append(f"  server watermark {badge(watermark['status'])}: {detail[0]}")

    sync = chat["sync_fetch_rate"]
    if sync.get("available"):
        lines.append(
            f"  sync fetch rate: {sync['total_requests']} POST {sync['path']} in "
            f"{sync['window_seconds']}s from {sync['source']} "
            f"(red >= {sync['red_per_second']}/s per client) {badge(sync['status'])}"
        )
        for client in sync.get("clients", [])[:3]:
            attributed = client.get("attributed_host")
            hot = client in sync.get("over_threshold", [])
            lines.append(
                f"    {client['ip']}"
                f"{f' ({attributed})' if attributed else ''}: "
                f"{client['rate_per_second']}/s" + (" [RED]" if hot else "")
            )
    elif sync.get("state") == "not-applicable":
        lines.append(f"  sync fetch rate: not applicable ({sync['reason']})")
    else:
        detail = sync.get("errors") or ["sync-rate evidence unavailable"]
        lines.append(f"  sync fetch rate {badge(sync['status'])}: {detail[0]}")

    cursors = chat["agent_cursors"]
    if cursors.get("applicable"):
        stores = cursors.get("stores", [])
        flagged = cursors.get("flagged_rooms", [])
        lines.append(
            f"  client cursors: {len(stores)} store(s) sampled; "
            f"{len(flagged)} frozen room(s) {badge(cursors['status'])}"
        )
        if cursors.get("reason"):
            lines.append(f"    {cursors['reason']}")
        for room in flagged[:5]:
            lines.append(
                f"    FROZEN {room['kind']} {room['label']}: room {room['room_id']} "
                f"head {room['head_seq']} vs cursor {room['cursor_max_seq']} "
                f"(lag {room['lag_ops']}); store idle {human_age(room['store_age_seconds'])}"
            )
        for store in stores:
            if store.get("error"):
                lines.append(f"    {store['kind']} {store['label']}: {store['error']}")
            elif not flagged:
                rooms = ", ".join(
                    f"{room['room_id']}@{room['cursor_max_seq']}"
                    for room in store["rooms"]
                )
                suffix = f"; {rooms}" if rooms else ""
                lines.append(
                    f"    {store['kind']} {store['label']}: store idle "
                    f"{human_age(store.get('age_seconds'))}{suffix}"
                )
        for entry in cursors.get("skipped", []):
            lines.append(
                f"    SKIPPED agent {entry.get('agent_runtime_id')}: "
                f"{entry.get('reason', 'client store not found')}"
            )
    elif cursors.get("state") == "not-applicable":
        lines.append(f"  client cursors: not applicable ({cursors['reason']})")
    else:
        detail = cursors.get("reason") or cursors.get("error") or "unavailable"
        lines.append(f"  client cursors {badge(cursors['status'])}: {detail}")
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
    raw["lifecycle_probe"] = collect_lifecycle_probe(
        raw.get("core", {}).get("runtimes", []), hostname
    )
    raw["chat_plane"] = collect_chat_plane(
        hostname, raw.get("core", {}).get("runtimes", []), now
    )
    return raw, now


def parse_args(arguments: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Canonical read-only Finite platform and fleet status"
    )
    parser.add_argument(
        "--json", action="store_true", help="emit finite.status.v1 JSON"
    )
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
                "chat_plane": {"status": "unknown", "error": str(error)},
            },
        }
    if options.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print(render_human(report))
    raise SystemExit(report["exit_code"])

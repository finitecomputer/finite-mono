#!/usr/bin/env python3
"""Publish Core-recorded Runtime artifact metrics for the Grafana MVP."""

from __future__ import annotations

import os
import sys
import tempfile
from collections import Counter
from pathlib import Path
from typing import Any

from scripts import finite_status


def label(value: object) -> str:
    return str(value).replace("\\", "\\\\").replace("\n", "\\n").replace('"', '\\"')


def sample(name: str, labels: dict[str, object], value: int = 1) -> str:
    rendered = ",".join(f'{key}="{label(value)}"' for key, value in labels.items())
    return f"{name}{{{rendered}}} {value}"


def digest(reference: str) -> str:
    _, separator, value = reference.rpartition("@")
    return value if separator and value.startswith("sha256:") else ""


def render(core: dict[str, Any]) -> str:
    artifacts = core.get("artifacts", [])
    target = finite_status.target_runtime_artifact(artifacts)
    if target is None:
        raise finite_status.CollectionError(
            "Core has no promoted, non-retired Runtime artifact"
        )
    by_id = {artifact["id"]: artifact for artifact in artifacts}
    active_counts: Counter[tuple[str, str]] = Counter()
    # Pre-artifact-era rows (NULL artifact id) and unknown-artifact references
    # must not take the whole exporter down: surface them as their own gauge
    # and keep publishing the healthy fleet. Never silently drop them.
    incomplete_by_host: Counter[str] = Counter()
    for runtime in core.get("runtimes", []):
        if runtime.get("link_state") != "active":
            continue
        host = runtime.get("source_host_id", "")
        artifact_id = runtime.get("runtime_artifact_id", "")
        if not host or artifact_id not in by_id:
            incomplete_by_host[host or "unknown"] += 1
            continue
        active_counts[(host, artifact_id)] += 1
    if not any(artifact_id == target["id"] for _, artifact_id in active_counts):
        active_counts[("unassigned", target["id"])] = 0

    lines: list[str] = []
    for host, count in sorted(incomplete_by_host.items()):
        lines.append(
            sample(
                "finite_runtime_incomplete_artifact_identity",
                {"source_host_id": host},
                count,
            )
        )
    by_host: dict[str, set[str]] = {}
    for (host, artifact_id), count in sorted(active_counts.items()):
        artifact = by_id[artifact_id]
        promoted = artifact_id == target["id"]
        artifact_labels = {
            "source_host_id": host,
            "artifact_id": artifact_id,
            "version_label": artifact["version_label"],
            "promoted": str(promoted).lower(),
        }
        lines.append(sample("finite_runtime_artifact_info", artifact_labels))
        lines.append(
            sample(
                "finite_runtime_artifact_active_agents",
                artifact_labels,
                count,
            )
        )
        lines.append(
            sample(
                "finite_component_build_info",
                {
                    "host": host,
                    "component": "finite-agent-runtime",
                    "version": artifact["version_label"],
                    "git_sha": artifact.get("source_git_sha", ""),
                    "image_digest": digest(artifact.get("reference", "")),
                    "source": "core",
                },
            )
        )
        by_host.setdefault(host, set()).add(artifact_id)

    for host, artifact_ids in sorted(by_host.items()):
        lines.append(
            sample(
                "finite_component_version_mismatch",
                {"host": host, "component": "finite-agent-runtime"},
                int(any(artifact_id != target["id"] for artifact_id in artifact_ids)),
            )
        )
    return "\n".join(lines) + "\n"


def write_atomic(path: Path, contents: str) -> None:
    temporary: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            "w", dir=path.parent, prefix=f"{path.name}.tmp.", delete=False
        ) as output:
            temporary = Path(output.name)
            output.write(contents)
            output.flush()
            os.fsync(output.fileno())
        temporary.chmod(0o640)
        temporary.replace(path)
        temporary = None
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(sys.argv[0]).name} OUTPUT")
    output = Path(sys.argv[1])
    write_atomic(output, render(finite_status.collect_core()))


if __name__ == "__main__":
    main()

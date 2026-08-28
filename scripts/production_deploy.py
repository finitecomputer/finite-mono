#!/usr/bin/env python3
"""Small production-deploy manifest validator and plan helper."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MANIFEST = ROOT / "infra" / "deployments" / "production.toml"

PLAN_SCHEMA = "finite.production-deploy-plan.v1"
RECORD_SCHEMA = "finite.production-deploy-record.v1"
RISKY_PATH_POLICY = "lat1-v1"
ZERO_SHA = "0" * 40
ALLOWED_CLASSIFICATIONS = {"ordinary", "schema-change", "forward-only"}
ALLOWED_MANIFEST_KEYS = {
    "environment",
    "scope",
    "classification",
    "risky_path_policy",
    "mutation_enabled",
    "rollback_policy",
    "required_gates",
}


@dataclass(frozen=True)
class RiskyRule:
    path: str
    reason: str
    directory: bool = False

    def matches(self, changed_path: str) -> bool:
        if self.directory:
            return changed_path.startswith(self.path)
        return changed_path == self.path


RISKY_RULES = (
    RiskyRule(
        "finitecomputer-v2/crates/finite-saas-core/migrations/",
        "core-postgres-migration",
        directory=True,
    ),
    RiskyRule(
        "finitecomputer-v2/crates/finite-saas-core/src/lib.rs",
        "core-schema-embedding-or-state-wire",
    ),
    RiskyRule(
        "finitecomputer-v2/crates/finite-saas-core/src/store.rs",
        "core-persistence-writer",
    ),
    RiskyRule(
        "finite-brain/crates/finite-brain-store/src/schema.rs",
        "brain-sqlite-schema",
    ),
    RiskyRule(
        "finite-brain/crates/finite-brain-store/src/lib.rs",
        "brain-store-migration-entrypoint",
    ),
    RiskyRule(
        "finite-sites/crates/finitesites-store/src/schema.rs",
        "sites-sqlite-schema",
    ),
    RiskyRule(
        "finite-sites/crates/finitesites-store/src/lib.rs",
        "sites-store-migration-entrypoint",
    ),
    RiskyRule(
        "finitechat/crates/finitechat-server/src/store/",
        "chat-server-store",
        directory=True,
    ),
    RiskyRule(
        "finitechat/crates/finitechat-server/src/legacy_store.rs",
        "chat-server-legacy-store",
    ),
    RiskyRule("infra/nixos/modules/postgres.nix", "lat1-postgres-module"),
    RiskyRule("infra/nixos/modules/finite-saas-core.nix", "lat1-core-module"),
    RiskyRule("infra/nixos/modules/backups.nix", "lat1-recovery-module"),
    RiskyRule("infra/nixos/modules/finite-litestream.nix", "lat1-litestream-module"),
)


class DeployConfigError(ValueError):
    pass


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run_git(args: list[str]) -> str:
    result = subprocess.run(
        ["git", *args],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        raise DeployConfigError(result.stderr.strip() or f"git {' '.join(args)} failed")
    return result.stdout.strip()


def resolve_rev(rev: str) -> str:
    resolved = run_git(["rev-parse", rev])
    if len(resolved) != 40 or any(char not in "0123456789abcdef" for char in resolved):
        raise DeployConfigError(f"revision did not resolve to a full SHA: {rev}")
    return resolved


def commit_parents(rev: str) -> list[str]:
    raw = run_git(["rev-list", "--parents", "-n", "1", rev])
    parts = raw.split()
    if not parts or parts[0] != rev:
        raise DeployConfigError(f"could not inspect commit parents for {rev}")
    return parts[1:]


def tree_sha(rev: str) -> str:
    return run_git(["rev-parse", f"{rev}^{{tree}}"])


def resolve_ci_source(source: str, push_before: str | None) -> dict[str, str]:
    source_sha = resolve_rev(source)
    before_sha = (
        resolve_rev(push_before) if push_before and push_before != ZERO_SHA else None
    )
    parents = commit_parents(source_sha)

    if before_sha and len(parents) == 2 and parents[0] == before_sha:
        ci_source_sha = parents[1]
        if tree_sha(source_sha) != tree_sha(ci_source_sha):
            raise DeployConfigError(
                "production merge commit tree does not match its promoted source parent"
            )
        return {
            "source_sha": source_sha,
            "ci_source_sha": ci_source_sha,
            "ci_source_reason": "production-merge-second-parent",
        }

    return {
        "source_sha": source_sha,
        "ci_source_sha": source_sha,
        "ci_source_reason": "source-sha",
    }


def changed_paths(base: str, head: str) -> list[str]:
    raw = run_git(["diff", "--name-only", f"{base}...{head}"])
    return [line for line in raw.splitlines() if line]


def load_manifest(path: Path = DEFAULT_MANIFEST) -> dict[str, Any]:
    try:
        manifest = tomllib.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise DeployConfigError(f"cannot read manifest {path}: {error}") from error
    except tomllib.TOMLDecodeError as error:
        raise DeployConfigError(f"cannot parse manifest {path}: {error}") from error
    return validate_manifest(manifest)


def validate_manifest(manifest: dict[str, Any]) -> dict[str, Any]:
    unknown = sorted(set(manifest) - ALLOWED_MANIFEST_KEYS)
    if unknown:
        raise DeployConfigError(f"unknown manifest keys: {unknown}")

    required = [
        "environment",
        "scope",
        "classification",
        "risky_path_policy",
        "mutation_enabled",
        "rollback_policy",
    ]
    missing = [key for key in required if key not in manifest]
    if missing:
        raise DeployConfigError(f"missing manifest keys: {missing}")

    if manifest["environment"] != "production":
        raise DeployConfigError("environment must be production")
    if manifest["scope"] != "lat1-nixos":
        raise DeployConfigError("scope must be lat1-nixos")
    if manifest["classification"] not in ALLOWED_CLASSIFICATIONS:
        raise DeployConfigError(
            f"classification must be one of {sorted(ALLOWED_CLASSIFICATIONS)}"
        )
    if manifest["risky_path_policy"] != RISKY_PATH_POLICY:
        raise DeployConfigError(f"risky_path_policy must be {RISKY_PATH_POLICY}")
    if not isinstance(manifest["mutation_enabled"], bool):
        raise DeployConfigError("mutation_enabled must be a boolean")
    if manifest["rollback_policy"] != "previous-lat1-closure":
        raise DeployConfigError("rollback_policy must be previous-lat1-closure")

    gates = manifest.get("required_gates", [])
    if not isinstance(gates, list) or not all(
        isinstance(gate, str) and gate for gate in gates
    ):
        raise DeployConfigError("required_gates must be a list of non-empty strings")

    return manifest


def classify_paths(paths: list[str]) -> list[dict[str, str]]:
    risky: list[dict[str, str]] = []
    for path in paths:
        for rule in RISKY_RULES:
            if rule.matches(path):
                risky.append({"path": path, "reason": rule.reason})
                break
    return risky


def validate_classification(
    manifest: dict[str, Any], risky: list[dict[str, str]]
) -> None:
    if risky and manifest["classification"] == "ordinary":
        risky_list = ", ".join(entry["path"] for entry in risky)
        raise DeployConfigError(
            "classification ordinary is not allowed when risky paths changed: "
            f"{risky_list}"
        )


def build_plan(manifest_path: Path, base: str, head: str) -> dict[str, Any]:
    manifest = load_manifest(manifest_path)
    base_sha = resolve_rev(base)
    head_sha = resolve_rev(head)
    paths = changed_paths(base_sha, head_sha)
    risky = classify_paths(paths)
    validate_classification(manifest, risky)
    return {
        "schema": PLAN_SCHEMA,
        "environment": manifest["environment"],
        "scope": manifest["scope"],
        "source_sha": head_sha,
        "production_base_sha": base_sha,
        "production_branch": "production",
        "manifest_path": str(manifest_path.relative_to(ROOT)),
        "manifest_sha256": sha256_file(manifest_path),
        "classification": manifest["classification"],
        "risky_path_policy": manifest["risky_path_policy"],
        "mutation_enabled": manifest["mutation_enabled"],
        "rollback_policy": manifest["rollback_policy"],
        "required_gates": manifest.get("required_gates", []),
        "changed_paths": paths,
        "risky_paths": risky,
    }


def render_plan_summary(plan: dict[str, Any]) -> str:
    risky = plan["risky_paths"]
    risky_lines = (
        "\n".join(f"- `{entry['path']}` ({entry['reason']})" for entry in risky)
        if risky
        else "- None"
    )
    merge_behavior = (
        "would be allowed to request production approval"
        if plan["mutation_enabled"]
        else "will stop before the Mutation Boundary because mutation is disabled"
    )
    return "\n".join(
        [
            "## Production Deploy Plan",
            "",
            f"- Source SHA: `{plan['source_sha']}`",
            f"- Current production base: `{plan['production_base_sha']}`",
            f"- Scope: `{plan['scope']}`",
            f"- Classification: `{plan['classification']}`",
            f"- Risky path policy: `{plan['risky_path_policy']}`",
            f"- Mutation enabled: `{str(plan['mutation_enabled']).lower()}`",
            f"- Post-merge behavior: {merge_behavior}.",
            "",
            "Risky paths:",
            risky_lines,
            "",
            "Required gates:",
            "\n".join(f"- `{gate}`" for gate in plan["required_gates"]),
            "",
        ]
    )


def build_record(
    plan: dict[str, Any],
    *,
    outcome: str,
    mutation_boundary_crossed: bool,
    system_path: str | None,
    override_reason: str | None,
) -> dict[str, Any]:
    now = utc_now()
    return {
        "schema": RECORD_SCHEMA,
        "environment": plan["environment"],
        "scope": plan["scope"],
        "source_sha": plan["source_sha"],
        "production_branch": plan["production_branch"],
        "manifest_sha256": plan["manifest_sha256"],
        "classification": plan["classification"],
        "mutation_enabled": plan["mutation_enabled"],
        "mutation_boundary_crossed": mutation_boundary_crossed,
        "system_path": system_path,
        "finite_status_before_artifact": "finite-status-before",
        "finite_status_after_artifact": "finite-status-after",
        "outcome": outcome,
        "started_at": now,
        "finished_at": now,
        "override_reason": override_reason,
    }


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


def command_validate_manifest(args: argparse.Namespace) -> int:
    load_manifest(args.manifest)
    return 0


def command_plan(args: argparse.Namespace) -> int:
    plan = build_plan(args.manifest, args.base, args.head)
    write_json(args.output, plan)
    if args.summary:
        args.summary.parent.mkdir(parents=True, exist_ok=True)
        args.summary.write_text(render_plan_summary(plan), encoding="utf-8")
    return 0


def command_ci_source(args: argparse.Namespace) -> int:
    resolution = resolve_ci_source(args.source, args.push_before)
    if args.output:
        write_json(args.output, resolution)
    if args.github_output:
        with args.github_output.open("a", encoding="utf-8") as output:
            output.write(f"ci_source_sha={resolution['ci_source_sha']}\n")
            output.write(f"ci_source_reason={resolution['ci_source_reason']}\n")
    if not args.output and not args.github_output:
        print(resolution["ci_source_sha"])
    return 0


def command_record(args: argparse.Namespace) -> int:
    plan = json.loads(args.plan.read_text(encoding="utf-8"))
    if plan.get("schema") != PLAN_SCHEMA:
        raise DeployConfigError("record input is not a production deploy plan")
    record = build_record(
        plan,
        outcome=args.outcome,
        mutation_boundary_crossed=args.mutation_boundary_crossed,
        system_path=args.system_path,
        override_reason=args.override_reason,
    )
    write_json(args.output, record)
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)

    validate = subcommands.add_parser("validate-manifest")
    validate.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    validate.set_defaults(func=command_validate_manifest)

    plan = subcommands.add_parser("plan")
    plan.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    plan.add_argument("--base", required=True)
    plan.add_argument("--head", required=True)
    plan.add_argument("--output", type=Path, required=True)
    plan.add_argument("--summary", type=Path)
    plan.set_defaults(func=command_plan)

    ci_source = subcommands.add_parser("ci-source")
    ci_source.add_argument("--source", required=True)
    ci_source.add_argument("--push-before")
    ci_source.add_argument("--output", type=Path)
    ci_source.add_argument("--github-output", type=Path)
    ci_source.set_defaults(func=command_ci_source)

    record = subcommands.add_parser("record")
    record.add_argument("--plan", type=Path, required=True)
    record.add_argument("--output", type=Path, required=True)
    record.add_argument("--outcome", required=True)
    record.add_argument("--mutation-boundary-crossed", action="store_true")
    record.add_argument("--system-path")
    record.add_argument("--override-reason")
    record.set_defaults(func=command_record)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except DeployConfigError as error:
        print(f"production deploy config error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())

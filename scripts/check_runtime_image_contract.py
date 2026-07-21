#!/usr/bin/env python3
"""Enforce the one canonical Agent Runtime image across every Runner class."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path
from typing import Iterable

CANONICAL_DOCKERFILE = Path(
    "finitecomputer-v2/deploy/finite-computer/images/runtime.Dockerfile"
)
CANONICAL_BUILDER = Path("finitecomputer-v2/scripts/build_runtime_image.py")
CANONICAL_WORKFLOW = Path(".github/workflows/runtime-image.yml")
PHALA_ADAPTER = Path("finitecomputer-v2/crates/finite-saas-runner/src/phala.rs")

CANONICAL_DOCKERFILE_ANCHORS = (
    '"hermes-agent[messaging]==${HERMES_AGENT_VERSION}"',
    "COPY finitechat/integrations/hermes/finitechat /runtime/hermes-plugin/finitechat",
    "COPY finite-skills/skills /runtime/finite-skills",
    "COPY finitechat/containers/agent/entrypoint.sh /opt/agent-entrypoint.sh",
    "ENV FBRAIN_CONFIG_DIR=/data/agent/fbrain",
    "ENV FBRAIN_WORKING_TREE_ROOT=/data/workspace/finitebrain",
    'ENTRYPOINT ["/opt/agent-entrypoint.sh"]',
)

CANONICAL_WORKFLOW_ANCHORS = (
    "--report target/runtime-image-durable-smoke/report.json",
    'open("finitechat/target/runtime-image-durable-smoke/report.json")',
)

WORKFLOW_BUILD_OR_PUBLISH = (
    re.compile(r"\bdocker\s+(?:build|buildx|push|tag)\b", re.IGNORECASE),
    re.compile(r"\bpodman\s+(?:build|push|tag)\b", re.IGNORECASE),
    re.compile(r"\bbuildah\s+(?:bud|build|push|tag)\b", re.IGNORECASE),
    re.compile(r"\boras\s+push\b", re.IGNORECASE),
    re.compile(r"docker/build-push-action", re.IGNORECASE),
    re.compile(r"build_runtime_image\.py", re.IGNORECASE),
    re.compile(r"\bpackages\s*:\s*write\b", re.IGNORECASE),
)

PROVIDER_RUNTIME_OVERRIDE = re.compile(
    r"(?:phala[^\n]{0,100}(?:hermes_(?:config|source)|skills?_(?:source|ref)|entrypoint)"
    r"|(?:hermes_(?:config|source)|skills?_(?:source|ref)|entrypoint)[^\n]{0,100}phala)",
    re.IGNORECASE,
)
PHALA_ENV_OVERRIDE = re.compile(
    r"\bFC_RUNNER_PHALA_(?:IMAGE|ENTRYPOINT|HERMES[^\s=:]*|SKILLS?[^\s=:]*)\b",
    re.IGNORECASE,
)
IMAGE_ASSIGNMENT = re.compile(
    r"(?:^|\s)(image|runtime_image|phala_image)\s*[:=]\s*['\"]?([^\s'\"#,}]+)",
    re.IGNORECASE | re.MULTILINE,
)
DIGEST_REFERENCE = re.compile(r"^[^\s@]+@sha256:[0-9a-fA-F]{64}$")

STRUCTURED_SUFFIXES = {
    ".env",
    ".json",
    ".nix",
    ".service",
    ".sh",
    ".toml",
    ".yaml",
    ".yml",
}
SCANNED_SOURCE_SUFFIXES = STRUCTURED_SUFFIXES | {".py", ".rs", ".ts", ".tsx"}


def tracked_files(root: Path) -> list[Path]:
    result = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=root,
        check=True,
        capture_output=True,
    )
    return [Path(raw.decode()) for raw in result.stdout.split(b"\0") if raw]


def active_text(path: Path, text: str) -> str:
    """Drop comment-only lines so prose cannot trip executable-config checks."""
    suffix = path.suffix.lower()
    if suffix in {".yml", ".yaml", ".nix", ".sh", ".env", ".service", ".toml"}:
        return "\n".join(
            line for line in text.splitlines() if not line.lstrip().startswith("#")
        )
    if suffix == ".rs":
        return "\n".join(
            line
            for line in text.splitlines()
            if not line.lstrip().startswith(("//", "//!", "///"))
        )
    return text


def is_test_or_prose(path: Path) -> bool:
    lowered = path.as_posix().lower()
    return (
        path.suffix.lower() in {".md", ".mdx", ".txt", ".excalidraw"}
        or "/tests/" in f"/{lowered}"
        or "/test/" in f"/{lowered}"
        or "/fixtures/" in f"/{lowered}"
        or lowered.startswith("scripts/tests/")
        or lowered == "scripts/check_runtime_image_contract.py"
    )


def has_phala_context(path: Path, text: str) -> bool:
    return "phala" in path.as_posix().lower() or bool(
        re.search(r"\bphala\b", text, re.IGNORECASE)
    )


def workflow_execution_text(text: str) -> str:
    """Select executable workflow fields, excluding names/descriptions/examples."""
    lines = text.splitlines()
    selected: list[str] = []
    run_indent: int | None = None
    for line in lines:
        stripped = line.lstrip()
        indent = len(line) - len(stripped)
        if not stripped or stripped.startswith("#"):
            continue
        if run_indent is not None:
            if indent > run_indent:
                selected.append(stripped)
                continue
            run_indent = None
        if re.match(r"(?:-\s*)?uses\s*:", stripped, re.IGNORECASE):
            selected.append(stripped)
            continue
        run_match = re.match(r"(?:-\s*)?run\s*:\s*(.*)$", stripped, re.IGNORECASE)
        if run_match:
            value = run_match.group(1).strip()
            if value not in {"|", ">", "|-", ">-", "|+", ">+"}:
                selected.append(value)
            run_indent = indent
            continue
        if re.match(r"packages\s*:\s*write\b", stripped, re.IGNORECASE):
            selected.append(stripped)
    return "\n".join(selected)


def check_repository(root: Path, files: Iterable[Path] | None = None) -> list[str]:
    files = list(files if files is not None else tracked_files(root))
    file_set = set(files)
    violations: list[str] = []

    for required in (
        CANONICAL_DOCKERFILE,
        CANONICAL_BUILDER,
        CANONICAL_WORKFLOW,
        PHALA_ADAPTER,
    ):
        if required not in file_set or not (root / required).is_file():
            violations.append(f"missing canonical contract file: {required}")

    if CANONICAL_DOCKERFILE in file_set:
        dockerfile = (root / CANONICAL_DOCKERFILE).read_text(encoding="utf-8")
        for anchor in CANONICAL_DOCKERFILE_ANCHORS:
            if anchor not in dockerfile:
                violations.append(
                    f"{CANONICAL_DOCKERFILE}: missing canonical Runtime anchor {anchor!r}"
                )

    if CANONICAL_BUILDER in file_set:
        builder = (root / CANONICAL_BUILDER).read_text(encoding="utf-8")
        expected = (
            "dockerfile = context / "
            '"finitecomputer-v2/deploy/finite-computer/images/runtime.Dockerfile"'
        )
        if expected not in builder:
            violations.append(
                f"{CANONICAL_BUILDER}: must select only {CANONICAL_DOCKERFILE}"
            )

    if CANONICAL_WORKFLOW in file_set:
        workflow = (root / CANONICAL_WORKFLOW).read_text(encoding="utf-8")
        for anchor in CANONICAL_WORKFLOW_ANCHORS:
            if anchor not in workflow:
                violations.append(
                    f"{CANONICAL_WORKFLOW}: missing canonical Runtime workflow anchor {anchor!r}"
                )

    if PHALA_ADAPTER in file_set:
        adapter = active_text(
            PHALA_ADAPTER, (root / PHALA_ADAPTER).read_text(encoding="utf-8")
        )
        if "validate_digest_pinned_image(&self.image)?;" not in adapter:
            violations.append(
                f"{PHALA_ADAPTER}: Phala must reject mutable Runtime image references"
            )

    for path in files:
        lowered = path.as_posix().lower()
        name = path.name.lower()
        if "phala" in lowered and "dockerfile" in name:
            violations.append(
                f"{path}: Phala cannot define a second Runtime Dockerfile"
            )

    workflows = [
        path
        for path in files
        if path.parts[:2] == (".github", "workflows")
        and path.suffix.lower() in {".yml", ".yaml"}
    ]
    for path in workflows:
        text = active_text(path, (root / path).read_text(encoding="utf-8"))
        execution = workflow_execution_text(text)
        matches = [
            pattern.pattern
            for pattern in WORKFLOW_BUILD_OR_PUBLISH
            if pattern.search(execution)
        ]
        phala = has_phala_context(path, text)
        if path != CANONICAL_WORKFLOW and phala and matches:
            violations.append(
                f"{path}: Phala workflows may inspect the canonical digest but cannot build/publish an image"
            )
        if (
            path != CANONICAL_WORKFLOW
            and re.search(r"\bagent-runtime\b", text, re.IGNORECASE)
            and any(
                re.search(pattern, execution, re.IGNORECASE)
                for pattern in (
                    r"\bdocker\s+push\b",
                    r"docker/build-push-action",
                    r"\boras\s+push\b",
                )
            )
        ):
            violations.append(
                f"{path}: {CANONICAL_WORKFLOW} is the sole Agent Runtime publisher"
            )

    for path in files:
        if (
            is_test_or_prose(path)
            or path.suffix.lower() not in SCANNED_SOURCE_SUFFIXES
            or not (root / path).is_file()
        ):
            continue
        text = active_text(
            path, (root / path).read_text(encoding="utf-8", errors="replace")
        )
        phala = has_phala_context(path, text)
        if PHALA_ENV_OVERRIDE.search(text):
            violations.append(
                f"{path}: Phala cannot override the canonical image, entrypoint, Hermes, or skills source"
            )
        if not phala:
            continue
        if PROVIDER_RUNTIME_OVERRIDE.search(text):
            violations.append(
                f"{path}: provider-specific Hermes/skills/entrypoint configuration is forbidden"
            )
        if (
            path.suffix.lower() in STRUCTURED_SUFFIXES
            and not path.as_posix().startswith("finitecomputer-v2/crates/")
        ):
            for match in IMAGE_ASSIGNMENT.finditer(text):
                key, reference = match.groups()
                if (
                    path.parts[:2] == (".github", "workflows")
                    and key.lower() == "image"
                    and "agent-runtime" not in reference.lower()
                ):
                    # A workflow's job/action container is not a production
                    # Runtime reference. Explicit runtime_image/phala_image
                    # keys and known Agent Runtime repositories remain gated.
                    continue
                if not DIGEST_REFERENCE.fullmatch(reference):
                    violations.append(
                        f"{path}: mutable Phala Runtime image reference {reference!r}"
                    )

    return sorted(set(violations))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--root", type=Path, default=Path(__file__).resolve().parents[1]
    )
    args = parser.parse_args()
    violations = check_repository(args.root.resolve())
    if violations:
        print("Agent Runtime image contract violations:", file=sys.stderr)
        for violation in violations:
            print(f"- {violation}", file=sys.stderr)
        return 1
    print(
        "Agent Runtime image contract OK: one canonical Dockerfile and publication workflow"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

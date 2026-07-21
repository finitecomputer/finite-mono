#!/usr/bin/env python3
"""Reject retired Brain product vocabulary from first-party surfaces."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SURFACES = (
    ROOT / "finite-brain",
    ROOT / "finite-skills/skills/software-development/finitebrain",
    ROOT / "finitechat/crates/finitechat-hosted-device",
    ROOT / "finitecomputer-v2/apps/dashboard/src",
    ROOT / "finitecomputer-v2/crates/finite-saas-runner/src/lib.rs",
    ROOT / "scripts/devfinity-saas-smoke",
)
DOCUMENTED_EXCEPTIONS = {
    ROOT / "finite-brain/CONTEXT.md",
    ROOT / "finite-brain/docs/adr/0027-use-brain-as-the-product-language-for-knowledge-spaces.md",
    ROOT / "finite-brain/docs/specs/brain-language-and-setup-reconciliation-spec.md",
}
TEXT_SUFFIXES = {
    ".css", ".html", ".js", ".json", ".md", ".mjs", ".rs", ".sh", ".toml", ".ts", ".tsx"
}
FORBIDDEN = re.compile(
    r"\bVaults?\b|\bVaultId\b|\bvaultId\b|\bvault_id\b|/_admin/vaults\b|\bfbrain\s+vault\b"
)
PRODUCT_TEXT_SUFFIXES = {".html", ".js", ".md", ".ts", ".tsx"}
RETIRED_PRODUCT_WORD = re.compile(r"\bvaults?\b", re.IGNORECASE)


def files_under(path: Path):
    if path.is_file():
        yield path
        return
    for candidate in path.rglob("*"):
        if candidate.is_file() and candidate.suffix in TEXT_SUFFIXES:
            yield candidate


violations: list[str] = []
for surface in SURFACES:
    for path in files_under(surface):
        if path in DOCUMENTED_EXCEPTIONS:
            continue
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if FORBIDDEN.search(line) or (
                path.suffix in PRODUCT_TEXT_SUFFIXES and RETIRED_PRODUCT_WORD.search(line)
            ):
                violations.append(f"{path.relative_to(ROOT)}:{number}: {line.strip()}")

if violations:
    print("retired Brain product language found:", file=sys.stderr)
    print("\n".join(violations), file=sys.stderr)
    sys.exit(1)

print("Brain product language check passed")

#!/usr/bin/env python3
"""Reject active first-party callers of the retired Brain /_admin surface."""

from __future__ import annotations

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CALLER_FILES = (
    ROOT / "finite-brain/crates/finite-brain-cli/src/admin.rs",
    ROOT / "finite-brain/crates/finite-brain-cli/src/http.rs",
    ROOT / "finite-brain/crates/finite-brain-cli/src/lib.rs",
    ROOT / "finite-brain/crates/finite-brain-cli/src/sync_engine.rs",
    ROOT / "finite-brain/crates/finite-brain-core/src/lib.rs",
    ROOT / "finite-brain/crates/finite-brain-server/src/product-client.js",
    ROOT / "finite-brain/crates/finite-brain-server/src/smoke-ui.js",
    ROOT / "finite-brain/crates/finite-brain-server/src/routes/brains.rs",
    ROOT / "finite-brain/crates/finite-brain-server/src/routes/sharing.rs",
    ROOT / "finitecomputer-v2/apps/dashboard/browser/agent-creation.browser.ts",
)

violations: list[str] = []
for path in CALLER_FILES:
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if "/_admin" in line:
            violations.append(f"{path.relative_to(ROOT)}:{number}: {line.strip()}")

if violations:
    print("active first-party Brain /_admin caller found:", file=sys.stderr)
    print("\n".join(violations), file=sys.stderr)
    sys.exit(1)

print("Brain API caller route check passed")

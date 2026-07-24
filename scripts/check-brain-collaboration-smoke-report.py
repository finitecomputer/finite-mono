#!/usr/bin/env python3
"""Validate the public, secret-free Organization Brain collaboration smoke report."""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any


EXPECTED_BOUNDARIES = [
    "fixtureSetup",
    "signedBrainHttp",
    "restrictedKnowledgeBeforeCollaboration",
    "nativeEmailCollaboration",
    "betaOpenAndRead",
    "betaEditAndSync",
    "alphaSyncAndObserve",
]
EXPECTED_FACTS = {
    "collaborationState": "complete",
    "independentFiniteHomes": True,
    "targetForm": "canonicalManagedAgentEmail",
    "existingRestrictedKnowledge": True,
    "recipientRead": True,
    "recipientEditAndSync": True,
    "inviterObservedRecipientEdit": True,
    "recordsCredentialsKeysGrantPlaintextCommandsOrToolOutput": False,
}


def string_values(value: Any) -> list[str]:
    if isinstance(value, str):
        return [value]
    if isinstance(value, list):
        return [item for child in value for item in string_values(child)]
    if isinstance(value, dict):
        return [item for child in value.values() for item in string_values(child)]
    return []


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: check-brain-collaboration-smoke-report.py REPORT")
    path = Path(sys.argv[1])
    report = json.loads(path.read_text(encoding="utf-8"))
    expected = {
        "format": "finite.brain.organization-collaboration-smoke.v1",
        "status": "passed",
        "failedBoundary": None,
        "passedBoundaries": EXPECTED_BOUNDARIES,
        "facts": EXPECTED_FACTS,
    }
    if report != expected:
        raise SystemExit(
            "Organization Brain collaboration smoke report did not prove every "
            "required product boundary"
        )
    forbidden_fragments = (
        "nsec1",
        "privatekey",
        "folderkey",
        "wrappedevent",
        "grantplaintext",
        "authorization:",
        "bearer ",
    )
    for value in string_values(report):
        normalized = value.lower().replace("_", "").replace("-", "")
        contains_forbidden = any(
            fragment.replace("_", "").replace("-", "") in normalized
            for fragment in forbidden_fragments
        )
        if contains_forbidden:
            raise SystemExit(
                "Organization Brain collaboration smoke report contains secret-bearing material"
            )
    print(f"Organization Brain collaboration smoke report passed: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

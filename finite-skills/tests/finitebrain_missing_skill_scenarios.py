#!/usr/bin/env python3
"""Verify the durable managed-agent missing-domain-skill scenarios."""

from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parent
FIXTURES = ROOT / "fixtures" / "finitebrain-missing-skill"


def fail(message: str) -> None:
    raise AssertionError(message)


def files_under(root: Path) -> dict[str, bytes]:
    if not root.is_dir():
        fail(f"missing scenario tree: {root}")
    return {
        str(path.relative_to(root)): path.read_bytes()
        for path in sorted(root.rglob("*"))
        if path.is_file()
    }


def load_json(path: Path) -> dict[str, object]:
    if not path.is_file():
        fail(f"missing scenario record: {path}")
    return json.loads(path.read_text(encoding="utf-8"))


def assert_catalog_was_inspected(scenario: Path, report: dict[str, object]) -> None:
    expected_skill = str(report["expectedSkill"])
    catalog = (
        scenario / "input" / "installed-skills.txt"
    ).read_text(encoding="utf-8").splitlines()
    installed = {entry.strip() for entry in catalog if entry.strip()}
    if expected_skill in installed:
        fail(f"{scenario.name}: expected skill is not absent from the catalog")
    if report.get("inspectedInstalledSkillCatalog") is not True:
        fail(f"{scenario.name}: agent did not record catalog inspection")
    if report.get("expectedSkillAvailable") is not False:
        fail(f"{scenario.name}: agent did not record the expected skill as absent")


def assert_sourced_success(scenario: Path) -> None:
    report = load_json(scenario / "result" / "agent-report.json")
    assert_catalog_was_inspected(scenario, report)
    if report.get("status") != "sourced_success":
        fail("sourced: scenario did not finish through the sourced-success path")

    primary = scenario / "input" / "authoritative-docs" / "orbit-api.md"
    if not primary.is_file():
        fail("sourced: authoritative primary documentation is missing")
    source_note = scenario / "result" / "brain" / "Research" / "raw" / "orbit-api.md"
    synthesis = (
        scenario
        / "result"
        / "brain"
        / "Research"
        / "wiki"
        / "orbit-api-client-behavior.md"
    )
    index = scenario / "result" / "brain" / "Research" / "index.md"
    log = scenario / "result" / "brain" / "Research" / "log.md"
    for path in (source_note, synthesis, index, log):
        if not path.is_file():
            fail(f"sourced: required durable artifact is missing: {path}")

    primary_text = primary.read_text(encoding="utf-8")
    source_text = source_note.read_text(encoding="utf-8")
    synthesis_text = synthesis.read_text(encoding="utf-8")
    index_text = index.read_text(encoding="utf-8")
    log_text = log.read_text(encoding="utf-8")
    for claim in (
        "Orbit-Version: 2026-07-01",
        "bearer token",
        "Retry-After",
    ):
        if claim not in primary_text or claim not in source_text or claim not in synthesis_text:
            fail(f"sourced: captured primary claim is not traceable: {claim}")
    if "Publisher: Orbit Project" not in source_text:
        fail("sourced: Source Note lacks publisher provenance")
    if "[[raw/orbit-api.md|" not in synthesis_text:
        fail("sourced: synthesis does not cite its durable Source Note")
    if "[[wiki/orbit-api-client-behavior.md|" not in index_text:
        fail("sourced: durable index was not closed over the new synthesis")
    if "[[raw/orbit-api.md|" not in index_text:
        fail("sourced: durable index was not closed over the captured source")
    if "[[wiki/orbit-api-client-behavior.md|" not in log_text:
        fail("sourced: durable log was not updated")
    if "no broader retry policy is asserted" not in synthesis_text:
        fail("sourced: synthesis does not bound authority to captured evidence")


def assert_no_source_stop(scenario: Path) -> None:
    report = load_json(scenario / "result" / "agent-report.json")
    assert_catalog_was_inspected(scenario, report)
    if report.get("status") != "blocked_no_authoritative_source":
        fail("no-source: scenario did not fail closed")
    if not str(report.get("blocker", "")).strip():
        fail("no-source: agent did not explain the blocker")
    if report.get("brainMutations") != []:
        fail("no-source: agent reported Brain mutations")

    initial = files_under(scenario / "input" / "brain")
    result = files_under(scenario / "result" / "brain")
    if result != initial:
        changed = sorted(set(initial) ^ set(result))
        fail(f"no-source: requested Brain content changed: {changed}")


def main() -> int:
    assert_sourced_success(FIXTURES / "sourced")
    assert_no_source_stop(FIXTURES / "no-source")
    print("finitebrain missing-skill managed-agent scenarios passed")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AssertionError, KeyError, json.JSONDecodeError) as error:
        print(error, file=sys.stderr)
        raise SystemExit(1)

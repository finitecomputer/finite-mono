from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts.check_finite_private_deepseek_candidate import (
    OFF_CANDIDATE,
    VLLM_IMAGE_PATTERN,
    check_repository,
)


ROOT = Path(__file__).resolve().parents[2]
RUNBOOK = Path("infra/runbooks/finite-private-deepseek-production-update.md")


def temporary_candidate(
    text: str, *, runbook_text: str | None = None
) -> tempfile.TemporaryDirectory[str]:
    temporary_directory = tempfile.TemporaryDirectory()
    root = Path(temporary_directory.name)
    target = root / OFF_CANDIDATE
    target.parent.mkdir(parents=True)
    target.write_text(text, encoding="utf-8")
    runbook = root / RUNBOOK
    runbook.parent.mkdir(parents=True, exist_ok=True)
    runbook.write_text(
        runbook_text
        if runbook_text is not None
        else (ROOT / RUNBOOK).read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    return temporary_directory


class FinitePrivateDeepSeekCandidateTests(unittest.TestCase):
    def test_checked_in_candidate_passes_both_contracts(self) -> None:
        self.assertEqual(check_repository(ROOT), [])
        self.assertEqual(check_repository(ROOT, release_ready=True), [])

    def test_old_scheduler_is_rejected(self) -> None:
        text = (ROOT / OFF_CANDIDATE).read_text(encoding="utf-8")
        text = text.replace(
            '"--max-num-seqs",\n        "128"',
            '"--max-num-seqs",\n        "64"',
        )
        with temporary_candidate(text) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertTrue(any("max-num-seqs" in item for item in violations), violations)

    def test_attempt_one_runtime_and_tensor_parallelism_are_rejected(self) -> None:
        text = (ROOT / OFF_CANDIDATE).read_text(encoding="utf-8")
        text = VLLM_IMAGE_PATTERN.sub(
            'image: "vllm/vllm-openai:v0.26.0@sha256:' + "b" * 64 + '"',
            text,
        ).replace('"--data-parallel-size",', '"--tensor-parallel-size",')
        with temporary_candidate(text) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertTrue(any("v0.26.0" in item for item in violations), violations)
        self.assertTrue(
            any("tensor-parallel-size" in item for item in violations), violations
        )

    def test_glm_limiter_fallback_is_rejected_but_alias_is_required(self) -> None:
        text = (ROOT / OFF_CANDIDATE).read_text(encoding="utf-8")
        text = text.replace(
            'FINITE_PRIVATE_MODEL: "deepseek-v4-flash-0731"',
            'FINITE_PRIVATE_MODEL: "glm-5-2"',
        )
        with temporary_candidate(text) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertTrue(
            any("FINITE_PRIVATE_MODEL" in item for item in violations), violations
        )

        text = (ROOT / OFF_CANDIDATE).read_text(encoding="utf-8")
        text = text.replace('        "glm-5-2",\n', "")
        with temporary_candidate(text) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertTrue(any('"glm-5-2"' in item for item in violations), violations)

    def test_satellite_release_must_start_from_the_production_commit(self) -> None:
        candidate = (ROOT / OFF_CANDIDATE).read_text(encoding="utf-8")
        runbook = (
            (ROOT / RUNBOOK)
            .read_text(encoding="utf-8")
            .replace(
                "e337db3606d67c53387113700362adec7b4dfdf7",
                "0" * 40,
            )
        )
        with temporary_candidate(
            candidate, runbook_text=runbook
        ) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertTrue(
            any("satellite rollback commit" in item for item in violations), violations
        )

    def test_runbook_must_not_require_a_second_eight_h200_cluster(self) -> None:
        candidate = (ROOT / OFF_CANDIDATE).read_text(encoding="utf-8")
        runbook = (ROOT / RUNBOOK).read_text(encoding="utf-8") + (
            "\nCreate deepseek-v4-release-candidate on a separate eight-H200 host.\n"
        )
        with temporary_candidate(
            candidate, runbook_text=runbook
        ) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertTrue(
            any("second eight-H200" in item for item in violations), violations
        )

    def test_runbook_requires_status_delta_instead_of_unrelated_repairs(self) -> None:
        candidate = (ROOT / OFF_CANDIDATE).read_text(encoding="utf-8")
        runbook = (
            (ROOT / RUNBOOK)
            .read_text(encoding="utf-8")
            .replace(
                "Any new or worsened red or unknown",
                "Any result",
            )
        )
        with temporary_candidate(
            candidate, runbook_text=runbook
        ) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertTrue(
            any("Any new or worsened" in item for item in violations), violations
        )

    def test_runbook_scopes_settlement_to_the_rollout_boundary(self) -> None:
        candidate = (ROOT / OFF_CANDIDATE).read_text(encoding="utf-8")
        runbook = (
            (ROOT / RUNBOOK)
            .read_text(encoding="utf-8")
            .replace(
                "all reservations created during this rollout settle",
                "all reservations settle",
            )
        )
        with temporary_candidate(
            candidate, runbook_text=runbook
        ) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertTrue(
            any("created during this rollout" in item for item in violations),
            violations,
        )


if __name__ == "__main__":
    unittest.main()

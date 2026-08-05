from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts.check_finite_private_deepseek_candidate import (
    OFF_CANDIDATE,
    VLLM_IMAGE_PLACEHOLDER,
    check_repository,
)


ROOT = Path(__file__).resolve().parents[2]
MEASURED_IMAGE = (
    'image: "ghcr.io/finitecomputer/deepseek-v4-vllm:'
    '0.25.1-0731-reasoning.1@sha256:' + "a" * 64 + '"'
)


def temporary_candidate(text: str) -> tempfile.TemporaryDirectory[str]:
    temporary_directory = tempfile.TemporaryDirectory()
    root = Path(temporary_directory.name)
    target = root / OFF_CANDIDATE
    target.parent.mkdir(parents=True)
    target.write_text(text, encoding="utf-8")
    return temporary_directory


class FinitePrivateDeepSeekCandidateTests(unittest.TestCase):
    def test_checked_in_retry_candidate_passes_prep_contract(self) -> None:
        self.assertEqual(check_repository(ROOT), [])

    def test_checked_in_placeholder_fails_release_ready_contract(self) -> None:
        violations = check_repository(ROOT, release_ready=True)
        self.assertTrue(
            any("lacks a measured DeepSeek vLLM image digest" in item for item in violations),
            violations,
        )

    def test_measured_retry_image_passes_release_ready_contract(self) -> None:
        text = (ROOT / OFF_CANDIDATE).read_text(encoding="utf-8")
        text = text.replace(VLLM_IMAGE_PLACEHOLDER, MEASURED_IMAGE)
        with temporary_candidate(text) as temporary_directory:
            self.assertEqual(
                check_repository(Path(temporary_directory), release_ready=True), []
            )

    def test_blackwell_only_flag_is_rejected_on_h200(self) -> None:
        text = (ROOT / OFF_CANDIDATE).read_text(encoding="utf-8")
        text = text.replace(
            '"--no-enable-flashinfer-autotune",',
            '"--no-enable-flashinfer-autotune",\n'
            '        "--moe-backend",\n'
            '        "deep_gemm_mega_moe",',
        )
        with temporary_candidate(text) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertTrue(
            any("deep_gemm_mega_moe" in violation for violation in violations),
            violations,
        )

    def test_attempt_one_runtime_and_parallelism_are_rejected(self) -> None:
        text = (ROOT / OFF_CANDIDATE).read_text(encoding="utf-8")
        text = text.replace(
            VLLM_IMAGE_PLACEHOLDER,
            'image: "vllm/vllm-openai:v0.26.0@sha256:' + "b" * 64 + '"',
        ).replace(
            '"--data-parallel-size",',
            '"--tensor-parallel-size",',
        )
        with temporary_candidate(text) as temporary_directory:
            violations = check_repository(Path(temporary_directory))

        self.assertTrue(any("v0.26.0" in item for item in violations), violations)
        self.assertTrue(any("tensor-parallel-size" in item for item in violations), violations)

    def test_wrong_data_parallel_width_is_rejected(self) -> None:
        text = (ROOT / OFF_CANDIDATE).read_text(encoding="utf-8")
        text = text.replace(
            '"--data-parallel-size",\n        "8"',
            '"--data-parallel-size",\n        "4"',
        )
        with temporary_candidate(text) as temporary_directory:
            violations = check_repository(Path(temporary_directory))

        self.assertTrue(
            any("data-parallel-size" in item for item in violations), violations
        )

    def test_dspark_is_rejected_from_target_only_retry(self) -> None:
        text = (ROOT / OFF_CANDIDATE).read_text(encoding="utf-8")
        text = text.replace(
            '"--no-enable-flashinfer-autotune",',
            '"--no-enable-flashinfer-autotune",\n'
            '        "--speculative-config",\n'
            '        "{\\"method\\":\\"dspark\\"}",',
        )
        with temporary_candidate(text) as temporary_directory:
            violations = check_repository(Path(temporary_directory))

        self.assertTrue(any("speculative-config" in item for item in violations), violations)
        self.assertTrue(any("dspark" in item for item in violations), violations)


if __name__ == "__main__":
    unittest.main()

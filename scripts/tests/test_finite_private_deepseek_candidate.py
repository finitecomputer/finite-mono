from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts.check_finite_private_deepseek_candidate import (
    OFF_CANDIDATE,
    ON_CANDIDATE,
    check_repository,
)


ROOT = Path(__file__).resolve().parents[2]


class FinitePrivateDeepSeekCandidateTests(unittest.TestCase):
    def test_checked_in_candidates_pass_release_ready_contract(self) -> None:
        self.assertEqual(check_repository(ROOT, release_ready=True), [])

    def test_placeholders_fail_release_ready_contract(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            temporary_root = Path(temporary_directory)
            for candidate in (ON_CANDIDATE, OFF_CANDIDATE):
                target = temporary_root / candidate
                target.parent.mkdir(parents=True, exist_ok=True)
                text = (ROOT / candidate).read_text(encoding="utf-8")
                text = text.replace(
                    'mpk: "9dd15749a2f9c554cefb41b9bb202c2994d64519b4efbd42af68b51e010d5e21_166898688000_8c68b40d-723e-50f0-b86b-4d3e05b5c113"',
                    'mpk: "REPLACE_WITH_TINFOIL_MODELWRAP_MPK"',
                ).replace(
                    '"/tinfoil/mpk/mpk-9dd15749a2f9c554cefb41b9bb202c2994d64519b4efbd42af68b51e010d5e21"',
                    '"/tinfoil/mpk/mpk-REPLACE_WITH_TINFOIL_ROOT_HASH"',
                )
                target.write_text(text, encoding="utf-8")

            violations = check_repository(temporary_root, release_ready=True)
            self.assertEqual(
                sum(
                    "modelwrap MPK/root hash is not release-ready" in item
                    for item in violations
                ),
                2,
            )

    def test_blackwell_only_flag_is_rejected_on_h200(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            temporary_root = Path(temporary_directory)
            for candidate in (ON_CANDIDATE, OFF_CANDIDATE):
                target = temporary_root / candidate
                target.parent.mkdir(parents=True, exist_ok=True)
                text = (ROOT / candidate).read_text(encoding="utf-8")
                target.write_text(
                    text.replace(
                        '"--no-enable-flashinfer-autotune",',
                        '"--no-enable-flashinfer-autotune",\n'
                        '        "--moe-backend",\n'
                        '        "deep_gemm_mega_moe",',
                    ),
                    encoding="utf-8",
                )

            violations = check_repository(temporary_root)
            self.assertTrue(
                any("deep_gemm_mega_moe" in violation for violation in violations),
                violations,
            )


if __name__ == "__main__":
    unittest.main()

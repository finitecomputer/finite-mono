from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts.check_finite_private_glm53_candidate import (
    BRIDGE_CANDIDATE,
    MAIN_CANDIDATE,
    RUNBOOK,
    check_repository,
)


ROOT = Path(__file__).resolve().parents[2]


def temporary_repository(
    *,
    main_text: str | None = None,
    bridge_text: str | None = None,
    runbook_text: str | None = None,
) -> tempfile.TemporaryDirectory[str]:
    temporary_directory = tempfile.TemporaryDirectory()
    root = Path(temporary_directory.name)
    for relative, text in (
        (
            MAIN_CANDIDATE,
            main_text
            if main_text is not None
            else (ROOT / MAIN_CANDIDATE).read_text(encoding="utf-8"),
        ),
        (
            BRIDGE_CANDIDATE,
            bridge_text
            if bridge_text is not None
            else (ROOT / BRIDGE_CANDIDATE).read_text(encoding="utf-8"),
        ),
        (
            RUNBOOK,
            runbook_text
            if runbook_text is not None
            else (ROOT / RUNBOOK).read_text(encoding="utf-8"),
        ),
    ):
        target = root / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")
    return temporary_directory


class FinitePrivateGlm53CandidateTests(unittest.TestCase):
    def test_checked_in_preparation_contract_passes(self) -> None:
        self.assertEqual(check_repository(ROOT), [])

    def test_release_contract_rejects_all_stop_markers(self) -> None:
        violations = check_repository(ROOT, release_ready=True)
        self.assertTrue(
            any("modelwrap MPK/root hash" in item for item in violations),
            violations,
        )
        self.assertTrue(
            any("SGLang image digest" in item for item in violations), violations
        )
        self.assertTrue(
            any("limiter image digest" in item for item in violations), violations
        )

    def test_checkpoint_and_h200_recipe_are_fixed(self) -> None:
        text = (ROOT / MAIN_CANDIDATE).read_text(encoding="utf-8")
        text = text.replace('"--tp-size",\n        "8"', '"--tp-size",\n        "4"')
        text = text.replace('"--kv-cache-dtype",\n        "bfloat16"', '')
        with temporary_repository(main_text=text) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertTrue(any("tp-size" in item for item in violations), violations)
        self.assertTrue(any("bfloat16" in item for item in violations), violations)

    def test_legacy_aliases_are_required_but_unknown_labels_are_not_wildcarded(self) -> None:
        text = (ROOT / MAIN_CANDIDATE).read_text(encoding="utf-8")
        text = text.replace(
            'FINITE_PRIVATE_MODEL_ALIASES: "deepseek-v4-flash-0731,glm-5-2"',
            'FINITE_PRIVATE_MODEL_ALIASES: "*"',
        )
        with temporary_repository(main_text=text) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertTrue(any("model aliases" in item for item in violations), violations)

    def test_bridge_must_be_cpu_only_secretless_and_point_at_generic_route(self) -> None:
        text = (ROOT / BRIDGE_CANDIDATE).read_text(encoding="utf-8")
        text = text.replace("memory: 2048", "memory: 2048\ngpus: 1")
        text = text.replace(
            "finite-private.finite.containers.tinfoil.dev",
            "somewhere-else.example",
        )
        with temporary_repository(bridge_text=text) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertTrue(any("GPU-free" in item for item in violations), violations)
        self.assertTrue(any("generic route" in item for item in violations), violations)

    def test_tinfoil_shims_proxy_the_service_owned_route_surface(self) -> None:
        main = (ROOT / MAIN_CANDIDATE).read_text(encoding="utf-8").replace(
            '- "/*"', "- /v1/chat/completions"
        )
        bridge = (ROOT / BRIDGE_CANDIDATE).read_text(encoding="utf-8").replace(
            '- "/*"', "- /v1/chat/completions"
        )
        with temporary_repository(
            main_text=main, bridge_text=bridge
        ) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertEqual(
            sum('lacks required anchor: - "/*"' in item for item in violations),
            2,
            violations,
        )

    def test_runbook_requires_capacity_rollback_and_mixed_version_proof(self) -> None:
        text = (ROOT / RUNBOOK).read_text(encoding="utf-8")
        for anchor in (
            "120/120",
            "FINITE_PRIVATE_ROLLBACK_TAG",
            "deepseek-v4-flash-0731",
        ):
            text = text.replace(anchor, "REMOVED")
        with temporary_repository(runbook_text=text) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertTrue(any("120/120" in item for item in violations), violations)
        self.assertTrue(
            any("FINITE_PRIVATE_ROLLBACK_TAG" in item for item in violations),
            violations,
        )
        self.assertTrue(
            any("deepseek-v4-flash-0731" in item for item in violations),
            violations,
        )


if __name__ == "__main__":
    unittest.main()

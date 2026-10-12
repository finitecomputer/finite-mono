from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts.check_finite_private_glm53_candidate import (
    BRIDGE_CANDIDATE,
    MAIN_CANDIDATE,
    check_repository,
)


ROOT = Path(__file__).resolve().parents[2]


def temporary_repository(
    *,
    main_text: str | None = None,
    bridge_text: str | None = None,
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
    ):
        target = root / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")
    return temporary_directory


class FinitePrivateGlm53CandidateTests(unittest.TestCase):
    def test_checked_in_preparation_contract_passes(self) -> None:
        self.assertEqual(check_repository(ROOT), [])

    def test_release_contract_accepts_committed_release_ready_candidate(self) -> None:
        self.assertEqual(check_repository(ROOT, release_ready=True), [])

    def test_main_cvm_must_match_the_bridge_compatible_recovery(self) -> None:
        text = (ROOT / MAIN_CANDIDATE).read_text(encoding="utf-8")
        text = text.replace("cvm-version: 0.14.13", "cvm-version: 0.10.8")
        with temporary_repository(main_text=text) as temporary_directory:
            violations = check_repository(Path(temporary_directory), release_ready=True)
        self.assertTrue(any("cvm-version" in item for item in violations), violations)

    def test_model_requires_private_ipc_and_256g_shared_memory(self) -> None:
        original = (ROOT / MAIN_CANDIDATE).read_text(encoding="utf-8")
        settings = "    ipc: private\n    shm_size: 256g\n"
        for replacement in (
            "    ipc: host\n    shm_size: 256g\n",
            "    shm_size: 256g\n",
            "    ipc: private\n",
            "    ipc: private\n    shm_size: 64g\n",
            settings + "    ipc: host\n",
            settings + "    shm_size: 64g\n",
        ):
            with self.subTest(replacement=replacement):
                text = original.replace(settings, replacement)
                with temporary_repository(main_text=text) as temporary_directory:
                    violations = check_repository(Path(temporary_directory))
                self.assertTrue(any("IPC" in item for item in violations), violations)

        # Declaring these settings on the limiter must not satisfy the model contract.
        text = original.replace(settings, "").replace(
            '  - name: "finite-private-limiter"\n',
            '  - name: "finite-private-limiter"\n' + settings,
        )
        with temporary_repository(main_text=text) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertTrue(any("IPC" in item for item in violations), violations)

    def test_limiter_digest_accepts_an_optional_tag_but_never_a_mutable_image(self) -> None:
        original = (ROOT / MAIN_CANDIDATE).read_text(encoding="utf-8")
        image_prefix = "ghcr.io/finitecomputer/private-limiter"
        digest = original.split(image_prefix + "@sha256:", 1)[1].split('"', 1)[0]
        for image, accepted in (
            (image_prefix + "@sha256:" + digest, True),
            (image_prefix + ":recovery@sha256:" + digest, True),
            (image_prefix + ":latest", False),
            (image_prefix + ":recovery@sha256:abc123", False),
        ):
            with self.subTest(image=image):
                text = original.replace(image_prefix + "@sha256:" + digest, image)
                with temporary_repository(main_text=text) as temporary_directory:
                    violations = check_repository(
                        Path(temporary_directory), release_ready=True
                    )
                if accepted:
                    self.assertEqual(violations, [])
                else:
                    self.assertTrue(
                        any(
                            "immutable limiter image digest" in item
                            for item in violations
                        ),
                        violations,
                    )

    def test_compatibility_bridge_retains_its_separate_cvm_contract(self) -> None:
        text = (ROOT / BRIDGE_CANDIDATE).read_text(encoding="utf-8")
        text = text.replace("cvm-version: 0.10.8", "cvm-version: 0.14.13")
        with temporary_repository(bridge_text=text) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertTrue(
            any(
                "compatibility bridge lacks required anchor: cvm-version" in item
                for item in violations
            ),
            violations,
        )

    def test_checkpoint_and_h200_recipe_are_fixed(self) -> None:
        text = (ROOT / MAIN_CANDIDATE).read_text(encoding="utf-8")
        text = text.replace('"--tp-size",\n        "8"', '"--tp-size",\n        "4"')
        text = text.replace('"--kv-cache-dtype",\n        "bfloat16"', "")
        with temporary_repository(main_text=text) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertTrue(any("tp-size" in item for item in violations), violations)
        self.assertTrue(any("bfloat16" in item for item in violations), violations)

    def test_chunked_prefill_size_is_pinned_to_the_h200_ab(self) -> None:
        text = (ROOT / MAIN_CANDIDATE).read_text(encoding="utf-8")
        text = text.replace(
            '"--chunked-prefill-size",\n        "16384"',
            '"--chunked-prefill-size",\n        "2048"',
        )
        with temporary_repository(main_text=text) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertTrue(
            any("chunked-prefill-size" in item for item in violations), violations
        )

    def test_legacy_aliases_are_required_but_unknown_labels_are_not_wildcarded(
        self,
    ) -> None:
        text = (ROOT / MAIN_CANDIDATE).read_text(encoding="utf-8")
        text = text.replace(
            'FINITE_PRIVATE_MODEL_ALIASES: "deepseek-v4-flash-0731,glm-5-2,glm-5.3-flash"',
            'FINITE_PRIVATE_MODEL_ALIASES: "*"',
        )
        with temporary_repository(main_text=text) as temporary_directory:
            violations = check_repository(Path(temporary_directory))
        self.assertTrue(any("model aliases" in item for item in violations), violations)

    def test_bridge_must_be_cpu_only_secretless_and_point_at_generic_route(
        self,
    ) -> None:
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
        main = (
            (ROOT / MAIN_CANDIDATE)
            .read_text(encoding="utf-8")
            .replace('- "/*"', "- /v1/chat/completions")
        )
        bridge = (
            (ROOT / BRIDGE_CANDIDATE)
            .read_text(encoding="utf-8")
            .replace('- "/*"', "- /v1/chat/completions")
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


if __name__ == "__main__":
    unittest.main()

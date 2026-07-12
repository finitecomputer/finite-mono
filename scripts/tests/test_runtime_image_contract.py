from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts.check_runtime_image_contract import (
    CANONICAL_BUILDER,
    CANONICAL_DOCKERFILE,
    CANONICAL_DOCKERFILE_ANCHORS,
    CANONICAL_WORKFLOW,
    PHALA_ADAPTER,
    check_repository,
)


class RuntimeImageContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tempdir = tempfile.TemporaryDirectory()
        self.root = Path(self.tempdir.name)
        self.files: list[Path] = []
        self.write(CANONICAL_DOCKERFILE, "\n".join(CANONICAL_DOCKERFILE_ANCHORS))
        self.write(
            CANONICAL_BUILDER,
            'dockerfile = context / "finitecomputer-v2/deploy/finite-computer/images/runtime.Dockerfile"',
        )
        self.write(
            CANONICAL_WORKFLOW,
            "name: Agent Runtime Image\nrun: docker build . && docker push agent-runtime",
        )
        self.write(
            PHALA_ADAPTER,
            "impl PhalaConfig { fn validate(&self) { validate_digest_pinned_image(&self.image)?; } }",
        )

    def tearDown(self) -> None:
        self.tempdir.cleanup()

    def write(self, path: Path | str, text: str) -> None:
        path = Path(path)
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(f"{text}\n", encoding="utf-8")
        if path not in self.files:
            self.files.append(path)

    def violations(self) -> list[str]:
        return check_repository(self.root, self.files)

    def test_canonical_contract_passes(self) -> None:
        self.assertEqual(self.violations(), [])

    def test_second_phala_dockerfile_fails(self) -> None:
        self.write("deploy/phala/Dockerfile", "FROM canonical-but-forked")
        self.assertTrue(any("second Runtime Dockerfile" in item for item in self.violations()))

    def test_phala_readonly_workflow_passes_but_build_lane_fails(self) -> None:
        workflow = Path(".github/workflows/phala-readonly-preflight.yml")
        self.write(
            workflow,
            "name: Phala read-only preflight\n"
            "description: 'Prose example only: docker build must stay forbidden'\n"
            "container:\n  image: ubuntu:24.04\n"
            "run: runner preflight --read-only",
        )
        self.assertEqual(self.violations(), [])
        self.write(workflow, "name: Phala image\nrun: docker build -f deploy/phala/Dockerfile .")
        self.assertTrue(any("cannot build/publish" in item for item in self.violations()))

    def test_second_agent_runtime_publisher_fails(self) -> None:
        self.write(
            ".github/workflows/runtime-backup-publisher.yml",
            "name: backup\nrun: docker push ghcr.io/example/agent-runtime:latest",
        )
        self.assertTrue(any("sole Agent Runtime publisher" in item for item in self.violations()))

    def test_mutable_phala_image_fails_and_digest_passes(self) -> None:
        config = Path("infra/phala-worker.yml")
        self.write(config, "runner: phala\nimage: ghcr.io/example/agent-runtime:latest")
        self.assertTrue(any("mutable Phala Runtime image" in item for item in self.violations()))
        self.write(
            config,
            f"runner: phala\nimage: ghcr.io/example/agent-runtime@sha256:{'a' * 64}",
        )
        self.assertEqual(self.violations(), [])

    def test_provider_specific_runtime_sources_fail(self) -> None:
        for setting in (
            "FC_RUNNER_PHALA_HERMES_CONFIG=/tmp/hermes.yml",
            "FC_RUNNER_PHALA_SKILLS_SOURCE=/tmp/skills",
            "FC_RUNNER_PHALA_ENTRYPOINT=/tmp/start",
        ):
            with self.subTest(setting=setting):
                self.write("infra/phala-worker.env", setting)
                self.assertTrue(any("cannot override" in item for item in self.violations()))

    def test_missing_digest_guard_fails(self) -> None:
        self.write(PHALA_ADAPTER, "impl PhalaConfig { fn validate(&self) {} }")
        self.assertTrue(any("reject mutable" in item for item in self.violations()))


if __name__ == "__main__":
    unittest.main()

from __future__ import annotations

import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / ".depot" / "workflows"


class DepotWorkflowContractTests(unittest.TestCase):
    def workflow(self, name: str) -> str:
        return (WORKFLOWS / name).read_text(encoding="utf-8")

    def test_native_workflows_do_not_request_unsupported_execution_boundaries(self) -> None:
        for path in sorted(WORKFLOWS.glob("*.yml")):
            text = path.read_text(encoding="utf-8")
            with self.subTest(path=path.name):
                self.assertNotIn("runs-on: macos-", text)
                self.assertNotIn("    environment:", text)
                self.assertNotIn("secrets.GITHUB_TOKEN", text)
                self.assertNotIn("github.token", text)
                self.assertNotIn("runs_on_json", text)
                self.assertNotIn("fromJSON(inputs.", text)

    def test_finitechat_release_defers_electron_and_uses_release_repository(self) -> None:
        workflow = self.workflow("release-finitechat.yml")
        self.assertNotIn("electron:", workflow)
        self.assertNotIn("APPLE_", workflow)
        self.assertIn("cargo zigbuild", workflow)
        self.assertIn("secrets.FINITE_RELEASES_GITHUB_TOKEN", workflow)
        self.assertIn("finitecomputer/finite-releases", workflow)

    def test_finitechat_x86_release_avoids_the_zig_hyper_relocation_bug(self) -> None:
        workflow = self.workflow("release-finitechat.yml")
        self.assertIn("profile_opt_level: 1", workflow)
        self.assertIn("CARGO_PROFILE_RELEASE_OPT_LEVEL", workflow)

    def test_release_workflows_reject_a_tag_that_does_not_match_the_crate(self) -> None:
        for name in (
            "release-finitechat.yml",
            "release-fbrain.yml",
            "release-fsite.yml",
        ):
            with self.subTest(path=name):
                self.assertIn(
                    "scripts/delivery.py verify-component-version",
                    self.workflow(name),
                )

    def test_release_publication_is_fail_closed_during_shadow_runs(self) -> None:
        for name in (
            "release-finitechat.yml",
            "release-fbrain.yml",
            "release-fsite.yml",
        ):
            workflow = self.workflow(name)
            with self.subTest(path=name):
                self.assertIn("FINITE_RELEASE_PUBLISH_ENABLED == 'true'", workflow)
                self.assertIn("inputs.alias_only", workflow)
                self.assertIn("inputs.release_tag", workflow)
                self.assertIn("git fetch --no-tags origin", workflow)
                self.assertIn("scripts/delivery.py promote-release-alias", workflow)
                self.assertIn("tar --sort=name --mtime='UTC 1970-01-01'", workflow)

    def test_image_workflows_use_the_bounded_ghcr_publisher(self) -> None:
        for name in (
            "deepseek-v4-vllm-image.yml",
            "runtime-image.yml",
            "service-images.yml",
        ):
            workflow = self.workflow(name)
            with self.subTest(path=name):
                self.assertIn("secrets.FINITE_GHCR_TOKEN", workflow)
                self.assertIn("secrets.FINITE_GHCR_USERNAME", workflow)
                self.assertIn("scripts/delivery.py verify-image-promotion", workflow)
                self.assertIn("scripts/delivery.py verify-image-index", workflow)
                self.assertIn("ghcr.io/finitecomputer/", workflow)
                self.assertIn("canary-", workflow)
                self.assertIn("docker logout ghcr.io", workflow)
                self.assertIn("FINITE_GHCR_PRODUCTION_PUBLISH_ENABLED == 'true'", workflow)
                self.assertIn("inputs.publish_production", workflow)
                self.assertNotIn("github.repository_owner", workflow)
                if name != "runtime-image.yml":
                    self.assertIn("save: true", workflow)
                    self.assertIn("depot push", workflow)
                else:
                    self.assertIn("--save", workflow)

    def test_production_workflow_has_no_mutating_job(self) -> None:
        workflow = self.workflow("production-deploy.yml")
        self.assertIn("scripts/delivery.py require-production-disabled", workflow)
        self.assertNotIn("\n  deploy:\n", workflow)
        self.assertNotIn("FINITE_PRODUCTION_SSH_KEY", workflow)

    def test_ci_gate_has_no_electron_dependency(self) -> None:
        workflow = self.workflow("ci.yml")
        self.assertNotIn("electron-alpha", workflow)
        self.assertNotIn("run_electron_alpha", workflow)


if __name__ == "__main__":
    unittest.main()

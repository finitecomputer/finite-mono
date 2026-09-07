from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / ".depot" / "workflows"


class DepotWorkflowContractTests(unittest.TestCase):
    def workflow(self, name: str) -> str:
        return (WORKFLOWS / name).read_text(encoding="utf-8")

    def test_native_workflows_do_not_request_unsupported_execution_boundaries(
        self,
    ) -> None:
        for path in sorted(WORKFLOWS.glob("*.yml")):
            text = path.read_text(encoding="utf-8")
            with self.subTest(path=path.name):
                self.assertNotIn("runs-on: macos-", text)
                self.assertNotIn("    environment:", text)
                self.assertNotIn("secrets.GITHUB_TOKEN", text)
                self.assertNotIn("github.token", text)
                self.assertNotIn("runs_on_json", text)
                self.assertNotIn("fromJSON(inputs.", text)

    def test_finitechat_release_defers_electron_and_uses_release_repository(
        self,
    ) -> None:
        workflow = self.workflow("release-finitechat.yml")
        self.assertNotIn("electron:", workflow)
        self.assertNotIn("APPLE_", workflow)
        self.assertIn("secrets.FINITE_RELEASES_GITHUB_TOKEN", workflow)
        self.assertIn("finitecomputer/finite-releases", workflow)

    def test_release_workflows_build_only_linux_until_the_mac_lane_returns(
        self,
    ) -> None:
        for name, asset_name in (
            ("release-finitechat.yml", "finitechat-linux-x86_64"),
            ("release-fbrain.yml", "fbrain-linux-x86_64"),
            ("release-fsite.yml", "fsite-linux-x86_64"),
        ):
            workflow = self.workflow(name)
            with self.subTest(path=name):
                self.assertEqual(workflow.count("- asset_name:"), 1)
                self.assertIn(f"asset_name: {asset_name}", workflow)
                self.assertIn("target: x86_64-unknown-linux-gnu", workflow)
                self.assertIn("rustup toolchain install 1.88.0 --profile minimal", workflow)
                self.assertNotIn("apple-darwin", workflow)
                self.assertNotIn("zigbuild", workflow)
                self.assertNotIn("release-ci", workflow)

        flake = (ROOT / "flake.nix").read_text(encoding="utf-8")
        delivery = (ROOT / "scripts" / "delivery.py").read_text(encoding="utf-8")
        brain_manifest = (
            ROOT / "finite-brain" / "crates" / "finite-brain-cli" / "Cargo.toml"
        ).read_text(encoding="utf-8")
        self.assertNotIn("releaseRustToolchain", flake)
        self.assertNotIn("cargo-zigbuild", flake)
        self.assertNotIn("macos-matrix", delivery)
        self.assertIn('notify = "8.2.0"', brain_manifest)

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
                self.assertIn(
                    "FINITE_RELEASE_CANARY_PUBLISH_ENABLED == 'true'", workflow
                )
                self.assertIn("inputs.canary_publish == true", workflow)
                self.assertIn("github.event_name == 'workflow_dispatch'", workflow)
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
                self.assertIn(
                    "FINITE_GHCR_PRODUCTION_PUBLISH_ENABLED == 'true'", workflow
                )
                self.assertIn("inputs.publish_production", workflow)
                self.assertIn("SOURCE_REV: ${{ inputs.rev }}", workflow)
                self.assertIn("ref: ${{ inputs.rev }}", workflow)
                self.assertIn('[[ "$SOURCE_REV" =~ ^[0-9a-f]{40}$ ]]', workflow)
                self.assertIn('test "$(git rev-parse HEAD)" = "$SOURCE_REV"', workflow)
                self.assertIn(
                    'git merge-base --is-ancestor "$SOURCE_REV" origin/main',
                    workflow,
                )
                self.assertNotIn("GITHUB_SHA", workflow)
                self.assertNotIn("github.repository_owner", workflow)
                if name != "runtime-image.yml":
                    self.assertIn("save: true", workflow)
                    self.assertIn("depot push", workflow)
                else:
                    self.assertIn("--save", workflow)

    def test_production_workflow_has_no_mutating_job(self) -> None:
        workflow = self.workflow("production-deploy.yml")
        self.assertIn("scripts/delivery.py require-production-disabled", workflow)
        self.assertIn("ref: ${{ github.sha }}", workflow)
        self.assertNotIn("&& 'production'", workflow)
        self.assertNotIn("\n  deploy:\n", workflow)
        self.assertNotIn("FINITE_PRODUCTION_SSH_KEY", workflow)

    def test_closure_workflows_require_full_github_main_ancestry(self) -> None:
        for name in ("lat1-nixos-closure.yml", "lat3-nixos-closure.yml"):
            workflow = self.workflow(name)
            with self.subTest(path=name):
                self.assertIn("fetch-depth: 0", workflow)
                self.assertIn("git fetch --no-tags origin main", workflow)
                self.assertIn(
                    'git merge-base --is-ancestor "$REV" origin/main', workflow
                )
                self.assertNotIn("--depth=1", workflow)

    def test_ci_gate_has_no_electron_dependency(self) -> None:
        workflow = self.workflow("ci.yml")
        self.assertNotIn("electron-alpha", workflow)
        self.assertNotIn("run_electron_alpha", workflow)

    def test_cachix_workflows_set_the_container_user(self) -> None:
        for path in sorted(WORKFLOWS.glob("*.yml")):
            workflow = path.read_text(encoding="utf-8")
            if "cachix/cachix-action" not in workflow:
                continue
            with self.subTest(path=path.name):
                self.assertIn("  USER: runner\n", workflow)

    def test_depot_cachix_does_not_trust_repository_flake_config(self) -> None:
        flake = (ROOT / "flake.nix").read_text(encoding="utf-8")
        self.assertNotIn("extra-substituters", flake)
        self.assertNotIn("extra-trusted-public-keys", flake)

        for path in sorted(WORKFLOWS.glob("*.yml")):
            with self.subTest(path=path.name):
                self.assertNotIn(
                    "--accept-flake-config", path.read_text(encoding="utf-8")
                )

    def test_every_depot_nix_job_configures_cachix(self) -> None:
        job_pattern = re.compile(
            r"^  (?P<name>[A-Za-z0-9_-]+):\n(?P<body>.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)",
            re.MULTILINE | re.DOTALL,
        )

        for path in sorted(WORKFLOWS.glob("*.yml")):
            workflow = path.read_text(encoding="utf-8")
            for match in job_pattern.finditer(workflow):
                body = match.group("body")
                if "DeterminateSystems/nix-installer-action@v16" not in body:
                    continue
                with self.subTest(path=path.name, job=match.group("name")):
                    self.assertIn("cachix/cachix-action@v17", body)
                    self.assertLess(
                        body.index("DeterminateSystems/nix-installer-action@v16"),
                        body.index("cachix/cachix-action@v17"),
                    )

    def test_ci_pull_requests_use_cachix_read_only(self) -> None:
        workflow = self.workflow("ci.yml")

        self.assertNotIn("PR_HEAD_REPO", workflow)
        self.assertEqual(workflow.count("push|merge_group)"), 2)

    def test_ci_hands_new_devfinity_closures_to_the_dependent_smoke_job(
        self,
    ) -> None:
        workflow = self.workflow("ci.yml")
        artifact_name = (
            "devfinity-nix-handoff-${{ github.run_id }}-${{ github.run_attempt }}"
        )

        self.assertIn(
            "devfinity_handoff_required: "
            "${{ steps.devfinity_handoff.outputs.required }}",
            workflow,
        )
        self.assertIn(
            "if: steps.cachix.outputs.push_enabled != 'true' && "
            "steps.build_packages.outputs.devfinity_handoff_required == 'true'",
            workflow,
        )
        self.assertIn(
            "scripts/ci/nix-service-package-handoff \\\n"
            '            pack "$GITHUB_SHA" "$PACKAGE_OUTPUTS" "$HANDOFF_DIR"',
            workflow,
        )
        self.assertEqual(workflow.count(artifact_name), 2)
        self.assertEqual(
            workflow.count(
                "if: needs.nix-service-packages.outputs."
                "devfinity_handoff_required == 'true'"
            ),
            2,
        )
        self.assertIn(
            'nix-service-package-handoff restore "$GITHUB_SHA"',
            workflow,
        )
        self.assertLess(
            workflow.index("Package devfinity Nix closure for this workflow run"),
            workflow.index("  devfinity-smoke:"),
        )


if __name__ == "__main__":
    unittest.main()

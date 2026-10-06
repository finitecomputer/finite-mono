from __future__ import annotations

import importlib.util
import re
import subprocess
import sys
import tempfile
import textwrap
import tomllib
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[5]
CHECKER = (
    ROOT
    / "finitecomputer-v2/deploy/finite-computer/images/scripts/check_runtime_image_contract.py"
)
spec = importlib.util.spec_from_file_location("check_runtime_image_contract", CHECKER)
assert spec is not None and spec.loader is not None
check_runtime_image_contract = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = check_runtime_image_contract
spec.loader.exec_module(check_runtime_image_contract)

BUILDER_SCRIPT = ROOT / "finitecomputer-v2/scripts/build_runtime_image.py"
builder_spec = importlib.util.spec_from_file_location(
    "build_runtime_image_for_contract_tests", BUILDER_SCRIPT
)
assert builder_spec is not None and builder_spec.loader is not None
build_runtime_image = importlib.util.module_from_spec(builder_spec)
sys.modules[builder_spec.name] = build_runtime_image
builder_spec.loader.exec_module(build_runtime_image)

rust_toolchain_channel = build_runtime_image.rust_toolchain_channel
MONOREPO_ROOT = build_runtime_image.MONOREPO_ROOT

CANONICAL_BUILDER = check_runtime_image_contract.CANONICAL_BUILDER
CANONICAL_DOCKERFILE = check_runtime_image_contract.CANONICAL_DOCKERFILE
CANONICAL_DOCKERFILE_ANCHORS = check_runtime_image_contract.CANONICAL_DOCKERFILE_ANCHORS
CANONICAL_WORKFLOW = check_runtime_image_contract.CANONICAL_WORKFLOW
CANONICAL_WORKFLOW_ANCHORS = check_runtime_image_contract.CANONICAL_WORKFLOW_ANCHORS
PHALA_ADAPTER = check_runtime_image_contract.PHALA_ADAPTER
check_repository = check_runtime_image_contract.check_repository
requester_diagnostics_env = check_runtime_image_contract.requester_diagnostics_env
requester_diagnostics_label = check_runtime_image_contract.requester_diagnostics_label
requester_diagnostics_violations = (
    check_runtime_image_contract.requester_diagnostics_violations
)


class RuntimeImageContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tempdir = tempfile.TemporaryDirectory()
        self.root = Path(self.tempdir.name)
        self.files: list[Path] = []
        self.write(CANONICAL_DOCKERFILE, "\n".join(CANONICAL_DOCKERFILE_ANCHORS))
        self.write(
            CANONICAL_BUILDER,
            'dockerfile = context / "finitecomputer-v2/deploy/finite-computer/images/runtime.Dockerfile"\n'
            '--build-arg f"RUST_TOOLCHAIN={rust_toolchain_channel(MONOREPO_ROOT)}"',
        )
        self.write(
            CANONICAL_WORKFLOW,
            "name: Agent Runtime Image\n"
            "run: docker build . && docker push agent-runtime\n"
            + "\n".join(CANONICAL_WORKFLOW_ANCHORS),
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

    def test_one_shot_migration_tool_is_not_baked_into_runtime_image(self) -> None:
        dockerfile = (ROOT / CANONICAL_DOCKERFILE).read_text(encoding="utf-8")

        self.assertNotIn("legacy_hermes_migration.py", dockerfile)
        self.assertNotIn("/opt/legacy-hermes-migration", dockerfile)

    def test_rust_builder_contains_every_workspace_member(self) -> None:
        dockerfile = (ROOT / CANONICAL_DOCKERFILE).read_text(encoding="utf-8")
        builder = dockerfile.split("AS finite-rust-builder", 1)[1].split("\nFROM ", 1)[
            0
        ]
        # Directory copies must preserve the root workspace's relative paths.
        copied = [
            Path(source)
            for source, destination in re.findall(
                r"^COPY (\S+) (\S+)$", builder, re.MULTILINE
            )
            if Path(source) == Path(destination) and (ROOT / source).is_dir()
        ]
        workspace = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        for member in workspace["workspace"]["members"]:
            with self.subTest(member=member):
                self.assertTrue(
                    any(Path(member).is_relative_to(directory) for directory in copied),
                    f"Rust image builder does not copy workspace member {member}",
                )

    def test_rust_version_flows_from_the_single_toolchain_pin(self) -> None:
        dockerfile = (ROOT / CANONICAL_DOCKERFILE).read_text(encoding="utf-8")
        self.assertIn(
            "FROM rust:${RUST_TOOLCHAIN}-trixie AS finite-rust-builder", dockerfile
        )
        self.assertIsNone(re.search(r"FROM rust:\d", dockerfile))

        pin = (ROOT / "rust-toolchain.toml").read_text(encoding="utf-8")
        pin_match = re.search(r'^channel\s*=\s*"([^"]+)"', pin, re.MULTILINE)
        self.assertIsNotNone(pin_match)
        self.assertEqual(rust_toolchain_channel(ROOT), pin_match.group(1))

        builder = (ROOT / CANONICAL_BUILDER).read_text(encoding="utf-8")
        self.assertIn("RUST_TOOLCHAIN=", builder)

    def test_literal_rust_pin_in_dockerfile_fails(self) -> None:
        self.write(
            CANONICAL_DOCKERFILE,
            "\n".join(CANONICAL_DOCKERFILE_ANCHORS).replace(
                "FROM rust:${RUST_TOOLCHAIN}-trixie AS finite-rust-builder",
                "FROM rust:1.88-trixie AS finite-rust-builder",
            ),
        )
        self.assertTrue(any("RUST_TOOLCHAIN" in item for item in self.violations()))

    def test_python_html2pdf_workflow_probe_missing_fails(self) -> None:
        self.write(
            CANONICAL_WORKFLOW,
            "name: Agent Runtime Image\n"
            "run: docker build . && docker push agent-runtime\n"
            + "\n".join(
                anchor
                for anchor in CANONICAL_WORKFLOW_ANCHORS
                if "subprocess.run" not in anchor
            ),
        )
        self.assertTrue(any("subprocess.run" in item for item in self.violations()))

    def test_html2pdf_workflow_probes_missing_fails(self) -> None:
        self.write(
            CANONICAL_WORKFLOW,
            "name: Agent Runtime Image\n"
            "run: docker build . && docker push agent-runtime\n"
            + "\n".join(
                anchor
                for anchor in CANONICAL_WORKFLOW_ANCHORS
                if anchor
                not in {
                    "weasyprint /tmp/probe.html /tmp/pdf-probes/probe-weasyprint.pdf",
                    "playwright pdf file:///tmp/probe.html /tmp/pdf-probes/probe-playwright.pdf",
                }
            ),
        )
        self.assertTrue(
            any(
                "missing canonical Runtime workflow anchor" in item
                for item in self.violations()
            )
        )

    def test_pdf_text_validation_missing_fails(self) -> None:
        self.write(
            CANONICAL_WORKFLOW,
            "name: Agent Runtime Image\n"
            "run: docker build . && docker push agent-runtime\n"
            + "\n".join(
                anchor
                for anchor in CANONICAL_WORKFLOW_ANCHORS
                if "pdftotext" not in anchor
            ),
        )
        self.assertTrue(any("pdftotext" in item for item in self.violations()))

    def test_builder_without_rust_toolchain_arg_fails(self) -> None:
        self.write(
            CANONICAL_BUILDER,
            'dockerfile = context / "finitecomputer-v2/deploy/finite-computer/images/runtime.Dockerfile"',
        )
        self.assertTrue(any("RUST_TOOLCHAIN" in item for item in self.violations()))

    def test_second_phala_dockerfile_fails(self) -> None:
        self.write("deploy/phala/Dockerfile", "FROM canonical-but-forked")
        self.assertTrue(
            any("second Runtime Dockerfile" in item for item in self.violations())
        )

    def test_runtime_smoke_report_path_contract_fails_closed(self) -> None:
        self.write(
            CANONICAL_WORKFLOW,
            "name: Agent Runtime Image\n"
            "run: docker build . && docker push agent-runtime\n"
            "--report finitechat/target/runtime-image-durable-smoke/report.json\n"
            'open("finitechat/target/runtime-image-durable-smoke/report.json")',
        )
        self.assertTrue(
            any(
                "missing canonical Runtime workflow anchor" in item
                for item in self.violations()
            )
        )

    def dockerfile_with_requester_diagnostics(self, old: str, new: str) -> None:
        dockerfile = "\n".join(CANONICAL_DOCKERFILE_ANCHORS)
        self.assertIn(old, dockerfile)
        self.write(CANONICAL_DOCKERFILE, dockerfile.replace(old, new))

    def assert_requester_diagnostics_violation(self) -> None:
        self.assertTrue(
            any("REQUESTER_DIAGNOSTICS" in item for item in self.violations()),
            self.violations(),
        )

    def test_normal_build_leaves_requester_diagnostics_absent(self) -> None:
        dockerfile = (ROOT / CANONICAL_DOCKERFILE).read_text(encoding="utf-8")
        self.assertIsNone(requester_diagnostics_env(dockerfile, None))
        self.assertIsNone(requester_diagnostics_env(dockerfile, "off"))
        self.assertEqual(requester_diagnostics_env(dockerfile, "on"), "1")
        self.assertEqual(
            requester_diagnostics_label(dockerfile, "on"), "brain-requester-lease"
        )
        self.assertIsNone(requester_diagnostics_label(dockerfile, None))

    def real_sources(self) -> tuple[str, str]:
        return (
            (ROOT / CANONICAL_DOCKERFILE).read_text(encoding="utf-8"),
            (ROOT / CANONICAL_WORKFLOW).read_text(encoding="utf-8"),
        )

    def test_actual_sources_pass_the_contract(self) -> None:
        dockerfile, workflow = self.real_sources()
        self.assertEqual(requester_diagnostics_violations(dockerfile), [])
        self.write(CANONICAL_DOCKERFILE, dockerfile)
        self.write(CANONICAL_WORKFLOW, workflow)
        self.assertEqual(self.violations(), [])

    def test_mutations_of_the_actual_sources_are_rejected(self) -> None:
        dockerfile, workflow = self.real_sources()
        start = workflow.index(
            "      - name: Keep a diagnostic image a non-production canary"
        )
        guard = workflow[start : workflow.index("\n      - ", start) + 1]
        promote = "      - name: Promote the same saved build to production tags"
        off_stage = "FROM runtime AS runtime-requester-diagnostics-off"
        mutations = {
            "flag set in the runtime stage": (
                CANONICAL_DOCKERFILE,
                dockerfile.replace(
                    off_stage, f"ENV FINITECHAT_REQUESTER_DIAGNOSTICS=1\n\n{off_stage}"
                ),
            ),
            "default switched on": (
                CANONICAL_DOCKERFILE,
                dockerfile.replace(
                    "ARG REQUESTER_DIAGNOSTICS=off", "ARG REQUESTER_DIAGNOSTICS=on"
                ),
            ),
            "off stage adds a line": (
                CANONICAL_DOCKERFILE,
                dockerfile.replace(off_stage, f"{off_stage}\nENV EXTRA=1"),
            ),
            "guard removed": (CANONICAL_WORKFLOW, workflow.replace(guard, "")),
            "guard moved after the build": (
                CANONICAL_WORKFLOW,
                workflow.replace(guard, "").replace(promote, guard + promote),
            ),
        }
        for name, (path, mutated) in mutations.items():
            with self.subTest(mutation=name):
                self.write(CANONICAL_DOCKERFILE, dockerfile)
                self.write(CANONICAL_WORKFLOW, workflow)
                original = dockerfile if path == CANONICAL_DOCKERFILE else workflow
                self.assertNotEqual(mutated, original)
                self.write(path, mutated)
                self.assertNotEqual(self.violations(), [])

    def test_guard_is_the_first_step_of_the_job(self) -> None:
        _, workflow = self.real_sources()
        steps = re.findall(r"^      - (.*)$", workflow, re.MULTILINE)
        self.assertEqual(
            steps[0], "name: Keep a diagnostic image a non-production canary"
        )
        guard = self.workflow_step("Keep a diagnostic image a non-production canary")
        self.assertNotIn("uses:", guard)
        self.assertNotIn("secrets.", guard)
        self.assertNotIn("vars.", guard)

    def test_image_that_sets_the_flag_outside_the_build_argument_fails(self) -> None:
        self.dockerfile_with_requester_diagnostics(
            "ENV FINITE_BRAIN_PUBLIC_BASE_URL=https://brain.finite.computer",
            "ENV FINITE_BRAIN_PUBLIC_BASE_URL=https://brain.finite.computer\n"
            "ENV FINITECHAT_REQUESTER_DIAGNOSTICS=1",
        )
        self.assert_requester_diagnostics_violation()

    def test_requester_diagnostics_default_on_fails(self) -> None:
        self.dockerfile_with_requester_diagnostics(
            "ARG REQUESTER_DIAGNOSTICS=off", "ARG REQUESTER_DIAGNOSTICS=on"
        )
        self.assert_requester_diagnostics_violation()

    def test_requester_diagnostics_off_stage_must_stay_empty(self) -> None:
        self.dockerfile_with_requester_diagnostics(
            "FROM runtime AS runtime-requester-diagnostics-off",
            "FROM runtime AS runtime-requester-diagnostics-off\nENV EXTRA=1",
        )
        self.assert_requester_diagnostics_violation()

    def test_final_stage_must_be_selected_by_the_build_argument(self) -> None:
        self.dockerfile_with_requester_diagnostics(
            "FROM runtime-requester-diagnostics-${REQUESTER_DIAGNOSTICS}",
            "FROM runtime-requester-diagnostics-on",
        )
        self.assert_requester_diagnostics_violation()

    def test_builder_refuses_a_production_diagnostic_image(self) -> None:
        with self.assertRaises(SystemExit) as raised:
            build_runtime_image.validate_requester_diagnostics(
                "ghcr.io/finitecomputer/agent-runtime:2026-09-28.diagnostic.1",
                requester_diagnostics=True,
                publish_production=True,
            )
        self.assertIn("non-production canary", str(raised.exception))

    def test_builder_refuses_a_diagnostic_image_without_diagnostic_version(
        self,
    ) -> None:
        for image_ref in (
            "ghcr.io/finitecomputer/agent-runtime:2026-09-28.1",
            "ghcr.io/finitecomputer/agent-runtime",
            "registry.example:5000/diagnostic/agent-runtime:2026-09-28.1",
        ):
            with (
                self.subTest(image_ref=image_ref),
                self.assertRaises(SystemExit) as raised,
            ):
                build_runtime_image.validate_requester_diagnostics(
                    image_ref, requester_diagnostics=True, publish_production=False
                )
            self.assertIn("diagnostic", str(raised.exception))

    def test_builder_accepts_normal_builds_and_diagnostic_canaries(self) -> None:
        validate = build_runtime_image.validate_requester_diagnostics
        validate(
            "ghcr.io/finitecomputer/agent-runtime:2026-09-28.1",
            requester_diagnostics=False,
            publish_production=True,
        )
        validate(
            "ghcr.io/finitecomputer/agent-runtime:2026-09-28.brain-requester-diagnostic.1",
            requester_diagnostics=True,
            publish_production=False,
        )
        self.assertEqual(
            build_runtime_image.requester_diagnostics_build_args(False),
            ["--build-arg", "REQUESTER_DIAGNOSTICS=off"],
        )
        self.assertEqual(
            build_runtime_image.requester_diagnostics_build_args(True),
            ["--build-arg", "REQUESTER_DIAGNOSTICS=on"],
        )

    def test_builder_main_refuses_before_building(self) -> None:
        args = build_runtime_image.argparse.Namespace(
            engine="depot",
            image_ref="ghcr.io/finitecomputer/agent-runtime:2026-09-28.diagnostic.1",
            context_dir=None,
            platform="linux/amd64",
            no_cache=False,
            push=False,
            save=False,
            metadata_file=None,
            report=None,
            requester_diagnostics=True,
            publish_production=True,
        )
        with (
            mock.patch.object(build_runtime_image, "parse_args", return_value=args),
            mock.patch.object(build_runtime_image, "build_image") as build,
            mock.patch.object(build_runtime_image, "repo_metadata") as metadata,
            self.assertRaises(SystemExit),
        ):
            build_runtime_image.main()
        build.assert_not_called()
        metadata.assert_not_called()

    def workflow_step(self, name: str) -> str:
        workflow = (ROOT / CANONICAL_WORKFLOW).read_text(encoding="utf-8")
        steps = {
            step.splitlines()[0].removeprefix("name: "): step
            for step in workflow.split("\n      - ")[1:]
        }
        return steps[name]

    def run_workflow_step(self, name: str, **env: str) -> int:
        step = self.workflow_step(name)
        script = textwrap.dedent(step.split("run: |\n", 1)[1])
        return subprocess.run(
            ["bash", "-e", "-c", script], env=env, capture_output=True, check=False
        ).returncode

    def test_workflow_refuses_diagnostic_production_and_unmarked_versions(self) -> None:
        workflow = (ROOT / CANONICAL_WORKFLOW).read_text(encoding="utf-8")
        self.assertIn(
            "      requester_diagnostics:\n        description: ",
            workflow,
        )
        guard = "Keep a diagnostic image a non-production canary"
        self.assertLess(
            workflow.index(f"- name: {guard}"),
            workflow.index("- name: Build, save, pull, and validate the runtime image"),
        )
        for diagnostics, production, version, expected in (
            ("false", "true", "2026-09-28.1", 0),
            ("false", "false", "2026-09-28.1", 0),
            ("true", "false", "2026-09-28.brain-requester-diagnostic.1", 0),
            ("true", "true", "2026-09-28.brain-requester-diagnostic.1", 1),
            ("true", "false", "2026-09-28.1", 1),
        ):
            with self.subTest(
                diagnostics=diagnostics, production=production, version=version
            ):
                self.assertEqual(
                    self.run_workflow_step(
                        guard,
                        REQUESTER_DIAGNOSTICS=diagnostics,
                        PUBLISH_PRODUCTION=production,
                        VERSION=version,
                    ),
                    expected,
                )
        promote = self.workflow_step("Promote the same saved build to production tags")
        self.assertIn("inputs.requester_diagnostics != true", promote.splitlines()[2])

    def test_workflow_in_image_assertion_matches_the_input(self) -> None:
        build = self.workflow_step("Build, save, pull, and validate the runtime image")
        self.assertIn(
            "REQUESTER_DIAGNOSTICS: ${{ inputs.requester_diagnostics }}", build
        )
        self.assertIn(
            '-e "EXPECTED_REQUESTER_DIAGNOSTICS=$expected_requester_diagnostics"', build
        )
        assertion = 'test "${FINITECHAT_REQUESTER_DIAGNOSTICS-absent}" = "$EXPECTED_REQUESTER_DIAGNOSTICS"'
        self.assertIn(assertion, build)
        self.assertIn(assertion, CANONICAL_WORKFLOW_ANCHORS)
        self.assertIn(
            '{{index .Config.Labels "computer.finite.runtime.diagnostic"}}\')" '
            '= "$expected_diagnostic_label"',
            build,
        )
        # The expectation block the step computes from the input.
        prelude = build.split("run: |\n", 1)[1].split("finitecomputer-v2/scripts/", 1)[
            0
        ]
        for diagnostics, flag, expected, label in (
            ("false", None, "absent", ""),
            ("true", "1", "1", "brain-requester-lease"),
        ):
            probe = (
                textwrap.dedent(prelude).replace(
                    'test -z "$(git status --porcelain)"', ""
                )
                + 'test "$expected_requester_diagnostics" = "$WANT_EXPECTED"\n'
                + 'test "$expected_diagnostic_label" = "$WANT_LABEL"\n'
                + 'case " ${build_flags[*]-} " in *" --requester-diagnostics "*) on=true ;; *) on=false ;; esac\n'
                + 'test "$on" = "$REQUESTER_DIAGNOSTICS"\n'
            )
            env = {
                "REQUESTER_DIAGNOSTICS": diagnostics,
                "PUBLISH_PRODUCTION": "false",
                "DEPOT_PROJECT_ID": "project",
                "IMAGE": "image",
                "VERSION": "v",
                "WANT_EXPECTED": expected,
                "WANT_LABEL": label,
            }
            with self.subTest(diagnostics=diagnostics):
                result = subprocess.run(
                    ["bash", "-e", "-c", probe],
                    env=env,
                    capture_output=True,
                    check=False,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                container_env = {"EXPECTED_REQUESTER_DIAGNOSTICS": expected}
                if flag is not None:
                    container_env["FINITECHAT_REQUESTER_DIAGNOSTICS"] = flag
                self.assertEqual(
                    subprocess.run(
                        ["sh", "-ec", assertion], env=container_env
                    ).returncode,
                    0,
                )
                mismatched = {**container_env, "FINITECHAT_REQUESTER_DIAGNOSTICS": "1"}
                if flag is None:
                    self.assertNotEqual(
                        subprocess.run(
                            ["sh", "-ec", assertion], env=mismatched
                        ).returncode,
                        0,
                    )

    def test_smoke_report_is_uploaded_on_failure_by_itself(self) -> None:
        workflow = (ROOT / CANONICAL_WORKFLOW).read_text(encoding="utf-8")
        steps = {
            step.splitlines()[0].removeprefix("name: "): step
            for step in workflow.split("\n      - ")[1:]
            if "actions/upload-artifact" in step
        }
        self.assertNotIn("if:", steps["Upload build report"])
        failure_upload = steps["Upload durable smoke failure report"]
        self.assertIn("if: failure()", failure_upload)
        paths = failure_upload.split("path:", 1)[1].split()
        self.assertEqual(
            paths[0], "finitechat/target/runtime-image-durable-smoke/report.json"
        )
        self.assertEqual(paths[1:], ["if-no-files-found:", "ignore"])

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
        self.write(
            workflow,
            "name: Phala image\nrun: docker build -f deploy/phala/Dockerfile .",
        )
        self.assertTrue(
            any("cannot build/publish" in item for item in self.violations())
        )
        self.write(
            workflow,
            "name: Phala image\nrun: depot build -f deploy/phala/Dockerfile .",
        )
        self.assertTrue(
            any("cannot build/publish" in item for item in self.violations())
        )

    def test_second_agent_runtime_publisher_fails(self) -> None:
        self.write(
            ".github/workflows/runtime-backup-publisher.yml",
            "name: backup\nrun: docker push ghcr.io/example/agent-runtime:latest",
        )
        self.assertTrue(
            any("sole Agent Runtime publisher" in item for item in self.violations())
        )
        self.write(
            ".github/workflows/runtime-backup-publisher.yml",
            "name: backup\nrun: depot build --push -t ghcr.io/example/agent-runtime:latest .",
        )
        self.assertTrue(
            any("sole Agent Runtime publisher" in item for item in self.violations())
        )

    def test_mutable_phala_image_fails_and_digest_passes(self) -> None:
        config = Path("infra/phala-worker.yml")
        self.write(config, "runner: phala\nimage: ghcr.io/example/agent-runtime:latest")
        self.assertTrue(
            any("mutable Phala Runtime image" in item for item in self.violations())
        )
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
                self.assertTrue(
                    any("cannot override" in item for item in self.violations())
                )

    def test_missing_digest_guard_fails(self) -> None:
        self.write(PHALA_ADAPTER, "impl PhalaConfig { fn validate(&self) {} }")
        self.assertTrue(any("reject mutable" in item for item in self.violations()))


BUILDER_SCRIPT = ROOT / "finitecomputer-v2/scripts/build_runtime_image.py"
builder_spec = importlib.util.spec_from_file_location(
    "build_runtime_image", BUILDER_SCRIPT
)
assert builder_spec is not None and builder_spec.loader is not None
build_runtime_image = importlib.util.module_from_spec(builder_spec)
sys.modules[builder_spec.name] = build_runtime_image
builder_spec.loader.exec_module(build_runtime_image)


class RuntimeImageBuildContextTests(unittest.TestCase):
    """The staged build context must never carry the repo .dockerignore.

    stage_repo copies the monorepo into the context ROOT, where a copied
    .dockerignore becomes active for the builder. Its `**/node_modules` rule
    then strips the vendored node_modules inside the staged Nix store
    (.finite-hermes-nix-store), breaking npm/npx and the Playwright CLI at
    image build time (first #525 image build, 2026-08-18).
    """

    def test_dockerignore_is_excluded_from_staged_context(self) -> None:
        self.assertIn(".dockerignore", build_runtime_image.BUILD_EXCLUDES)

    def test_stage_repo_drops_dockerignore_and_repo_node_modules(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "repo"
            context = Path(temp) / "ctx"
            source.mkdir()
            (source / ".dockerignore").write_text("**/node_modules\n", encoding="utf-8")
            (source / "apps/web/node_modules/leftpad").mkdir(parents=True)
            (source / "apps/web/node_modules/leftpad/index.js").write_text(
                "//\n", encoding="utf-8"
            )
            (source / "package.json").write_text("{}\n", encoding="utf-8")

            build_runtime_image.stage_repo(source, context)

            self.assertFalse((context / ".dockerignore").exists())
            self.assertFalse((context / "apps/web/node_modules").exists())
            self.assertTrue((context / "package.json").is_file())

        # The Nix store is staged AFTER stage_repo by stage_store_paths with a
        # plain `rsync -a` (no exclude list), so its vendored node_modules
        # survive — provided no .dockerignore in the context root re-excludes
        # them at build time, which the first assertion pins.


if __name__ == "__main__":
    unittest.main()

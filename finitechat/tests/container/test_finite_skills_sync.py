from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

MONOREPO_ROOT = Path(__file__).resolve().parents[3]
FINITE = MONOREPO_ROOT / "finitechat/containers/agent/finite.py"
RUNTIME_DOCKERFILE = (
    MONOREPO_ROOT / "finitecomputer-v2/deploy/finite-computer/images/runtime.Dockerfile"
)
HEALTHCHECK = (
    MONOREPO_ROOT / "finitecomputer-v2/deploy/finite-computer/runtime-template/healthcheck.sh"
)
RUNTIME_IMAGE_WORKFLOW = MONOREPO_ROOT / ".github/workflows/runtime-image.yml"
GOOGLE_WORKSPACE_SKILL = MONOREPO_ROOT / "finite-skills/skills/productivity/google-workspace-finite"
GOOGLE_WORKSPACE_DASHBOARD_SCOPES = (
    MONOREPO_ROOT / "finitecomputer-v2/apps/dashboard/src/contracts/google-workspace-scopes.json"
)


def write_skill(root: Path, relative: str, name: str, marker: str) -> None:
    skill = root / relative / "SKILL.md"
    skill.parent.mkdir(parents=True, exist_ok=True)
    _ = skill.write_text(
        f"---\nname: {name}\ndescription: Test fixture for {name}.\n---\n\n{marker}\n",
        encoding="utf-8",
    )


def write_valid_bundle(root: Path, marker: str) -> None:
    write_skill(
        root,
        "software-development/finitebrain",
        "finitebrain",
        marker,
    )
    write_skill(
        root,
        "software-development/finite-sites-publishing-finite",
        "finite-sites-publishing-finite",
        marker,
    )


def run_sync(
    source: Path, agent_home: Path, *, failpoint: str | None = None
) -> subprocess.CompletedProcess[str]:
    env = {
        **os.environ,
        "FINITE_SKILLS_SYNC_TESTING": "1",
        "FINITE_SKILLS_SYNC_TEST_SOURCE": str(source),
        "FINITE_SKILLS_SYNC_TEST_AGENT_HOME": str(agent_home),
    }
    if failpoint is not None:
        env["FINITE_SKILLS_SYNC_TEST_FAILPOINT"] = failpoint
    return subprocess.run(
        ["python3", str(FINITE), "skills", "sync"],
        env=env,
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )


class FiniteSkillsSyncTest(unittest.TestCase):
    def test_success_atomically_replaces_only_the_managed_baseline(self) -> None:
        with tempfile.TemporaryDirectory() as raw_tmp:
            tmp = Path(raw_tmp)
            source = tmp / "image-bundle"
            agent_home = tmp / "agent"
            current = agent_home / "managed-skills/finite/current"
            write_valid_bundle(source, "image-v2")
            write_valid_bundle(current, "installed-v1")
            _ = (current / "removed-by-sync.txt").write_text("old\n", encoding="utf-8")
            user_skill = agent_home / "hermes-home/skills/my-skill/SKILL.md"
            user_skill.parent.mkdir(parents=True)
            _ = user_skill.write_text("user-owned\n", encoding="utf-8")

            result = run_sync(source, agent_home)

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("Finite Skills synced from this Runtime image", result.stdout)
            self.assertIn("/reload-skills", result.stdout)
            self.assertIn(
                "image-v2",
                (
                    current / "software-development/finite-sites-publishing-finite/SKILL.md"
                ).read_text(encoding="utf-8"),
            )
            self.assertFalse((current / "removed-by-sync.txt").exists())
            self.assertEqual(user_skill.read_text(encoding="utf-8"), "user-owned\n")
            self.assertEqual(
                list((agent_home / "managed-skills/finite").glob(".skills-sync-*")),
                [],
            )

    def test_invalid_bundle_leaves_prior_baseline_and_user_skill_unchanged(self) -> None:
        with tempfile.TemporaryDirectory() as raw_tmp:
            tmp = Path(raw_tmp)
            source = tmp / "invalid-image-bundle"
            agent_home = tmp / "agent"
            current = agent_home / "managed-skills/finite/current"
            write_skill(source, "software-development/finitebrain", "finitebrain", "bad")
            write_valid_bundle(current, "installed-v1")
            user_skill = agent_home / "hermes-home/skills/my-skill/SKILL.md"
            user_skill.parent.mkdir(parents=True)
            _ = user_skill.write_text("user-owned\n", encoding="utf-8")

            result = run_sync(source, agent_home)

            self.assertEqual(result.returncode, 1)
            self.assertIn("missing required skill", result.stderr)
            self.assertIn(
                "installed-v1",
                (
                    current / "software-development/finite-sites-publishing-finite/SKILL.md"
                ).read_text(encoding="utf-8"),
            )
            self.assertEqual(user_skill.read_text(encoding="utf-8"), "user-owned\n")

    def test_ordinary_failure_after_swap_restores_prior_baseline(self) -> None:
        with tempfile.TemporaryDirectory() as raw_tmp:
            tmp = Path(raw_tmp)
            source = tmp / "image-bundle"
            agent_home = tmp / "agent"
            current = agent_home / "managed-skills/finite/current"
            write_valid_bundle(source, "image-v2")
            write_valid_bundle(current, "installed-v1")

            result = run_sync(source, agent_home, failpoint="after_swap")

            self.assertEqual(result.returncode, 1)
            self.assertIn("injected test failure", result.stderr)
            self.assertIn(
                "installed-v1",
                (
                    current / "software-development/finite-sites-publishing-finite/SKILL.md"
                ).read_text(encoding="utf-8"),
            )
            self.assertNotIn(
                "image-v2",
                (
                    current / "software-development/finite-sites-publishing-finite/SKILL.md"
                ).read_text(encoding="utf-8"),
            )

    def test_source_override_is_rejected_outside_explicit_test_mode(self) -> None:
        with tempfile.TemporaryDirectory() as raw_tmp:
            env = {
                **os.environ,
                "FINITE_SKILLS_SYNC_TEST_SOURCE": raw_tmp,
            }
            _ = env.pop("FINITE_SKILLS_SYNC_TESTING", None)

            result = subprocess.run(
                ["python3", str(FINITE), "skills", "sync"],
                env=env,
                capture_output=True,
                text=True,
                timeout=30,
                check=False,
            )

            self.assertEqual(result.returncode, 1)
            self.assertIn("test-only skills sync overrides require", result.stderr)

    def test_managed_path_cannot_redirect_sync_into_user_skills(self) -> None:
        with tempfile.TemporaryDirectory() as raw_tmp:
            tmp = Path(raw_tmp)
            source = tmp / "image-bundle"
            agent_home = tmp / "agent"
            user_skills = agent_home / "hermes-home/skills"
            user_skills.mkdir(parents=True)
            user_marker = user_skills / "user-owned.txt"
            _ = user_marker.write_text("keep me\n", encoding="utf-8")
            managed_root = agent_home / "managed-skills"
            managed_root.parent.mkdir(parents=True, exist_ok=True)
            managed_root.symlink_to(user_skills, target_is_directory=True)
            write_valid_bundle(source, "image-v2")

            result = run_sync(source, agent_home)

            self.assertEqual(result.returncode, 1)
            self.assertIn("managed-skills path is not a directory", result.stderr)
            self.assertEqual(user_marker.read_text(encoding="utf-8"), "keep me\n")
            self.assertFalse((user_skills / "finite").exists())

    def test_runtime_image_packages_one_finite_utility(self) -> None:
        dockerfile = RUNTIME_DOCKERFILE.read_text(encoding="utf-8")
        healthcheck = HEALTHCHECK.read_text(encoding="utf-8")

        self.assertIn(
            "COPY finitechat/containers/agent/finite.py /runtime/bin/finite",
            dockerfile,
        )
        self.assertIn(
            "ln -sf /runtime/bin/finite /usr/local/bin/finite",
            dockerfile,
        )
        self.assertNotIn("/runtime/bin/finite", healthcheck)


class GoogleWorkspaceSkillPackagingTest(unittest.TestCase):
    def test_scope_contract_ships_with_skill_and_matches_dashboard_copy(self) -> None:
        skill_scopes = json.loads(
            (GOOGLE_WORKSPACE_SKILL / "references/google-workspace-scopes.json").read_text(
                encoding="utf-8"
            )
        )
        dashboard_scopes = json.loads(GOOGLE_WORKSPACE_DASHBOARD_SCOPES.read_text(encoding="utf-8"))

        self.assertEqual(skill_scopes, dashboard_scopes)
        self.assertGreater(len(skill_scopes), 0)

    def test_detached_synced_skill_loads_scope_contract_and_writes_private_files(self) -> None:
        with tempfile.TemporaryDirectory() as raw_tmp:
            tmp = Path(raw_tmp)
            detached = tmp / "google-workspace-finite"
            shutil.copytree(GOOGLE_WORKSPACE_SKILL, detached)
            hermes_home = tmp / "hermes-home"
            env = {**os.environ, "HERMES_HOME": str(hermes_home)}
            env.pop("FC_PROFILE_ASSETS_ROOT", None)

            check = subprocess.run(
                [sys.executable, str(detached / "scripts/setup.py"), "--check"],
                env=env,
                capture_output=True,
                text=True,
                timeout=15,
                check=False,
            )
            api_help = subprocess.run(
                [sys.executable, str(detached / "scripts/google_api.py"), "--help"],
                env=env,
                capture_output=True,
                text=True,
                timeout=15,
                check=False,
            )
            client_secret = tmp / "client-secret.json"
            client_secret.write_text(
                json.dumps({"installed": {"client_id": "test-client"}}),
                encoding="utf-8",
            )
            store = subprocess.run(
                [
                    sys.executable,
                    str(detached / "scripts/setup.py"),
                    "--client-secret",
                    str(client_secret),
                ],
                env=env,
                capture_output=True,
                text=True,
                timeout=15,
                check=False,
            )

            self.assertEqual(check.returncode, 1, check.stderr)
            self.assertIn("NOT_AUTHENTICATED", check.stdout)
            self.assertEqual(api_help.returncode, 0, api_help.stderr)
            self.assertEqual(store.returncode, 0, store.stderr)
            stored_secret = hermes_home / "google_client_secret.json"
            self.assertEqual(stored_secret.stat().st_mode & 0o777, 0o600)

    def test_runtime_pins_and_import_checks_google_clients(self) -> None:
        dockerfile = RUNTIME_DOCKERFILE.read_text(encoding="utf-8")
        runtime_image_workflow = RUNTIME_IMAGE_WORKFLOW.read_text(encoding="utf-8")
        setup = (GOOGLE_WORKSPACE_SKILL / "scripts/setup.py").read_text(encoding="utf-8")
        requirements = (
            "google-api-python-client==2.198.0",
            "google-auth-oauthlib==1.4.0",
            "google-auth-httplib2==0.4.0",
        )

        for requirement in requirements:
            self.assertIn(requirement, dockerfile)
            self.assertIn(requirement, setup)
        for module in ("googleapiclient", "google_auth_oauthlib", "google_auth_httplib2"):
            self.assertIn(module, runtime_image_workflow)
        ensure_deps = setup.split("def _ensure_deps():", 1)[1].split("def check_auth():", 1)[0]
        self.assertNotIn("install_deps()", ensure_deps)


if __name__ == "__main__":
    _ = unittest.main()

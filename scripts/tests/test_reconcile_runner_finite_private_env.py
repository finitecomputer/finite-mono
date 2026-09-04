from __future__ import annotations

from pathlib import Path
import os
import stat
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
RECONCILE = ROOT / "scripts" / "reconcile-runner-finite-private-env"

RETIRED_ENV = """\
FC_CORE_RUNNER_API_TOKEN=synthetic-secret-value
FC_RUNNER_RUNTIME_ARTIFACT_ID=finite-agent-runtime-test
FC_RUNNER_FINITE_PRIVATE_BASE_URL=https://kimi-k2-6.finite.containers.tinfoil.dev/v1
FC_RUNNER_FINITE_PRIVATE_MODEL=deepseek-v4-flash-0731
FC_RUNNER_DRAIN=false
"""


class ReconcileRunnerFinitePrivateEnvTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.runner_env = self.root / "runner.env"
        self.runner_env.write_text(RETIRED_ENV, encoding="utf-8")
        self.runner_env.chmod(0o600)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_reconcile(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(RECONCILE), *arguments, str(self.runner_env)],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )

    def test_check_reports_exact_retired_pair_without_reading_secrets(self) -> None:
        before = self.runner_env.read_bytes()
        mode = stat.S_IMODE(self.runner_env.stat().st_mode)

        result = self.run_reconcile("--check")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "needs-migration\n")
        self.assertNotIn("synthetic-secret-value", result.stdout + result.stderr)
        self.assertEqual(self.runner_env.read_bytes(), before)
        self.assertEqual(stat.S_IMODE(self.runner_env.stat().st_mode), mode)

    def test_apply_removes_only_retired_overrides_and_creates_rollback_copy(
        self,
    ) -> None:
        before = self.runner_env.read_bytes()
        before_stat = self.runner_env.stat()

        result = self.run_reconcile("--apply")

        self.assertEqual(result.returncode, 0, result.stderr)
        backup = Path(f"{self.runner_env}.pre-glm53-route")
        self.assertEqual(backup.read_bytes(), before)
        self.assertEqual(
            self.runner_env.read_text(encoding="utf-8"),
            """\
FC_CORE_RUNNER_API_TOKEN=synthetic-secret-value
FC_RUNNER_RUNTIME_ARTIFACT_ID=finite-agent-runtime-test
FC_RUNNER_DRAIN=false
""",
        )
        after_stat = self.runner_env.stat()
        self.assertEqual(stat.S_IMODE(after_stat.st_mode), 0o600)
        self.assertEqual(after_stat.st_uid, before_stat.st_uid)
        self.assertEqual(after_stat.st_gid, before_stat.st_gid)
        self.assertNotIn("synthetic-secret-value", result.stdout + result.stderr)
        self.assertIn(str(backup), result.stdout)

    def test_apply_is_a_no_op_when_operator_file_has_no_route_overrides(self) -> None:
        clean = """\
FC_CORE_RUNNER_API_TOKEN=synthetic-secret-value
FC_RUNNER_RUNTIME_ARTIFACT_ID=finite-agent-runtime-test
FC_RUNNER_DRAIN=false
"""
        self.runner_env.write_text(clean, encoding="utf-8")

        result = self.run_reconcile("--apply")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "clean\n")
        self.assertEqual(self.runner_env.read_text(encoding="utf-8"), clean)
        self.assertFalse(Path(f"{self.runner_env}.pre-glm53-route").exists())
        self.assertFalse(Path(f"{self.runner_env}.reconcile-lock").exists())

    def test_ambiguous_override_states_fail_closed_without_secret_output(self) -> None:
        cases = {
            "partial": RETIRED_ENV.replace(
                "FC_RUNNER_FINITE_PRIVATE_MODEL=deepseek-v4-flash-0731\n", ""
            ),
            "custom": RETIRED_ENV.replace(
                "https://kimi-k2-6.finite.containers.tinfoil.dev/v1",
                "https://custom.example/v1",
            ),
            "duplicate": RETIRED_ENV
            + "FC_RUNNER_FINITE_PRIVATE_MODEL=deepseek-v4-flash-0731\n",
        }
        for name, contents in cases.items():
            with self.subTest(name=name):
                self.runner_env.write_text(contents, encoding="utf-8")
                before = self.runner_env.read_bytes()

                result = self.run_reconcile("--apply")

                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(self.runner_env.read_bytes(), before)
                self.assertNotIn(
                    "synthetic-secret-value", result.stdout + result.stderr
                )
                self.assertFalse(
                    Path(f"{self.runner_env}.pre-glm53-route").exists()
                )

    def test_unsafe_file_and_rollback_states_fail_closed(self) -> None:
        cases = ("symlink", "hardlink", "existing-backup", "existing-lock")
        for name in cases:
            with self.subTest(name=name):
                case_root = self.root / name
                case_root.mkdir()
                target = case_root / "runner.env"
                target.write_text(RETIRED_ENV, encoding="utf-8")
                self.runner_env = target
                if name == "symlink":
                    link = case_root / "runner-link.env"
                    link.symlink_to(target)
                    self.runner_env = link
                elif name == "hardlink":
                    os.link(target, case_root / "runner.env.other-link")
                elif name == "existing-backup":
                    Path(f"{target}.pre-glm53-route").write_text(
                        "existing rollback evidence\n", encoding="utf-8"
                    )
                else:
                    Path(f"{target}.reconcile-lock").mkdir()
                before = target.read_bytes()

                result = self.run_reconcile("--apply")

                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(target.read_bytes(), before)
                self.assertNotIn(
                    "synthetic-secret-value", result.stdout + result.stderr
                )
                if name == "existing-lock":
                    self.assertTrue(Path(f"{target}.reconcile-lock").is_dir())

    def test_apply_preserves_an_unrelated_final_line_without_a_newline(self) -> None:
        contents = RETIRED_ENV.rstrip("\n")
        self.runner_env.write_text(contents, encoding="utf-8")

        result = self.run_reconcile("--apply")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            self.runner_env.read_bytes(),
            b"FC_CORE_RUNNER_API_TOKEN=synthetic-secret-value\n"
            b"FC_RUNNER_RUNTIME_ARTIFACT_ID=finite-agent-runtime-test\n"
            b"FC_RUNNER_DRAIN=false",
        )


if __name__ == "__main__":
    unittest.main()

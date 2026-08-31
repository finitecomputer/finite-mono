"""Unit checks for the container entrypoint."""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
ENTRYPOINT = REPO_ROOT / "containers" / "agent" / "entrypoint.sh"


class AgentEntrypointTest(unittest.TestCase):
    def test_runs_command_without_restore(self) -> None:
        result = subprocess.run(
            [str(ENTRYPOINT), "sh", "-c", "echo command-ran"],
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertIn("command-ran", result.stdout)

    def test_finite_home_defaults_to_agent_home(self) -> None:
        # The shared Finite identity (identity/identity.json) must land on the
        # durable agent mount so the account key survives restarts.
        with tempfile.TemporaryDirectory() as tmp_value:
            home = Path(tmp_value) / "agent"
            env = os.environ.copy()
            env.pop("FINITE_HOME", None)
            env["FINITECHAT_HOME"] = str(home)
            result = subprocess.run(
                [str(ENTRYPOINT), "sh", "-c", 'echo "finite-home=$FINITE_HOME"'],
                capture_output=True,
                text=True,
                env=env,
                check=True,
            )
        self.assertIn(f"finite-home={home}", result.stdout)

    def test_finite_home_override_is_honored(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_value:
            tmp = Path(tmp_value)
            env = os.environ.copy()
            env.update(
                {
                    "FINITECHAT_HOME": str(tmp / "agent"),
                    "FINITE_HOME": str(tmp / "identity-home"),
                }
            )
            result = subprocess.run(
                [str(ENTRYPOINT), "sh", "-c", 'echo "finite-home=$FINITE_HOME"'],
                capture_output=True,
                text=True,
                env=env,
                check=True,
            )
        self.assertIn(f"finite-home={tmp / 'identity-home'}", result.stdout)


if __name__ == "__main__":
    unittest.main()

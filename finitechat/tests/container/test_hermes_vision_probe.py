from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
PROBE = REPO_ROOT / "containers/agent/probe_hermes_vision.py"
MARKER = "FINITE_AEON_HERMES_PROBE "


class HermesVisionProbeTest(unittest.TestCase):
    def run_probe(
        self,
        result: dict[str, object],
        *,
        video_admitted: bool = True,
    ) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as raw_tmp:
            root = Path(raw_tmp)
            tools = root / "tools"
            tools.mkdir()
            (tools / "__init__.py").write_text("", encoding="utf-8")
            (tools / "vision_tools.py").write_text(
                "import json\n"
                f"RESULT = {result!r}\n"
                "async def vision_analyze_tool(image_url, prompt):\n"
                "    assert image_url.startswith('data:image/png;base64,')\n"
                "    assert 'uppercase color word' in prompt\n"
                "    return json.dumps(RESULT)\n",
                encoding="utf-8",
            )
            hermes_cli = root / "hermes_cli"
            hermes_cli.mkdir()
            (hermes_cli / "__init__.py").write_text("", encoding="utf-8")
            (hermes_cli / "config.py").write_text(
                "def load_config():\n    return {}\n",
                encoding="utf-8",
            )
            (hermes_cli / "plugins.py").write_text(
                "def discover_plugins():\n    return None\n",
                encoding="utf-8",
            )
            (hermes_cli / "tools_config.py").write_text(
                "def _get_platform_tools(config, platform):\n"
                "    assert platform == 'finitechat'\n"
                "    return ['hermes-cli', 'video']\n",
                encoding="utf-8",
            )
            (root / "model_tools.py").write_text(
                "VIDEO_ADMITTED = " + repr(video_admitted) + "\n"
                "def get_tool_definitions(*, enabled_toolsets, quiet_mode):\n"
                "    assert enabled_toolsets == ['hermes-cli', 'video']\n"
                "    assert quiet_mode is True\n"
                "    return ([{'function': {'name': 'video_analyze'}}] "
                "if VIDEO_ADMITTED else [])\n",
                encoding="utf-8",
            )
            env = os.environ.copy()
            env["PYTHONPATH"] = str(root)
            return subprocess.run(
                [sys.executable, str(PROBE)],
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

    def test_exact_red_passes(self) -> None:
        completed = self.run_probe({"success": True, "analysis": "RED"})
        self.assertEqual(completed.returncode, 0)
        payload = json.loads(completed.stdout.removeprefix(MARKER))
        self.assertEqual(
            payload,
            {
                "success": True,
                "analysis": "RED",
                "video_analyze": True,
                "hermes_version": "unknown",
            },
        )

    def test_wrong_hermes_semantics_fail_without_echoing_details(self) -> None:
        completed = self.run_probe({"success": True, "analysis": "BLUE secret"})
        self.assertNotEqual(completed.returncode, 0)
        payload = json.loads(completed.stdout.removeprefix(MARKER))
        self.assertEqual(
            payload,
            {
                "success": False,
                "analysis": None,
                "video_analyze": True,
                "hermes_version": "unknown",
            },
        )
        self.assertNotIn("secret", completed.stdout)

    def test_missing_native_video_tool_fails_admission(self) -> None:
        completed = self.run_probe(
            {"success": True, "analysis": "RED"},
            video_admitted=False,
        )
        self.assertNotEqual(completed.returncode, 0)
        payload = json.loads(completed.stdout.removeprefix(MARKER))
        self.assertEqual(payload["success"], False)
        self.assertEqual(payload["analysis"], None)
        self.assertEqual(payload["video_analyze"], False)


if __name__ == "__main__":
    unittest.main()

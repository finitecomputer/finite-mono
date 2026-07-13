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
    def run_probe(self, result: dict[str, object]) -> subprocess.CompletedProcess[str]:
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
        self.assertEqual(payload, {"success": True, "analysis": "RED"})

    def test_wrong_hermes_semantics_fail_without_echoing_details(self) -> None:
        completed = self.run_probe({"success": True, "analysis": "BLUE secret"})
        self.assertNotEqual(completed.returncode, 0)
        payload = json.loads(completed.stdout.removeprefix(MARKER))
        self.assertEqual(payload, {"success": False, "analysis": None})
        self.assertNotIn("secret", completed.stdout)


if __name__ == "__main__":
    unittest.main()

"""Sequential real-version state compatibility, never a live-writer downgrade.

Run with HERMES_BASELINE_PYTHON pointing at the previous pinned interpreter.
The current interpreter must be the candidate Hermes environment. Every stage
uses a fresh process and a synthetic HERMES_HOME. Passing proves only the state
shapes below, not rollback of new feature state or concurrent mixed writers.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

STAGE = r"""
import json
import os
from pathlib import Path

from gateway.config import GatewayConfig, Platform, load_gateway_config
from gateway.pairing import PairingStore
from gateway.session import SessionSource, SessionStore
from hermes_cli.config import load_config
from hermes_state import SCHEMA_VERSION

home = Path(os.environ["HERMES_HOME"])
stage = os.environ["UPGRADE_STAGE"]
store = SessionStore(home / "sessions", GatewayConfig())
manifest_path = home / "contract-manifest.json"
manifest = json.loads(manifest_path.read_text()) if manifest_path.exists() else {}
pairing = PairingStore()
if stage == "baseline":
    code = pairing.generate_code("telegram", "101", "Synthetic owner")
    assert code and pairing.approve_code("telegram", code)
    code = pairing.generate_code("simplex", "7", "Synthetic pending")
    assert code
    manifest["pending_code"] = code

config_bytes = (home / "config.yaml").read_bytes()
config = load_config()
assert config["model"]["provider"] == "custom"
assert config["model"]["base_url"] == "https://example.invalid/v1"
assert config["model"]["api_key"] == "${SYNTHETIC_INFERENCE_KEY}"
gateway = load_gateway_config()
assert gateway.platforms[Platform.TELEGRAM].enabled
assert (home / "config.yaml").read_bytes() == config_bytes
assert (home / ".env").read_text() == "SYNTHETIC_INFERENCE_KEY=synthetic-not-a-credential\n"
assert pairing.is_approved("telegram", "101")
assert not pairing.is_approved("telegram", "102")
assert not pairing.is_approved("simplex", "101")
if stage == "candidate":
    assert pairing.approve_code("simplex", manifest["pending_code"])
if stage not in ("baseline", "backup_reopen"):
    assert pairing.is_approved("simplex", "7")

routes = {}
for platform, thread in [(Platform.LOCAL, "chat-a"), (Platform.LOCAL, "chat-b"),
                         (Platform.TELEGRAM, None)]:
    label = platform.value + ":" + str(thread)
    source = SessionSource(platform=platform, chat_id="room-1", user_id="101",
                           thread_id=thread, chat_type="dm")
    entry = store.get_or_create_session(source)
    routes[label] = entry.session_id
    if stage != "baseline":
        assert entry.session_id == manifest["routes"][label], (stage, label, entry.session_id)
    transcript = store.load_transcript(entry.session_id)
    expected_stages = {"baseline": [], "candidate": ["baseline"],
                       "old_reopen": ["baseline", "candidate"], "backup_reopen": ["baseline"],
                       "candidate_reopen": ["baseline", "candidate", "old_reopen"]}[stage]
    assert [(m["role"], m["content"]) for m in transcript] == [
        pair for prior in expected_stages for pair in
        [("user", prior + " question " + label), ("assistant", prior + " answer " + label)]
    ], (stage, label, transcript)
    for role, text in [("user", " question "), ("assistant", " answer ")]:
        store.append_to_transcript(entry.session_id,
            {"role": role, "content": stage + text + label,
             "platform_message_id": stage + "-" + role + "-" + label})
assert len(set(routes.values())) == len(routes)
manifest["routes"] = routes
manifest_path.write_text(json.dumps(manifest))
# Use Hermes' opened handle: upstream's database isolation marker deliberately
# rejects unprepared raw SQLite readers, including a stock sqlite3 connection.
version = store._db._conn.execute("SELECT version FROM schema_version").fetchone()[0]
assert version == (25 if stage in ("baseline", "backup_reopen") else 26), version
assert store._db._conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
print("CONTRACT_RESULT=" + json.dumps({"schema": SCHEMA_VERSION, "routes": routes}))
"""


class PinnedHermesUpgradeStateTests(unittest.TestCase):
    def test_old_state_upgrade_and_sequential_old_reopen(self):
        baseline = os.environ.get("HERMES_BASELINE_PYTHON")
        if not baseline:
            self.skipTest("set HERMES_BASELINE_PYTHON to test the previous pin")
        self.assertTrue(Path(baseline).is_file(), "configured baseline interpreter is missing")
        with tempfile.TemporaryDirectory(prefix="hermes-upgrade-contract-") as directory:
            home = Path(directory) / "active"
            home.mkdir()
            backup = Path(directory) / "backup"
            (home / "config.yaml").write_text(
                "model:\n  default: synthetic-model\n  provider: custom\n"
                "  base_url: https://example.invalid/v1\n"
                "  api_key: ${SYNTHETIC_INFERENCE_KEY}\n"
                "gateway:\n  platforms:\n    telegram:\n      enabled: true\n"
                "      extra:\n        fixture: synthetic\n"
            )
            (home / ".env").write_text("SYNTHETIC_INFERENCE_KEY=synthetic-not-a-credential\n")
            results = []
            for stage, executable in [
                ("baseline", baseline),
                ("candidate", sys.executable),
                ("old_reopen", baseline),
                ("candidate_reopen", sys.executable),
                ("backup_reopen", baseline),
            ]:
                stage_home = backup if stage == "backup_reopen" else home
                env = {
                    key: value
                    for key, value in os.environ.items()
                    if not key.startswith(("HERMES_", "TELEGRAM_", "SIMPLEX_"))
                }
                env.update(HOME=str(stage_home), HERMES_HOME=str(stage_home), UPGRADE_STAGE=stage)
                completed = subprocess.run(
                    [executable, "-c", STAGE],
                    env=env,
                    cwd=directory,
                    capture_output=True,
                    text=True,
                    timeout=60,
                )
                self.assertEqual(
                    completed.returncode, 0, f"{stage}: {completed.stdout}\n{completed.stderr}"
                )
                result = next(
                    line.removeprefix("CONTRACT_RESULT=")
                    for line in completed.stdout.splitlines()
                    if line.startswith("CONTRACT_RESULT=")
                )
                results.append(json.loads(result))
                if stage == "baseline":
                    # Stopped-writer backup restored into a separate, empty target.
                    shutil.copytree(home, backup)
            self.assertEqual([result["schema"] for result in results], [25, 26, 25, 26, 25])
            self.assertTrue(all(result["routes"] == results[0]["routes"] for result in results))


if __name__ == "__main__":
    unittest.main()

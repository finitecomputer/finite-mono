import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "ios-local-agent.sh"
STRESS_SEEDER = REPO_ROOT / "scripts" / "seed-local-chat-stress.mjs"
ELECTRON_SCRIPT = REPO_ROOT / "scripts" / "electron-local-agent.sh"


class IOSLocalAgentContractTests(unittest.TestCase):
    def test_binding_canonical_room_becomes_hermes_home_channel(self) -> None:
        script = SCRIPT.read_text(encoding="utf-8")
        ensure_index = script.index("/v1/app/agent-bindings/ensure")
        canonical_index = script.index(".hosted_agent_binding.canonical_room_id")
        setter_index = script.index("/v1/hermes/home-channel-set")

        self.assertLess(ensure_index, canonical_index)
        self.assertLess(canonical_index, setter_index)
        self.assertIn('--arg room_id "${canonical_room_id}"', script)
        self.assertIn("for _ in {1..120}", script)
        self.assertIn(".home_channel.room_id == $room_id", script)

    def test_stress_seed_runs_after_pairing_source_is_ready(self) -> None:
        script = SCRIPT.read_text(encoding="utf-8")
        home_channel_index = script.index('if [[ "${home_channel_ready}" != "1" ]]')
        stress_index = script.index("seed-local-chat-stress.mjs")
        dashboard_index = script.index('echo "Starting the local dashboard..."')

        self.assertLess(home_channel_index, stress_index)
        self.assertLess(stress_index, dashboard_index)
        self.assertIn('kill "${hermes_pid}"', script)
        self.assertIn("FINITECHAT_STRESS_HOSTED_API_TOKEN", script)
        self.assertTrue(STRESS_SEEDER.is_file())

    def test_ports_are_rejected_before_local_services_start(self) -> None:
        script = SCRIPT.read_text(encoding="utf-8")
        preflight_index = script.index('require_port_available "chat server"')
        build_index = script.index('echo "Building local chat services..."')

        self.assertLess(preflight_index, build_index)
        self.assertIn('"/dev/tcp/127.0.0.1/${port}"', script)

    def test_electron_harness_is_isolated_and_uses_the_local_pairing_stack(self) -> None:
        script = ELECTRON_SCRIPT.read_text(encoding="utf-8")

        self.assertIn("FINITECHAT_USER_DATA_DIR", script)
        self.assertIn("FINITECHAT_DISABLE_SINGLE_INSTANCE_LOCK=1", script)
        self.assertIn("http://127.0.0.1:23002", script)
        self.assertIn("http://127.0.0.1:28788", script)
        self.assertIn("cargo build -q -p finitechat-daemon", script)
        self.assertIn("--remote-debugging-address=127.0.0.1", script)


if __name__ == "__main__":
    unittest.main()

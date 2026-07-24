from pathlib import Path
import unittest


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "ios-local-agent.sh"


class IOSLocalAgentContractTests(unittest.TestCase):
    def test_binding_canonical_room_becomes_hermes_home_channel(self) -> None:
        script = SCRIPT.read_text(encoding="utf-8")
        ensure_index = script.index("/v1/app/agent-bindings/ensure")
        canonical_index = script.index(".hosted_agent_binding.canonical_room_id")
        setter_index = script.index("/v1/hermes/home-channel-set")

        self.assertLess(ensure_index, canonical_index)
        self.assertLess(canonical_index, setter_index)
        self.assertIn("--arg room_id \"${canonical_room_id}\"", script)
        self.assertIn("for _ in {1..120}", script)
        self.assertIn(".home_channel.room_id == $room_id", script)


if __name__ == "__main__":
    unittest.main()

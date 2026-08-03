import importlib.util
import unittest
from pathlib import Path

FINITECHAT_ROOT = Path(__file__).resolve().parents[1]
SCRIPT = FINITECHAT_ROOT / "scripts" / "server-contract-gate.py"
SPEC = importlib.util.spec_from_file_location("server_contract_gate", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
SERVER_CONTRACT_GATE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SERVER_CONTRACT_GATE)


class ServerContractGateTests(unittest.TestCase):
    def test_nix_fingerprint_is_the_production_identity(self) -> None:
        health = {
            "status": "ok",
            "server_contract_version": 6,
            "server_version": "0.1.0",
            "source_fingerprint": "nix-chat-source",
            "source_dirty": False,
        }

        self.assertEqual(
            SERVER_CONTRACT_GATE.health_failures(
                health,
                expected_contract=6,
                expected_fingerprint="nix-chat-source",
            ),
            [],
        )

    def test_wrong_nix_fingerprint_fails_closed(self) -> None:
        health = {
            "status": "ok",
            "server_contract_version": 6,
            "server_version": "0.1.0",
            "source_fingerprint": "nix-other-source",
            "source_dirty": False,
        }

        failures = SERVER_CONTRACT_GATE.health_failures(
            health,
            expected_contract=6,
            expected_fingerprint="nix-chat-source",
        )

        self.assertEqual(len(failures), 1)
        self.assertIn("source_fingerprint", failures[0])

    def test_legacy_commit_gate_remains_available_for_non_nix_builds(self) -> None:
        health = {
            "status": "ok",
            "server_contract_version": 6,
            "server_version": "0.1.0",
            "source_commit": "0123456789ab",
            "source_dirty": False,
        }

        self.assertEqual(
            SERVER_CONTRACT_GATE.health_failures(
                health,
                expected_contract=6,
                expected_source="0123456789ab",
            ),
            [],
        )


if __name__ == "__main__":
    unittest.main()

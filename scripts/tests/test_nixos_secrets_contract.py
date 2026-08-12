from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
CHECKER = REPO_ROOT / "scripts/check_nixos_secrets_contract.py"


def contract_entry(**overrides: object) -> dict[str, object]:
    entry: dict[str, object] = {
        "backend": "legacy",
        "consumers": ["synthetic.service"],
        "destinationPath": "/run/secrets/finite/synthetic-env",
        "group": "root",
        "kind": "env",
        "legacyPath": "/etc/finite/synthetic.env",
        "mode": "0600",
        "owner": "root",
        "path": "/etc/finite/synthetic.env",
        "reloadUnits": [],
        "requiredEnvNames": ["SYNTHETIC_TOKEN"],
        "restartUnits": [],
        "scope": ["finite-lat-1"],
        "sopsFile": None,
        "sopsFormat": "binary",
        "sopsKey": None,
    }
    entry.update(overrides)
    return entry


class NixosSecretsContractTest(unittest.TestCase):
    def run_checker(self, contract: dict[str, object]) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "contract.json"
            path.write_text(json.dumps(contract), encoding="utf-8")
            return subprocess.run(
                [str(CHECKER), "--contract-json", str(path)],
                check=False,
                capture_output=True,
                text=True,
            )

    def test_legacy_entry_passes_without_secret_values(self) -> None:
        result = self.run_checker({"synthetic-env": contract_entry()})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("values-free", result.stdout)

    def test_sops_entry_requires_resolved_destination_and_source_file(self) -> None:
        result = self.run_checker(
            {
                "synthetic-env": contract_entry(
                    backend="sops",
                    path="/etc/finite/synthetic.env",
                    sopsFile=None,
                )
            }
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("resolved path does not match sops backend", result.stderr)
        self.assertIn("sops backend requires sopsFile", result.stderr)

    def test_checker_never_prints_unknown_field_values(self) -> None:
        secret_value = "synthetic-secret-value"
        result = self.run_checker(
            {
                "synthetic-env": contract_entry(
                    unexpectedSecretValue=secret_value,
                )
            }
        )
        self.assertEqual(result.returncode, 1)
        combined = result.stdout + result.stderr
        self.assertIn("unexpectedSecretValue", combined)
        self.assertNotIn(secret_value, combined)


if __name__ == "__main__":
    unittest.main()

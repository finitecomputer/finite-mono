"""Unit tests for the read-only service-directory cache accessor.

The accessor reads the cache `finite-agentd` has already fetched, verified,
and atomically written — it performs no crypto and no network I/O itself.
"""

from __future__ import annotations

import importlib.util
import json
import types
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

MONOREPO_ROOT = Path(__file__).resolve().parents[3]
MODULE_PATH = MONOREPO_ROOT / "finitechat/containers/agent/finite_service_directory.py"


def load_module() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location(
        "finite_service_directory_under_test", MODULE_PATH
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


DIRECTORY = {
    "schema": "finite_service_directory.v1",
    "generated_at": "2026-08-06T00:00:00Z",
    "services": {
        "sites": {
            "base_url": "http://192.168.64.1:18789",
            "client_version": {"min": "0.0.0", "current": "0.0.0"},
        },
        "core": {"base_url": "http://192.168.64.1:4200"},
    },
    "channels": {},
    "signature": "ed25519:already-verified-by-agentd",
}


class FiniteServiceDirectoryAccessorTest(unittest.TestCase):
    def test_base_urls_come_from_the_verified_cache(self) -> None:
        module = load_module()
        with TemporaryDirectory() as tmp:
            cache = Path(tmp) / "service-directory.json"
            cache.write_text(json.dumps(DIRECTORY), encoding="utf-8")

            self.assertEqual(
                module.service_base_urls(cache),
                {
                    "sites": "http://192.168.64.1:18789",
                    "core": "http://192.168.64.1:4200",
                },
            )
            self.assertEqual(
                module.service_base_url("sites", cache), "http://192.168.64.1:18789"
            )
            with self.assertRaises(module.ServiceDirectoryError):
                module.service_base_url("brain", cache)

    def test_missing_or_foreign_cache_fails_loudly(self) -> None:
        module = load_module()
        with TemporaryDirectory() as tmp:
            missing = Path(tmp) / "absent.json"
            with self.assertRaises(module.ServiceDirectoryError):
                module.service_base_urls(missing)

            wrong_schema = Path(tmp) / "wrong.json"
            wrong_schema.write_text(
                json.dumps({"schema": "something_else.v1", "services": {}}),
                encoding="utf-8",
            )
            with self.assertRaises(module.ServiceDirectoryError):
                module.service_base_urls(wrong_schema)


if __name__ == "__main__":
    unittest.main()

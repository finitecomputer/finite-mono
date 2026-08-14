from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
GATE_PATH = ROOT / "finite-identity" / "scripts" / "identity-edge-contract-gate.py"

spec = importlib.util.spec_from_file_location("identity_edge_contract_gate", GATE_PATH)
assert spec is not None and spec.loader is not None
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class ExtractionTests(unittest.TestCase):
    def test_real_sources_extract_exactly_the_manifest(self) -> None:
        cli_routes = gate.extract_cli_identity_routes(
            gate.CLI_API_SOURCE.read_text(encoding="utf-8"),
            gate.IDENTITY_CLIENT_SOURCE.read_text(encoding="utf-8"),
        )
        self.assertEqual(cli_routes, set(gate.MANIFEST))

    def test_real_public_router_covers_the_manifest(self) -> None:
        public_paths = gate.extract_public_router_paths(
            gate.AUTHORITY_SOURCE.read_text(encoding="utf-8")
        )
        self.assertLessEqual(set(gate.MANIFEST), public_paths)
        self.assertFalse(
            gate.static_failures(gate.MANIFEST, set(gate.MANIFEST), public_paths)
        )

    def test_cli_direct_call_pattern_ignores_sites_api_paths(self) -> None:
        cli_api = """
        let url = format!("{}/api/v1/nip05-resolution", self.base_url);
        let sites = format!("{}{}", self.base_url, "/api/v1/projects/init");
        """
        routes = gate.extract_cli_identity_routes(cli_api, "")
        self.assertEqual(routes, {"/api/v1/nip05-resolution"})


class StaticFailureTableTests(unittest.TestCase):
    CASES = [
        (
            "cli_adds_a_call_the_manifest_does_not_know",
            ["/api/v1/email-challenges"],
            {"/api/v1/email-challenges", "/api/v1/new-call"},
            {"/api/v1/email-challenges", "/api/v1/new-call"},
            "missing from MANIFEST",
        ),
        (
            "manifest_lists_a_route_the_cli_no_longer_calls",
            ["/api/v1/email-challenges", "/api/v1/retired"],
            {"/api/v1/email-challenges"},
            {"/api/v1/email-challenges", "/api/v1/retired"},
            "no longer calls",
        ),
        (
            "cli_called_route_missing_from_public_router",
            ["/api/v1/email-challenges"],
            {"/api/v1/email-challenges"},
            set(),
            "missing from public_router",
        ),
        (
            "private_route_leaks_onto_the_public_surface",
            ["/api/v1/email-challenges"],
            {"/api/v1/email-challenges"},
            {"/api/v1/email-challenges", "/api/v1/operator/inspect"},
            "private routes mounted",
        ),
        (
            "mailbox_proof_consume_stays_loopback_only",
            ["/api/v1/email-challenges"],
            {"/api/v1/email-challenges"},
            {"/api/v1/email-challenges", "/api/v1/mailbox-proofs/consume"},
            "private routes mounted",
        ),
    ]

    def test_static_failures(self) -> None:
        for name, manifest, cli_routes, public_paths, expected in self.CASES:
            with self.subTest(name=name):
                failures = gate.static_failures(manifest, cli_routes, public_paths)
                self.assertTrue(
                    any(expected in failure for failure in failures),
                    f"expected {expected!r} in {failures}",
                )

    def test_consistent_inputs_pass(self) -> None:
        routes = {"/api/v1/email-challenges", "/api/v1/nip05-resolution"}
        self.assertEqual(
            gate.static_failures(sorted(routes), routes, routes),
            [],
        )


class ProbeTableTests(unittest.TestCase):
    CASES = [
        ("ok", 200, False),
        ("bad_request_still_proves_the_route_exists", 400, False),
        ("unauthorized_still_proves_the_route_exists", 401, False),
        ("unsupported_media_type_still_proves_the_route_exists", 415, False),
        ("not_found_means_the_route_is_not_public", 404, True),
    ]

    def test_probe_failures(self) -> None:
        routes = ["/api/v1/nip05-resolution"]
        for name, status, expect_failure in self.CASES:
            with self.subTest(name=name):
                failures = gate.probe_failures(
                    "https://identity.test",
                    routes,
                    post_status=lambda _url: status,
                )
                self.assertEqual(bool(failures), expect_failure)


if __name__ == "__main__":
    unittest.main()

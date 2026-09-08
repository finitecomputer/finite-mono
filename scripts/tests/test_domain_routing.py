#!/usr/bin/env python3
"""Static safety contract for the finite.vip apex redirect."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "infra/hosts/clawland/finite-identity-nip05-route.yaml"


class DomainRoutingTests(unittest.TestCase):
    def test_apex_redirect_preserves_the_higher_priority_nip05_route(self) -> None:
        source = MANIFEST.read_text(encoding="utf-8")

        self.assertIn("match: Host(`finite.vip`) && Path(`/.well-known/nostr.json`)", source)
        self.assertIn("priority: 10000", source)
        self.assertIn("regex: '^https://finite\\.vip(.*)'", source)
        self.assertIn("replacement: 'https://finite.computer${1}'", source)
        self.assertIn("permanent: true", source)
        self.assertIn("priority: 1", source)

    def test_authority_backend_is_the_live_lat2_app_plane(self) -> None:
        source = MANIFEST.read_text(encoding="utf-8")

        self.assertIn("- ip: 64.34.80.19", source)
        self.assertNotIn("64.34.82.77", source)


if __name__ == "__main__":
    unittest.main()

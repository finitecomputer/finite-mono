from __future__ import annotations

import unittest

from scripts import backfill_releases


class BackfillReleaseTests(unittest.TestCase):
    def test_release_facts_parses_component_version_and_sort_key(self) -> None:
        self.assertEqual(
            backfill_releases.release_facts("fbrain/v0.5.0"),
            ("fbrain", "0.5.0", (0, 5, 0)),
        )

    def test_selects_versioned_releases_oldest_first(self) -> None:
        releases = [
            {"tag_name": "fsite-latest"},
            {"tag_name": "fsite/v0.5.0"},
            {"tag_name": "finitechat/v0.1.9"},
            {"tag_name": "fsite/v0.4.0"},
        ]

        self.assertEqual(
            [release["tag_name"] for release in backfill_releases.versioned(releases)],
            ["finitechat/v0.1.9", "fsite/v0.4.0", "fsite/v0.5.0"],
        )

    def test_electron_assets_are_not_backfilled(self) -> None:
        assets = [
            {"name": "finitechat-macos-aarch64.tar.gz"},
            {"name": "finitechat-macos-aarch64.tar.gz.sha256"},
            {"name": "finitechat-electron-macos-aarch64.zip"},
            {"name": "latest-mac.yml"},
        ]

        self.assertEqual(
            backfill_releases.cli_assets(assets),
            assets[:2],
        )


if __name__ == "__main__":
    unittest.main()

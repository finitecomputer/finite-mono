from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from scripts import delivery


class DeliveryTests(unittest.TestCase):
    def test_release_metadata_records_contract_assets_and_checksums(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            assets = Path(directory)
            archive = assets / "fsite-macos-aarch64.tar.gz"
            archive.write_bytes(b"hello")
            checksum = assets / "fsite-macos-aarch64.tar.gz.sha256"
            checksum.write_text(
                "2cf24dba5fb0a30e26e83b2ac5b9e29e"
                "1b161e5c1fa7425e73043362938b9824  "
                "fsite-macos-aarch64.tar.gz\n",
                encoding="utf-8",
            )

            metadata = delivery.build_release_metadata(
                component="fsite",
                version="0.4.0",
                source_sha="a" * 40,
                run_id="github-run-123",
                assets_dir=assets,
            )

        self.assertEqual(metadata["schema"], "finite-release/v1")
        self.assertEqual(metadata["tag"], "fsite/v0.4.0")
        self.assertEqual(metadata["source_sha"], "a" * 40)
        self.assertEqual(metadata["build_run"], "github-run-123")
        self.assertEqual(
            metadata["assets"],
            [
                {
                    "name": "fsite-macos-aarch64.tar.gz",
                    "sha256": (
                        "2cf24dba5fb0a30e26e83b2ac5b9e29e"
                        "1b161e5c1fa7425e73043362938b9824"
                    ),
                    "size": 5,
                },
                {
                    "name": "fsite-macos-aarch64.tar.gz.sha256",
                    "sha256": (
                        "d5746c6b892d8ff59dcf750b6974d3b1"
                        "73670d993cbc8d4b7c0a0feea4f92409"
                    ),
                    "size": 93,
                },
            ],
        )

    def test_release_metadata_rejects_unknown_component(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(delivery.DeliveryError, "component"):
                delivery.build_release_metadata(
                    component="unknown",
                    version="1.0.0",
                    source_sha="a" * 40,
                    run_id="run",
                    assets_dir=Path(directory),
                )

    def test_release_metadata_rejects_an_archive_without_a_checksum(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            assets = Path(directory)
            (assets / "fsite-linux-x86_64.tar.gz").write_bytes(b"archive")

            with self.assertRaisesRegex(delivery.DeliveryError, "missing checksum"):
                delivery.build_release_metadata(
                    component="fsite",
                    version="1.2.3",
                    source_sha="a" * 40,
                    run_id="run",
                    assets_dir=assets,
                )

    def test_component_version_matches_the_package_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root / "finite-sites/crates/fsite-cli/Cargo.toml"
            manifest.parent.mkdir(parents=True)
            manifest.write_text(
                '[package]\nname = "fsite-cli"\nversion = "1.2.3"\n',
                encoding="utf-8",
            )

            delivery.verify_component_version("fsite", "1.2.3", root)
            with self.assertRaisesRegex(delivery.DeliveryError, "does not match"):
                delivery.verify_component_version("fsite", "1.2.4", root)

    def test_image_promotion_rejects_a_changed_digest(self) -> None:
        with self.assertRaisesRegex(delivery.DeliveryError, "digest"):
            delivery.verify_image_promotion(
                "sha256:" + "a" * 64,
                "sha256:" + "b" * 64,
            )

    def test_image_index_requires_the_target_platform_and_attestation(self) -> None:
        manifest = {
            "schemaVersion": 2,
            "manifests": [
                {
                    "digest": "sha256:" + "a" * 64,
                    "platform": {"os": "linux", "architecture": "amd64"},
                },
                {
                    "digest": "sha256:" + "b" * 64,
                    "platform": {"os": "unknown", "architecture": "unknown"},
                    "annotations": {
                        "vnd.docker.reference.type": "attestation-manifest"
                    },
                },
            ],
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "manifest.json"
            path.write_text(json.dumps(manifest), encoding="utf-8")

            delivery.verify_image_index(
                path,
                platform="linux/amd64",
                require_attestation=True,
            )
            manifest["manifests"].pop()
            path.write_text(json.dumps(manifest), encoding="utf-8")
            with self.assertRaisesRegex(delivery.DeliveryError, "attestation"):
                delivery.verify_image_index(
                    path,
                    platform="linux/amd64",
                    require_attestation=True,
                )

    def test_production_guard_rejects_enabled_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "production.json"
            manifest.write_text(
                json.dumps({"environment": "production", "mutation_enabled": True}),
                encoding="utf-8",
            )

            with self.assertRaisesRegex(delivery.DeliveryError, "disabled"):
                delivery.require_production_disabled(manifest)

    def test_release_retry_rejects_changed_versioned_asset(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            local = root / "local"
            remote = root / "remote"
            local.mkdir()
            remote.mkdir()
            (local / "fsite-linux-x86_64.tar.gz").write_bytes(b"candidate")
            (remote / "fsite-linux-x86_64.tar.gz").write_bytes(b"different")

            with self.assertRaisesRegex(delivery.DeliveryError, "refusing to replace"):
                delivery.verify_remote_release_assets(
                    local_assets=[local / "fsite-linux-x86_64.tar.gz"],
                    downloaded_dir=remote,
                )

    def test_release_retry_reuses_original_provenance_for_same_release(self) -> None:
        original = {
            "schema": "finite-release/v1",
            "component": "fsite",
            "version": "1.2.3",
            "tag": "fsite/v1.2.3",
            "source_sha": "a" * 40,
            "build_run": "github-run-original",
            "assets": [
                {
                    "name": "fsite-linux-x86_64.tar.gz",
                    "sha256": "b" * 64,
                    "size": 123,
                }
            ],
        }
        retried = {**original, "build_run": "github-run-retry"}
        original_bytes = (
            json.dumps(original, indent=2, sort_keys=True) + "\n"
        ).encode()

        self.assertEqual(
            delivery.canonical_release_metadata(original_bytes, retried),
            original_bytes,
        )

    def test_release_retry_rejects_changed_provenance_facts(self) -> None:
        original = {
            "schema": "finite-release/v1",
            "component": "fsite",
            "version": "1.2.3",
            "tag": "fsite/v1.2.3",
            "source_sha": "a" * 40,
            "build_run": "github-run-original",
            "assets": [],
        }
        changed = {**original, "source_sha": "b" * 40}
        original_bytes = (
            json.dumps(original, indent=2, sort_keys=True) + "\n"
        ).encode()

        with self.assertRaisesRegex(delivery.DeliveryError, "different facts"):
            delivery.canonical_release_metadata(original_bytes, changed)

    def test_alias_promotion_reuses_verified_versioned_assets(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source"
            source.mkdir()
            archive = source / "fsite-linux-x86_64.tar.gz"
            archive.write_bytes(b"verified archive")
            checksum = source / "fsite-linux-x86_64.tar.gz.sha256"
            checksum.write_text(
                f"{delivery.sha256_file(archive)}  {archive.name}\n",
                encoding="utf-8",
            )
            metadata = delivery.build_release_metadata(
                component="fsite",
                version="1.2.3",
                source_sha="a" * 40,
                run_id="original-run",
                assets_dir=source,
            )
            (source / "release.json").write_text(
                json.dumps(metadata, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )

            def download(**kwargs: object) -> None:
                destination = kwargs["destination"]
                self.assertIsInstance(destination, Path)
                for name in kwargs["names"]:
                    (destination / name).parent.mkdir(parents=True, exist_ok=True)
                    (destination / name).write_bytes((source / name).read_bytes())

            with (
                mock.patch.object(
                    delivery,
                    "_release_assets",
                    return_value={path.name for path in source.iterdir()},
                ),
                mock.patch.object(
                    delivery, "_download_release_assets", side_effect=download
                ),
                mock.patch.object(
                    delivery,
                    "_ensure_metadata_commit",
                    return_value=("b" * 40, (source / "release.json").read_bytes()),
                ),
                mock.patch.object(delivery, "_ensure_tag") as ensure_tag,
                mock.patch.object(delivery, "_publish_alias") as publish_alias,
            ):
                delivery.promote_release_alias(
                    component="fsite",
                    version="1.2.3",
                    expected_source_sha="a" * 40,
                    repository="finitecomputer/finite-releases",
                )

            self.assertEqual(ensure_tag.call_count, 2)
            publish_alias.assert_called_once()

    def test_partial_versioned_release_is_not_treated_as_alias_ready(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source"
            source.mkdir()
            archive = source / "fsite-linux-x86_64.tar.gz"
            archive.write_bytes(b"partial upload")

            def download(**kwargs: object) -> None:
                destination = kwargs["destination"]
                self.assertIsInstance(destination, Path)
                (destination / archive.name).parent.mkdir(parents=True, exist_ok=True)
                (destination / archive.name).write_bytes(archive.read_bytes())

            with mock.patch.object(
                delivery,
                "_download_release_assets",
                side_effect=download,
            ):
                self.assertFalse(
                    delivery._versioned_release_is_complete(
                        component="fsite",
                        version="1.2.3",
                        expected_source_sha="a" * 40,
                        repository="finitecomputer/finite-releases",
                        remote_names={archive.name},
                        command_runner=mock.Mock(),
                    )
                )

    def test_backfill_can_publish_versioned_release_without_alias_refresh(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            assets = Path(directory)
            archive = assets / "fsite-linux-x86_64.tar.gz"
            archive.write_bytes(b"already published")
            checksum = assets / "fsite-linux-x86_64.tar.gz.sha256"
            checksum.write_text(
                f"{delivery.sha256_file(archive)}  {archive.name}\n",
                encoding="utf-8",
            )

            with (
                mock.patch.object(
                    delivery,
                    "_release_assets",
                    return_value={archive.name, checksum.name, "release.json"},
                ),
                mock.patch.object(
                    delivery,
                    "_versioned_release_is_complete",
                    return_value=True,
                ),
                mock.patch.object(delivery, "promote_release_alias") as promote_alias,
            ):
                delivery.publish_release(
                    component="fsite",
                    version="1.2.3",
                    source_sha="a" * 40,
                    run_id="github-release-backfill:123",
                    assets_dir=assets,
                    repository="finitecomputer/finite-releases",
                    refresh_alias=False,
                )

            promote_alias.assert_not_called()

    def test_missing_github_release_is_available_for_creation(self) -> None:
        runner = mock.Mock(
            return_value=mock.Mock(
                returncode=1,
                stdout="",
                stderr="release not found\n",
            )
        )

        self.assertIsNone(
            delivery._release_assets(
                repository="finitecomputer/finite-releases",
                tag="fsite/v1.2.3",
                command_runner=runner,
            )
        )


if __name__ == "__main__":
    unittest.main()

import argparse
import importlib.util
import os
import pathlib
import sys
import unittest
from importlib.machinery import SourceFileLoader


ROOT = pathlib.Path(__file__).resolve().parents[2]
CI_DIR = ROOT / "scripts" / "ci"
AFFECTED_RUST_PACKAGES = CI_DIR / "affected-rust-packages"

# The selector scripts import their sibling changed_paths module; running them
# directly puts scripts/ci on sys.path, so do the same here.
sys.path.insert(0, str(CI_DIR))

import changed_paths

loader = SourceFileLoader("affected_rust_packages", str(AFFECTED_RUST_PACKAGES))
spec = importlib.util.spec_from_loader(loader.name, loader)
affected_rust_packages = importlib.util.module_from_spec(spec)
assert spec.loader is not None
sys.modules[spec.name] = affected_rust_packages
spec.loader.exec_module(affected_rust_packages)


class NormalizePathTests(unittest.TestCase):
    def test_dotfile_path_is_preserved(self) -> None:
        self.assertEqual(
            changed_paths.normalize_path(".depot/workflows/ci.yml"),
            ".depot/workflows/ci.yml",
        )

    def test_dot_slash_prefix_is_stripped(self) -> None:
        self.assertEqual(changed_paths.normalize_path("./justfile"), "justfile")

    def test_bare_path_is_unchanged(self) -> None:
        self.assertEqual(changed_paths.normalize_path("justfile"), "justfile")

    def test_empty_path_stays_empty(self) -> None:
        self.assertEqual(changed_paths.normalize_path(""), "")

    def test_absolute_path_becomes_relative(self) -> None:
        self.assertEqual(changed_paths.normalize_path(os.path.abspath("justfile")), "justfile")

    def test_normalized_paths_drops_empty_entries(self) -> None:
        self.assertEqual(
            changed_paths.normalized_paths(["", "./justfile"]),
            ["justfile"],
        )


class AffectedRustPackagesChangedFileTests(unittest.TestCase):
    def test_dotfile_changed_file_selects_full_workspace(self) -> None:
        args = argparse.Namespace(
            event="pull_request",
            base=None,
            head=None,
            changed_files=[".depot/workflows/ci.yml"],
        )

        selection = affected_rust_packages.select_package_scope(args)

        self.assertEqual(selection["mode"], "full")
        self.assertEqual(
            selection["reason"],
            ".depot/workflows/ci.yml can affect the whole Rust workspace",
        )

    def test_dot_slash_prefix_matches_bare_path(self) -> None:
        prefixed = argparse.Namespace(
            event="pull_request",
            base=None,
            head=None,
            changed_files=["./justfile"],
        )
        bare = argparse.Namespace(
            event="pull_request",
            base=None,
            head=None,
            changed_files=["justfile"],
        )

        self.assertEqual(
            affected_rust_packages.select_package_scope(prefixed),
            affected_rust_packages.select_package_scope(bare),
        )


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3
"""Source-set contract for the scoped Rust packages in infra/nixos/packages.nix.

Each package's store path is derived from its source set. A file in that set
that the build does not read still changes the store path when it changes, and
activation then restarts the service. These checks evaluate the source sets
without building anything.
"""

from __future__ import annotations

import json
from pathlib import Path
import subprocess
import unittest


ROOT = Path(__file__).resolve().parents[2]

PACKAGES = (
    "devfinity",
    "finite-saas-core",
    "finite-saas-local",
    "finite-saas-runner",
    "finitechat-server",
    "finitechat-hosted-device",
    "finite-agentd",
    "finitesitesd",
    "finite-brain",
    "finite-identity",
    "fsite",
    "fbrain",
    "finitechat",
)

SAAS_CORE_MIGRATIONS = "finitecomputer-v2/crates/finite-saas-core/migrations/"
HERMES_ADAPTER_FILES = (
    "finitechat/integrations/hermes/finitechat/__init__.py",
    "finitechat/integrations/hermes/finitechat/adapter.py",
    "finitechat/integrations/hermes/finitechat/plugin.yaml",
    "finitechat/integrations/hermes/finitechat/simplex_topics.py",
    "finitechat/integrations/hermes/finitechat/slash_policy.json",
    "finitechat/integrations/hermes/finitechat/slash_policy.py",
)

# Files outside a crate's manifest, build script and src/ tree that non-test
# code reads at compile time through include_str! or include_bytes!.
DECLARED_COMPILE_INPUTS = (SAAS_CORE_MIGRATIONS, *HERMES_ADAPTER_FILES)


def evaluate_source_files() -> dict[str, list[str]]:
    names = " ".join(f'"{name}"' for name in PACKAGES)
    apply = (
        "ps: builtins.listToAttrs (map (name: { inherit name; "
        f"value = ps.${{name}}.sourceFiles; }}) [ {names} ])"
    )
    result = subprocess.run(
        ["nix", "eval", "--json", ".#packages.x86_64-linux", "--apply", apply],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        raise AssertionError(f"nix eval failed:\n{result.stderr}")
    return {
        name: [path.removeprefix("./") for path in paths]
        for name, paths in json.loads(result.stdout).items()
    }


def crate_roots(files: list[str]) -> set[str]:
    return {
        path.removesuffix("/Cargo.toml")
        for path in files
        if path.endswith("/Cargo.toml")
    }


def is_compile_input(path: str, crates: set[str]) -> bool:
    if path in {"Cargo.lock", "Cargo.toml"}:
        return True
    if path.startswith(DECLARED_COMPILE_INPUTS):
        return True
    return any(
        path in {f"{crate}/Cargo.toml", f"{crate}/build.rs"}
        or path.startswith(f"{crate}/src/")
        for crate in crates
    )


class NixPackageSourceTests(unittest.TestCase):
    source_files: dict[str, list[str]]

    @classmethod
    def setUpClass(cls) -> None:
        cls.source_files = evaluate_source_files()

    def test_every_package_source_set_is_evaluated(self) -> None:
        self.assertEqual(set(self.source_files), set(PACKAGES))
        for name, files in self.source_files.items():
            with self.subTest(package=name):
                self.assertIn("Cargo.lock", files)

    def test_source_sets_leave_out_markdown(self) -> None:
        for name, files in self.source_files.items():
            with self.subTest(package=name):
                self.assertEqual([path for path in files if path.endswith(".md")], [])

    def test_source_sets_hold_only_compile_inputs(self) -> None:
        for name, files in self.source_files.items():
            crates = crate_roots(files)
            with self.subTest(package=name):
                self.assertEqual(
                    [path for path in files if not is_compile_input(path, crates)],
                    [],
                )

    def test_saas_packages_keep_every_migration(self) -> None:
        tracked = subprocess.run(
            ["git", "ls-files", SAAS_CORE_MIGRATIONS],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=True,
        ).stdout.split()
        self.assertTrue(tracked)
        for name in ("finite-saas-core", "finite-saas-runner", "finite-saas-local"):
            with self.subTest(package=name):
                self.assertLessEqual(set(tracked), set(self.source_files[name]))

    def test_finitechat_cli_keeps_the_embedded_hermes_adapter(self) -> None:
        self.assertLessEqual(
            set(HERMES_ADAPTER_FILES), set(self.source_files["finitechat"])
        )

    def test_chat_server_and_hosted_device_leave_out_the_hermes_adapter(self) -> None:
        for name in ("finitechat-server", "finitechat-hosted-device"):
            with self.subTest(package=name):
                self.assertTrue(
                    set(HERMES_ADAPTER_FILES).isdisjoint(self.source_files[name])
                )


if __name__ == "__main__":
    unittest.main()

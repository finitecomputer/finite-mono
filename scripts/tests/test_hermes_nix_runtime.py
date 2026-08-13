from __future__ import annotations

import unittest
from dataclasses import replace

from scripts.hermes_nix_runtime import (
    HermesRuntimeClosure,
    image_build_args,
    nix_system_for_platform,
    toolchain_attr,
)


def _closure() -> HermesRuntimeClosure:
    return HermesRuntimeClosure(
        attr=".#packages.x86_64-linux.hermes-agent-runtime",
        python_attr=".#packages.x86_64-linux.hermes-agent-runtime-python",
        toolchain_attr=".#packages.x86_64-linux.agent-runtime-toolchains",
        nix_system="x86_64-linux",
        store_path="/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-hermes-agent",
        python_store_path="/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-hermes-venv",
        toolchain_store_path="/nix/store/cccccccccccccccccccccccccccccccc-agent-runtime-toolchains",
        playwright_browsers_path="/nix/store/dddddddddddddddddddddddddddddddd-playwright-browsers",
        version="0.20.0",
        closure_count=3,
    )


class HermesNixRuntimeTests(unittest.TestCase):
    def test_toolchain_attr_follows_image_platform(self) -> None:
        self.assertEqual(
            toolchain_attr(nix_system_for_platform("linux/amd64")),
            ".#packages.x86_64-linux.agent-runtime-toolchains",
        )
        self.assertEqual(
            toolchain_attr(nix_system_for_platform("linux/arm64")),
            ".#packages.aarch64-linux.agent-runtime-toolchains",
        )

    def test_image_build_args_pin_hermes_and_toolchains(self) -> None:
        args = image_build_args(_closure(), hermes_agent_version="0.20.0")
        self.assertEqual(
            args[1::2],
            [
                "HERMES_AGENT_VERSION=0.20.0",
                "HERMES_AGENT_STORE_PATH=/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-hermes-agent",
                "HERMES_AGENT_PYTHON_PATH=/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-hermes-venv",
                "HERMES_AGENT_NIX_ATTR=.#packages.x86_64-linux.hermes-agent-runtime",
                "HERMES_AGENT_NIX_SYSTEM=x86_64-linux",
                "AGENT_RUNTIME_TOOLCHAIN_PATH=/nix/store/cccccccccccccccccccccccccccccccc-agent-runtime-toolchains",
                "AGENT_RUNTIME_TOOLCHAIN_ATTR=.#packages.x86_64-linux.agent-runtime-toolchains",
                "PLAYWRIGHT_BROWSERS_PATH=/nix/store/dddddddddddddddddddddddddddddddd-playwright-browsers",
            ],
        )

    def test_image_build_args_fail_closed_on_missing_toolchain(self) -> None:
        runtime = replace(_closure(), toolchain_store_path="")
        with self.assertRaises(SystemExit):
            image_build_args(runtime, hermes_agent_version="0.20.0")

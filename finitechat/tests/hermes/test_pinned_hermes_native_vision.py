"""Exercise Finite's generated config through the pinned Hermes vision paths."""

import base64
import copy
import os
import tempfile
import unittest
from pathlib import Path
from typing import Any, cast
from unittest.mock import AsyncMock, patch

from agent.auxiliary_client import scoped_runtime_main
from gateway.run import GatewayRunner
from tools import vision_tools

from tests.container import test_agent_runtime_launcher as launcher_tests

RED_PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAQAAAAEAQMAAACTPww9AAAAA1BMVEX/AAAZ4gk3"
    "AAAAC0lEQVQI12NggAAAAAgAAS8g3TEAAAAASUVORK5CYII="
)


class PinnedHermesNativeVisionTests(unittest.IsolatedAsyncioTestCase):
    async def test_fresh_and_existing_private_configs_use_native_images(self):
        fixture = launcher_tests.AgentRuntimeLauncherConfigTest
        settings = fixture._reconciler_settings()
        old_config = fixture._reconcile_config(None, settings)
        old_config["model"].pop("supports_vision", None)
        old_config["auxiliary"]["vision"] = {"provider": "auto"}
        retired_aeon_configs = []
        for block in fixture._retired_aeon_vision_blocks():
            aeon_config = copy.deepcopy(old_config)
            aeon_config["auxiliary"]["vision"] = block
            retired_aeon_configs.append(aeon_config)
        for case, existing in enumerate((None, old_config, *retired_aeon_configs)):
            with self.subTest(case=case):
                config = fixture._reconcile_config(existing, settings)
                model = config["model"]
                runtime = {**model, "model": model["default"]}
                runner = object.__new__(GatewayRunner)
                self.assertEqual(
                    runner._decide_image_input_mode(
                        user_config=config, provider=model["provider"], model=model["default"]
                    ),
                    "native",
                )
                with (
                    tempfile.TemporaryDirectory() as home,
                    patch.dict(os.environ, {"HERMES_HOME": home}),
                    patch("hermes_cli.config.load_config", return_value=config),
                    scoped_runtime_main(runtime),
                    patch.object(
                        vision_tools,
                        "vision_analyze_tool",
                        new_callable=AsyncMock,
                        side_effect=AssertionError(
                            "native vision must not call auxiliary analysis"
                        ),
                    ) as auxiliary,
                ):
                    path = Path(home) / "red.png"
                    path.write_bytes(RED_PNG)
                    result = await vision_tools._handle_vision_analyze(
                        {"image_url": str(path), "question": "What color is this?"}
                    )
                auxiliary.assert_not_awaited()
                self.assertIsInstance(result, dict)
                # Upstream annotates this handler as str even for its native image result.
                multimodal = cast(dict[str, Any], result)
                self.assertTrue(multimodal["_multimodal"])
                self.assertTrue(multimodal["meta"]["native_vision"])
                images = [part for part in multimodal["content"] if part["type"] == "image_url"]
                self.assertEqual(len(images), 1)
                self.assertTrue(images[0]["image_url"]["url"].startswith("data:image/png;base64,"))

    def test_retired_aeon_backend_no_longer_outranks_native_images(self):
        fixture = launcher_tests.AgentRuntimeLauncherConfigTest
        settings = fixture._reconciler_settings()
        runner = object.__new__(GatewayRunner)

        def decision(config: dict[str, Any]) -> str:
            return runner._decide_image_input_mode(
                user_config=config, provider="custom", model="glm-5-3-flash"
            )

        for block in fixture._retired_aeon_vision_blocks():
            with self.subTest(model=block["model"], extra_body="extra_body" in block):
                existing = fixture._reconcile_config(None, settings)
                existing["model"].pop("supports_vision")
                existing["auxiliary"]["vision"] = block
                # The capability flag alone cannot win over this stored backend.
                flagged_only = copy.deepcopy(existing)
                flagged_only["model"]["supports_vision"] = True
                self.assertEqual(decision(flagged_only), "text")

                config = fixture._reconcile_config(existing, settings)

                self.assertNotIn("vision", config["auxiliary"])
                self.assertIs(config["model"]["supports_vision"], True)
                self.assertEqual(decision(config), "native")

    def test_explicit_text_routing_remains_authoritative(self):
        fixture = launcher_tests.AgentRuntimeLauncherConfigTest
        config = fixture._reconcile_config(None, fixture._reconciler_settings())
        config["agent"] = {"image_input_mode": "text"}
        runner = object.__new__(GatewayRunner)
        self.assertEqual(
            runner._decide_image_input_mode(
                user_config=config, provider="custom", model="glm-5-3-flash"
            ),
            "text",
        )

    def test_explicit_auxiliary_backend_remains_authoritative(self):
        fixture = launcher_tests.AgentRuntimeLauncherConfigTest
        config = fixture._reconcile_config(None, fixture._reconciler_settings())
        config["auxiliary"]["vision"] = {
            "provider": "custom",
            "base_url": "https://user-vision.invalid/v1",
            "model": "user-vision-model",
        }
        runner = object.__new__(GatewayRunner)
        self.assertEqual(
            runner._decide_image_input_mode(
                user_config=config, provider="custom", model="glm-5-3-flash"
            ),
            "text",
        )

"""Exercise Finite's generated config through the pinned Hermes vision paths."""

import base64
import os
import tempfile
import unittest
from pathlib import Path
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
        for existing in (None, old_config):
            with self.subTest(existing=existing is not None):
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
                self.assertTrue(result["_multimodal"])
                self.assertTrue(result["meta"]["native_vision"])
                images = [part for part in result["content"] if part["type"] == "image_url"]
                self.assertEqual(len(images), 1)
                self.assertTrue(images[0]["image_url"]["url"].startswith("data:image/png;base64,"))

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

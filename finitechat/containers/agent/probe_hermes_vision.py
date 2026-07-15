#!/usr/bin/env python3
"""Verify the admitted Hermes multimodal tools, then exercise auxiliary.vision."""

import asyncio
import importlib.metadata
import json

from hermes_cli.config import load_config
from hermes_cli.plugins import discover_plugins
from hermes_cli.tools_config import _get_platform_tools
from model_tools import get_tool_definitions
from tools.vision_tools import vision_analyze_tool

MARKER = "FINITE_AEON_HERMES_PROBE "
RED_FIXTURE = (
    "data:image/png;base64,"
    "iVBORw0KGgoAAAANSUhEUgAAAAQAAAAEAQMAAACTPww9AAAAA1BMVEX/AAAZ4gk3"
    "AAAAC0lEQVQI12NggAAAAAgAAS8g3TEAAAAASUVORK5CYII="
)
RED_PROMPT = "What is the dominant color of this image? Reply with one uppercase color word."


def video_analyze_is_admitted() -> bool:
    """Inspect the same resolved Finite Chat catalog that the agent receives."""
    discover_plugins()
    config = load_config()
    enabled_toolsets = sorted(_get_platform_tools(config, "finitechat"))
    definitions = get_tool_definitions(
        enabled_toolsets=enabled_toolsets,
        quiet_mode=True,
    )
    return any(
        definition.get("function", {}).get("name") == "video_analyze" for definition in definitions
    )


def hermes_version() -> str:
    try:
        return importlib.metadata.version("hermes-agent")
    except importlib.metadata.PackageNotFoundError:
        return "unknown"


async def probe() -> int:
    try:
        video_analyze = video_analyze_is_admitted()
        raw = await vision_analyze_tool(RED_FIXTURE, RED_PROMPT)
        result = json.loads(raw)
        analysis = result.get("analysis")
        passed = video_analyze and result.get("success") is True and analysis == "RED"
    except Exception:
        analysis = None
        video_analyze = False
        passed = False

    print(
        MARKER
        + json.dumps(
            {
                "success": passed,
                "analysis": analysis if passed else None,
                "video_analyze": video_analyze,
                "hermes_version": hermes_version(),
            },
            separators=(",", ":"),
        )
    )
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(asyncio.run(probe()))

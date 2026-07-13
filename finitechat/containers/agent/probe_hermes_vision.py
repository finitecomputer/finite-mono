#!/usr/bin/env python3
"""Exercise the installed Hermes auxiliary.vision runtime with a fixed image."""

import asyncio
import json

from tools.vision_tools import vision_analyze_tool


MARKER = "FINITE_AEON_HERMES_PROBE "
RED_FIXTURE = (
    "data:image/png;base64,"
    "iVBORw0KGgoAAAANSUhEUgAAAAQAAAAEAQMAAACTPww9AAAAA1BMVEX/AAAZ4gk3"
    "AAAAC0lEQVQI12NggAAAAAgAAS8g3TEAAAAASUVORK5CYII="
)
RED_PROMPT = "What is the dominant color of this image? Reply with one uppercase color word."


async def probe() -> int:
    try:
        raw = await vision_analyze_tool(RED_FIXTURE, RED_PROMPT)
        result = json.loads(raw)
        analysis = result.get("analysis")
        passed = result.get("success") is True and analysis == "RED"
    except Exception:
        analysis = None
        passed = False

    print(
        MARKER
        + json.dumps(
            {"success": passed, "analysis": analysis if passed else None},
            separators=(",", ":"),
        )
    )
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(asyncio.run(probe()))

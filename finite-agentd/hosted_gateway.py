#!/usr/bin/env python3
"""Dedicated supervisor child; never replaces the Finite Chat gateway process."""

import json
import os
import signal
from pathlib import Path


def main():
    path = Path(os.environ.get("FINITECHAT_HOME", "/data/agent")) / "agentd/hosted-gateway.json"
    config = json.loads(path.read_text()) if path.exists() else {"enabled": False}
    if not config.get("enabled"):
        while True:
            signal.pause()
    token = config.get("token")
    if not isinstance(token, str) or len(token) != 64:
        raise RuntimeError("Hosted gateway credential is unavailable")
    # The image's pinned Hermes shares its durable session store with the
    # regular gateway. Its isolated web listener has a separate process lock.
    environment = dict(os.environ, HERMES_DASHBOARD_SESSION_TOKEN=token)
    os.execvpe(
        "hermes",
        ["hermes", "serve", "--host", "127.0.0.1", "--port", "9120", "--isolated"],
        environment,
    )


if __name__ == "__main__":
    main()

"""Disposable integration driver for the ignored Rust Sites/Hermes proof."""

import hashlib
import json
import os
import shlex
import subprocess
import sys
import tempfile
from pathlib import Path
from unittest.mock import patch

from hermes_cli import finite_requester_context, plugins
from model_tools import handle_function_call
from tui_gateway import server

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "finitechat/tests/hermes"))


def main():
    from test_pinned_hermes_sender_context import HookOnlyPluginContext, load_adapter_module

    fixture = json.load(sys.stdin)
    for name, secret, expected in fixture["agents"]:
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            config = home / "finite.toml"
            config.write_text(f'[project]\nslug = "native-{name}"\n')
            result_path = home / "result.json"
            session = f"native-sites-{name}"
            manager = plugins.PluginManager()
            with (
                patch.dict(
                    os.environ, {"FINITE_HOME": directory, "FINITE_SITES_API": fixture["api"]}
                ),
                patch.object(plugins, "_plugin_manager", manager),
            ):
                imported = subprocess.run(
                    [fixture["fsite"], "auth", "import", "--output", "json"],
                    input=secret + "\n",
                    text=True,
                    capture_output=True,
                    timeout=20,
                    check=False,
                )
                assert imported.returncode == 0, "synthetic identity import failed"
                load_adapter_module().register(HookOnlyPluginContext(manager))
                tokens = server._set_session_context(session)
                requester_tokens = finite_requester_context.bind(fixture["requester"])
                try:
                    command = (
                        shlex.join(
                            [
                                fixture["fsite"],
                                "project",
                                "init",
                                "--config",
                                str(config),
                                "--output",
                                "json",
                            ]
                        )
                        + " > "
                        + shlex.quote(str(result_path))
                    )
                    result = json.loads(
                        handle_function_call(
                            "terminal",
                            {"command": command},
                            task_id=session,
                            session_id=session,
                            tool_call_id=name,
                        )
                    )
                    assert result.get("exit_code") == expected, result
                    if expected == 0:
                        payload = json.loads(result_path.read_text())
                        assert payload["owner_email"] == fixture["requester"]["email"], payload
                    else:
                        assert "403" in result.get("output", ""), result
                    filename = hashlib.sha256(session.encode()).hexdigest() + ".json"
                    assert not (home / "requester-context-v2" / filename).exists()
                    assert not (home / "requester-context-v1" / filename).exists()
                finally:
                    finite_requester_context.reset(requester_tokens)
                    server._clear_session_context(tokens)
    print(
        "native terminal -> fsite -> Sites: exact-agent success, wrong-agent denial, lease cleanup",
        file=sys.__stdout__,
    )


if __name__ == "__main__":
    main()

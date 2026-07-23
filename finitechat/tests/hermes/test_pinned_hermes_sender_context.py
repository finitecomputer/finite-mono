import hashlib
import importlib.util
import os
import shlex
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from gateway.config import GatewayConfig, Platform
from gateway.session import SessionSource, build_session_context, build_session_context_prompt
from gateway.session_context import clear_session_vars, set_session_vars
from hermes_cli import plugins
from model_tools import handle_function_call

REPO_ROOT = Path(__file__).resolve().parents[2]
ADAPTER_PATH = REPO_ROOT / "integrations" / "hermes" / "finitechat" / "adapter.py"


class HookOnlyPluginContext:
    def __init__(self, manager):
        self.manager = manager

    def register_hook(self, name, callback):
        self.manager._hooks.setdefault(name, []).append(callback)

    def register_platform(self, **_kwargs):
        pass


def load_adapter_module():
    module_name = "finitechat_pinned_hook_adapter_under_test"
    sys.modules.pop(module_name, None)
    spec = importlib.util.spec_from_file_location(module_name, ADAPTER_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load adapter from {ADAPTER_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


class PinnedHermesSenderContextTests(unittest.TestCase):
    def test_terminal_hooks_lease_task_local_sender_only_around_the_process(self):
        account_id = "a1" * 32
        session_id = "session-a"
        with tempfile.TemporaryDirectory() as finite_home:
            finite_home_path = Path(finite_home)
            context_name = hashlib.sha256(session_id.encode("utf-8")).hexdigest() + ".json"
            context_path = finite_home_path / "requester-context-v1" / context_name
            observed_path = finite_home_path / "observed"
            manager = plugins.PluginManager()
            previous_manager = plugins._plugin_manager
            plugins._plugin_manager = manager
            tokens = set_session_vars(
                platform="finitechat",
                user_id=account_id,
                session_id=session_id,
            )
            try:
                with patch.dict(os.environ, {"FINITE_HOME": finite_home}):
                    adapter = load_adapter_module()
                    adapter.register(HookOnlyPluginContext(manager))
                    command = (
                        f"test -f {shlex.quote(str(context_path))} "
                        f"&& printf observed > {shlex.quote(str(observed_path))}"
                    )
                    handle_function_call(
                        "terminal",
                        {"command": command},
                        task_id="task-a",
                        session_id=session_id,
                        tool_call_id="call-a",
                    )
                self.assertEqual(observed_path.read_text(), "observed")
                self.assertFalse(context_path.exists())
            finally:
                clear_session_vars(tokens)
                plugins._plugin_manager = previous_manager

    def test_threaded_group_requires_per_turn_authenticated_sender_context(self):
        account_id = "a1" * 32
        source = SessionSource(
            platform=Platform.TELEGRAM,
            chat_id="room-agent-1",
            chat_type="group",
            user_id=account_id,
            user_name=None,
            thread_id="home-chat",
        )

        context = build_session_context(source, GatewayConfig())
        prompt = build_session_context_prompt(context)
        sender_prompt = (
            "Authenticated Finite Chat sender metadata for this turn: "
            f"event.source.user_id is `{account_id}`."
        )
        combined_prompt = f"{prompt}\n\n{sender_prompt}"

        self.assertTrue(context.shared_multi_user_session)
        self.assertNotIn(account_id, prompt)
        self.assertIn("Multi-user thread", prompt)
        self.assertIn(account_id, combined_prompt)


if __name__ == "__main__":
    unittest.main()

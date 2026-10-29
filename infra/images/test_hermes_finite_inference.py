"""Exercise real packaged model commands and durable session routes with fake keys."""

import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch


class FiniteInferenceTests(unittest.IsolatedAsyncioTestCase):
    @classmethod
    def setUpClass(cls):
        cls.scratch = tempfile.TemporaryDirectory(prefix="finite-inference-")
        cls.home = Path(cls.scratch.name)
        cls.env = patch.dict(
            os.environ,
            {
                "HERMES_HOME": str(cls.home),
                "HOME": str(cls.home),
                "XDG_CONFIG_HOME": str(cls.home / "config"),
                "XDG_STATE_HOME": str(cls.home / "state"),
                "PATH": "",
                "FINITE_PRIVATE_API_KEY": "synthetic-private-key",
                "FINITE_PRIVATE_BASE_URL": "https://private.example.invalid/v1",
                "FINITE_PRIVATE_MODEL": "synthetic-private-model",
                "OPENAI_API_KEY": "synthetic-private-key",
            },
            clear=True,
        )
        cls.env.start()
        from gateway import run
        from gateway.config import GatewayConfig, Platform
        from gateway.platforms.base import MessageEvent
        from gateway.session import SessionSource
        from hermes_cli.runtime_provider import resolve_runtime_provider

        cls.gateway = run
        cls.GatewayConfig = GatewayConfig
        cls.MessageEvent = MessageEvent
        cls.sources = [
            SessionSource(platform=Platform.LOCAL, chat_id=name, chat_type="dm")
            for name in ("research", "writing")
        ]
        cls.resolve = staticmethod(resolve_runtime_provider)

    @classmethod
    def tearDownClass(cls):
        cls.env.stop()
        cls.scratch.cleanup()

    def setUp(self):
        self.config_path = self.home / "config.yaml"
        self.config_path.write_text(
            "model:\n  provider: custom\n  default: synthetic-private-model\n  base_url: https://private.example.invalid/v1\n  api_key: ${FINITE_PRIVATE_API_KEY}\n"
        )
        self.original_config = self.config_path.read_bytes()
        self.save_key("")
        self.runners = []
        self.runner = self.new_runner()
        for source in self.sources:
            self.runner.session_store.get_or_create_session(source)
        # Only external catalog/price/context probes are stubbed. Commands,
        # credential resolution, override persistence and rehydration are real.
        for target, kwargs in [
            ("hermes_cli.models.validate_requested_model", {"return_value": {"accepted": True}}),
            ("hermes_cli.model_switch.get_model_info", {"return_value": None}),
            (
                "hermes_cli.model_switch.resolve_display_context_length_async",
                {"new": AsyncMock(return_value=32768)},
            ),
            (
                "hermes_cli.context_switch_guard.enrich_model_switch_warnings_for_gateway",
                {"return_value": None},
            ),
            (
                "hermes_cli.model_selection_guards.combined_selection_warning",
                {"return_value": None},
            ),
            (
                "requests.sessions.Session.request",
                {"side_effect": AssertionError("unexpected network request")},
            ),
            (
                "socket.socket.connect",
                {"side_effect": AssertionError("unexpected network connection")},
            ),
        ]:
            context = patch(target, **kwargs)
            context.start()
            self.addCleanup(context.stop)

    def tearDown(self):
        for runner in self.runners:
            runner.close_all_session_db_handles()
            runner.session_store.close_all_db_handles()
            runner._shutdown_executor()

    def new_runner(self):
        runner = self.gateway.GatewayRunner(self.GatewayConfig(sessions_dir=self.home / self.id()))
        runner._session_db = None
        self.runners.append(runner)
        return runner

    def save_key(self, key):
        (self.home / ".env").write_text(f"OPENROUTER_API_KEY='{key}'\n")

    async def command(self, text, source=0):
        return await self.runner._handle_model_command(
            self.MessageEvent(text=text, source=self.sources[source])
        )

    def route(self, source=0, runner=None):
        return (runner or self.runner)._resolve_session_agent_runtime(source=self.sources[source])

    async def test_key_added_after_start_switches_only_one_session_and_survives_restart(self):
        session_id = self.runner.session_store.get_or_create_session(self.sources[0]).session_id
        self.runner.session_store.append_to_transcript(
            session_id, {"role": "user", "content": "Keep this conversation."}
        )
        history = self.runner.session_store.load_transcript(session_id)
        self.assertTrue(history)
        self.save_key("synthetic-openrouter-key")
        reply = await self.command(
            "/model anthropic/claude-sonnet-4.6 --provider openrouter --session"
        )
        self.assertIsNotNone(reply)
        model, route = self.route()
        self.assertEqual(model, "anthropic/claude-sonnet-4.6")
        self.assertEqual(route["base_url"], "https://openrouter.ai/api/v1")
        self.assertEqual(route["api_key"], "synthetic-openrouter-key")
        self.assertEqual(self.route(1)[1]["api_key"], "synthetic-private-key")
        self.assertEqual(self.config_path.read_bytes(), self.original_config)
        restarted = self.new_runner()
        self.assertEqual(self.route(runner=restarted)[1]["api_key"], "synthetic-openrouter-key")
        self.assertEqual(self.route(1, restarted)[0], "synthetic-private-model")
        self.assertEqual(restarted.session_store.load_transcript(session_id), history)

    async def test_model_list_discovers_a_key_saved_after_startup(self):
        self.save_key("synthetic-openrouter-key")
        with (
            patch("agent.models_dev.fetch_models_dev", return_value={}),
            patch("hermes_cli.models.cached_provider_model_ids", return_value=["synthetic-model"]),
        ):
            reply = await self.command("/model")
        self.assertIn("--provider openrouter", reply)
        self.assertIn("/model private", reply)
        self.assertEqual(self.route()[0], "synthetic-private-model")
        self.assertEqual(self.config_path.read_bytes(), self.original_config)

    async def test_rotation_refreshes_an_existing_override_and_empty_key_fails_closed(self):
        self.save_key("synthetic-first-key")
        await self.command("/model anthropic/claude-sonnet-4.6 --provider openrouter --session")
        self.save_key("synthetic-rotated-key")
        self.assertEqual(self.route()[1]["api_key"], "synthetic-rotated-key")
        self.save_key("")
        with self.assertRaisesRegex(ValueError, "Save an OpenRouter key"):
            self.route()
        self.assertEqual(self.route(1)[1]["api_key"], "synthetic-private-key")
        self.assertEqual(self.config_path.read_bytes(), self.original_config)

    async def test_missing_key_cannot_use_the_private_key_or_change_session(self):
        with self.assertRaisesRegex(ValueError, "Save an OpenRouter key"):
            self.resolve(requested="openrouter")
        await self.command("/model anthropic/claude-sonnet-4.6 --provider openrouter --session")
        self.assertEqual(self.route()[0], "synthetic-private-model")
        self.assertEqual(self.config_path.read_bytes(), self.original_config)

    async def test_private_alias_restores_private_route_without_new_session(self):
        self.save_key("synthetic-openrouter-key")
        await self.command("/model anthropic/claude-sonnet-4.6 --provider openrouter --session")
        before = self.runner.session_store.get_or_create_session(self.sources[0]).session_id
        await self.command("/model private")
        self.assertEqual(self.route()[0], "synthetic-private-model")
        self.assertEqual(self.route()[1]["base_url"], "https://private.example.invalid/v1")
        self.assertEqual(self.route()[1]["api_key"], "synthetic-private-key")
        self.assertEqual(
            self.runner.session_store.get_or_create_session(self.sources[0]).session_id, before
        )
        self.assertEqual(self.config_path.read_bytes(), self.original_config)

    async def test_new_session_clears_override(self):
        self.save_key("synthetic-openrouter-key")
        await self.command("/model anthropic/claude-sonnet-4.6 --provider openrouter --session")
        await self.runner._handle_reset_command(
            self.MessageEvent(text="/new", source=self.sources[0])
        )
        self.assertEqual(self.route()[0], "synthetic-private-model")
        self.assertEqual(self.config_path.read_bytes(), self.original_config)

    async def test_private_session_survives_restart_when_default_is_openrouter(self):
        self.save_key("synthetic-openrouter-key")
        self.config_path.write_text(
            "model:\n  provider: openrouter\n  default: anthropic/claude-sonnet-4.6\n  base_url: https://openrouter.ai/api/v1\n"
        )
        original = self.config_path.read_bytes()
        await self.command("/model private")
        self.assertEqual(self.route()[1]["api_key"], "synthetic-private-key")
        restarted = self.new_runner()
        self.assertEqual(self.route(runner=restarted)[1]["api_key"], "synthetic-private-key")
        self.assertEqual(self.route(1, restarted)[1]["api_key"], "synthetic-openrouter-key")
        self.assertEqual(self.config_path.read_bytes(), original)

    async def test_private_compression_failure_cannot_use_saved_openrouter_key(self):
        from agent import auxiliary_client as aux

        self.save_key("synthetic-openrouter-key")
        # Cover both a private default and a private session over an OR default.
        for use_private_alias in (False, True):
            if use_private_alias:
                self.config_path.write_text(
                    "model:\n  provider: openrouter\n  default: anthropic/claude-sonnet-4.6\n"
                )
                await self.command("/model private")
            model, runtime = self.route()
            runtime = {**runtime, "model": model}
            history = [{"role": "user", "content": "Private conversation transcript."}]
            for asynchronous in (False, True):
                with self.subTest(private_alias=use_private_alias, asynchronous=asynchronous):
                    aux.shutdown_cached_clients()
                    aux._reset_aux_unhealthy_cache()
                    attempts = []

                    def fail_completion(client, request, attempts=attempts, **kwargs):
                        attempts.append((str(client.base_url).rstrip("/"), request["messages"]))
                        raise TimeoutError("synthetic private compression timeout")

                    async def fail_async_completion(client, request, **kwargs):
                        return fail_completion(client, request, **kwargs)

                    with (
                        patch.object(aux, "_relay_sync_completion", side_effect=fail_completion),
                        patch.object(
                            aux, "_relay_async_completion", side_effect=fail_async_completion
                        ),
                        self.assertRaises(TimeoutError),
                    ):
                        if asynchronous:
                            await aux.async_call_llm(
                                task="compression", main_runtime=runtime, messages=history
                            )
                        else:
                            aux.call_llm(task="compression", main_runtime=runtime, messages=history)
                    self.assertTrue(attempts)
                    self.assertTrue(
                        all(url == "https://private.example.invalid/v1" for url, _ in attempts),
                        attempts,
                    )
                    self.assertEqual(attempts[0][1], history)
                    aux.shutdown_cached_clients()

    async def test_auxiliary_openrouter_requires_the_effective_session_choice(self):
        from agent import auxiliary_client as aux

        self.save_key("synthetic-openrouter-key")
        await self.command("/model anthropic/claude-sonnet-4.6 --provider openrouter --session")
        model, runtime = self.route()
        client, selected = aux.get_text_auxiliary_client(main_runtime={**runtime, "model": model})
        self.assertEqual(selected, model)
        self.assertEqual(str(client.base_url).rstrip("/"), "https://openrouter.ai/api/v1")
        self.assertEqual(client.api_key, "synthetic-openrouter-key")
        client.close()
        await self.command("/model private")
        model, runtime = self.route()
        self.config_path.write_text(
            "model:\n  provider: openrouter\n  default: anthropic/claude-sonnet-4.6\n"
        )
        with patch.object(aux, "_main_model_supports_vision", return_value=False):
            provider, client, _ = aux.resolve_vision_provider_client(
                main_runtime={**runtime, "model": model}
            )
        self.assertNotEqual(provider, "openrouter")
        if client:
            client.close()
        with patch.object(
            aux, "_is_provider_unhealthy", side_effect=lambda label: label == "local/custom"
        ):
            client, _ = aux.get_text_auxiliary_client(main_runtime={**runtime, "model": model})
        if client:
            self.assertNotEqual(str(client.base_url).rstrip("/"), "https://openrouter.ai/api/v1")
            client.close()
        client, _ = aux.get_text_auxiliary_client()
        self.assertEqual(str(client.base_url).rstrip("/"), "https://openrouter.ai/api/v1")
        self.assertEqual(client.api_key, "synthetic-openrouter-key")
        client.close()

    async def test_cached_auxiliary_clients_follow_private_switch_and_key_rotation(self):
        from agent import auxiliary_client as aux

        aux.shutdown_cached_clients()
        self.addCleanup(aux.shutdown_cached_clients)
        self.save_key("synthetic-first-key")
        auxiliary_config = (
            "auxiliary:\n  compression:\n    provider: openrouter\n"
            "    model: anthropic/claude-sonnet-4.6\n"
        )
        self.config_path.write_text(self.config_path.read_text() + auxiliary_config)
        await self.command("/model anthropic/claude-sonnet-4.6 --provider openrouter --session")
        model, runtime = self.route()
        runtime = {**runtime, "model": model}
        attempts = []

        def complete(client, request, **kwargs):
            attempts.append((str(client.base_url).rstrip("/"), client.api_key))
            return SimpleNamespace(
                choices=[
                    SimpleNamespace(
                        message=SimpleNamespace(content="Summary", tool_calls=None),
                        finish_reason="stop",
                    )
                ]
            )

        async def async_complete(client, request, **kwargs):
            return complete(client, request, **kwargs)

        async def compress_both(selected):
            history = [{"role": "user", "content": "Conversation to summarize."}]
            aux.call_llm(task="compression", main_runtime=selected, messages=history)
            await aux.async_call_llm(task="compression", main_runtime=selected, messages=history)

        with (
            patch.object(aux, "_relay_sync_completion", side_effect=complete),
            patch.object(aux, "_relay_async_completion", side_effect=async_complete),
        ):
            await compress_both(runtime)
            self.assertEqual(
                attempts, [("https://openrouter.ai/api/v1", "synthetic-first-key")] * 2
            )
            # Keep both caches warm across private return and credential changes.
            self.config_path.write_text(
                "model:\n  provider: openrouter\n  default: anthropic/claude-sonnet-4.6\n"
                + auxiliary_config
            )
            await self.command("/model private")
            model, private_runtime = self.route()
            attempts.clear()
            await compress_both({**private_runtime, "model": model})
            self.assertEqual(
                attempts, [("https://private.example.invalid/v1", "synthetic-private-key")] * 2
            )
            await self.command("/model anthropic/claude-sonnet-4.6 --provider openrouter --session")
            model, runtime = self.route()
            runtime = {**runtime, "model": model}
            attempts.clear()
            self.save_key("synthetic-rotated-key")
            await compress_both(runtime)
            self.assertEqual(
                attempts, [("https://openrouter.ai/api/v1", "synthetic-rotated-key")] * 2
            )
            attempts.clear()
            self.save_key("")
            for asynchronous in (False, True):
                with (
                    self.subTest(asynchronous=asynchronous),
                    self.assertRaises((ValueError, RuntimeError)),
                ):
                    if asynchronous:
                        await aux.async_call_llm(
                            task="compression", main_runtime=runtime, messages=[]
                        )
                    else:
                        aux.call_llm(task="compression", main_runtime=runtime, messages=[])
            self.assertEqual(attempts, [])


if __name__ == "__main__":
    unittest.main()

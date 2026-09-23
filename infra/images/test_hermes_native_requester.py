"""Exercise the sealed Hermes requester handoff without model inference."""

import concurrent.futures
import subprocess
import sys
import threading
import time
import unittest
from unittest.mock import Mock, patch

from gateway.session_context import get_session_env
from hermes_cli import finite_requester_context as requester
from tui_gateway import server


def envelope(user="a1"):
    return {
        "userId": user * 32,
        "email": "owner@example.com",
        "sitesAssertion": "b2" * 32,
        "expiresAt": int(time.time()) + 60,
    }


class NativeRequesterTests(unittest.TestCase):
    def test_gateway_startup_cannot_shadow_sealed_requester_handler(self):
        # A fresh interpreter matters: importing server first hides the
        # gateway's sys.path mutation behind Python's module cache.
        result = subprocess.run(
            [
                sys.executable,
                "-c",
                "import gateway.platforms.base; "
                "import inspect; from tui_gateway import server; "
                "assert 'finite_requester' in inspect.signature(server._run_prompt_submit).parameters, server.__file__",
            ],
            capture_output=True,
            text=True,
            timeout=30,
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_invalid_and_expired_context_has_no_identity(self):
        valid = envelope()
        self.assertEqual(requester.normalize(valid), valid)
        for field, value in (
            ("expiresAt", True),
            ("expiresAt", int(time.time()) - 1),
            ("userId", "wrong"),
            ("sitesAssertion", "unsigned"),
            ("email", "missing-at"),
        ):
            with self.subTest(field=field, value=value):
                self.assertIsNone(requester.normalize({**valid, field: value}))
        self.assertIsNone(requester.normalize(None))

    def test_identity_without_sites_claims_remains_turn_scoped(self):
        identity = {"userId": "a1" * 32, "expiresAt": int(time.time()) + 60}
        tokens = requester.bind(identity)
        try:
            self.assertEqual(requester.current(), identity)
            self.assertEqual(get_session_env("HERMES_SESSION_USER_ID"), identity["userId"])
        finally:
            requester.reset(tokens)
        self.assertIsNone(requester.current())
        for partial in ({"email": "owner@example.com"}, {"sitesAssertion": "b2" * 32}):
            self.assertIsNone(requester.normalize({**identity, **partial}))

    def test_concurrent_turns_do_not_share_identity_and_cleanup_restores_context(self):
        barrier = threading.Barrier(2)

        def run(user):
            before = get_session_env("HERMES_SESSION_USER_ID", "")
            tokens = requester.bind(envelope(user))
            try:
                barrier.wait(timeout=5)
                self.assertEqual(requester.current()["userId"], user * 32)
                self.assertEqual(get_session_env("HERMES_SESSION_USER_ID"), user * 32)
                self.assertEqual(get_session_env("HERMES_SESSION_PLATFORM"), "local")
            finally:
                requester.reset(tokens)
            self.assertIsNone(requester.current())
            self.assertEqual(get_session_env("HERMES_SESSION_USER_ID", ""), before)

        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            list(pool.map(run, ("a1", "c3")))
        self.assertIsNone(requester.current())

    def test_queue_keeps_distinct_requesters_in_separate_envelopes(self):
        session = {}
        first, second = envelope(), envelope("c3")
        server._enqueue_prompt(session, "first", None, finite_requester=first)
        server._enqueue_prompt(session, "second", None, finite_requester=second)
        self.assertEqual(session["queued_prompt"]["text"], "first")
        self.assertEqual(session["queued_prompt"]["finite_requester"], first)
        self.assertEqual(session["queued_prompts"][0]["text"], "second")
        self.assertEqual(session["queued_prompts"][0]["finite_requester"], second)

    def test_compute_frame_carries_assertion_outside_history_and_snapshot(self):
        context = envelope()
        session = {"history_lock": threading.RLock(), "history": []}
        server._enqueue_prompt(session, "publish my site", None, finite_requester=context)
        self.assertEqual(server._queued_prompt_snapshot(session), {"user": "publish my site"})
        frame = server._compute_host_turn_frame(
            "r", "s", session, "publish my site", finite_requester=context
        )
        self.assertEqual(frame["finite_requester"], context)
        self.assertEqual(frame["text"], "publish my site")
        self.assertEqual(frame["history"], [])

    def test_compute_completion_during_dispatch_does_not_restore_old_requester(self):
        for next_user in (None, "c3" * 32):
            with self.subTest(next_user=next_user):
                session = {"history_lock": threading.RLock(), "history": [], "running": True}

                def complete(frame, *, on_complete):
                    on_complete({"type": "turn.done", "session_info_emitted": True})

                def drain(*args):
                    if next_user:
                        session["_finite_requester_user"] = next_user

                supervisor = Mock(submit_turn=Mock(side_effect=complete))
                with (
                    patch.object(server, "_load_dashboard_process_isolation_config"),
                    patch.object(server, "_get_compute_host_supervisor", return_value=supervisor),
                    patch.object(server, "_session_info", return_value={}),
                    patch.object(server, "_drain_queued_prompt", side_effect=drain),
                ):
                    response = server._submit_prompt_to_compute_host(
                        "r", "s", session, "hello", finite_requester=envelope()
                    )
                self.assertNotIn("error", response)
                self.assertEqual(session.get("_finite_requester_user"), next_user)

    def test_failed_compute_dispatch_clears_requester_for_in_process_retry(self):
        session = {"history_lock": threading.RLock(), "history": [], "running": True}

        observed = []

        def fail(frame, *, on_complete):
            observed.append(session.get("_finite_requester_user"))
            on_complete({"type": "turn.error", "reason": "send_failed"})
            raise OSError("closed fixture pipe")

        with (
            patch.object(server, "_load_dashboard_process_isolation_config"),
            patch.object(
                server, "_get_compute_host_supervisor", return_value=Mock(submit_turn=fail)
            ),
        ):
            response = server._submit_prompt_to_compute_host(
                "r", "s", session, "hello", finite_requester=envelope()
            )
        self.assertIn("error", response)
        self.assertEqual(observed, [envelope()["userId"]])
        self.assertNotIn("_finite_requester_user", session)
        self.assertTrue(session["running"])

    def test_other_requester_cannot_steer_an_active_turn(self):
        agent = Mock()
        session = {
            "history_lock": threading.RLock(),
            "running": True,
            "agent": agent,
            "_finite_requester_user": "a1" * 32,
        }
        second = envelope("c3")
        with patch.object(server, "_load_busy_input_mode", return_value="steer"):
            server._handle_busy_submit("r", "s", session, "second", None, finite_requester=second)
        agent.steer.assert_not_called()
        agent.interrupt.assert_not_called()
        self.assertEqual(session["queued_prompt"]["finite_requester"], second)


if __name__ == "__main__":
    unittest.main()

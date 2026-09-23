"""Exercise the sealed Hermes requester handoff without model inference."""

import concurrent.futures
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

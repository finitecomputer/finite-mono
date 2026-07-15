import unittest

from gateway.config import GatewayConfig, Platform
from gateway.session import SessionSource, build_session_context, build_session_context_prompt


class PinnedHermesSenderContextTests(unittest.TestCase):
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

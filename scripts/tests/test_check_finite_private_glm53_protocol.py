from __future__ import annotations

import json
import unittest

from scripts.check_finite_private_glm53_protocol import (
    MODEL,
    accumulate_tool_calls,
    completion_message,
    response_usage,
    score_tool_calls,
)


class Glm53ProtocolTests(unittest.TestCase):
    def test_completion_message_requires_canonical_identity(self) -> None:
        message = completion_message(
            {
                "model": MODEL,
                "choices": [
                    {
                        "message": {
                            "reasoning_content": "reason",
                            "content": "answer",
                        }
                    }
                ],
            }
        )
        self.assertEqual(message["content"], "answer")
        with self.assertRaisesRegex(ValueError, "expected"):
            completion_message(
                {
                    "model": "wrong-model",
                    "choices": [{"message": {"content": "answer"}}],
                }
            )

    def test_streaming_tool_deltas_are_merged_by_index(self) -> None:
        calls = accumulate_tool_calls(
            [
                {
                    "model": MODEL,
                    "choices": [
                        {
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call-1",
                                        "function": {
                                            "name": "get_weather",
                                            "arguments": '{"city":"Aus',
                                        },
                                    }
                                ]
                            }
                        }
                    ],
                },
                {
                    "model": MODEL,
                    "choices": [
                        {
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "function": {
                                            "arguments": 'tin","state":"Texas"}',
                                        },
                                    }
                                ]
                            }
                        }
                    ],
                },
            ]
        )
        self.assertEqual(calls[0]["function"]["name"], "get_weather")
        self.assertEqual(
            json.loads(calls[0]["function"]["arguments"])["city"], "Austin"
        )

    def test_tool_scoring_requires_valid_json_and_requested_city(self) -> None:
        calls = [
            {
                "id": "call-1",
                "function": {
                    "name": "get_weather",
                    "arguments": '{"city":"Austin","state":"Texas"}',
                },
            }
        ]
        self.assertEqual(score_tool_calls(calls, {"austin"}), (True, "matched"))
        calls[0]["function"]["arguments"] = "not json"
        self.assertEqual(
            score_tool_calls(calls, {"austin"}), (False, "invalid tool JSON")
        )

    def test_usage_accepts_openai_chat_shape(self) -> None:
        self.assertEqual(
            response_usage(
                {
                    "usage": {
                        "prompt_tokens": 128000,
                        "completion_tokens": 16,
                    }
                }
            ),
            (128000, 16),
        )


if __name__ == "__main__":
    unittest.main()

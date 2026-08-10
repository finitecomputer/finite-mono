from __future__ import annotations

import json
import unittest
from unittest import mock

from scripts.check_deepseek_v4_0731_quality import CASES, _payload, _score, main


class DeepSeekV40731QualityTests(unittest.TestCase):
    def test_self_hosted_payload_uses_official_sampling_and_thinking(self) -> None:
        payload = _payload(
            CASES[0],
            model="deepseek-v4-flash-0731",
            effort="max",
            lane="self-hosted",
        )

        self.assertEqual(payload["reasoning_effort"], "max")
        self.assertEqual(payload["temperature"], 1.0)
        self.assertEqual(payload["top_p"], 0.95)
        self.assertEqual(payload["chat_template_kwargs"], {"thinking": True})

    def test_hosted_payload_uses_native_reasoning_contract(self) -> None:
        payload = _payload(
            CASES[0],
            model="deepseek-v4-flash",
            effort="max",
            lane="deepseek-hosted",
        )

        self.assertNotIn("chat_template_kwargs", payload)
        self.assertEqual(payload["reasoning_effort"], "max")

    def test_exact_answer_requires_separate_reasoning(self) -> None:
        response = {
            "choices": [
                {
                    "message": {
                        "reasoning_content": "17 times 19 is 323.",
                        "content": "323",
                    }
                }
            ]
        }

        passed, _, reasoning_characters = _score(CASES[0], response)
        self.assertTrue(passed)
        self.assertGreater(reasoning_characters, 0)

        response["choices"][0]["message"]["reasoning_content"] = ""
        passed, detail, _ = _score(CASES[0], response)
        self.assertFalse(passed)
        self.assertIn("reasoning_content", detail)

    def test_vllm_025_reasoning_field_is_accepted(self) -> None:
        response = {
            "choices": [
                {
                    "message": {
                        "reasoning": "17 times 19 is 323.",
                        "content": "323",
                    }
                }
            ]
        }

        passed, _, reasoning_characters = _score(CASES[0], response)
        self.assertTrue(passed)
        self.assertGreater(reasoning_characters, 0)

    def test_tool_case_requires_named_tool_and_city(self) -> None:
        case = next(item for item in CASES if item.expected_tool)
        response = {
            "choices": [
                {
                    "message": {
                        "reasoning_content": "I should call the weather tool.",
                        "content": None,
                        "tool_calls": [
                            {
                                "type": "function",
                                "function": {
                                    "name": "get_weather",
                                    "arguments": json.dumps(
                                        {"city": "Austin", "state": "Texas"}
                                    ),
                                },
                            }
                        ],
                    }
                }
            ]
        }

        passed, _, _ = _score(case, response)
        self.assertTrue(passed)

    @mock.patch("scripts.check_deepseek_v4_0731_quality.run", return_value=[])
    @mock.patch.dict("os.environ", {"QUALITY_TEST_KEY": "secret"})
    def test_report_records_host_and_schema_but_not_key(self, _run: mock.Mock) -> None:
        arguments = [
            "quality",
            "--endpoint",
            "https://reference.example/v1",
            "--model",
            "deepseek-v4-flash",
            "--api-key-env",
            "QUALITY_TEST_KEY",
        ]
        with mock.patch("sys.argv", arguments), mock.patch("builtins.print") as output:
            self.assertEqual(main(), 0)

        report = json.loads(output.call_args.args[0])
        self.assertEqual(report["schema"], "finite-deepseek-quality-v1")
        self.assertEqual(report["endpoint_host"], "reference.example")
        self.assertNotIn("secret", output.call_args.args[0])


if __name__ == "__main__":
    unittest.main()

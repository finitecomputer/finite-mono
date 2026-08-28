from __future__ import annotations

import json
import unittest
from unittest import mock

from scripts.check_finite_private_glm53_quality import (
    CASES,
    MODEL,
    _identity_violations,
    _payload,
    _score,
    main,
)


class Glm53QualityTests(unittest.TestCase):
    def test_payload_pins_thinking_and_official_sampling(self) -> None:
        payload = _payload(CASES[0], model=MODEL, effort="max")
        self.assertEqual(payload["reasoning_effort"], "max")
        self.assertEqual(payload["temperature"], 1.0)
        self.assertEqual(payload["top_p"], 0.95)
        self.assertEqual(
            payload["chat_template_kwargs"], {"enable_thinking": True}
        )

    def test_exact_answer_requires_separate_parsed_reasoning(self) -> None:
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
        self.assertIn("reasoning", detail)

    def test_tool_case_requires_named_tool_and_valid_json_arguments(self) -> None:
        case = next(item for item in CASES if item.expected_tool)
        response = {
            "choices": [
                {
                    "message": {
                        "reasoning": "I should call the weather tool.",
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
        response["choices"][0]["message"]["tool_calls"][0]["function"][
            "arguments"
        ] = "not json"
        passed, detail, _ = _score(case, response)
        self.assertFalse(passed)
        self.assertIn("JSON", detail)

    def test_identity_requires_canonical_request_and_response_model(self) -> None:
        violations = _identity_violations(
            model="deepseek-v4-flash-0731",
            results=[
                {
                    "effort": "high",
                    "case": "logic",
                    "response_model": "wrong-model",
                }
            ],
        )
        self.assertTrue(any("requested model" in item for item in violations))
        self.assertTrue(any("wrong-model" in item for item in violations))

    @mock.patch(
        "scripts.check_finite_private_glm53_quality.run",
        return_value=[
            {
                "case": "arithmetic",
                "effort": "high",
                "passed": True,
                "response_model": MODEL,
            }
        ],
    )
    @mock.patch.dict("os.environ", {"QUALITY_TEST_KEY": "secret-value"})
    def test_report_records_identity_but_never_key(self, _run: mock.Mock) -> None:
        arguments = [
            "quality",
            "--endpoint",
            "https://finite-private.example/v1",
            "--model",
            MODEL,
            "--api-key-env",
            "QUALITY_TEST_KEY",
            "--efforts",
            "high",
        ]
        with mock.patch("sys.argv", arguments), mock.patch("builtins.print") as output:
            self.assertEqual(main(), 0)
        report_text = output.call_args.args[0]
        report = json.loads(report_text)
        self.assertEqual(report["schema"], "finite-private-glm53-quality-v1")
        self.assertEqual(report["model"], MODEL)
        self.assertNotIn("secret-value", report_text)


if __name__ == "__main__":
    unittest.main()

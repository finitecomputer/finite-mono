#!/usr/bin/env python3
"""Small protocol/correctness gate for DeepSeek V4 serving candidates."""

from __future__ import annotations

import json
import sys
import urllib.request


URL = "http://127.0.0.1:8000/v1/chat/completions"
MODEL = "deepseek-v4-flash-0731"


def complete(payload: dict) -> dict:
    body = json.dumps({"model": MODEL, **payload}).encode()
    request = urllib.request.Request(
        URL, data=body, headers={"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(request, timeout=180) as response:
        return json.load(response)


def main() -> int:
    failures: list[str] = []

    plain = complete(
        {
            "messages": [{"role": "user", "content": "Return only 19 + 23."}],
            "temperature": 0,
            "max_tokens": 32,
            "chat_template_kwargs": {"enable_thinking": False},
        }
    )["choices"][0]["message"]
    plain_text = plain.get("content") or ""
    if "42" not in plain_text or "<think" in plain_text.lower():
        failures.append(f"plain arithmetic response was {plain_text!r}")

    reasoning = complete(
        {
            "messages": [
                {
                    "role": "user",
                    "content": (
                        "Prove that sqrt(2) is irrational, then give a concise "
                        "one-sentence conclusion."
                    ),
                }
            ],
            "temperature": 0,
            "max_tokens": 512,
            "chat_template_kwargs": {"enable_thinking": True},
            "reasoning_effort": "high",
        }
    )["choices"][0]["message"]
    hidden_reasoning = reasoning.get("reasoning") or reasoning.get("reasoning_content")
    final_text = reasoning.get("content") or ""
    if not hidden_reasoning:
        failures.append("reasoning field was empty")
    if "<think" in final_text.lower():
        failures.append("raw reasoning marker leaked into final content")

    tool = complete(
        {
            "messages": [
                {
                    "role": "user",
                    "content": "What is the weather in Austin? Use the tool.",
                }
            ],
            "temperature": 0,
            "max_tokens": 256,
            "chat_template_kwargs": {"enable_thinking": False},
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "description": "Get current weather for a city.",
                        "parameters": {
                            "type": "object",
                            "properties": {"city": {"type": "string"}},
                            "required": ["city"],
                        },
                    },
                }
            ],
            "tool_choice": "auto",
        }
    )["choices"][0]["message"]
    calls = tool.get("tool_calls") or []
    if not calls or calls[0].get("function", {}).get("name") != "get_weather":
        failures.append(f"tool response was {tool!r}")

    report = {
        "plain": plain_text.strip(),
        "reasoning_chars": len(hidden_reasoning or ""),
        "final_chars": len(final_text),
        "tool_name": (calls[0].get("function", {}).get("name") if calls else None),
        "failures": failures,
    }
    print(json.dumps(report))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())

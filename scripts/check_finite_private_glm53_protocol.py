#!/usr/bin/env python3
"""Gate GLM-5.3-Flash model identity, reasoning, tools, and long context.

The key is read only from a named environment variable. Output contains case
status, timings, token usage, and protocol shapes, never credentials, prompts,
or generated content.
"""

from __future__ import annotations

import argparse
import json
import os
import time
import urllib.error
import urllib.request
import uuid
from typing import Any


MODEL = "glm-5-3-flash"
CONTEXT_TARGETS = (128_000, 360_000)
WEATHER_TOOL = {
    "type": "function",
    "function": {
        "name": "get_weather",
        "description": "Get weather for a city.",
        "parameters": {
            "type": "object",
            "properties": {
                "city": {"type": "string"},
                "state": {"type": "string"},
            },
            "required": ["city", "state"],
            "additionalProperties": False,
        },
    },
}


def response_usage(response: dict[str, Any]) -> tuple[int, int]:
    usage = response.get("usage")
    if not isinstance(usage, dict):
        return 0, 0
    return int(usage.get("prompt_tokens") or 0), int(
        usage.get("completion_tokens") or 0
    )


def completion_message(response: dict[str, Any]) -> dict[str, Any]:
    if response.get("model") != MODEL:
        raise ValueError(
            f"returned model {response.get('model')!r}; expected {MODEL!r}"
        )
    choices = response.get("choices")
    if not isinstance(choices, list) or not choices or not isinstance(choices[0], dict):
        raise ValueError("response lacked a first choice")
    message = choices[0].get("message")
    if not isinstance(message, dict):
        raise ValueError("response lacked an assistant message")
    return message


def accumulate_tool_calls(events: list[dict[str, Any]]) -> list[dict[str, Any]]:
    calls: dict[int, dict[str, Any]] = {}
    for event in events:
        choices = event.get("choices")
        if not isinstance(choices, list):
            continue
        for choice in choices:
            delta = choice.get("delta") if isinstance(choice, dict) else None
            tool_deltas = delta.get("tool_calls") if isinstance(delta, dict) else None
            if not isinstance(tool_deltas, list):
                continue
            for tool_delta in tool_deltas:
                if not isinstance(tool_delta, dict):
                    continue
                index = int(tool_delta.get("index") or 0)
                call = calls.setdefault(
                    index,
                    {
                        "id": "",
                        "type": "function",
                        "function": {"name": "", "arguments": ""},
                    },
                )
                if isinstance(tool_delta.get("id"), str):
                    call["id"] += tool_delta["id"]
                function = tool_delta.get("function")
                if isinstance(function, dict):
                    if isinstance(function.get("name"), str):
                        call["function"]["name"] += function["name"]
                    if isinstance(function.get("arguments"), str):
                        call["function"]["arguments"] += function["arguments"]
    return [calls[index] for index in sorted(calls)]


def score_tool_calls(
    calls: list[dict[str, Any]], expected_cities: set[str]
) -> tuple[bool, str]:
    observed_cities: set[str] = set()
    for call in calls:
        function = call.get("function") if isinstance(call, dict) else None
        if not isinstance(function, dict) or function.get("name") != "get_weather":
            return False, "unexpected tool name"
        arguments = function.get("arguments")
        if not isinstance(arguments, str):
            return False, "tool arguments were not a string"
        try:
            decoded = json.loads(arguments)
        except json.JSONDecodeError:
            return False, "invalid tool JSON"
        if not isinstance(decoded, dict) or not isinstance(decoded.get("city"), str):
            return False, "tool JSON lacked city"
        observed_cities.add(decoded["city"].lower())
    return (
        (True, "matched")
        if expected_cities.issubset(observed_cities)
        else (
            False,
            f"missing requested cities: {sorted(expected_cities - observed_cities)}",
        )
    )


class ProtocolClient:
    def __init__(self, endpoint: str, api_key: str, timeout: float) -> None:
        self.endpoint = endpoint.rstrip("/")
        self.api_key = api_key
        self.timeout = timeout

    def _request(self, body: bytes) -> urllib.request.Request:
        return urllib.request.Request(
            self.endpoint + "/chat/completions",
            data=body,
            headers={
                "authorization": f"Bearer {self.api_key}",
                "content-type": "application/json",
                "x-request-id": f"fp_glm53_protocol_{uuid.uuid4().hex}",
            },
            method="POST",
        )

    def post(self, payload: dict[str, Any]) -> tuple[dict[str, Any], float]:
        started = time.monotonic()
        try:
            with urllib.request.urlopen(
                self._request(json.dumps(payload, separators=(",", ":")).encode()),
                timeout=self.timeout,
            ) as response:
                body = response.read()
        except urllib.error.HTTPError as error:
            detail = error.read(500).decode("utf-8", "replace")
            raise RuntimeError(f"HTTP {error.code}: {detail}") from error
        value = json.loads(body)
        if not isinstance(value, dict):
            raise RuntimeError("response was not a JSON object")
        return value, time.monotonic() - started

    def stream(
        self, payload: dict[str, Any]
    ) -> tuple[list[dict[str, Any]], bool, tuple[int, int], float]:
        payload = dict(payload)
        payload.update(stream=True, stream_options={"include_usage": True})
        started = time.monotonic()
        events: list[dict[str, Any]] = []
        saw_done = False
        usage = (0, 0)
        with urllib.request.urlopen(
            self._request(json.dumps(payload, separators=(",", ":")).encode()),
            timeout=self.timeout,
        ) as response:
            for raw_line in response:
                line = raw_line.decode("utf-8", "replace").strip()
                if not line.startswith("data: "):
                    continue
                data = line[6:]
                if data == "[DONE]":
                    saw_done = True
                    continue
                event = json.loads(data)
                if event.get("model") not in (None, MODEL):
                    raise RuntimeError(
                        f"stream returned wrong model {event.get('model')!r}"
                    )
                event_usage = response_usage(event)
                if event_usage != (0, 0):
                    usage = event_usage
                events.append(event)
        return events, saw_done, usage, time.monotonic() - started

    def malformed_status(self) -> tuple[int, float]:
        started = time.monotonic()
        try:
            urllib.request.urlopen(self._request(b'{"model":'), timeout=self.timeout)
        except urllib.error.HTTPError as error:
            error.read()
            return error.code, time.monotonic() - started
        return 200, time.monotonic() - started

    def cancel_after_first_event(self, payload: dict[str, Any]) -> float:
        payload = dict(payload)
        payload.update(stream=True, max_tokens=4096)
        started = time.monotonic()
        with urllib.request.urlopen(
            self._request(json.dumps(payload, separators=(",", ":")).encode()),
            timeout=self.timeout,
        ) as response:
            for raw_line in response:
                if raw_line.startswith(b"data: {"):
                    response.close()
                    return time.monotonic() - started
        raise RuntimeError("cancel stream produced no event")


def base_payload(
    messages: list[dict[str, Any]], *, thinking: bool = True
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "model": MODEL,
        "messages": messages,
        "max_tokens": 1024,
        "temperature": 1.0 if thinking else 0.0,
        "top_p": 0.95 if thinking else 1.0,
        "chat_template_kwargs": {"enable_thinking": thinking},
    }
    if thinking:
        payload["reasoning_effort"] = "high"
    return payload


def result(
    case: str, passed: bool, detail: str, elapsed: float, **facts: Any
) -> dict[str, Any]:
    return {
        "case": case,
        "passed": passed,
        "detail": detail,
        "elapsed_seconds": round(elapsed, 3),
        **facts,
    }


def context_targets(max_tokens: int) -> tuple[int, ...]:
    if max_tokens <= 0:
        raise ValueError("max_context_tokens must be positive")
    return tuple(target for target in CONTEXT_TARGETS if target <= max_tokens)


def run_gate(
    client: ProtocolClient, *, max_context_tokens: int = CONTEXT_TARGETS[-1]
) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []

    response, elapsed = client.post(
        base_payload(
            [{"role": "user", "content": "Reply exactly: protocol ok"}],
            thinking=False,
        )
    )
    message = completion_message(response)
    reasoning = message.get("reasoning_content") or message.get("reasoning")
    results.append(
        result(
            "thinking_off",
            isinstance(message.get("content"), str) and not reasoning,
            "content separated without reasoning"
            if not reasoning
            else "reasoning leaked",
            elapsed,
        )
    )

    response, elapsed = client.post(
        base_payload([{"role": "user", "content": "Explain why 17 * 19 is 323."}])
    )
    message = completion_message(response)
    reasoning = message.get("reasoning_content") or message.get("reasoning")
    thinking_passed = (
        isinstance(reasoning, str)
        and bool(reasoning.strip())
        and isinstance(message.get("content"), str)
    )
    results.append(
        result(
            "thinking_high",
            thinking_passed,
            "parsed" if thinking_passed else "missing separated reasoning/content",
            elapsed,
        )
    )

    tool_payload = base_payload(
        [{"role": "user", "content": "Use get_weather for Austin, Texas."}]
    )
    tool_payload.update(
        tools=[WEATHER_TOOL],
        tool_choice={"type": "function", "function": {"name": "get_weather"}},
    )
    response, elapsed = client.post(tool_payload)
    tool_message = completion_message(response)
    calls = tool_message.get("tool_calls")
    calls = calls if isinstance(calls, list) else []
    passed, detail = score_tool_calls(calls, {"austin"})
    results.append(
        result("forced_tool", passed, detail, elapsed, tool_calls=len(calls))
    )

    if calls:
        follow_messages = list(tool_payload["messages"])
        follow_messages.append(tool_message)
        follow_messages.append(
            {
                "role": "tool",
                "tool_call_id": calls[0].get("id"),
                "content": json.dumps({"temperature_f": 91}),
            }
        )
        follow_payload = base_payload(follow_messages)
        follow_payload["chat_template_kwargs"]["clear_thinking"] = True
        follow_payload.update(tools=[WEATHER_TOOL], tool_choice="none")
        response, elapsed = client.post(follow_payload)
        follow_message = completion_message(response)
        passed = isinstance(follow_message.get("content"), str) and bool(
            follow_message["content"].strip()
        )
        results.append(
            result(
                "tool_result_second_turn",
                passed,
                "completed" if passed else "missing final content",
                elapsed,
            )
        )
    else:
        results.append(
            result("tool_result_second_turn", False, "first tool call unavailable", 0.0)
        )

    events, saw_done, usage, elapsed = client.stream(tool_payload)
    stream_calls = accumulate_tool_calls(events)
    passed, detail = score_tool_calls(stream_calls, {"austin"})
    passed = passed and saw_done and usage[1] > 0
    results.append(
        result(
            "streaming_tool",
            passed,
            detail if passed else f"{detail}; done={saw_done}; usage={usage}",
            elapsed,
            tool_calls=len(stream_calls),
            completion_tokens=usage[1],
        )
    )

    parallel_payload = base_payload(
        [
            {
                "role": "user",
                "content": "Call get_weather separately for Austin, Texas and Boston, Massachusetts.",
            }
        ]
    )
    parallel_payload.update(
        tools=[WEATHER_TOOL], tool_choice="required", parallel_tool_calls=True
    )
    response, elapsed = client.post(parallel_payload)
    parallel_message = completion_message(response)
    parallel_calls = parallel_message.get("tool_calls")
    parallel_calls = parallel_calls if isinstance(parallel_calls, list) else []
    passed, detail = score_tool_calls(parallel_calls, {"austin", "boston"})
    results.append(
        result(
            "parallel_tools", passed, detail, elapsed, tool_calls=len(parallel_calls)
        )
    )

    json_payload = base_payload(
        [
            {
                "role": "user",
                "content": 'Return one JSON object with integer field "answer" equal to 323.',
            }
        ],
        thinking=False,
    )
    json_payload["response_format"] = {"type": "json_object"}
    response, elapsed = client.post(json_payload)
    json_message = completion_message(response)
    try:
        decoded = json.loads(json_message.get("content", ""))
        passed = decoded == {"answer": 323}
    except json.JSONDecodeError:
        passed = False
    results.append(
        result(
            "json_object",
            passed,
            "matched" if passed else "invalid or wrong JSON object",
            elapsed,
        )
    )

    history_payload = base_payload(
        [
            {"role": "user", "content": "What is 2 + 2?"},
            {
                "role": "assistant",
                "reasoning_content": "Add two and two.",
                "content": "4",
            },
            {"role": "user", "content": "Now multiply that result by 3."},
        ]
    )
    history_payload["chat_template_kwargs"]["clear_thinking"] = True
    response, elapsed = client.post(history_payload)
    history_message = completion_message(response)
    passed = (
        isinstance(history_message.get("content"), str)
        and "12" in history_message["content"]
    )
    results.append(
        result(
            "clear_thinking_history",
            passed,
            "matched" if passed else "history result was not 12",
            elapsed,
        )
    )

    status, elapsed = client.malformed_status()
    results.append(
        result("malformed_json", 400 <= status < 500, f"HTTP {status}", elapsed)
    )

    elapsed = client.cancel_after_first_event(
        base_payload(
            [
                {
                    "role": "user",
                    "content": "Write a long explanation of replicated logs.",
                }
            ]
        )
    )
    response, recovery_elapsed = client.post(
        base_payload(
            [{"role": "user", "content": "Reply exactly: recovered"}], thinking=False
        )
    )
    recovery_message = completion_message(response)
    passed = isinstance(recovery_message.get("content"), str)
    results.append(
        result(
            "cancel_and_recover",
            passed,
            "recovered" if passed else "recovery failed",
            elapsed + recovery_elapsed,
        )
    )

    ratio = 1.0
    for target_tokens in context_targets(max_context_tokens):
        units = max(1, int(target_tokens / ratio))
        long_payload = base_payload(
            [{"role": "user", "content": (" x" * units) + "\nReply only: context ok"}],
            thinking=False,
        )
        long_payload["max_tokens"] = 16
        response, elapsed = client.post(long_payload)
        message = completion_message(response)
        prompt_tokens, completion_tokens = response_usage(response)
        if prompt_tokens > 0:
            ratio = prompt_tokens / units
        tolerance = abs(prompt_tokens - target_tokens) / target_tokens
        passed = (
            isinstance(message.get("content"), str)
            and "context ok" in message["content"].lower()
            and completion_tokens > 0
            and tolerance <= 0.10
        )
        results.append(
            result(
                f"context_{target_tokens}",
                passed,
                "completed"
                if passed
                else "token count outside 10% or response incomplete",
                elapsed,
                prompt_tokens=prompt_tokens,
                completion_tokens=completion_tokens,
                target_prompt_tokens=target_tokens,
            )
        )
        recovery, recovery_elapsed = client.post(
            base_payload(
                [{"role": "user", "content": "Reply exactly: healthy"}], thinking=False
            )
        )
        recovery_message = completion_message(recovery)
        recovery_passed = (
            isinstance(recovery_message.get("content"), str)
            and "healthy" in recovery_message["content"].lower()
        )
        results.append(
            result(
                f"context_{target_tokens}_recovery",
                recovery_passed,
                "healthy" if recovery_passed else "recovery failed",
                recovery_elapsed,
            )
        )
    return results


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--endpoint", required=True, help="OpenAI-compatible /v1 base URL"
    )
    parser.add_argument("--api-key-env", default="FINITE_PRIVATE_CANARY_API_KEY")
    parser.add_argument("--timeout-seconds", type=float, default=1200)
    parser.add_argument(
        "--max-context-tokens",
        type=int,
        default=CONTEXT_TARGETS[-1],
        help=(
            "Run context cases whose target is at most this many tokens. "
            "128000 proves the cheap long-prefill path; 360000 is the "
            "near-limit case and is skipped until this image family is "
            "cleared above 262144."
        ),
    )
    arguments = parser.parse_args()
    api_key = os.environ.get(arguments.api_key_env, "")
    if not api_key:
        parser.error(f"{arguments.api_key_env} is required")
    if arguments.timeout_seconds <= 0:
        parser.error("--timeout-seconds must be positive")
    if arguments.max_context_tokens <= 0:
        parser.error("--max-context-tokens must be positive")

    try:
        results = run_gate(
            ProtocolClient(arguments.endpoint, api_key, arguments.timeout_seconds),
            max_context_tokens=arguments.max_context_tokens,
        )
    except Exception as error:
        results = [result("unhandled", False, str(error), 0.0)]
    report = {
        "schema": "finite-private-glm53-protocol-v1",
        "model": MODEL,
        "max_context_tokens": arguments.max_context_tokens,
        "passed": sum(bool(item["passed"]) for item in results),
        "total": len(results),
        "results": results,
    }
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["total"] > 0 and report["passed"] == report["total"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

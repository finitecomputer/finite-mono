#!/usr/bin/env python3
"""Scored DeepSeek-V4-Flash-0731 reasoning and tool-use comparison gate.

The fixed cases are intentionally small and deterministic. Run the same cases
against the self-hosted candidate and the hosted DeepSeek reference. Keys are
read only from named environment variables and never accepted as values on the
command line or written to the JSON report.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from dataclasses import dataclass
from typing import Any


@dataclass(frozen=True)
class QualityCase:
    case_id: str
    prompt: str
    expected_content: str | None = None
    expected_tool: str | None = None
    tools: tuple[dict[str, Any], ...] = ()


WEATHER_TOOL = {
    "type": "function",
    "function": {
        "name": "get_weather",
        "description": "Get the current weather for a city.",
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

CASES = (
    QualityCase(
        "arithmetic",
        "Calculate 17 * 19. Return only the final integer.",
        expected_content=r"\A\s*323\s*\Z",
    ),
    QualityCase(
        "logic",
        "All lorps are nims. No nims are zafs. Can any lorp be a zaf? Return only yes or no.",
        expected_content=r"\A\s*no[.!]?\s*\Z",
    ),
    QualityCase(
        "code_execution",
        "For the Python list [3, 1, 4, 1, 5], sum the values at even zero-based indices. Return only the integer.",
        expected_content=r"\A\s*12\s*\Z",
    ),
    QualityCase(
        "instruction_following",
        "Return exactly this marker and nothing else: FINITE-0731-OK",
        expected_content=r"\A\s*FINITE-0731-OK\s*\Z",
    ),
    QualityCase(
        "tool_selection",
        "Use get_weather for Austin, Texas. Do not answer the weather question directly.",
        expected_tool="get_weather",
        tools=(WEATHER_TOOL,),
    ),
)

HOSTED_ENDPOINT_HOST = "api.deepseek.com"
HOSTED_MODEL = "deepseek-v4-flash"
SELF_HOSTED_MODEL = "deepseek-v4-flash-0731"


def _post_json(
    endpoint: str,
    api_key: str,
    payload: dict[str, Any],
    *,
    timeout_seconds: float,
) -> tuple[dict[str, Any], float]:
    request = urllib.request.Request(
        endpoint.rstrip("/") + "/chat/completions",
        data=json.dumps(payload, separators=(",", ":")).encode("utf-8"),
        headers={
            "authorization": f"Bearer {api_key}",
            "content-type": "application/json",
            "x-request-id": f"fp_quality_{uuid.uuid4().hex}",
        },
        method="POST",
    )
    started = time.monotonic()
    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            body = response.read()
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", errors="replace")[:500]
        raise RuntimeError(f"HTTP {error.code}: {detail}") from error
    elapsed = time.monotonic() - started
    decoded = json.loads(body)
    if not isinstance(decoded, dict):
        raise RuntimeError("chat completion response was not a JSON object")
    return decoded, elapsed


def _payload(
    case: QualityCase,
    *,
    model: str,
    effort: str,
    lane: str,
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "model": model,
        "messages": [{"role": "user", "content": case.prompt}],
        "reasoning_effort": effort,
        "temperature": 1.0,
        "top_p": 0.95,
        "max_tokens": 4096,
    }
    if lane == "self-hosted":
        payload["chat_template_kwargs"] = {"thinking": True}
    if case.tools:
        payload["tools"] = list(case.tools)
        payload["tool_choice"] = "auto"
    return payload


def _score(case: QualityCase, response: dict[str, Any]) -> tuple[bool, str, int]:
    choices = response.get("choices")
    if not isinstance(choices, list) or not choices or not isinstance(choices[0], dict):
        return False, "missing first choice", 0
    message = choices[0].get("message")
    if not isinstance(message, dict):
        return False, "missing assistant message", 0
    reasoning = message.get("reasoning_content")
    if reasoning is None:
        reasoning = message.get("reasoning")
    if not isinstance(reasoning, str) or not reasoning.strip():
        return False, "missing parsed reasoning/reasoning_content", 0

    if case.expected_tool is not None:
        tool_calls = message.get("tool_calls")
        if not isinstance(tool_calls, list) or not tool_calls:
            return False, "missing tool call", len(reasoning)
        function = (
            tool_calls[0].get("function") if isinstance(tool_calls[0], dict) else None
        )
        if not isinstance(function, dict) or function.get("name") != case.expected_tool:
            return False, f"expected tool {case.expected_tool}", len(reasoning)
        arguments = function.get("arguments")
        if not isinstance(arguments, str) or "Austin" not in arguments:
            return False, "tool arguments did not contain Austin", len(reasoning)
        return True, "tool call matched", len(reasoning)

    content = message.get("content")
    if not isinstance(content, str):
        return False, "missing final content", len(reasoning)
    if (
        case.expected_content is None
        or re.fullmatch(case.expected_content, content, re.I) is None
    ):
        return False, "final content did not match the scored answer", len(reasoning)
    return True, "answer matched", len(reasoning)


def run(
    *,
    endpoint: str,
    api_key: str,
    model: str,
    lane: str,
    efforts: tuple[str, ...],
    timeout_seconds: float,
) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    for effort in efforts:
        for case in CASES:
            result: dict[str, Any] = {"case": case.case_id, "effort": effort}
            try:
                response, elapsed = _post_json(
                    endpoint,
                    api_key,
                    _payload(case, model=model, effort=effort, lane=lane),
                    timeout_seconds=timeout_seconds,
                )
                passed, detail, reasoning_characters = _score(case, response)
                result.update(
                    passed=passed,
                    detail=detail,
                    elapsed_seconds=round(elapsed, 3),
                    reasoning_characters=reasoning_characters,
                    response_model=response.get("model"),
                    system_fingerprint=response.get("system_fingerprint"),
                )
            except Exception as error:
                result.update(passed=False, detail=str(error))
            results.append(result)
    return results


def _identity_violations(
    *,
    endpoint: str,
    model: str,
    lane: str,
    results: list[dict[str, Any]],
) -> list[str]:
    parsed_endpoint = urllib.parse.urlsplit(endpoint)
    violations: list[str] = []
    expected_model = HOSTED_MODEL if lane == "deepseek-hosted" else SELF_HOSTED_MODEL
    if model != expected_model:
        violations.append(f"{lane} requested model must be {expected_model}")
    if lane == "deepseek-hosted" and (
        parsed_endpoint.scheme != "https"
        or parsed_endpoint.hostname != HOSTED_ENDPOINT_HOST
        or parsed_endpoint.username is not None
        or parsed_endpoint.password is not None
    ):
        violations.append(
            f"hosted reference endpoint must be https://{HOSTED_ENDPOINT_HOST}"
        )
    if not results:
        violations.append("quality report has no results")
    for result in results:
        if result.get("response_model") != expected_model:
            violations.append(
                f"{result.get('effort')}/{result.get('case')} returned model "
                f"{result.get('response_model')!r}, expected {expected_model!r}"
            )
    return violations


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--endpoint", required=True, help="OpenAI-compatible /v1 base URL"
    )
    parser.add_argument("--model", required=True)
    parser.add_argument(
        "--api-key-env",
        default="FINITE_PRIVATE_CANARY_API_KEY",
        help="environment variable containing the key; raw keys are not accepted",
    )
    parser.add_argument(
        "--lane", choices=("self-hosted", "deepseek-hosted"), default="self-hosted"
    )
    parser.add_argument("--efforts", default="high,max")
    parser.add_argument("--timeout-seconds", type=float, default=300)
    arguments = parser.parse_args()

    api_key = os.environ.get(arguments.api_key_env, "")
    if not api_key:
        parser.error(f"{arguments.api_key_env} is required")
    efforts = tuple(
        item.strip() for item in arguments.efforts.split(",") if item.strip()
    )
    if not efforts or any(item not in {"low", "high", "max"} for item in efforts):
        parser.error("--efforts must contain only low,high,max")

    results = run(
        endpoint=arguments.endpoint,
        api_key=api_key,
        model=arguments.model,
        lane=arguments.lane,
        efforts=efforts,
        timeout_seconds=arguments.timeout_seconds,
    )
    identity_violations = _identity_violations(
        endpoint=arguments.endpoint,
        model=arguments.model,
        lane=arguments.lane,
        results=results,
    )
    report = {
        "schema": "finite-deepseek-quality-v1",
        "lane": arguments.lane,
        "endpoint_host": urllib.parse.urlsplit(arguments.endpoint).hostname,
        "model": arguments.model,
        "passed": sum(bool(item["passed"]) for item in results),
        "total": len(results),
        "identity_violations": identity_violations,
        "results": results,
    }
    print(json.dumps(report, indent=2, sort_keys=True))
    return (
        0
        if report["total"] > 0
        and report["passed"] == report["total"]
        and not identity_violations
        else 1
    )


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Serial synthetic Inkling protocol probes; no production endpoint default.

Reports contain verdicts, counts, and timings, never response bodies or keys.
Run against the direct candidate and the limiter separately. A passing probe
does not establish GPU performance, attestation, billing, or overall quality.
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

if __package__:
    from .check_finite_private_glm53_protocol import (
        WEATHER_TOOL,
        accumulate_tool_calls,
        response_usage,
    )
else:
    from check_finite_private_glm53_protocol import (
        WEATHER_TOOL,
        accumulate_tool_calls,
        response_usage,
    )


MODEL = "inkling-2-small"
MARKER = "FINITE-INKLING-OK"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError("redirect refused")


def validate_endpoint(endpoint: str, *, allow_remote: bool) -> str:
    url = urllib.parse.urlsplit(endpoint)
    if (
        url.scheme not in {"http", "https"}
        or not url.hostname
        or url.username is not None
        or url.password is not None
        or url.query
        or url.fragment
        or url.path.rstrip("/") != "/v1"
    ):
        raise ValueError(
            "endpoint must be an HTTP(S) /v1 URL without credentials or query"
        )
    if url.hostname not in {"127.0.0.1", "::1", "localhost"}:
        if not allow_remote or url.scheme != "https":
            raise ValueError("remote probes require --allow-remote and HTTPS")
    return endpoint.rstrip("/")


def payload(model: str, prompt: str, *, effort: str = "none") -> dict:
    return {
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "reasoning_effort": effort,
        "max_tokens": 4096,
        "temperature": 0 if effort == "none" else 1,
    }


def message(response: dict, expected_model: str) -> dict:
    if response.get("model") != expected_model:
        raise ValueError("response identity mismatch")
    choices = response.get("choices")
    if not isinstance(choices, list) or not choices or not isinstance(choices[0], dict):
        raise ValueError("missing choice")
    if choices[0].get("finish_reason") not in {"stop", "tool_calls"}:
        raise ValueError("incomplete completion")
    result = choices[0].get("message")
    if not isinstance(result, dict):
        raise ValueError("missing assistant message")
    return result


def reasoning(msg: dict) -> str:
    return msg.get("reasoning_content") or msg.get("reasoning") or ""


def answer(msg: dict, expected: str, *, thinking: bool | None = None) -> bool:
    content = msg.get("content")
    return (
        isinstance(content, str)
        and content.strip().rstrip(".") == expected
        and not any(token in content for token in ("<think>", "<|content_thinking|>"))
        and (thinking is None or bool(reasoning(msg)) == thinking)
        and not msg.get("tool_calls")
    )


def tool_calls_valid(calls: list, cities: set[str]) -> bool:
    observed = set()
    ids = set()
    for call in calls:
        if not isinstance(call, dict) or not isinstance(call.get("id"), str):
            return False
        if not call["id"] or call["id"] in ids or call.get("type") != "function":
            return False
        ids.add(call["id"])
        fn = call.get("function", {})
        if fn.get("name") != "get_weather" or not isinstance(fn.get("arguments"), str):
            return False
        try:
            args = json.loads(fn["arguments"])
        except json.JSONDecodeError:
            return False
        if not isinstance(args, dict) or set(args) != {"city", "state"}:
            return False
        city, state = str(args["city"]).lower(), str(args["state"]).lower()
        if (city, state) not in {
            ("austin", "texas"),
            ("austin", "tx"),
            ("boston", "massachusetts"),
            ("boston", "ma"),
        }:
            return False
        observed.add(city)
    return observed == cities and len(calls) == len(cities)


class Client:
    def __init__(
        self, endpoint: str, api_key: str, expected_model: str, timeout: float
    ):
        self.endpoint = endpoint
        self.api_key = api_key
        self.expected_model = expected_model
        self.timeout = timeout
        self.opener = urllib.request.build_opener(NoRedirect())

    def open(self, body: dict):
        request = urllib.request.Request(
            self.endpoint + "/chat/completions",
            data=json.dumps(body).encode(),
            headers={
                "Authorization": f"Bearer {self.api_key}",
                "Content-Type": "application/json",
                "X-Request-ID": f"fp_inkling_qualification_{uuid.uuid4().hex}",
            },
        )
        return self.opener.open(request, timeout=self.timeout)

    def post(self, body: dict) -> tuple[dict, dict]:
        with self.open(body) as response:
            decoded = json.load(response)
        return message(decoded, self.expected_model), decoded

    def stream(self, body: dict, *, cancel: bool = False) -> dict:
        body = dict(body, stream=True, stream_options={"include_usage": True})
        started = time.monotonic()
        events = []
        done = False
        terminal = False
        usage = (0, 0)
        first_output = None
        with self.open(body) as response:
            if "text/event-stream" not in response.headers.get("Content-Type", ""):
                raise ValueError("missing event-stream content type")
            for raw in response:
                line = raw.decode().strip()
                if not line.startswith("data:"):
                    continue
                data = line[5:].strip()
                if done:
                    raise ValueError("data after terminal marker")
                if data == "[DONE]":
                    done = True
                    continue
                event = json.loads(data)
                if event.get("model") != self.expected_model or "error" in event:
                    raise ValueError("stream identity mismatch or error")
                events.append(event)
                event_usage = response_usage(event)
                if event_usage != (0, 0):
                    usage = event_usage
                for choice in event.get("choices", []):
                    finish = choice.get("finish_reason")
                    if finish is not None:
                        if finish not in {"stop", "tool_calls"}:
                            raise ValueError("incomplete stream")
                        terminal = True
                    delta = choice.get("delta", {})
                    if (
                        any(
                            delta.get(k)
                            for k in (
                                "content",
                                "reasoning_content",
                                "reasoning",
                                "tool_calls",
                            )
                        )
                        and first_output is None
                    ):
                        first_output = time.monotonic() - started
                        if cancel:
                            return {"cancelled": True}
        if cancel:
            raise ValueError("no output to cancel")
        if not done or not terminal or usage[0] <= 0 or usage[1] <= 0:
            raise ValueError("missing terminal stream, finish reason, or token usage")
        content = "".join(
            choice.get("delta", {}).get("content") or ""
            for event in events
            for choice in event.get("choices", [])
        )
        return {
            "content": content,
            "tool_calls": accumulate_tool_calls(events),
            "ttft_seconds": first_output,
            "prompt_tokens": usage[0],
            "completion_tokens": usage[1],
        }


def run(client: Client, model: str, *, context_tokens: int = 0) -> list[dict]:
    results = []

    def case(name, check):
        started = time.monotonic()
        facts = {}
        try:
            checked = check()
            if isinstance(checked, tuple):
                checked, facts = checked
            passed = bool(checked)
            detail = "matched" if passed else "protocol or scored answer mismatch"
        except urllib.error.HTTPError as error:
            passed, detail = False, f"HTTP {error.code}"
        except Exception as error:
            # No exception text: upstreams may echo keys, prompts or generated content.
            passed, detail = False, type(error).__name__
        results.append(
            {
                "case": name,
                "passed": passed,
                "detail": detail,
                "elapsed_seconds": round(time.monotonic() - started, 3),
                **facts,
            }
        )

    for effort in ("none", "low", "high", "max"):
        case(
            f"effort_{effort}",
            lambda effort=effort: answer(
                client.post(payload(model, f"Return exactly: {MARKER}", effort=effort))[
                    0
                ],
                MARKER,
                thinking=False if effort == "none" else None,
            ),
        )
    case(
        "parsed_reasoning",
        lambda: answer(
            client.post(
                payload(
                    model,
                    "Compute 17 * 19 carefully. Return only the final integer.",
                    effort="high",
                )
            )[0],
            "323",
            thinking=True,
        ),
    )

    # Test the payload already emitted by existing clients and limiter defaults.
    # The published template ignores enable_thinking; do not silently adapt it here.
    legacy = payload(model, f"Return exactly: {MARKER}", effort="high")
    legacy["chat_template_kwargs"] = {"enable_thinking": False}
    case(
        "legacy_fast_control",
        lambda: answer(
            client.post(legacy)[0],
            MARKER,
            thinking=False,
        ),
    )

    history = payload(model, "unused")
    history["chat_template_kwargs"] = {"enable_thinking": True, "clear_thinking": True}
    history["messages"] = [
        {"role": "user", "content": "What is 2 + 2?"},
        {"role": "assistant", "reasoning_content": "Add two and two.", "content": "4"},
        {
            "role": "user",
            "content": "Multiply that answer by 3. Return only the integer.",
        },
    ]
    case("existing_reasoning_history", lambda: answer(client.post(history)[0], "12"))

    tool = payload(model, "Use get_weather for Austin, Texas.", effort="high")
    tool.update(tools=[WEATHER_TOOL], tool_choice="auto")

    def round_trip():
        msg, _ = client.post(tool)
        calls = msg.get("tool_calls") or []
        if not tool_calls_valid(calls, {"austin"}):
            return False
        follow = dict(tool)
        follow.update(
            tool_choice="none",
            messages=[
                *tool["messages"],
                msg,
                {
                    "role": "tool",
                    "tool_call_id": calls[0]["id"],
                    "content": '{"temperature_f":91}',
                },
                {
                    "role": "user",
                    "content": "Return only the temperature as an integer.",
                },
            ],
        )
        return answer(client.post(follow)[0], "91")

    case("automatic_tool_and_result_round_trip", round_trip)
    streamed_tool = dict(tool, tool_choice="required")
    case(
        "streaming_tool_arguments",
        lambda: tool_calls_valid(
            client.stream(streamed_tool)["tool_calls"], {"austin"}
        ),
    )
    parallel = payload(
        model,
        "Call get_weather separately for Austin, Texas and Boston, Massachusetts.",
        effort="high",
    )
    parallel.update(
        tools=[WEATHER_TOOL], tool_choice="required", parallel_tool_calls=True
    )
    case(
        "parallel_tools",
        lambda: tool_calls_valid(
            client.stream(parallel)["tool_calls"], {"austin", "boston"}
        ),
    )

    def streaming_answer():
        streamed = client.stream(payload(model, f"Return exactly: {MARKER}"))
        return answer(streamed, MARKER), {
            key: streamed[key]
            for key in ("ttft_seconds", "prompt_tokens", "completion_tokens")
        }

    case("streaming_answer_and_usage", streaming_answer)

    def cancellation():
        client.stream(
            payload(model, "Write a long explanation of replicated logs."), cancel=True
        )
        return answer(
            client.post(payload(model, f"Return exactly: {MARKER}"))[0], MARKER
        )

    case("cancel_and_recover", cancellation)
    if context_tokens:

        def long_context():
            calibration = payload(model, " x" * 1024 + f"\nReturn exactly: {MARKER}")
            _, response = client.post(calibration)
            count, _ = response_usage(response)
            if count <= 0:
                return False
            units = int(context_tokens * 1024 / count)
            long = payload(model, " x" * units + f"\nReturn exactly: {MARKER}")
            msg, response = client.post(long)
            count, completion = response_usage(response)
            passed = (
                answer(msg, MARKER)
                and completion > 0
                and abs(count - context_tokens) <= context_tokens * 0.1
                and count <= 389120
            )
            return passed, {"prompt_tokens": count, "completion_tokens": completion}

        case(f"context_{context_tokens}", long_context)
        case(
            "long_context_recovery",
            lambda: answer(
                client.post(payload(model, f"Return exactly: {MARKER}"))[0], MARKER
            ),
        )
    return results


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--endpoint", required=True, help="Explicit candidate /v1 URL")
    parser.add_argument("--allow-remote", action="store_true")
    parser.add_argument(
        "--model", default=MODEL, help="Request label; also test each retained alias"
    )
    parser.add_argument("--expected-model", default=MODEL)
    parser.add_argument("--api-key-env", default="FINITE_PRIVATE_CANARY_API_KEY")
    parser.add_argument(
        "--tag", required=True, help="Evidence label, not proof of attestation"
    )
    parser.add_argument("--timeout-seconds", type=float, default=180)
    parser.add_argument(
        "--context-tokens", type=int, choices=(0, 128000, 360000), default=0
    )
    args = parser.parse_args()
    try:
        endpoint = validate_endpoint(args.endpoint, allow_remote=args.allow_remote)
    except ValueError as error:
        parser.error(str(error))
    if not 0 < args.timeout_seconds <= 1200:
        parser.error("timeout must be positive and at most 1200 seconds")
    if not re.fullmatch(r"[a-zA-Z0-9._-]+", args.tag):
        parser.error(
            "tag must contain only letters, digits, dots, hyphens or underscores"
        )
    api_key = os.environ.get(args.api_key_env, "")
    if not api_key:
        parser.error("the named API-key environment variable is required")
    client = Client(endpoint, api_key, args.expected_model, args.timeout_seconds)
    results = run(client, args.model, context_tokens=args.context_tokens)
    passed = all(row["passed"] for row in results)
    print(
        json.dumps(
            {
                "schema": "finite-private-inkling-protocol-v1",
                "tag": args.tag,
                "passed": passed,
                "context_target_tokens": args.context_tokens,
                "request_model": args.model,
                "expected_model": args.expected_model,
                "results": results,
            },
            sort_keys=True,
        )
    )
    return int(not passed)


if __name__ == "__main__":
    raise SystemExit(main())

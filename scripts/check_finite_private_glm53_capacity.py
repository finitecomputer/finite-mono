#!/usr/bin/env python3
"""Measure and gate GLM-5.3-Flash at Finite Private's 120-user floor.

The API key is read only from a named environment variable. Reports contain
timings, token counts, and errors, never credentials or response content.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import math
import os
import resource
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any

MODEL = "glm-5-3-flash"
TASKS = (
    "Explain quorum reads and writes under partial failure with examples.",
    "Design an idempotent payment webhook handler and state its invariants.",
    "Compare optimistic and pessimistic concurrency control for a busy API.",
    "Describe a safe rolling database migration across mixed application versions.",
    "Explain how a replicated log recovers after a leader crashes mid-commit.",
    "Design retry behavior that avoids a thundering herd during an outage.",
    "Explain isolation boundaries for running untrusted agent-generated code.",
    "Describe how to preserve ordered chat history across reconnects and retries.",
)


@dataclass(frozen=True)
class AcceptanceThresholds:
    required_concurrency: int = 120
    minimum_p10_output_tok_s: float = 10.0
    minimum_p50_output_tok_s: float = 20.0
    minimum_aggregate_output_tok_s: float = 2400.0
    maximum_p95_ttft_s: float = 10.0


@dataclass
class RequestResult:
    ttft_s: float | None = None
    latency_s: float | None = None
    completion_tokens: int = 0
    terminal_stream: bool = False
    error: str | None = None


def percentile(values: list[float], fraction: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    index = max(0, math.ceil(fraction * len(ordered)) - 1)
    return ordered[index]


def rounded(value: float | None) -> float | None:
    return None if value is None else round(value, 3)


def evaluate(
    measurement: dict[str, Any], thresholds: AcceptanceThresholds
) -> dict[str, Any]:
    violations: list[str] = []
    expected = thresholds.required_concurrency
    checks = (
        ("successes", measurement.get("successes") == expected),
        ("errors", measurement.get("errors") == 0),
        ("terminal_streams", measurement.get("terminal_streams") == expected),
        (
            "aggregate_output_tok_s",
            float(measurement.get("aggregate_output_tok_s") or 0)
            >= thresholds.minimum_aggregate_output_tok_s,
        ),
        (
            "per_request_output_tok_s_p10",
            float(measurement.get("per_request_output_tok_s_p10") or 0)
            >= thresholds.minimum_p10_output_tok_s,
        ),
        (
            "per_request_output_tok_s_p50",
            float(measurement.get("per_request_output_tok_s_p50") or 0)
            >= thresholds.minimum_p50_output_tok_s,
        ),
        (
            "ttft_p95_s",
            float(measurement.get("ttft_p95_s") or math.inf)
            <= thresholds.maximum_p95_ttft_s,
        ),
    )
    for name, passed in checks:
        if not passed:
            violations.append(name)
    return {
        "schema": "finite-private-glm53-capacity-acceptance-v1",
        "passed": not violations,
        "required_concurrency": expected,
        "thresholds": {
            "minimum_p10_output_tok_s": thresholds.minimum_p10_output_tok_s,
            "minimum_p50_output_tok_s": thresholds.minimum_p50_output_tok_s,
            "minimum_aggregate_output_tok_s": (
                thresholds.minimum_aggregate_output_tok_s
            ),
            "maximum_p95_ttft_s": thresholds.maximum_p95_ttft_s,
        },
        "violations": violations,
    }


def make_payload(args: argparse.Namespace, index: int, run_tag: str) -> dict[str, Any]:
    marker = hashlib.sha256(f"{run_tag}-{index}".encode()).hexdigest()
    payload: dict[str, Any] = {
        "model": args.model,
        "messages": [
            {
                "role": "user",
                "content": (
                    f"{marker} Request {index}. {TASKS[index % len(TASKS)]} "
                    "Continue until the response budget is exhausted."
                ),
            }
        ],
        "temperature": 0.7,
        "top_p": 1.0,
        "max_tokens": args.output_tokens,
        "ignore_eos": True,
        "stream": True,
        "stream_options": {"include_usage": True},
    }
    if args.thinking != "default":
        payload["chat_template_kwargs"] = {
            "enable_thinking": args.thinking == "on"
        }
    if args.reasoning_effort:
        payload["reasoning_effort"] = args.reasoning_effort
    return payload


def event_has_output(event: dict[str, Any]) -> bool:
    choices = event.get("choices")
    if not isinstance(choices, list) or not choices:
        return False
    delta = choices[0].get("delta") if isinstance(choices[0], dict) else None
    if not isinstance(delta, dict):
        return False
    return any(
        bool(delta.get(field))
        for field in ("content", "reasoning", "reasoning_content", "tool_calls")
    )


def request_once(
    args: argparse.Namespace,
    index: int,
    run_tag: str,
) -> RequestResult:
    started = time.perf_counter()
    first_output_at: float | None = None
    completion_tokens = 0
    saw_done = False
    request = urllib.request.Request(
        f"{args.url.rstrip('/')}/v1/chat/completions",
        data=json.dumps(make_payload(args, index, run_tag)).encode(),
        headers={
            "authorization": f"Bearer {args.api_key}",
            "content-type": "application/json",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=args.timeout) as response:
            if response.status != 200:
                body = response.read(300).decode("utf-8", "replace")
                return RequestResult(error=f"HTTP {response.status}: {body}")
            for raw_line in response:
                line = raw_line.decode("utf-8", "replace").strip()
                if not line.startswith("data: "):
                    continue
                data = line[6:]
                if data == "[DONE]":
                    saw_done = True
                    continue
                event = json.loads(data)
                if first_output_at is None and event_has_output(event):
                    first_output_at = time.perf_counter()
                usage = event.get("usage")
                if isinstance(usage, dict):
                    completion_tokens = int(
                        usage.get("completion_tokens", completion_tokens)
                    )
    except urllib.error.HTTPError as error:
        body = error.read(300).decode("utf-8", "replace")
        return RequestResult(error=f"HTTP {error.code}: {body}")
    except Exception as error:  # The report must retain every transport failure.
        return RequestResult(error=repr(error))
    ended = time.perf_counter()
    if first_output_at is None:
        return RequestResult(error="stream contained no output delta")
    if not saw_done:
        return RequestResult(error="stream lacked terminal [DONE]")
    if completion_tokens <= 0:
        return RequestResult(error="stream lacked positive completion usage")
    return RequestResult(
        ttft_s=first_output_at - started,
        latency_s=ended - started,
        completion_tokens=completion_tokens,
        terminal_stream=True,
    )


def run_once(
    args: argparse.Namespace,
    concurrency: int,
    repetition: int,
) -> dict[str, Any]:
    with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as executor:
        if args.warmup > 0:
            warmup_args = argparse.Namespace(**vars(args))
            warmup_args.output_tokens = min(16, args.output_tokens)
            warmups = list(
                executor.map(
                    lambda index: request_once(
                        warmup_args,
                        index,
                        f"{args.tag}-warmup-c{concurrency}-r{repetition}",
                    ),
                    range(min(args.warmup, concurrency)),
                )
            )
            warmup_errors = [result.error for result in warmups if result.error]
            if warmup_errors:
                raise RuntimeError(f"warmup failed: {warmup_errors[0]}")

        started = time.perf_counter()
        results = list(
            executor.map(
                lambda index: request_once(
                    args,
                    index,
                    f"{args.tag}-c{concurrency}-r{repetition}",
                ),
                range(concurrency),
            )
        )
        wall_s = time.perf_counter() - started

    good = [result for result in results if result.error is None]
    errors = [result.error for result in results if result.error is not None]
    ttfts = [result.ttft_s for result in good if result.ttft_s is not None]
    latencies = [
        result.latency_s for result in good if result.latency_s is not None
    ]
    per_request_rates = [
        result.completion_tokens
        / max((result.latency_s or 0) - (result.ttft_s or 0), 0.001)
        for result in good
    ]
    output_tokens = sum(result.completion_tokens for result in good)
    return {
        "schema": "finite-private-glm53-capacity-measurement-v1",
        "tag": args.tag,
        "model": args.model,
        "thinking": args.thinking,
        "reasoning_effort": args.reasoning_effort,
        "concurrency": concurrency,
        "repetition": repetition,
        "successes": len(good),
        "errors": len(errors),
        "terminal_streams": sum(result.terminal_stream for result in good),
        "wall_s": rounded(wall_s),
        "output_tokens": output_tokens,
        "aggregate_output_tok_s": round(output_tokens / wall_s, 3),
        "per_request_output_tok_s_p10": rounded(percentile(per_request_rates, 0.10)),
        "per_request_output_tok_s_p50": rounded(percentile(per_request_rates, 0.50)),
        "per_request_output_tok_s_p95": rounded(percentile(per_request_rates, 0.95)),
        "ttft_p50_s": rounded(percentile(ttfts, 0.50)),
        "ttft_p95_s": rounded(percentile(ttfts, 0.95)),
        "ttft_p99_s": rounded(percentile(ttfts, 0.99)),
        "latency_p50_s": rounded(percentile(latencies, 0.50)),
        "latency_p95_s": rounded(percentile(latencies, 0.95)),
        "first_error": errors[0] if errors else None,
    }


def parse_concurrency(raw: str) -> tuple[int, ...]:
    try:
        values = tuple(int(value) for value in raw.split(","))
    except ValueError as error:
        raise argparse.ArgumentTypeError(
            "concurrency must be comma-separated integers"
        ) from error
    if not values or any(value <= 0 for value in values):
        raise argparse.ArgumentTypeError("concurrency values must be positive")
    return values


def run_gate(args: argparse.Namespace) -> int:
    thresholds = AcceptanceThresholds(
        required_concurrency=args.required_concurrency,
        minimum_p10_output_tok_s=args.minimum_p10_output_tok_s,
        minimum_p50_output_tok_s=args.minimum_p50_output_tok_s,
        minimum_aggregate_output_tok_s=args.minimum_aggregate_output_tok_s,
        maximum_p95_ttft_s=args.maximum_p95_ttft_s,
    )
    if thresholds.required_concurrency not in args.concurrency:
        raise SystemExit(
            "--concurrency must include the --required-concurrency acceptance tier"
        )

    failed = False
    acceptance_reports: list[dict[str, Any]] = []
    for concurrency in args.concurrency:
        for repetition in range(1, args.repetitions + 1):
            measurement = run_once(args, concurrency, repetition)
            if concurrency == thresholds.required_concurrency:
                acceptance = evaluate(measurement, thresholds)
                measurement["acceptance"] = acceptance
                acceptance_reports.append(acceptance)
                failed = failed or not acceptance["passed"]
            else:
                failed = failed or measurement["errors"] > 0
            print(json.dumps(measurement, sort_keys=True), flush=True)

    summary = {
        "schema": "finite-private-glm53-capacity-summary-v1",
        "passed": not failed and len(acceptance_reports) == args.repetitions,
        "required_concurrency": thresholds.required_concurrency,
        "required_repetitions": args.repetitions,
        "passing_repetitions": sum(
            bool(report["passed"]) for report in acceptance_reports
        ),
    }
    print(json.dumps(summary, sort_keys=True), flush=True)
    return 0 if summary["passed"] else 1


def main() -> int:
    soft_limit, hard_limit = resource.getrlimit(resource.RLIMIT_NOFILE)
    if soft_limit < hard_limit:
        resource.setrlimit(resource.RLIMIT_NOFILE, (hard_limit, hard_limit))

    parser = argparse.ArgumentParser()
    parser.add_argument("--url", required=True, help="OpenAI-compatible /v1 base URL")
    parser.add_argument("--model", default=MODEL)
    parser.add_argument(
        "--api-key-env", default="FINITE_PRIVATE_CANARY_API_KEY"
    )
    parser.add_argument("--concurrency", type=parse_concurrency, default=(1, 32, 64, 120))
    parser.add_argument("--required-concurrency", type=int, default=120)
    parser.add_argument("--output-tokens", type=int, default=256)
    parser.add_argument("--warmup", type=int, default=8)
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--tag", required=True)
    parser.add_argument(
        "--thinking", choices=("default", "on", "off"), default="on"
    )
    parser.add_argument(
        "--reasoning-effort", choices=("low", "high", "max"), default="high"
    )
    parser.add_argument("--minimum-p10-output-tok-s", type=float, default=10.0)
    parser.add_argument("--minimum-p50-output-tok-s", type=float, default=20.0)
    parser.add_argument("--minimum-aggregate-output-tok-s", type=float, default=2400.0)
    parser.add_argument("--maximum-p95-ttft-s", type=float, default=10.0)
    args = parser.parse_args()

    args.api_key = os.environ.get(args.api_key_env, "")
    if not args.api_key:
        parser.error(f"{args.api_key_env} is required")
    for name in (
        "required_concurrency",
        "output_tokens",
        "repetitions",
    ):
        if getattr(args, name) <= 0:
            parser.error(f"--{name.replace('_', '-')} must be positive")
    if args.warmup < 0:
        parser.error("--warmup must not be negative")

    return run_gate(args)


if __name__ == "__main__":
    raise SystemExit(main())

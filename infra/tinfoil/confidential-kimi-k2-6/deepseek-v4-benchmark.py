#!/usr/bin/env python3
"""Deterministic streaming load loop for DeepSeek V4 GPU-lab comparisons."""

from __future__ import annotations

import argparse
import asyncio
import hashlib
import json
import resource
import time
from dataclasses import dataclass

import aiohttp


TASKS = (
    "Explain quorum reads and writes under partial failure with examples.",
    "Design an idempotent payment webhook handler and discuss its invariants.",
    "Compare optimistic and pessimistic concurrency control for a busy API.",
    "Describe a safe rolling database migration across mixed application versions.",
    "Explain how a replicated log recovers after a leader crashes mid-commit.",
    "Design retry behavior that avoids a thundering herd during an outage.",
    "Explain isolation boundaries for running untrusted agent-generated code.",
    "Describe how to preserve ordered chat history across reconnects and retries.",
)


@dataclass
class Result:
    ttft: float | None = None
    latency: float | None = None
    completion_tokens: int = 0
    error: str | None = None


def percentile(values: list[float], fraction: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    index = min(len(ordered) - 1, int((len(ordered) - 1) * fraction))
    return ordered[index]


def rounded(value: float | None) -> float | None:
    return None if value is None else round(value, 3)


def make_payload(args: argparse.Namespace, index: int, run_tag: str) -> dict:
    marker = hashlib.sha256(f"{run_tag}-{index}".encode()).hexdigest()
    payload: dict = {
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


async def request_once(
    session: aiohttp.ClientSession,
    args: argparse.Namespace,
    index: int,
    run_tag: str,
) -> Result:
    started = time.perf_counter()
    first = None
    completion_tokens = 0
    try:
        async with session.post(
            f"{args.url.rstrip('/')}/v1/chat/completions",
            json=make_payload(args, index, run_tag),
        ) as response:
            if response.status != 200:
                body = await response.text()
                return Result(error=f"HTTP {response.status}: {body[:300]}")
            async for raw in response.content:
                line = raw.decode("utf-8", "replace").strip()
                if not line.startswith("data: ") or line == "data: [DONE]":
                    continue
                event = json.loads(line[6:])
                if first is None and event.get("choices"):
                    first = time.perf_counter()
                usage = event.get("usage")
                if usage:
                    completion_tokens = usage.get(
                        "completion_tokens", completion_tokens
                    )
    except Exception as error:  # Benchmark must report every transport failure.
        return Result(error=repr(error))
    ended = time.perf_counter()
    return Result(
        ttft=None if first is None else first - started,
        latency=ended - started,
        completion_tokens=completion_tokens,
    )


async def run(args: argparse.Namespace, concurrency: int) -> dict:
    timeout = aiohttp.ClientTimeout(total=args.timeout)
    connector = aiohttp.TCPConnector(limit=0)
    async with aiohttp.ClientSession(timeout=timeout, connector=connector) as session:
        warmup_args = argparse.Namespace(**vars(args))
        warmup_args.output_tokens = min(16, args.output_tokens)
        warmups = await asyncio.gather(
            *(
                request_once(
                    session,
                    warmup_args,
                    index,
                    f"{args.tag}-warmup-{concurrency}",
                )
                for index in range(args.warmup)
            )
        )
        warmup_errors = [result.error for result in warmups if result.error]
        if warmup_errors:
            raise RuntimeError(f"warmup failed: {warmup_errors[0]}")

        started = time.perf_counter()
        results = await asyncio.gather(
            *(
                request_once(
                    session, args, index, f"{args.tag}-c{concurrency}"
                )
                for index in range(concurrency)
            )
        )
        wall = time.perf_counter() - started

    good = [result for result in results if result.error is None]
    errors = [result.error for result in results if result.error is not None]
    ttfts = [result.ttft for result in good if result.ttft is not None]
    latencies = [result.latency for result in good if result.latency is not None]
    tokens = sum(result.completion_tokens for result in good)
    return {
        "tag": args.tag,
        "thinking": args.thinking,
        "reasoning_effort": args.reasoning_effort,
        "concurrency": concurrency,
        "successes": len(good),
        "errors": len(errors),
        "wall_s": rounded(wall),
        "output_tokens": tokens,
        "aggregate_output_tok_s": round(tokens / wall, 2),
        "ttft_p50_s": rounded(percentile(ttfts, 0.50)),
        "ttft_p95_s": rounded(percentile(ttfts, 0.95)),
        "latency_p50_s": rounded(percentile(latencies, 0.50)),
        "latency_p95_s": rounded(percentile(latencies, 0.95)),
        "first_error": errors[0] if errors else None,
    }


async def main() -> int:
    soft_limit, hard_limit = resource.getrlimit(resource.RLIMIT_NOFILE)
    if soft_limit < hard_limit:
        resource.setrlimit(resource.RLIMIT_NOFILE, (hard_limit, hard_limit))

    parser = argparse.ArgumentParser()
    parser.add_argument("--url", default="http://127.0.0.1:8000")
    parser.add_argument("--model", default="deepseek-v4-flash-0731")
    parser.add_argument("--concurrency", default="1,8,32,64")
    parser.add_argument("--output-tokens", type=int, default=128)
    parser.add_argument("--warmup", type=int, default=8)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--thinking", choices=("default", "on", "off"), default="on")
    parser.add_argument("--reasoning-effort", choices=("low", "high", "max"))
    args = parser.parse_args()

    failed = False
    for concurrency in (int(value) for value in args.concurrency.split(",")):
        measurement = await run(args, concurrency)
        print(json.dumps(measurement), flush=True)
        failed = failed or measurement["errors"] > 0
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))

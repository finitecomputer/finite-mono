#!/usr/bin/env python3
"""Exercise a DeepSeek V4 candidate near its advertised context limit."""

from __future__ import annotations

import json
import time
import urllib.request


def main() -> int:
    # For the pinned DeepSeek tokenizer, each leading-space `a` is one token.
    # Leave headroom for the chat template and eight generated tokens.
    prompt = " a" * 380_000 + "\nReply with OK."
    payload = {
        "model": "deepseek-v4-flash-0731",
        "messages": [{"role": "user", "content": prompt}],
        "temperature": 0,
        "max_tokens": 8,
        "chat_template_kwargs": {"enable_thinking": False},
    }
    request = urllib.request.Request(
        "http://127.0.0.1:8000/v1/chat/completions",
        data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json"},
    )
    started = time.perf_counter()
    with urllib.request.urlopen(request, timeout=600) as response:
        result = json.load(response)
    elapsed = time.perf_counter() - started
    usage = result.get("usage") or {}
    prompt_tokens = usage.get("prompt_tokens", 0)
    report = {
        "prompt_tokens": prompt_tokens,
        "completion_tokens": usage.get("completion_tokens"),
        "elapsed_s": round(elapsed, 3),
        "content": result["choices"][0]["message"].get("content"),
    }
    print(json.dumps(report))
    return 0 if 375_000 <= prompt_tokens <= 393_216 else 1


if __name__ == "__main__":
    raise SystemExit(main())

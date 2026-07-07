#!/usr/bin/env python3
from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import shlex
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Iterable


DEFAULT_MODELS = [
    "openai/gpt-5.5",
    "anthropic/claude-sonnet-4.6",
    "google/gemini-3.1-pro-preview",
    "x-ai/grok-4.3",
]

MODE_GUIDANCE = {
    "decision": (
        "Evaluate the decision. Name the real tradeoffs, hidden assumptions, "
        "reversibility, downside risk, and what evidence would change your mind."
    ),
    "strategy": (
        "Evaluate the strategy. Look for leverage, sequencing, positioning, "
        "operational drag, and where the plan is pretending uncertainty is certainty."
    ),
    "research": (
        "Evaluate the question as a research analyst. Separate known facts, "
        "uncertain inferences, missing evidence, and useful next searches."
    ),
    "code-review": (
        "Evaluate the technical plan or code direction. Focus on correctness, "
        "maintainability, integration risk, test strategy, and simpler alternatives."
    ),
}


@dataclass(frozen=True)
class CouncilResult:
    model: str
    ok: bool
    content: str
    elapsed_seconds: float


def env_models() -> list[str]:
    raw = os.getenv("MODEL_COUNCIL_MODELS", "").strip()
    if not raw:
        return DEFAULT_MODELS
    return [item.strip() for item in raw.split(",") if item.strip()]


def load_dotenv_if_present() -> None:
    env_path = os.path.expanduser(os.getenv("MODEL_COUNCIL_ENV_FILE", "~/.hermes/.env"))
    if not os.path.exists(env_path):
        return
    with open(env_path, "r", encoding="utf-8") as handle:
        for raw_line in handle:
            line = raw_line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            key, value = line.split("=", 1)
            key = key.strip()
            if not key or key in os.environ:
                continue
            value = value.strip()
            if value and value[0] in {"'", '"'}:
                try:
                    value = shlex.split(value)[0]
                except ValueError:
                    value = value.strip("'\"")
            os.environ[key] = value


def openrouter_url() -> str:
    base = os.getenv("OPENROUTER_BASE_URL", "https://openrouter.ai/api/v1").rstrip("/")
    return f"{base}/chat/completions"


def read_question(args: argparse.Namespace) -> str:
    parts: list[str] = []
    if args.question:
        parts.append(args.question.strip())
    if args.input_file:
        with open(args.input_file, "r", encoding="utf-8") as handle:
            parts.append(handle.read().strip())
    if not parts and not sys.stdin.isatty():
        parts.append(sys.stdin.read().strip())
    question = "\n\n".join(part for part in parts if part)
    if not question:
        raise SystemExit("Provide --question, --input-file, or stdin.")
    return question


def chat_completion(
    *,
    api_key: str,
    model: str,
    messages: list[dict[str, str]],
    temperature: float,
    max_tokens: int,
    timeout: int,
) -> str:
    payload = {
        "model": model,
        "messages": messages,
        "temperature": temperature,
        "max_tokens": max_tokens,
    }
    data = json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(
        openrouter_url(),
        data=data,
        method="POST",
        headers={
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
            "HTTP-Referer": "https://finite.computer",
            "X-Title": "Finite Model Council",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        error_body = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"HTTP {exc.code}: {error_body[:1000]}") from exc
    except urllib.error.URLError as exc:
        raise RuntimeError(str(exc)) from exc

    try:
        return str(body["choices"][0]["message"]["content"]).strip()
    except (KeyError, IndexError, TypeError) as exc:
        raise RuntimeError(f"Unexpected OpenRouter response: {json.dumps(body)[:1000]}") from exc


def panel_messages(mode: str, question: str) -> list[dict[str, str]]:
    return [
        {
            "role": "system",
            "content": (
                "You are one member of a multi-model council. Work independently. "
                "Do not try to predict consensus. Be direct, skeptical, and useful. "
                "Do not invent citations or benchmark numbers. Treat concrete numbers "
                "that are not supplied in the prompt as estimates, and say so. "
                "Separate sourced facts, assumptions, and inferences. "
                + MODE_GUIDANCE[mode]
            ),
        },
        {
            "role": "user",
            "content": (
                "Question or task for the council:\n\n"
                f"{question}\n\n"
                "Return:\n"
                "1. Your recommendation\n"
                "2. Sourced or prompt-provided facts you relied on\n"
                "3. Assumptions and estimates\n"
                "4. Main risks or objections\n"
                "5. What would change your mind\n"
                "6. Confidence from 0.0 to 1.0"
            ),
        },
    ]


def synth_messages(mode: str, question: str, results: Iterable[CouncilResult]) -> list[dict[str, str]]:
    panel_text = "\n\n".join(
        f"## {result.model}\n{result.content}" for result in results if result.ok
    )
    return [
        {
            "role": "system",
            "content": (
                "You are the chair of a multi-model council. Synthesize the panel. "
                "Do not average weak opinions. Preserve important disagreement. "
                "Downgrade unsupported concrete numbers to estimates. Distinguish "
                "facts supplied in the prompt from model inferences. "
                + MODE_GUIDANCE[mode]
            ),
        },
        {
            "role": "user",
            "content": (
                "Original question or task:\n\n"
                f"{question}\n\n"
                "Panel responses:\n\n"
                f"{panel_text}\n\n"
                "Return a concise council report with these headings:\n"
                "- Council recommendation\n"
                "- Consensus\n"
                "- Live disagreements\n"
                "- Evidence quality\n"
                "- Biggest risks\n"
                "- Suggested next action\n"
                "- Confidence"
            ),
        },
    ]


def ask_panel(
    api_key: str,
    model: str,
    mode: str,
    question: str,
    temperature: float,
    max_tokens: int,
    timeout: int,
) -> CouncilResult:
    started = time.monotonic()
    try:
        content = chat_completion(
            api_key=api_key,
            model=model,
            messages=panel_messages(mode, question),
            temperature=temperature,
            max_tokens=max_tokens,
            timeout=timeout,
        )
        return CouncilResult(model, True, content, time.monotonic() - started)
    except Exception as exc:
        return CouncilResult(model, False, str(exc), time.monotonic() - started)


def render_report(
    *,
    mode: str,
    question: str,
    models: list[str],
    results: list[CouncilResult],
    synthesis: CouncilResult | None,
) -> str:
    lines = [
        "# Model Council Report",
        "",
        f"Mode: `{mode}`",
        f"Models: {', '.join(models)}",
        "",
        "## Question",
        "",
        question,
        "",
    ]
    if synthesis:
        lines.extend([
            "## Synthesis",
            "",
            synthesis.content if synthesis.ok else f"Synthesis failed: {synthesis.content}",
            "",
        ])
    lines.extend(["## Individual Opinions", ""])
    for result in results:
        status = "ok" if result.ok else "failed"
        lines.extend([
            f"### {result.model} ({status}, {result.elapsed_seconds:.1f}s)",
            "",
            result.content,
            "",
        ])
    return "\n".join(lines).rstrip() + "\n"


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Run a small OpenRouter-backed model council.")
    parser.add_argument("--mode", choices=sorted(MODE_GUIDANCE), default="decision")
    parser.add_argument("--question", default="")
    parser.add_argument("--input-file")
    parser.add_argument("--models", default="", help="Comma-separated OpenRouter model IDs.")
    parser.add_argument("--synthesis-model", default="", help="Defaults to the first successful panel model.")
    parser.add_argument("--temperature", type=float, default=0.2)
    parser.add_argument("--max-tokens", type=int, default=1400)
    parser.add_argument("--synthesis-max-tokens", type=int, default=1800)
    parser.add_argument("--timeout", type=int, default=180)
    return parser


def main() -> int:
    args = build_parser().parse_args()
    load_dotenv_if_present()
    api_key = os.getenv("OPENROUTER_API_KEY", "").strip()
    if not api_key:
        print("OPENROUTER_API_KEY is not set.", file=sys.stderr)
        return 2

    question = read_question(args)
    models = [item.strip() for item in args.models.split(",") if item.strip()] if args.models else env_models()
    if not models:
        print("No models configured.", file=sys.stderr)
        return 2

    with concurrent.futures.ThreadPoolExecutor(max_workers=min(len(models), 6)) as executor:
        futures = [
            executor.submit(
                ask_panel,
                api_key,
                model,
                args.mode,
                question,
                args.temperature,
                args.max_tokens,
                args.timeout,
            )
            for model in models
        ]
        results = [future.result() for future in concurrent.futures.as_completed(futures)]
    results.sort(key=lambda result: models.index(result.model))

    successful = [result for result in results if result.ok]
    synthesis = None
    if successful:
        synthesis_model = args.synthesis_model.strip() or successful[0].model
        started = time.monotonic()
        try:
            synthesis_content = chat_completion(
                api_key=api_key,
                model=synthesis_model,
                messages=synth_messages(args.mode, question, successful),
                temperature=0.1,
                max_tokens=args.synthesis_max_tokens,
                timeout=args.timeout,
            )
            synthesis = CouncilResult(synthesis_model, True, synthesis_content, time.monotonic() - started)
        except Exception as exc:
            synthesis = CouncilResult(synthesis_model, False, str(exc), time.monotonic() - started)

    print(render_report(mode=args.mode, question=question, models=models, results=results, synthesis=synthesis))
    return 0 if successful else 1


if __name__ == "__main__":
    raise SystemExit(main())

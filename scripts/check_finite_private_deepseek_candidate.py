#!/usr/bin/env python3
from __future__ import annotations

import argparse
import re
from pathlib import Path


ON_CANDIDATE = Path(
    "infra/tinfoil/confidential-kimi-k2-6/"
    "tinfoil-config.deepseek-v4-flash-0731-dspark-on.candidate.yml"
)
OFF_CANDIDATE = Path(
    "infra/tinfoil/confidential-kimi-k2-6/"
    "tinfoil-config.deepseek-v4-flash-0731-dspark-off.candidate.yml"
)
MODEL_REPO = (
    'repo: "deepseek-ai/DeepSeek-V4-Flash-0731@'
    '7872f01b1d1fe23eabc4c98b48bffcef5a386062"'
)
VLLM_IMAGE = (
    'image: "vllm/vllm-openai:v0.26.0@sha256:'
    '770fe65b2c73ee74a5c42165cf3433de4048cc2cd9c57a937ca4e35aba5aa87b"'
)
LIMITER_IMAGE = (
    'image: "ghcr.io/finitecomputer/finite-private-limiter:'
    '2026-07-02.glm52.health.1@sha256:'
    'f977b238439ff4caa3f416bf1ec8f16ed383640d7417262d26ed4388c8624d5c"'
)
DSPARK_CONFIG = (
    "{\"method\":\"dspark\",\"num_speculative_tokens\":7,"
    "\"draft_sample_method\":\"greedy\"}"
)
MPK_PATTERN = re.compile(
    r'mpk: "(?P<root>[0-9a-f]{64})_(?P<offset>[0-9]+)_'
    r'(?P<uuid>[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})"'
)


def _without_dspark(lines: list[str]) -> list[str]:
    result: list[str] = []
    skip_next = False
    for line in lines:
        if skip_next:
            if DSPARK_CONFIG not in line:
                result.append(line)
            skip_next = False
            continue
        if line.strip() == '"--speculative-config",':
            skip_next = True
            continue
        result.append(line)
    return result


def _validate_one(text: str, *, dspark: bool, release_ready: bool) -> list[str]:
    violations: list[str] = []
    required = (
        "cvm-version: 0.10.8",
        "cpus: 32",
        "memory: 524288",
        "gpus: 8",
        MODEL_REPO,
        VLLM_IMAGE,
        LIMITER_IMAGE,
        '"--trust-remote-code"',
        '"--kv-cache-dtype"',
        '"fp8"',
        '"--block-size"',
        '"256"',
        '"--tensor-parallel-size"',
        '"--enable-expert-parallel"',
        '"--tokenizer-mode"',
        '"--tool-call-parser"',
        '"--enable-auto-tool-choice"',
        '"--reasoning-parser"',
        '"deepseek_v4"',
        '"--enable-prompt-tokens-details"',
        '"--max-model-len"',
        '"393216"',
        '"--max-num-seqs"',
        '"256"',
        '"--max-num-batched-tokens"',
        '"8192"',
        '"--gpu-memory-utilization"',
        '"0.95"',
        '"--no-enable-flashinfer-autotune"',
        '"--compilation-config"',
        '"FULL_DECODE_ONLY"',
        '"--served-model-name"',
        '"deepseek-v4-flash-0731"',
        '"glm-5-2"',
        'UPSTREAM_BASE_URL: "http://deepseek-v4-flash-0731:8001"',
        'FINITE_PRIVATE_MODEL: "glm-5-2"',
        "upstream-container: finite-private-limiter",
        "upstream-port: 8002",
        "authenticated: false",
        "- VLLM_API_KEY",
        "- VLLM_INTERNAL_API_KEY",
        "- FINITE_USAGE_API_SERVICE_KEY",
    )
    for anchor in required:
        if anchor not in text:
            violations.append(f"missing required candidate anchor: {anchor}")

    for forbidden in (
        "deep_gemm_mega_moe",
        "use_fp4_indexer_cache",
        "glm47",
        "glm45",
        '"--decode-context-parallel-size"',
        '"--dcp-sparse-indexer-mode"',
        '"--attention-backend"',
        '"runai_streamer"',
        '"synthetic"',
    ):
        if forbidden in text:
            violations.append(f"forbidden carried-forward or unsupported flag: {forbidden}")

    has_dspark = DSPARK_CONFIG in text
    if dspark and not has_dspark:
        violations.append("DSpark-on candidate lacks the exact verified DSpark config")
    if not dspark and has_dspark:
        violations.append("DSpark-off candidate still enables DSpark")

    mpk_match = MPK_PATTERN.search(text)
    model_path_match = re.search(r'"/tinfoil/mpk/mpk-([0-9a-f]{64})"', text)
    has_placeholders = (
        'mpk: "REPLACE_WITH_TINFOIL_MODELWRAP_MPK"' in text
        and '"/tinfoil/mpk/mpk-REPLACE_WITH_TINFOIL_ROOT_HASH"' in text
    )
    if mpk_match and model_path_match:
        if mpk_match.group("root") != model_path_match.group(1):
            violations.append("model mount path does not match the modelwrap MPK root hash")
    elif release_ready:
        violations.append("Tinfoil modelwrap MPK/root hash is not release-ready")
    elif not has_placeholders:
        violations.append("candidate has neither a valid modelwrap MPK nor both prep placeholders")

    return violations


def check_repository(root: Path, *, release_ready: bool = False) -> list[str]:
    violations: list[str] = []
    try:
        on_text = (root / ON_CANDIDATE).read_text(encoding="utf-8")
        off_text = (root / OFF_CANDIDATE).read_text(encoding="utf-8")
    except FileNotFoundError as error:
        return [f"missing DeepSeek candidate: {error.filename}"]

    violations.extend(_validate_one(on_text, dspark=True, release_ready=release_ready))
    violations.extend(_validate_one(off_text, dspark=False, release_ready=release_ready))
    if _without_dspark(on_text.splitlines()) != off_text.splitlines():
        violations.append("DSpark-on/off candidates differ by more than speculative decoding")
    return violations


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--release-ready", action="store_true")
    arguments = parser.parse_args()
    violations = check_repository(arguments.root, release_ready=arguments.release_ready)
    if violations:
        for violation in violations:
            print(f"ERROR: {violation}")
        return 1
    mode = "release-ready" if arguments.release_ready else "prep"
    print(f"Finite Private DeepSeek candidates pass the {mode} contract")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

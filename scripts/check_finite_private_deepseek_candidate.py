#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import re
from pathlib import Path


OFF_CANDIDATE = Path(
    "infra/tinfoil/confidential-kimi-k2-6/"
    "tinfoil-config.deepseek-v4-flash-0731-dspark-off.candidate.yml"
)
RUNBOOK = Path("infra/runbooks/finite-private-deepseek-production-update.md")
SATELLITE_ROLLBACK_COMMIT = "e337db3606d67c53387113700362adec7b4dfdf7"
MODEL_REPO = (
    'repo: "deepseek-ai/DeepSeek-V4-Flash-0731@'
    '7872f01b1d1fe23eabc4c98b48bffcef5a386062"'
)
VLLM_IMAGE_PLACEHOLDER = 'image: "REPLACE_WITH_MEASURED_DEEPSEEK_V4_VLLM_IMAGE"'
VLLM_IMAGE_PATTERN = re.compile(
    r'image: "ghcr\.io/finitecomputer/deepseek-v4-vllm:'
    r'0\.25\.1-0731-reasoning\.[0-9]+@sha256:[0-9a-f]{64}"'
)
LIMITER_IMAGE = (
    'image: "ghcr.io/finitecomputer/finite-private-limiter:'
    "2026-07-02.glm52.health.1@sha256:"
    'f977b238439ff4caa3f416bf1ec8f16ed383640d7417262d26ed4388c8624d5c"'
)
MPK_PATTERN = re.compile(
    r'mpk: "(?P<root>[0-9a-f]{64})_(?P<offset>[0-9]+)_'
    r'(?P<uuid>[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})"'
)


def _validate_one(text: str, *, release_ready: bool) -> list[str]:
    violations: list[str] = []
    required = (
        "cvm-version: 0.10.8",
        "cpus: 32",
        "memory: 524288",
        "gpus: 8",
        MODEL_REPO,
        LIMITER_IMAGE,
        '"--trust-remote-code"',
        '"--kv-cache-dtype",\n        "fp8"',
        '"--block-size",\n        "256"',
        '"--data-parallel-size",\n        "8"',
        '"--enable-expert-parallel"',
        '"--tokenizer-mode",\n        "deepseek_v4"',
        '"--tool-call-parser",\n        "deepseek_v4"',
        '"--enable-auto-tool-choice"',
        '"--reasoning-parser",\n        "deepseek_v4"',
        '"--default-chat-template-kwargs",\n        \'{"enable_thinking":true}\'',
        '"--enable-prompt-tokens-details"',
        '"--max-model-len",\n        "393216"',
        '"--max-num-seqs",\n        "128"',
        '"--max-num-batched-tokens",\n        "2048"',
        '"--gpu-memory-utilization",\n        "0.95"',
        '"--no-enable-flashinfer-autotune"',
        '"--compilation-config",\n        \'{"mode":0,"cudagraph_mode":"FULL_DECODE_ONLY"}\'',
        '"--served-model-name"',
        '"deepseek-v4-flash-0731"',
        '"glm-5-2"',
        'UPSTREAM_BASE_URL: "http://deepseek-v4-flash-0731:8001"',
        'FINITE_PRIVATE_MODEL: "deepseek-v4-flash-0731"',
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
        '"--tensor-parallel-size"',
        '"--disable-custom-all-reduce"',
        '"--speculative-config"',
        "dspark",
        "vllm/vllm-openai:v0.26.0",
        'FINITE_PRIVATE_MODEL: "glm-5-2"',
    ):
        if forbidden in text:
            violations.append(
                f"forbidden carried-forward or unsupported flag: {forbidden}"
            )

    has_measured_image = VLLM_IMAGE_PATTERN.search(text) is not None
    has_image_placeholder = VLLM_IMAGE_PLACEHOLDER in text
    if release_ready and not has_measured_image:
        violations.append("DeepSeek candidate lacks a measured vLLM image digest")
    elif not release_ready and not (has_measured_image or has_image_placeholder):
        violations.append(
            "DeepSeek candidate has neither a measured image nor the prep placeholder"
        )

    mpk_match = MPK_PATTERN.search(text)
    model_path_match = re.search(r'"/tinfoil/mpk/mpk-([0-9a-f]{64})"', text)
    has_placeholders = (
        'mpk: "REPLACE_WITH_TINFOIL_MODELWRAP_MPK"' in text
        and '"/tinfoil/mpk/mpk-REPLACE_WITH_TINFOIL_ROOT_HASH"' in text
    )
    if mpk_match and model_path_match:
        if mpk_match.group("root") != model_path_match.group(1):
            violations.append(
                "model mount path does not match the modelwrap MPK root hash"
            )
    elif release_ready:
        violations.append("Tinfoil modelwrap MPK/root hash is not release-ready")
    elif not has_placeholders:
        violations.append(
            "candidate has neither a valid modelwrap MPK nor both prep placeholders"
        )

    return violations


def check_repository(root: Path, *, release_ready: bool = False) -> list[str]:
    violations: list[str] = []
    try:
        off_text = (root / OFF_CANDIDATE).read_text(encoding="utf-8")
    except FileNotFoundError as error:
        return [f"missing DeepSeek candidate: {error.filename}"]

    violations.extend(_validate_one(off_text, release_ready=release_ready))
    try:
        runbook_text = (root / RUNBOOK).read_text(encoding="utf-8")
    except FileNotFoundError as error:
        violations.append(f"missing DeepSeek production runbook: {error.filename}")
        return violations

    if SATELLITE_ROLLBACK_COMMIT not in runbook_text:
        violations.append(
            "DeepSeek runbook lacks the exact satellite rollback commit"
        )
    for anchor in (
        'never from satellite `main`',
        '--ref "$SATELLITE_BRANCH"',
        "control.inf12.tinfoil.sh",
        "one active eight-H200 cluster",
        "b6018f87da91d19d0ab4cf979885689b469cdd41",
        "mixed-version-canary",
        "compat/matrix.toml",
        "pre-existing non-causal exception",
        "Any new or worsened red or unknown",
        (
            "Do not trigger a Runtime, NixOS, Litestream, storage-policy, "
            "or host-storage"
        ),
    ):
        if anchor not in runbook_text:
            violations.append(f"DeepSeek runbook lacks release anchor: {anchor}")

    if "Any red or unresolved unknown result stops the rollout" in runbook_text:
        violations.append(
            "DeepSeek runbook incorrectly couples unrelated fleet repairs to the "
            "scheduler rollout"
        )

    if "deepseek-v4-release-candidate" in runbook_text:
        violations.append(
            "DeepSeek runbook incorrectly requires a second eight-H200 candidate target"
        )

    candidate_sha256 = hashlib.sha256(off_text.encode()).hexdigest()
    if candidate_sha256 not in runbook_text:
        violations.append(
            "DeepSeek runbook does not pin the checked-in candidate SHA-256"
        )
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

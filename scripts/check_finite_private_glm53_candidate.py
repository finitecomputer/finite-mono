#!/usr/bin/env python3
from __future__ import annotations

import argparse
import re
from pathlib import Path


MAIN_CANDIDATE = Path(
    "infra/tinfoil/confidential-finite-private/"
    "tinfoil-config.glm-5.3-flash.candidate.yml"
)
BRIDGE_CANDIDATE = Path(
    "infra/tinfoil/confidential-kimi-k2-6/"
    "tinfoil-config.compatibility-bridge.candidate.yml"
)
RUNBOOK = Path(
    "infra/runbooks/finite-private-glm-5.3-flash-production-cutover.md"
)

CHECKPOINT = (
    'repo: "zai-org/GLM-5.3-Flash@'
    '04c4e9e95c5da8862dced7e5056455116f83a7e0"'
)
SGLANG_IMAGE_PLACEHOLDER = 'image: "REPLACE_WITH_VERIFIED_GL53_SGLANG_IMAGE"'
LIMITER_IMAGE_PLACEHOLDER = 'image: "REPLACE_WITH_VERIFIED_GL53_LIMITER_IMAGE"'
SGLANG_IMAGE_PATTERN = re.compile(
    r'image: "ghcr\.io/finitecomputer/glm-5-3-flash-sglang:'
    r'[a-zA-Z0-9._-]+@sha256:[0-9a-f]{64}"'
)
LIMITER_IMAGE_PATTERN = re.compile(
    r'image: "ghcr\.io/finitecomputer/(?:finite-)?private-limiter:'
    r'[a-zA-Z0-9._-]+@sha256:[0-9a-f]{64}"'
)
MPK_PATTERN = re.compile(
    r'mpk: "(?P<root>[0-9a-f]{64})_(?P<offset>[0-9]+)_'
    r'(?P<uuid>[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})"'
)
BRIDGE_IMAGE = (
    'image: "caddy:2.10.2-alpine@sha256:'
    'd8c17a862962def15cde69863a3a463f25a2664942eafd7bdbf050e9c3116b83"'
)


def _require(text: str, anchors: tuple[str, ...], scope: str) -> list[str]:
    return [
        f"{scope} lacks required anchor: {anchor}"
        for anchor in anchors
        if anchor not in text
    ]


def _require_once(text: str, anchors: tuple[str, ...], scope: str) -> list[str]:
    violations: list[str] = []
    for anchor in anchors:
        count = text.count(anchor)
        if count != 1:
            violations.append(
                f"{scope} must contain exactly one anchor ({count} found): {anchor}"
            )
    return violations


def _check_main(text: str, *, release_ready: bool) -> list[str]:
    violations = _require(
        text,
        (
            "cvm-version: 0.10.8",
            "cpus: 32",
            "memory: 524288",
            "gpus: 8",
            CHECKPOINT,
            'name: "glm-5-3-flash"',
            '"--served-model-name",\n        "glm-5-3-flash"',
            '"--tp-size",\n        "8"',
            '"--ep-size",\n        "8"',
            '"--context-length",\n        "393216"',
            '"--dsa-prefill-backend",\n        "tilelang"',
            '"--dsa-decode-backend",\n        "tilelang"',
            '"--kv-cache-dtype",\n        "bfloat16"',
            '"--moe-runner-backend",\n        "deep_gemm"',
            '"--reasoning-parser",\n        "glm45"',
            '"--tool-call-parser",\n        "glm47"',
            'UPSTREAM_BASE_URL: "http://glm-5-3-flash:8001"',
            'FINITE_PRIVATE_MODEL: "glm-5-3-flash"',
            'FINITE_PRIVATE_UPSTREAM_MODEL: "glm-5-3-flash"',
            (
                'FINITE_PRIVATE_MODEL_ALIASES: '
                '"deepseek-v4-flash-0731,glm-5-2"'
            ),
            "upstream-container: finite-private-limiter",
            "authenticated: false",
            '- "/*"',
            "- VLLM_API_KEY",
            "- VLLM_INTERNAL_API_KEY",
            "- FINITE_USAGE_API_SERVICE_KEY",
        ),
        "GLM candidate",
    )
    violations.extend(
        _require_once(
            text,
            (
                "cvm-version: 0.10.8",
                "cpus: 32",
                "memory: 524288",
                "gpus: 8",
                CHECKPOINT,
                '"--served-model-name",\n        "glm-5-3-flash"',
                '"--tp-size",\n        "8"',
                '"--ep-size",\n        "8"',
                '"--context-length",\n        "393216"',
                'FINITE_PRIVATE_UPSTREAM_MODEL: "glm-5-3-flash"',
                (
                    'FINITE_PRIVATE_MODEL_ALIASES: '
                    '"deepseek-v4-flash-0731,glm-5-2"'
                ),
                '- "/*"',
            ),
            "GLM candidate",
        )
    )
    top_level_resources = re.findall(
        r"^(?:cvm-version|cpus|memory|gpus):.*$", text, re.MULTILINE
    )
    if top_level_resources != [
        "cvm-version: 0.10.8",
        "cpus: 32",
        "memory: 524288",
        "gpus: 8",
    ]:
        violations.append(
            "GLM candidate has duplicate, reordered, or conflicting top-level resources"
        )
    image_lines = re.findall(r'^\s+image: "([^"]+)"$', text, re.MULTILINE)
    if len(image_lines) != 2:
        violations.append(
            f"GLM candidate must declare exactly two container images ({len(image_lines)} found)"
        )

    for forbidden in (
        'FINITE_PRIVATE_MODEL_ALIASES: "*"',
        '"--speculative-algorithm"',
        '"--speculative-num-steps"',
        '"--speculative-eagle-topk"',
        '"--mem-fraction-static"',
        '"--kv-cache-dtype",\n        "fp8"',
        "vllm/vllm-openai",
        "lmsysorg/sglang:glm-5.3-flash\"",
    ):
        if forbidden in text:
            violations.append(f"GLM candidate contains forbidden anchor: {forbidden}")

    aliases = (
        'FINITE_PRIVATE_MODEL_ALIASES: "deepseek-v4-flash-0731,glm-5-2"'
    )
    if aliases not in text:
        violations.append(
            "GLM candidate model aliases must be exactly "
            "deepseek-v4-flash-0731,glm-5-2"
        )

    has_sglang_image = SGLANG_IMAGE_PATTERN.search(text) is not None
    has_limiter_image = LIMITER_IMAGE_PATTERN.search(text) is not None
    if release_ready and not has_sglang_image:
        violations.append("GLM candidate lacks an immutable SGLang image digest")
    elif not release_ready and not (has_sglang_image or SGLANG_IMAGE_PLACEHOLDER in text):
        violations.append("GLM candidate lacks an SGLang image digest or prep placeholder")
    if release_ready and not has_limiter_image:
        violations.append("GLM candidate lacks an immutable limiter image digest")
    elif not release_ready and not (has_limiter_image or LIMITER_IMAGE_PLACEHOLDER in text):
        violations.append("GLM candidate lacks a limiter image digest or prep placeholder")

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
        violations.append(
            "candidate lacks either a valid modelwrap MPK/root hash or both prep placeholders"
        )
    return violations


def _check_bridge(text: str) -> list[str]:
    violations = _require(
        text,
        (
            "cvm-version: 0.10.8",
            "cpus: 2",
            "memory: 2048",
            BRIDGE_IMAGE,
            "finite-private.finite.containers.tinfoil.dev",
            '"--change-host-header"',
            "upstream-container: finite-private-compatibility-bridge",
            "authenticated: false",
            '- "/*"',
        ),
        "compatibility bridge",
    )
    if re.search(r"^gpus\s*:", text, re.MULTILINE) or re.search(
        r"^\s+-?\s*gpus\s*:", text, re.MULTILINE
    ):
        violations.append("compatibility bridge must remain GPU-free")
    if re.search(r"^\s*secrets\s*:", text, re.MULTILINE):
        violations.append("compatibility bridge must remain secretless")
    if "models:" in text:
        violations.append("compatibility bridge must not carry model weights")
    if "finite-private.finite.containers.tinfoil.dev" not in text:
        violations.append("compatibility bridge does not point at the generic route")
    top_level_resources = re.findall(
        r"^(?:cvm-version|cpus|memory|gpus):.*$", text, re.MULTILINE
    )
    if top_level_resources != [
        "cvm-version: 0.10.8",
        "cpus: 2",
        "memory: 2048",
    ]:
        violations.append(
            "compatibility bridge has duplicate, reordered, or conflicting resources"
        )
    image_lines = re.findall(r'^\s+image: "([^"]+)"$', text, re.MULTILINE)
    if image_lines != [BRIDGE_IMAGE.removeprefix('image: "').removesuffix('"')]:
        violations.append("compatibility bridge must declare exactly the fixed Caddy image")
    if text.count('- "/*"') != 1:
        violations.append("compatibility bridge must proxy the route surface exactly once")
    return violations


def _check_runbook(text: str) -> list[str]:
    return _require(
        text,
        (
            "2026-08-28 03:00 America/Chicago",
            "preparation only",
            "scripts/finite-status --json",
            "### Resume rules",
            "FINITE_PRIVATE_ROLLBACK_TAG",
            "FINITE_PRIVATE_ROLLBACK_CONTAINER_ID",
            "--replace",
            "finite-private.finite.containers.tinfoil.dev",
            "kimi-k2-6.finite.containers.tinfoil.dev",
            "glm-5-3-flash",
            "deepseek-v4-flash-0731",
            "glm-5-2",
            "120/120",
            "scripts/check_finite_private_glm53_quality.py",
            "scripts/check_finite_private_glm53_protocol.py",
            "scripts/prepare_glm53_blind_comparison.py",
            "p50 decode",
            "p10 decode",
            "2,400 aggregate output tokens/second",
            "p95 TTFT",
            "35-minute soak",
            "clear_thinking=true",
            "360,000-token",
            "Account enrollment",
            "existing internal canary",
            "Two reviewers independently score",
            "settlement-status",
            "rollback immediately",
            "Do not migrate durable Runtime configurations",
        ),
        "GLM cutover runbook",
    )


def check_repository(root: Path, *, release_ready: bool = False) -> list[str]:
    texts: dict[Path, str] = {}
    violations: list[str] = []
    for relative, label in (
        (MAIN_CANDIDATE, "GLM candidate"),
        (BRIDGE_CANDIDATE, "compatibility bridge"),
        (RUNBOOK, "GLM cutover runbook"),
    ):
        try:
            texts[relative] = (root / relative).read_text(encoding="utf-8")
        except FileNotFoundError:
            violations.append(f"missing {label}: {relative}")
    if violations:
        return violations
    violations.extend(_check_main(texts[MAIN_CANDIDATE], release_ready=release_ready))
    violations.extend(_check_bridge(texts[BRIDGE_CANDIDATE]))
    violations.extend(_check_runbook(texts[RUNBOOK]))
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
    mode = "release-ready" if arguments.release_ready else "preparation"
    print(f"Finite Private GLM-5.3-Flash candidates pass the {mode} contract")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

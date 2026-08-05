#!/usr/bin/env python3
"""Backport the upstream DeepSeek V4 0731 reasoning map onto vLLM 0.25.1.

The base image is pinned by platform digest in the adjacent Dockerfile.  This
script also pins the exact Python sources it expects, so a changed base image
cannot silently receive a fuzzy or partial patch.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
from pathlib import Path


UPSTREAM_FIX = "77434861904a9f01ea4818fe9f0c7b2a5c05686e"

TOKENIZER_RELATIVE_PATH = Path("tokenizers/deepseek_v4.py")
ENCODING_RELATIVE_PATH = Path("tokenizers/deepseek_v4_encoding.py")

BASE_SHA256 = {
    TOKENIZER_RELATIVE_PATH: "f1ebaaa58fc7f453ebb38c07234591abe35780641822f872558c964cf702fcf3",
    ENCODING_RELATIVE_PATH: "582f735bf75ad5fbe2b4a8801d8280a65ff0e41175874249bd62433e78c487ff",
}

# Filled from the deterministic transform below.  These hashes describe the
# v0.25.1 files plus only the behavioral parts of upstream vLLM #50580.
PATCHED_SHA256 = {
    TOKENIZER_RELATIVE_PATH: "b1a547bc1ffe38e4166945eb2ff4d7bdd6210e6ede3b991d36dc90b2b50fe14c",
    ENCODING_RELATIVE_PATH: "b671a214386f1bb9c3c2391329490ed8ba731f5282983ce49b828f5edb597958",
}


class PatchError(RuntimeError):
    pass


def _sha256(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def _replace_once(text: str, old: str, new: str, *, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise PatchError(f"{label}: expected one source block, found {count}")
    return text.replace(old, new, 1)


def patch_tokenizer(text: str) -> str:
    text = _replace_once(
        text,
        '''            thinking = kwargs.get("thinking", False)
            enable_thinking = kwargs.get("enable_thinking", False)
            thinking = thinking or enable_thinking
            thinking_mode = "thinking" if thinking else "chat"
''',
        '''            thinking = kwargs.get("thinking")
            enable_thinking = kwargs.get("enable_thinking")
            thinking_enabled = bool(thinking) or bool(enable_thinking)
            if "thinking" not in kwargs and "enable_thinking" not in kwargs:
                thinking_enabled = True
            thinking_mode = "thinking" if thinking_enabled else "chat"
''',
        label="thinking default",
    )
    return _replace_once(
        text,
        '''            reasoning_effort = kwargs.get("reasoning_effort")
            if not isinstance(reasoning_effort, str):
                reasoning_effort = None
            elif reasoning_effort == "none":
                thinking_mode = "chat"
                reasoning_effort = None
            elif reasoning_effort in ("max", "xhigh"):
                reasoning_effort = "max"
            else:
                reasoning_effort = "high"
''',
        '''            reasoning_effort = kwargs.get("reasoning_effort")
            if not isinstance(reasoning_effort, str):
                reasoning_effort = "high" if thinking_enabled else None
            elif reasoning_effort == "none":
                thinking_mode = "chat"
                reasoning_effort = None
            elif reasoning_effort == "max":
                reasoning_effort = "max"
            elif reasoning_effort in ("low", "minimal", "medium"):
                reasoning_effort = "low"
            else:
                reasoning_effort = "high"
''',
        label="reasoning effort mapping",
    )


def patch_encoding(text: str) -> str:
    text = _replace_once(
        text,
        '''REASONING_EFFORT_MAX = (
    "Reasoning Effort: Absolute maximum with no shortcuts permitted.\\n"
    "You MUST be very thorough in your thinking and comprehensively decompose the problem to resolve the root cause, rigorously stress-testing your logic against all potential paths, edge cases, and adversarial scenarios.\\n"
    "Explicitly write out your entire deliberation process, documenting every intermediate step, considered alternative, and rejected hypothesis to ensure absolutely no assumption is left unchecked.\\n\\n"
)
''',
        '''REASONING_EFFORT_PROMPTS: Dict[str, str] = {
    "low": "",
    "high": (
        "Reasoning Effort: Absolute maximum with no shortcuts permitted.\\n"
        "You MUST be very thorough in your thinking and comprehensively decompose the problem to resolve the root cause, rigorously stress-testing your logic against all potential paths, edge cases, and adversarial scenarios.\\n"
        "Explicitly write out your entire deliberation process, documenting every intermediate step, considered alternative, and rejected hypothesis to ensure absolutely no assumption is left unchecked.\\n\\n"
    ),
    "max": (
        "Reasoning Effort: Beyond maximum — exhaustive, relentless, and uncompromising.\\n"
        "You MUST reason with the utmost depth and rigor, leaving absolutely nothing to chance: exhaustively decompose the problem into its most fundamental components, trace every causal chain to its root, and resolve the underlying cause rather than any surface symptom.\\n"
        "Do not stop reasoning until you have independently verified the solution from multiple angles and are certain that no assumption remains unchecked and no error remains undiscovered.\\n\\n"
    ),
}
DEFAULT_REASONING_EFFORT = "low"
''',
        label="reasoning effort prompts",
    )
    return _replace_once(
        text,
        '''    # Reasoning effort prefix (only at index 0 in thinking mode with max effort)
    assert reasoning_effort in ['max', None, 'high'], f"Invalid reasoning effort: {reasoning_effort}"
    if index == 0 and thinking_mode == "thinking" and reasoning_effort == 'max':
        prompt += REASONING_EFFORT_MAX
''',
        '''    reasoning_effort = reasoning_effort or DEFAULT_REASONING_EFFORT
    assert reasoning_effort in REASONING_EFFORT_PROMPTS, (
        f"Invalid reasoning effort: {reasoning_effort}, expected one of "
        f"{list(REASONING_EFFORT_PROMPTS)}"
    )
    if index == 0 and thinking_mode == "thinking":
        prompt += REASONING_EFFORT_PROMPTS[reasoning_effort]
''',
        label="reasoning effort rendering",
    )


PATCHERS = {
    TOKENIZER_RELATIVE_PATH: patch_tokenizer,
    ENCODING_RELATIVE_PATH: patch_encoding,
}


def _vllm_root() -> Path:
    spec = importlib.util.find_spec("vllm")
    if spec is None or spec.submodule_search_locations is None:
        raise PatchError("installed vllm package was not found")
    locations = list(spec.submodule_search_locations)
    if len(locations) != 1:
        raise PatchError(f"expected one installed vllm package, found {locations}")
    return Path(locations[0])


def apply(root: Path) -> None:
    for relative_path, patcher in PATCHERS.items():
        path = root / relative_path
        text = path.read_text(encoding="utf-8")
        actual_hash = _sha256(text)
        if actual_hash == PATCHED_SHA256[relative_path]:
            continue
        if actual_hash != BASE_SHA256[relative_path]:
            raise PatchError(
                f"{relative_path}: base sha256 {actual_hash} does not match "
                f"pinned vLLM 0.25.1 source {BASE_SHA256[relative_path]}"
            )
        patched = patcher(text)
        patched_hash = _sha256(patched)
        if patched_hash != PATCHED_SHA256[relative_path]:
            raise PatchError(
                f"{relative_path}: patched sha256 {patched_hash} does not match "
                f"the reviewed result {PATCHED_SHA256[relative_path]}"
            )
        path.write_text(patched, encoding="utf-8")


def check(root: Path) -> None:
    for relative_path in PATCHERS:
        actual_hash = _sha256((root / relative_path).read_text(encoding="utf-8"))
        if actual_hash != PATCHED_SHA256[relative_path]:
            raise PatchError(
                f"{relative_path}: installed sha256 {actual_hash} does not match "
                f"the reviewed 0731 backport {PATCHED_SHA256[relative_path]}"
            )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("apply", "check"))
    parser.add_argument("--root", type=Path)
    arguments = parser.parse_args()
    root = arguments.root or _vllm_root()
    if arguments.mode == "apply":
        apply(root)
    else:
        check(root)
    print(
        f"vLLM DeepSeek V4 0731 reasoning backport {UPSTREAM_FIX} "
        f"passes {arguments.mode} verification"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

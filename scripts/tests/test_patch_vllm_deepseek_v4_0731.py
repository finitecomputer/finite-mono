from __future__ import annotations

import hashlib
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from infra.images.patch_vllm_deepseek_v4_0731 import (
    BASE_SHA256,
    ENCODING_RELATIVE_PATH,
    PATCHED_SHA256,
    PatchError,
    TOKENIZER_RELATIVE_PATH,
    apply,
    check,
    patch_encoding,
    patch_tokenizer,
)


TOKENIZER_FIXTURE = '''            thinking = kwargs.get("thinking", False)
            enable_thinking = kwargs.get("enable_thinking", False)
            thinking = thinking or enable_thinking
            thinking_mode = "thinking" if thinking else "chat"

            reasoning_effort = kwargs.get("reasoning_effort")
            if not isinstance(reasoning_effort, str):
                reasoning_effort = None
            elif reasoning_effort == "none":
                thinking_mode = "chat"
                reasoning_effort = None
            elif reasoning_effort in ("max", "xhigh"):
                reasoning_effort = "max"
            else:
                reasoning_effort = "high"
'''

ENCODING_FIXTURE = '''REASONING_EFFORT_MAX = (
    "Reasoning Effort: Absolute maximum with no shortcuts permitted.\\n"
    "You MUST be very thorough in your thinking and comprehensively decompose the problem to resolve the root cause, rigorously stress-testing your logic against all potential paths, edge cases, and adversarial scenarios.\\n"
    "Explicitly write out your entire deliberation process, documenting every intermediate step, considered alternative, and rejected hypothesis to ensure absolutely no assumption is left unchecked.\\n\\n"
)

    # Reasoning effort prefix (only at index 0 in thinking mode with max effort)
    assert reasoning_effort in ['max', None, 'high'], f"Invalid reasoning effort: {reasoning_effort}"
    if index == 0 and thinking_mode == "thinking" and reasoning_effort == 'max':
        prompt += REASONING_EFFORT_MAX
'''


def sha256(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


class DeepSeekV40731VllmPatchTests(unittest.TestCase):
    def test_transform_contains_all_three_0731_effort_levels(self) -> None:
        tokenizer = patch_tokenizer(TOKENIZER_FIXTURE)
        encoding = patch_encoding(ENCODING_FIXTURE)

        self.assertIn('reasoning_effort = "high" if thinking_enabled else None', tokenizer)
        self.assertIn('reasoning_effort == "max"', tokenizer)
        self.assertIn('("low", "minimal", "medium")', tokenizer)
        self.assertIn('"low": ""', encoding)
        self.assertIn('"high": (', encoding)
        self.assertIn('"max": (', encoding)
        self.assertIn("Reasoning Effort: Beyond maximum", encoding)
        self.assertIn("REASONING_EFFORT_PROMPTS[reasoning_effort]", encoding)

    def test_apply_is_exact_and_idempotent(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            tokenizer_path = root / TOKENIZER_RELATIVE_PATH
            encoding_path = root / ENCODING_RELATIVE_PATH
            tokenizer_path.parent.mkdir(parents=True)
            tokenizer_path.write_text(TOKENIZER_FIXTURE, encoding="utf-8")
            encoding_path.write_text(ENCODING_FIXTURE, encoding="utf-8")

            patched_tokenizer = patch_tokenizer(TOKENIZER_FIXTURE)
            patched_encoding = patch_encoding(ENCODING_FIXTURE)
            base_hashes = {
                TOKENIZER_RELATIVE_PATH: sha256(TOKENIZER_FIXTURE),
                ENCODING_RELATIVE_PATH: sha256(ENCODING_FIXTURE),
            }
            patched_hashes = {
                TOKENIZER_RELATIVE_PATH: sha256(patched_tokenizer),
                ENCODING_RELATIVE_PATH: sha256(patched_encoding),
            }
            with (
                patch.dict(BASE_SHA256, base_hashes, clear=True),
                patch.dict(PATCHED_SHA256, patched_hashes, clear=True),
            ):
                apply(root)
                check(root)
                apply(root)
                check(root)

            self.assertEqual(tokenizer_path.read_text(), patched_tokenizer)
            self.assertEqual(encoding_path.read_text(), patched_encoding)

    def test_apply_rejects_an_unknown_base(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            for relative_path in (TOKENIZER_RELATIVE_PATH, ENCODING_RELATIVE_PATH):
                path = root / relative_path
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("unexpected source", encoding="utf-8")

            with self.assertRaisesRegex(PatchError, "does not match pinned vLLM"):
                apply(root)


if __name__ == "__main__":
    unittest.main()

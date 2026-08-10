"""Cross-language tree-digest parity for Finite Skills bundles.

`finite-release` (Rust) must reproduce `_validate()`'s digest from
`finitechat/containers/agent/finite.py` byte for byte. Both sides assert the
same golden digest over the fixture tree checked into
`finite-release/tests/fixtures/skills-tree`; the Rust half lives in
`finite-release/src/lib.rs`
(`fixture_tree_digest_matches_the_shared_golden_constant`). If a digest
change is intentional, update both constants together.
"""

from __future__ import annotations

import importlib.util
import types
import unittest
from pathlib import Path

MONOREPO_ROOT = Path(__file__).resolve().parents[3]
FINITE = MONOREPO_ROOT / "finitechat/containers/agent/finite.py"
FIXTURE_TREE = MONOREPO_ROOT / "finite-release/tests/fixtures/skills-tree"

EXPECTED_TREE_DIGEST = "bc07f6443ed59aff6738c3a132ddfc5c70eb05ab231599faf725779214d06413"
EXPECTED_SKILL_COUNT = 3


def load_finite_module() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("finite_cli_under_digest_test", FINITE)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class FiniteSkillsDigestParityTest(unittest.TestCase):
    def test_finite_py_digest_of_the_shared_fixture_matches_the_rust_constant(self) -> None:
        module = load_finite_module()
        digest, skill_count = module._validate(FIXTURE_TREE)

        self.assertEqual(digest, EXPECTED_TREE_DIGEST)
        self.assertEqual(skill_count, EXPECTED_SKILL_COUNT)

    def test_fixture_pins_full_string_path_ordering(self) -> None:
        # "note-extra/SKILL.md" sorts before "note.md" ('-' < '.') only under
        # the full-string ordering both implementations must share.
        module = load_finite_module()
        files = module._regular_tree_files(FIXTURE_TREE)
        relative = [path.relative_to(FIXTURE_TREE).as_posix() for path in files]

        self.assertEqual(
            relative,
            [
                "productivity/note-extra/SKILL.md",
                "productivity/note.md",
                "software-development/finite-sites-publishing-finite/SKILL.md",
                "software-development/finitebrain/SKILL.md",
                "software-development/finitebrain/references/guide.md",
            ],
        )

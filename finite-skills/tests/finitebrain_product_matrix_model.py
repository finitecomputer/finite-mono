#!/usr/bin/env python3
"""Fast contract checks for the deterministic managed-agent product matrix."""

from __future__ import annotations

import importlib.machinery
import importlib.util
import sys
from pathlib import Path


sys.dont_write_bytecode = True


ROOT = Path(__file__).resolve().parents[2]
MATRIX_PATH = ROOT / "scripts" / "devfinity-brain-product-matrix"


def load_matrix_module():
    loader = importlib.machinery.SourceFileLoader("brain_product_matrix", str(MATRIX_PATH))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    if spec is None:
        raise AssertionError(f"could not load {MATRIX_PATH}")
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


def personal_affirmative_command(module) -> str:
    model = module.MatrixModel()
    payload = {
        "messages": [
            {
                "role": "user",
                "content": "Create Matrix agent Personal resumed task 1-2",
            },
            {"role": "assistant", "content": "Would you like setup?"},
            {"role": "user", "content": "Yes, set it up"},
        ],
        "tools": [{"function": {"name": "terminal"}}],
    }
    kind, command, _ = model.classify(payload)
    if kind != "tool":
        raise AssertionError(f"expected a terminal tool call, received {kind!r}")
    return command


def main() -> None:
    command = personal_affirmative_command(load_matrix_module())
    expected_fragments = (
        'folder_path="$(fbrain folder create ',
        'json.load(sys.stdin)["folders"]',
        'folder["name"] == name',
        "'Matrix agent Personal resumed task 1-2')\"",
        'mkdir -p "$folder_path/wiki"',
        '"$folder_path/wiki/matrix-acceptance.md"',
    )
    missing = [fragment for fragment in expected_fragments if fragment not in command]
    if missing:
        raise AssertionError(
            "Personal Brain matrix authoring must use the authoritative returned Folder "
            "path and canonical wiki/ projection; "
            f"missing {missing!r}; "
            f"generated command was: {command}"
        )
    print("FiniteBrain product matrix model checks passed")


if __name__ == "__main__":
    main()

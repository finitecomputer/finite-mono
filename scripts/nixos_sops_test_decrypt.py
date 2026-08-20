#!/usr/bin/env python3
"""Check whether the current operator can decrypt NixOS SOPS secret files."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SECRETS_ROOT = ROOT / "infra/nixos/secrets"
SKIPPED_NAMES = {
    ".sops.yaml",
    "README.md",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Check local decrypt access for NixOS SOPS source files."
    )
    parser.add_argument(
        "--secrets-root",
        type=Path,
        default=DEFAULT_SECRETS_ROOT,
        help=argparse.SUPPRESS,
    )
    parser.add_argument(
        "--sops-bin",
        default=os.environ.get("SOPS_BIN", "sops"),
        help=argparse.SUPPRESS,
    )
    return parser.parse_args()


def is_sops_json_file(path: Path) -> bool:
    if not path.is_file() or path.name in SKIPPED_NAMES:
        return False
    try:
        loaded = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError):
        return False
    return isinstance(loaded, dict) and isinstance(loaded.get("sops"), dict)


def discover_files(secrets_root: Path) -> list[Path]:
    return sorted(
        path
        for path in secrets_root.rglob("*")
        if is_sops_json_file(path)
    )


def display_path(path: Path) -> str:
    try:
        return path.relative_to(ROOT).as_posix()
    except ValueError:
        return path.as_posix()


def can_decrypt_file(sops_bin: str, secrets_root: Path, path: Path) -> bool:
    result = subprocess.run(
        [
            sops_bin,
            "decrypt",
            "--input-type",
            "json",
            "--output-type",
            "binary",
            str(path),
        ],
        cwd=secrets_root,
        check=False,
        stdin=subprocess.DEVNULL,
        capture_output=True,
    )
    return result.returncode == 0


def main() -> int:
    args = parse_args()
    secrets_root = args.secrets_root.resolve()
    if not (secrets_root / ".sops.yaml").exists():
        print("false")
        print(
            f"Missing {display_path(secrets_root / '.sops.yaml')}; SOPS "
            "recipients are not configured yet. Add the public recipients "
            "before testing decrypt access."
        )
        return 2

    files = discover_files(secrets_root)
    if not files:
        print("true")
        print(
            "No existing NixOS SOPS secret files were found, so there is "
            "nothing to decrypt yet. This does not prove access to future "
            "secrets; rerun this after encrypted files exist."
        )
        return 0

    for path in files:
        if can_decrypt_file(args.sops_bin, secrets_root, path):
            continue
        print("false")
        print(
            "This operator cannot decrypt existing NixOS SOPS secrets. "
            f"First failing file: {display_path(path)}. Run "
            "`just nixos nixos-sops-operator-key`, add only the printed public "
            "age recipient to infra/nixos/secrets/.sops.yaml, then ask an "
            "existing operator to run `just nixos nixos-sops-updatekeys` and retry."
        )
        return 1

    print("true")
    print(
        "This operator can decrypt all existing NixOS SOPS secret files with "
        "the current local age key and can use the SOPS helpers to encrypt or "
        "seed secrets for this recipient set."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Refresh SOPS recipients for encrypted NixOS secret source files."""

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
        description="Run sops updatekeys on tracked NixOS SOPS source files."
    )
    parser.add_argument(
        "files",
        nargs="*",
        help="Optional paths under infra/nixos/secrets. Defaults to every SOPS JSON file.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="List files that would be updated without invoking sops.",
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


def relative_file(secrets_root: Path, supplied: str) -> Path:
    path = Path(supplied)
    if path.is_absolute():
        try:
            path = path.resolve().relative_to(secrets_root)
        except ValueError as error:
            raise ValueError(f"{supplied!r} is outside {secrets_root}") from error
    if ".." in path.parts:
        raise ValueError(f"{supplied!r} must not contain '..'")
    if path.name in {"", ".", ".."}:
        raise ValueError(f"{supplied!r} must include a filename")
    return path


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


def selected_files(secrets_root: Path, supplied: list[str]) -> list[Path]:
    if not supplied:
        return discover_files(secrets_root)

    selected: list[Path] = []
    for item in supplied:
        relative = relative_file(secrets_root, item)
        path = secrets_root / relative
        if not path.exists():
            raise ValueError(f"{path} does not exist")
        if not is_sops_json_file(path):
            raise ValueError(f"{path} is not a SOPS JSON file")
        selected.append(path)
    return sorted(dict.fromkeys(selected))


def update_file(sops_bin: str, secrets_root: Path, path: Path) -> None:
    subprocess.run(
        [
            sops_bin,
            "updatekeys",
            "--yes",
            "--input-type",
            "json",
            str(path),
        ],
        cwd=secrets_root,
        check=True,
        capture_output=True,
        text=True,
    )


def display_path(path: Path) -> str:
    try:
        return path.relative_to(ROOT).as_posix()
    except ValueError:
        return path.as_posix()


def main() -> int:
    args = parse_args()
    secrets_root = args.secrets_root.resolve()
    if not (secrets_root / ".sops.yaml").exists():
        print(
            f"nixos-sops-updatekeys: missing {secrets_root / '.sops.yaml'}",
            file=sys.stderr,
        )
        return 2

    try:
        files = selected_files(secrets_root, args.files)
    except ValueError as error:
        print(f"nixos-sops-updatekeys: {error}", file=sys.stderr)
        return 2

    if not files:
        print("nixos-sops-updatekeys: no SOPS JSON files found")
        return 0

    for path in files:
        if args.dry_run:
            print(f"would update: {display_path(path)}")
            continue
        try:
            update_file(args.sops_bin, secrets_root, path)
        except (OSError, subprocess.CalledProcessError) as error:
            print(
                f"nixos-sops-updatekeys: failed to update {display_path(path)}: {error}",
                file=sys.stderr,
            )
            return 1
        print(f"updated: {display_path(path)}")

    action = "would update" if args.dry_run else "updated"
    print(f"{action} {len(files)} SOPS file(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

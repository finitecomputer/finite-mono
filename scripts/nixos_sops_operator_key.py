#!/usr/bin/env python3
"""Create or inspect the local operator age key used by SOPS."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import re
import stat
import subprocess
import sys


AGE_PUBLIC_KEY = re.compile(r"^age1[ac-hj-np-z02-9]+$")


def default_key_file() -> Path:
    configured = os.environ.get("SOPS_AGE_KEY_FILE")
    if configured:
        return Path(configured).expanduser()
    return Path("~/.config/sops/age/keys.txt").expanduser()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Create a local SOPS age key if missing and print its public recipient."
    )
    parser.add_argument(
        "--key-file",
        type=Path,
        default=default_key_file(),
        help="Operator age key path. Defaults to SOPS_AGE_KEY_FILE or ~/.config/sops/age/keys.txt.",
    )
    parser.add_argument(
        "--age-keygen-bin",
        default=os.environ.get("AGE_KEYGEN_BIN", "age-keygen"),
        help=argparse.SUPPRESS,
    )
    return parser.parse_args()


def chmod_private(path: Path) -> None:
    path.chmod(0o600)
    path.parent.chmod(0o700)


def create_key(key_file: Path, age_keygen_bin: str) -> bool:
    if key_file.exists():
        if not key_file.is_file():
            raise RuntimeError(f"{key_file} exists but is not a regular file")
        return False

    key_file.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
    key_file.parent.chmod(0o700)
    subprocess.run(
        [age_keygen_bin, "-o", str(key_file)],
        check=True,
        capture_output=True,
        text=True,
    )
    chmod_private(key_file)
    return True


def public_recipient(key_file: Path, age_keygen_bin: str) -> str:
    result = subprocess.run(
        [age_keygen_bin, "-y", str(key_file)],
        check=True,
        capture_output=True,
        text=True,
    )
    recipient = result.stdout.strip()
    if AGE_PUBLIC_KEY.fullmatch(recipient) is None:
        raise RuntimeError("age-keygen did not return an age public recipient")
    return recipient


def mode(path: Path) -> str:
    return f"{stat.S_IMODE(path.stat().st_mode):04o}"


def main() -> int:
    args = parse_args()
    key_file = args.key_file.expanduser()
    try:
        created = create_key(key_file, args.age_keygen_bin)
        chmod_private(key_file)
        recipient = public_recipient(key_file, args.age_keygen_bin)
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"nixos-sops-operator-key: {error}", file=sys.stderr)
        return 1

    status = "created" if created else "existing"
    print(f"operator age key: {status}")
    print(f"key file: {key_file}")
    print(f"key file mode: {mode(key_file)}")
    print(f"key directory mode: {mode(key_file.parent)}")
    print(f"export SOPS_AGE_KEY_FILE={key_file}")
    print(f"public recipient: {recipient}")
    print("Add only the public recipient to infra/nixos/secrets/.sops.yaml.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

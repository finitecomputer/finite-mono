#!/usr/bin/env python3
"""Encrypt one NixOS secret from stdin into infra/nixos/secrets.

The plaintext is read from stdin and passed directly to sops. It is never
printed, logged, or written to an intermediate file by this helper.
"""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import re
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SECRETS_ROOT = ROOT / "infra/nixos/secrets"
ALLOWED_SCOPES = {"shared", "finite-lat-1", "finite-lat-3"}
ENV_NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
SAFE_LOGICAL_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]*$")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Encrypt stdin into a tracked SOPS file under infra/nixos/secrets."
    )
    parser.add_argument("scope", choices=sorted(ALLOWED_SCOPES))
    parser.add_argument(
        "target",
        help="Target path under the scope, for example metrics-remote-write.env.",
    )
    parser.add_argument(
        "--logical-name",
        help="finite.secrets.files key. Defaults to the target filename without a .env suffix.",
    )
    parser.add_argument(
        "--required-env-name",
        action="append",
        default=[],
        help="Required env variable name for contract metadata. May be repeated.",
    )
    parser.add_argument(
        "--consumer",
        action="append",
        default=[],
        help="Consumer service/job for contract metadata. May be repeated.",
    )
    parser.add_argument("--owner", default="root")
    parser.add_argument("--group", default="root")
    parser.add_argument("--mode", default="0600")
    parser.add_argument(
        "--kind",
        choices=["env", "opaque"],
        default="env",
    )
    parser.add_argument(
        "--restart-unit",
        action="append",
        default=[],
        help="Systemd unit to restart when sops-nix updates the secret. May be repeated.",
    )
    parser.add_argument(
        "--reload-unit",
        action="append",
        default=[],
        help="Systemd unit to reload when sops-nix updates the secret. May be repeated.",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Overwrite an existing encrypted target.",
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


def relative_target(scope: str, target: str) -> Path:
    target_path = Path(target)
    if target_path.is_absolute() or ".." in target_path.parts:
        raise ValueError("target must be a relative path without '..'")
    if target_path.name in {"", ".", ".."}:
        raise ValueError("target must include a filename")
    return Path(scope) / target_path


def default_logical_name(target: Path) -> str:
    name = target.name
    return name.removesuffix(".env")


def nix_list(items: list[str]) -> str:
    if not items:
        return "[ ]"
    quoted = " ".join(f'"{item}"' for item in items)
    return f"[ {quoted} ]"


def contract_snippet(args: argparse.Namespace, rel_target: Path) -> str:
    logical_name = args.logical_name or default_logical_name(rel_target)
    if args.scope == "shared":
        scope = '[ "finite-lat-1" "finite-lat-3" ]'
    else:
        scope = f'[ "{args.scope}" ]'

    lines = [
        f'finite.secrets.files."{logical_name}" = {{',
        f"  scope = {scope};",
        f"  sopsFile = ../secrets/{rel_target.as_posix()};",
        f'  destinationPath = "/run/secrets/finite/{logical_name}";',
        f'  owner = "{args.owner}";',
        f'  group = "{args.group}";',
        f'  mode = "{args.mode}";',
        f'  kind = "{args.kind}";',
        f"  requiredEnvNames = {nix_list(args.required_env_name)};",
        f"  consumers = {nix_list(args.consumer)};",
    ]
    if args.restart_unit:
        lines.append(f"  restartUnits = {nix_list(args.restart_unit)};")
    if args.reload_unit:
        lines.append(f"  reloadUnits = {nix_list(args.reload_unit)};")
    lines.append("};")
    return "\n".join(lines)


def display_path(path: Path) -> str:
    try:
        return path.relative_to(ROOT).as_posix()
    except ValueError:
        return path.as_posix()


def validate_args(args: argparse.Namespace, rel_target: Path) -> str:
    logical_name = args.logical_name or default_logical_name(rel_target)
    if not SAFE_LOGICAL_NAME.fullmatch(logical_name):
        raise ValueError(f"invalid logical name: {logical_name!r}")
    for name in args.required_env_name:
        if not ENV_NAME.fullmatch(name):
            raise ValueError(f"invalid required env name: {name!r}")
    if not re.fullmatch(r"[0-7]{4}", args.mode):
        raise ValueError("mode must be four octal digits")
    return logical_name


def main() -> int:
    args = parse_args()
    try:
        rel_target = relative_target(args.scope, args.target)
        logical_name = validate_args(args, rel_target)
    except ValueError as error:
        print(f"nixos-sops-ingest: {error}", file=sys.stderr)
        return 2

    secrets_root = args.secrets_root.resolve()
    config_path = secrets_root / ".sops.yaml"
    if not config_path.exists():
        print(
            f"nixos-sops-ingest: missing {config_path}; add recipients before ingesting secrets",
            file=sys.stderr,
        )
        return 2

    destination = secrets_root / rel_target
    if destination.exists() and not args.force:
        print(
            f"nixos-sops-ingest: refusing to overwrite {destination}; pass --force to replace it",
            file=sys.stderr,
        )
        return 1

    if sys.stdin.buffer.isatty():
        print(
            "nixos-sops-ingest: refusing to read secret from an interactive tty",
            file=sys.stderr,
        )
        return 2
    plaintext = sys.stdin.buffer.read()
    if plaintext == b"":
        print("nixos-sops-ingest: stdin was empty", file=sys.stderr)
        return 2

    destination.parent.mkdir(parents=True, exist_ok=True)
    command = [
        args.sops_bin,
        "encrypt",
        "--filename-override",
        rel_target.as_posix(),
        "--input-type",
        "binary",
        "--output-type",
        "json",
        "/dev/stdin",
    ]
    try:
        encrypted = subprocess.run(
            command,
            cwd=secrets_root,
            input=plaintext,
            check=True,
            capture_output=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError) as error:
        print(f"nixos-sops-ingest: sops encryption failed: {error}", file=sys.stderr)
        return 1
    if encrypted == b"":
        print("nixos-sops-ingest: sops produced no encrypted output", file=sys.stderr)
        return 1

    destination.write_bytes(encrypted)
    print(f"encrypted SOPS file: {display_path(destination)}")
    print(f"logical name: {logical_name}")
    print()
    print("Nix contract sketch:")
    print(contract_snippet(args, rel_target))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

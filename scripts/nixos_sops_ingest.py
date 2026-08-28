#!/usr/bin/env python3
"""Encrypt one NixOS secret from stdin into infra/nixos/secrets.

The plaintext is read from stdin and passed directly to sops. It is never
printed, logged, or written to an intermediate file by this helper.
"""

from __future__ import annotations

import argparse
import json
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
SKIPPED_NAMES = {
    ".sops.yaml",
    "README.md",
}


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


def is_sops_json_file(path: Path) -> bool:
    if not path.is_file() or path.name in SKIPPED_NAMES:
        return False
    try:
        loaded = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError):
        return False
    return isinstance(loaded, dict) and isinstance(loaded.get("sops"), dict)


def discover_sops_files(secrets_root: Path) -> list[Path]:
    return sorted(path for path in secrets_root.rglob("*") if is_sops_json_file(path))


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


def verify_existing_access(sops_bin: str, secrets_root: Path) -> Path | None:
    for path in discover_sops_files(secrets_root):
        if not can_decrypt_file(sops_bin, secrets_root, path):
            return path
    return None


def age_recipients_from_bytes(payload: bytes) -> set[str]:
    loaded = json.loads(payload.decode("utf-8"))
    sops_metadata = loaded.get("sops")
    if not isinstance(sops_metadata, dict):
        raise ValueError("encrypted payload is missing SOPS metadata")
    age_entries = sops_metadata.get("age")
    if not isinstance(age_entries, list):
        raise ValueError("encrypted payload is missing age recipients")
    recipients = {
        entry["recipient"]
        for entry in age_entries
        if isinstance(entry, dict) and isinstance(entry.get("recipient"), str)
    }
    if not recipients:
        raise ValueError("encrypted payload has no age recipients")
    return recipients


def age_recipients_from_file(path: Path) -> set[str]:
    return age_recipients_from_bytes(path.read_bytes())


def same_scope_sops_files(
    secrets_root: Path, rel_target: Path, destination: Path
) -> list[Path]:
    target_scope = rel_target.parts[0]
    return [
        path
        for path in discover_sops_files(secrets_root)
        if path != destination
        and path.relative_to(secrets_root).parts[0] == target_scope
    ]


def mismatched_recipient_file(
    secrets_root: Path, rel_target: Path, destination: Path, new_recipients: set[str]
) -> Path | None:
    for path in same_scope_sops_files(secrets_root, rel_target, destination):
        if age_recipients_from_file(path) != new_recipients:
            return path
    return None


def verify_decryptable(
    sops_bin: str, secrets_root: Path, rel_target: Path, encrypted: bytes
) -> bool:
    result = subprocess.run(
        [
            sops_bin,
            "decrypt",
            "--filename-override",
            rel_target.as_posix(),
            "--input-type",
            "json",
            "--output-type",
            "binary",
            "/dev/stdin",
        ],
        cwd=secrets_root,
        input=encrypted,
        check=False,
        capture_output=True,
    )
    return result.returncode == 0


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

    inaccessible = verify_existing_access(args.sops_bin, secrets_root)
    if inaccessible is not None:
        print(
            "nixos-sops-ingest: this operator cannot decrypt existing SOPS file "
            f"{display_path(inaccessible)}; add your public recipient to .sops.yaml "
            "and ask an existing operator to run `just infra secrets updatekeys` "
            "before adding or seeding secrets",
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
    try:
        new_recipients = age_recipients_from_bytes(encrypted)
        mismatch = mismatched_recipient_file(
            secrets_root, rel_target, destination, new_recipients
        )
    except (OSError, UnicodeError, ValueError, json.JSONDecodeError) as error:
        print(
            f"nixos-sops-ingest: invalid SOPS recipient metadata: {error}",
            file=sys.stderr,
        )
        return 1
    if mismatch is not None:
        print(
            "nixos-sops-ingest: new recipient set differs from existing SOPS file "
            f"{display_path(mismatch)} in the same scope; run "
            "`just infra secrets updatekeys` after .sops.yaml changes, then retry",
            file=sys.stderr,
        )
        return 1
    if not verify_decryptable(args.sops_bin, secrets_root, rel_target, encrypted):
        print(
            "nixos-sops-ingest: encrypted file is not decryptable by this operator; "
            "add your public recipient to .sops.yaml and ask an existing operator to "
            "run `just infra secrets updatekeys` before ingesting",
            file=sys.stderr,
        )
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

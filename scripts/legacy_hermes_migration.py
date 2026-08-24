#!/usr/bin/env python3
"""Build, verify, and install a Finite legacy-Hermes migration bundle.

The target installer is deliberately offline. It imports conversation history
through the target Hermes version, preserves the complete source home in an
inert sealed archive, activates only compatible state, and hash-fences the
newly-created Finite identity and Chat client store.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Iterable
from pathlib import Path

from legacy_hermes_contract import (
    SOURCE_EXPORT_BATCH_SIZE as SOURCE_EXPORT_BATCH_SIZE,
    MigrationError as MigrationError,
    SourceMetadata as SourceMetadata,
    _memory_database_fact_count as _memory_database_fact_count,
    create_manifest as create_manifest,
    verify_bundle as verify_bundle,
)
from legacy_hermes_source import (
    check_source_writers as check_source_writers,
    export_source_sessions as export_source_sessions,
    inventory_source_integrations as inventory_source_integrations,
    inventory_source_sites as inventory_source_sites,
    inventory_source_volume as inventory_source_volume,
    snapshot_source_memory as snapshot_source_memory,
)
from legacy_hermes_target import install_bundle as install_bundle


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    export = subparsers.add_parser(
        "source-export", help="snapshot and export sessions through legacy Hermes"
    )
    export.add_argument("--output", type=Path, required=True)
    export.add_argument("--source-database", type=Path, required=True)

    memory = subparsers.add_parser(
        "source-memory-snapshot",
        help="snapshot the legacy structured memory database through SQLite",
    )
    memory.add_argument("--output", type=Path, required=True)
    memory.add_argument("--source-database", type=Path, required=True)

    inventory = subparsers.add_parser(
        "source-volume-inventory",
        help="classify every file on the legacy /home/node volume",
    )
    inventory.add_argument("--source-root", type=Path, required=True)
    inventory.add_argument("--output", type=Path, required=True)

    sites = subparsers.add_parser(
        "source-sites-inventory",
        help="bind authoritative legacy Sites records to preserved source paths",
    )
    sites.add_argument("--published-endpoints", type=Path, required=True)
    sites.add_argument("--source-volume-inventory", type=Path, required=True)
    sites.add_argument("--expected-machine-id", required=True)
    sites.add_argument("--output", type=Path, required=True)

    integrations = subparsers.add_parser(
        "source-integrations-inventory",
        help="record configured integrations and migration policy without secrets",
    )
    integrations.add_argument("--source-root", type=Path, required=True)
    integrations.add_argument("--source-volume-inventory", type=Path, required=True)
    integrations.add_argument("--output", type=Path, required=True)

    writers = subparsers.add_parser(
        "source-writer-check",
        help=(
            "prove no Linux process has a writable file descriptor or memory map "
            "below the frozen source PVC"
        ),
    )
    writers.add_argument("--source-root", type=Path, required=True)
    writers.add_argument("--proc-root", type=Path, default=Path("/proc"))

    manifest = subparsers.add_parser(
        "manifest", help="hash and seal complete preserved and active source state"
    )
    manifest.add_argument("--bundle", type=Path, required=True)
    manifest.add_argument("--source-host-id", required=True)
    manifest.add_argument("--source-machine-id", required=True)
    manifest.add_argument("--source-owner-email", required=True)
    manifest.add_argument("--source-hermes-version", required=True)
    manifest.add_argument("--source-image-reference", required=True)
    manifest.add_argument("--source-image-manifest-digest", required=True)
    manifest.add_argument("--source-container-image-id", required=True)
    manifest.add_argument("--source-volume-inventory-sha256", required=True)

    verify = subparsers.add_parser(
        "verify", help="verify a sealed bundle without mutation"
    )
    verify.add_argument("--bundle", type=Path, required=True)

    install = subparsers.add_parser(
        "install", help="install into one stopped target /data root"
    )
    install.add_argument("--bundle", type=Path, required=True)
    install.add_argument("--target-root", type=Path, required=True)
    install.add_argument("--expected-source-machine-id", required=True)
    install.add_argument("--expected-manifest-sha256", required=True)
    install.add_argument("--expected-target-identity-sha256", required=True)
    install.add_argument("--expected-target-chat-client-sha256", required=True)
    return parser


def main(argv: Iterable[str] | None = None) -> int:
    args = _parser().parse_args(list(argv) if argv is not None else None)
    try:
        if args.command == "source-export":
            result = export_source_sessions(args.output, args.source_database)
        elif args.command == "source-memory-snapshot":
            result = snapshot_source_memory(args.output, args.source_database)
        elif args.command == "source-volume-inventory":
            result = inventory_source_volume(args.output, args.source_root)
            blocked = result["classifications"]["blocked"]["entries"]
            if blocked:
                raise MigrationError(
                    f"source inventory has {blocked} structurally blocked entries; "
                    f"review {args.output}"
                )
        elif args.command == "source-sites-inventory":
            result = inventory_source_sites(
                args.output,
                args.published_endpoints,
                args.source_volume_inventory,
                expected_machine_id=args.expected_machine_id,
            )
        elif args.command == "source-integrations-inventory":
            result = inventory_source_integrations(
                args.output,
                args.source_root,
                args.source_volume_inventory,
            )
        elif args.command == "source-writer-check":
            result = check_source_writers(args.source_root, args.proc_root)
        elif args.command == "manifest":
            result = create_manifest(
                args.bundle,
                SourceMetadata(
                    host_id=args.source_host_id,
                    machine_id=args.source_machine_id,
                    owner_email=args.source_owner_email,
                    hermes_version=args.source_hermes_version,
                    image_reference=args.source_image_reference,
                    image_manifest_digest=args.source_image_manifest_digest,
                    container_image_id=args.source_container_image_id,
                    source_inventory_sha256=args.source_volume_inventory_sha256,
                ),
            )
        elif args.command == "verify":
            result = verify_bundle(args.bundle)
        else:
            result = install_bundle(
                args.bundle,
                args.target_root,
                expected_machine_id=args.expected_source_machine_id,
                expected_manifest_sha256=args.expected_manifest_sha256,
                expected_identity_sha256=args.expected_target_identity_sha256,
                expected_chat_client_sha256=args.expected_target_chat_client_sha256,
            )
    except MigrationError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

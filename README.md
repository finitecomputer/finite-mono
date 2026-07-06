# Finite Mono

`finite-mono` is the new Finite monorepo.

The first migration is intentionally conservative. The starting repos are copied
into top-level folders with their internal structure intact:

- `finitecomputer-v2`
- `finitechat`
- `finite-sites`

Do not reorganize dashboards, mobile code, deployment files, integrations, or
service wiring during the first copy. The initial goal is to make the copied
repos work in one checkout, then add shared workspace structure around them.

## Start Here

- [Monorepo plan](docs/monorepo-plan.md)
- [Migration log](docs/monorepo-migration-log.md)
- [Fedimint monorepo structure analysis](docs/fedimint-monorepo-structure-analysis.md)

## Current Root Tools

- `just --list`: show available root commands.
- `nix develop`: enter the pinned Rust development shell.
- `cargo metadata --no-deps`: verify the root Cargo workspace shape.

Root Cargo commands are intentionally minimal until the source repos are copied
and added as workspace members.


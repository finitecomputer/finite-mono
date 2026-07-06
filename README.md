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

- `just`: show available root commands and repo modules.
- `just metadata`: verify the root Cargo workspace shape.
- `just check`: check imported Rust workspace crates.
- `just fmt`: format Rust code across the root workspace.
- `just test`: test imported Rust workspace crates.
- `just sites build`: build the Finite Sites packages.
- `just sites test`: test the Finite Sites packages.
- `just sites lint`: run Finite Sites formatting and Clippy checks.
- `just dev-up`: start the initial local stack with `devfinity` and the
  process-compose TUI.
- `just dev-up --headless`: start the same local stack without the TUI.
- `just dev-cleanup`: best-effort cleanup for orphaned local stack processes.
- `nix develop`: enter the pinned Rust development shell.

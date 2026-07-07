# Finite Mono

`finite-mono` is the new Finite monorepo.

The first migration is intentionally conservative. Imported repos are copied
into top-level folders with their internal structure mostly intact:

- `finitecomputer-v2`
- `finitechat`
- `finite-sites`
- `finite-identity`
- `finite-nostr`
- `finite-auth`
- `finite-brain`
- `finite-search`
- `finite-skills`

Do not reorganize dashboards, mobile code, deployment files, integrations, or
service wiring during import. The goal is to make copied repos work in one
checkout, then add shared workspace structure around them.

## Start Here

- [Docs index](docs/README.md)
- [Monorepo plan](docs/monorepo-plan.md)
- [Migration log](docs/monorepo-migration-log.md)
- [Fedimint monorepo structure analysis](docs/fedimint-monorepo-structure-analysis.md)

## Current Root Tools

- `just`: show available root commands and repo modules.
- `just check`: check imported Rust workspace crates.
- `just fmt`: format Rust code across the root workspace.
- `just test`: test imported Rust workspace crates.
- `just sites build`: build the Finite Sites packages.
- `just sites test`: test the Finite Sites packages.
- `just sites lint`: run Finite Sites formatting and Clippy checks.
- `just search check`: run Finite Search static checks.
- `just skills check`: validate the shared Finite skills tree.
- `just dev up`: start the local stack with Rust-owned `devfinity`
  orchestration.
- `just dev up --headless`: start the same local stack without an interactive
  log viewer.
- `just dev smoke`: start the headless stack, run the integration smoke test,
  and tear it down.
- `just dev rust-smoke`: start the headless stack, run the ignored Rust
  integration smoke test, and tear it down.
- `just dev status`: print devfinity process and service status.
- `just dev cleanup`: best-effort cleanup for orphaned local stack processes.

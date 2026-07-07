# Finite Mono Agent Guide

Use the repository's current docs and local conventions as the source of truth
for implementation details. Keep changes scoped to the task and avoid broad
reorganization unless explicitly requested.

## Development Environment

- Dependencies and toolchains for this repository are managed by Nix. Do not
  install `cargo`, Rust, Node, Postgres, OpenSSL, or other repo dependencies on
  the user system to satisfy project commands.
- The recommended local workflow is Direnv loading the repo flake through
  `.envrc` (`use flake`). If the user is struggling to get the environment set
  up, direct them to install and enable Direnv, then run `direnv allow` at the
  repo root.
- Prefer root `just` commands for repo workflows; `just` recipes use
  `scripts/dev-shell` to enter the pinned development environment when needed.

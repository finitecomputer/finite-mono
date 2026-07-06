# Finite Mono Agent Guide

This repository is being built as the Finite monorepo. Follow
`docs/monorepo-plan.md` and check off completed steps as work lands.

Before implementing a monorepo component, check the corresponding Fedimint
pattern described in `docs/fedimint-monorepo-structure-analysis.md`.

## Current Migration Rules

- Keep `finitecomputer-v2`, `finitechat`, and `finite-sites` as top-level
  copied folders at first.
- Do not preserve source repo git history.
- Do not mutate the existing source repos while constructing `finite-mono`.
- Do not split apps, services, integrations, examples, or deployment files into
  new root folders during the initial copy.
- Keep the root `justfile` and `scripts/` minimal.
- Defer the local integration harness, CI, quality gates, and broad Nix build
  system until their phases in the plan.

## Editing

- Use `apply_patch` for manual file edits.
- Keep docs and config ASCII unless a file already requires otherwise.
- Update `docs/monorepo-migration-log.md` when recording migration facts.
- Update checkboxes in `docs/monorepo-plan.md` as phases progress.

## Development Environment

- Prefer root `just` commands for repo workflows; `just` recipes use
  `scripts/dev-shell` to enter the pinned development environment when needed.
- For direct commands that are not in a `justfile`, run them through
  `scripts/with-dev-env` unless `IN_NIX_SHELL` is already set.

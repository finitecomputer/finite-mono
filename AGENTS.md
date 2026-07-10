# Finite Mono Agent Guide

This is THE Finite company repository — all first-party code, apps, protocols,
and infrastructure definitions live here. `docs/monorepo-doctrine.md` is the
constitution; `docs/monorepo-plan.md` and `docs/monorepo-migration-log.md`
record how we got here.

Before implementing a monorepo-level component, check the corresponding
Fedimint pattern described in `docs/fedimint-monorepo-structure-analysis.md`.

## Ground rules

- **This repo is public.** Never commit a secret value, token, or key — not
  in code, config, tests, or docs. Secrets are documented by NAME and
  location only (see `infra/README.md`). If one slips in: rotate first, then
  remove.
- **Work lands here first.** The old per-component repos are archived (or
  awaiting archive); never "sync back." A stray commit on an unarchived
  source repo is merged in with `scripts/import-sync <name>`.
- **Releases are component-scoped tags**: `finitechat/vX.Y.Z`, `fsite/vX.Y.Z`,
  `fbrain/vX.Y.Z`; images version via workflow dispatch. Release asset names
  are product contracts — never rename them. Installers use the per-component
  rolling alias releases (`finitechat-latest` etc.), refreshed by the release
  workflows — this repo is the ONLY release host (doctrine §4).
- **Deploys are defined in `infra/`** — per-host trees, CI-built digest-pinned
  images, runbooks. Nothing is built on a prod box.
- **User data availability is the first security invariant.** Follow
  `docs/adr/0001-recoverability-precedes-operator-blindness.md`: do not remove a
  Recovery Authority, couple compute teardown to data purge, or claim stronger
  operator-blindness until the same Recovery Set has restored onto an empty
  target. A TEE and a Provider Durable Volume are not backups.
- One root Cargo workspace, one root `Cargo.lock`. Imported components keep
  their internal layout; their crates are root workspace members and their
  old sub-workspace `Cargo.toml`/`Cargo.lock` files stay deleted. New crates
  get added to the root members list.
- Update `docs/monorepo-migration-log.md` when recording migration facts.

## Development Environment

- Dependencies and toolchains are managed by Nix. Do not install `cargo`,
  Rust, Node, Postgres, OpenSSL, or other repo dependencies on the user
  system to satisfy project commands.
- Recommended local workflow: Direnv loads the repo flake via `.envrc`
  (`use flake`); run `direnv allow` at the repo root.
- Prefer root `just` commands; recipes enter the pinned dev environment via
  `scripts/dev-shell`. For direct commands not in a justfile, use
  `scripts/with-dev-env` unless `IN_NIX_SHELL` is already set.
- `just dev up` boots the full local stack (devfinity); `just dev smoke` is
  the integration gate CI runs. Keep it green.

## CI and quality gates

`.github/workflows/ci.yml` runs on every PR: rustfmt, clippy (`-D warnings`),
`cargo test --workspace --locked` against real Postgres, dashboard
lint/test/build, the finitechat Hermes bridge suite, and skills/search static
checks. Release and image workflows are described in `infra/images/README.md`
and the workflow files themselves.

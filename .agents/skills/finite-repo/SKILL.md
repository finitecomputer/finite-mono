---
name: finite-repo
description: Set up Finite monorepo development tools, choose local and CI checks, or work on workspace membership, imports, releases, and agent documentation routing.
---

# Finite repository workflow

## Development and checks

Dependencies and toolchains come from the root Nix flake. Do not install repo
dependencies on the host to satisfy project commands. Prefer root `just`
recipes, which enter the pinned environment through `scripts/dev-shell`.
For direct commands use `scripts/with-dev-env` unless `IN_NIX_SHELL` is set.

Direnv loads `.envrc` (`use flake`); run `direnv allow` at the worktree root.
If environment reloads are slow, configure nix-direnv so evaluations are
cached: install `nixpkgs#nix-direnv` in the Nix profile, then source
`$HOME/.nix-profile/share/nix-direnv/direnvrc` from
`~/.config/direnv/direnvrc`. Plain direnv re-runs `nix print-dev-env` on every
evaluation.

`just dev up` starts devfinity; `just dev smoke` is the services integration
gate. `.github/workflows/ci.yml` is the authority for current CI selection and
commands: rustfmt, clippy (`-D warnings`), real-Postgres Rust tests, dashboard
lint/tests/build, Hermes bridge tests, and skills/search checks. Use devfinity
to run Rust tests against isolated infrastructure (`just test`, or its scoped
`cargo run --locked -p devfinity -- run -- cargo test ...` equivalent).

`just source-structure-check` checks the cleaned Core scope and short agent
guides. See `docs/agents/source-structure.md` for limits and expansion rules.

## Workspace and release ownership

One root Cargo workspace and one root `Cargo.lock`; add new crates to the
root members. Imported components retain their internal layout without nested
workspace manifests or lockfiles. Depend on sibling crates rather than copying
their implementations.

All first-party work lands here. Old component repositories are provenance;
never sync back. Import stray source-repository commits with
`scripts/import-sync <name>`. See `docs/monorepo-doctrine.md`.

Releases use component tags such as `finitechat/vX.Y.Z`, `fsite/vX.Y.Z`, and
`fbrain/vX.Y.Z`; image versions use workflow dispatch. Release asset names are
contracts; never rename them. Public downloads live in `finitecomputer/finite-releases` under
component rolling aliases such as `finitechat-latest`, refreshed by release
workflows. Consult `infra/images/README.md` and the owning workflow before
changing release behavior. Images are built in CI and pinned by digest.

## Finding context

`docs/agents/issue-tracker.md` defines product authority and issue conventions;
`docs/agents/triage-labels.md` defines triage labels. Use
`docs/agents/domain.md` for retained contracts and `CONTEXT-MAP.md` for owners.
Prune stale legacy documents and scripts after checking current callers,
production boundaries, migration gates, and test contracts; history is the
archive.

For organization Brain knowledge outside git, use the `orgbrain` skill and
`fbrain` to open, sync, and search before concluding it is missing. Typical
reads: `fbrain doctor`, `fbrain brain list`, `fbrain open ...`,
`fbrain sync now --summary`, `fbrain conflicts --json`. Non-read-only Brain
work follows `finite-skills/skills/software-development/finitebrain/SKILL.md`.

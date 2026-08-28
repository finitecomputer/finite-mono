# Finite Mono Agent Guide

This is THE Finite company repository — all first-party code, apps, protocols,
and infrastructure definitions live here. `docs/monorepo-doctrine.md` is the
constitution; `docs/monorepo-plan.md` and `docs/monorepo-migration-log.md`
record how we got here.

Before implementing a monorepo-level component, check the corresponding
Fedimint pattern described in `docs/fedimint-monorepo-structure-analysis.md`.

## Ground rules

- **Don't Break Chat!** Chat availability and durable history are the primary
  product promise. Changes to persisted chat state, protocols, Device identity,
  Agent Runtime state, or deployment topology must trace the production
  through-line, name every writer and reader, and prove the relevant
  existing-state and mixed-version edges. An all-candidate test is not
  compatibility proof. Prefer fewer authoritative paths; reject features whose
  compatibility and recovery contracts cannot be made clear and affordable.
- **Don't break new-user onboarding.** Distinguish account enrollment, Agent
  admission, launch, identity readiness, and chat readiness, then prove the
  end-to-end promise. Runner drain and capacity are product availability state,
  not merely operator configuration.
- **Never commit secrets.** Do not commit a secret value, token, or key — not
  in code, config, tests, or docs. Secrets are documented by NAME and location
  only (see `infra/README.md`). If one slips in: rotate first, then remove.
- **Work lands here first.** The old per-component repos are archived (or
  awaiting archive); never "sync back." A stray commit on an unarchived
  source repo is merged in with `scripts/import-sync <name>`.
- **Releases are component-scoped tags**: `finitechat/vX.Y.Z`, `fsite/vX.Y.Z`,
  `fbrain/vX.Y.Z`; images version via workflow dispatch. Release asset names
  are product contracts — never rename them. Source tags live in this repo;
  public downloads live in `finitecomputer/finite-releases` under the
  per-component rolling alias releases (`finitechat-latest` etc.), refreshed
  by the release workflows (doctrine §4).
- **Deploys are defined in `infra/`** — per-host trees, CI-built digest-pinned
  images, runbooks. Nothing is built on a prod box.
- **Services own their public route surface in code; the edge proxies, never
  filters.** A service exposes exactly one public router (e.g. Finite
  Identity's `public_router`) bound to a dedicated listener; Caddy
  reverse-proxies that listener verbatim and never keeps a per-route
  allowlist, so CLI/server contract skew at the edge is impossible by
  construction.
- **User data availability is the first security invariant.** Follow
  `docs/adr/0001-recoverability-precedes-operator-blindness.md`: do not remove a
  Recovery Authority, couple compute teardown to data purge, or claim stronger
  operator-blindness until the same Recovery Set has restored onto an empty
  target. A TEE and a Provider Durable Volume are not backups.
- **Production repair is never speculative.** Before proposing a migration or
  repair, gather read-only evidence, reproduce the failure, prove the change on
  synthetic state, and name the backup and rollback boundary. Production
  mutation requires explicit user authorization. A selected row, sort order,
  identifier order, or other navigation state never confers authority to choose
  or rewrite durable user state; ambiguous state fails closed without mutation.
- **`scripts/finite-status` is the only platform/fleet status command.** Run it
  before and after every rollout; add any missing incident probe to that
  read-only command instead of preserving an ad-hoc operator query.
- Never open a snapshot SQLite file directly; inspect it only through
  `scripts/snapshot-sqlite` or from a scratch copy.
- One root Cargo workspace, one root `Cargo.lock`. Imported components keep
  their internal layout; their crates are root workspace members and their
  old sub-workspace `Cargo.toml`/`Cargo.lock` files stay deleted. New crates
  get added to the root members list.

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

## Agent skills

- **Issue tracker:** GitHub. See `docs/agents/issue-tracker.md`.
- **Triage labels:** Canonical Matt Pocock skill labels. See
  `docs/agents/triage-labels.md`.
- **Domain documentation:** Multi-context monorepo guidance. See
  `docs/agents/domain.md`.
- **Organization Brain:** Finite has an organization FiniteBrain instance that
  holds wiki/postmortem/runbook knowledge outside the git tree. `fbrain` is the
  CLI control plane that materializes FiniteBrain content into a local plaintext
  Working Tree; when the user refers to "the Brain", org wiki pages,
  postmortems, runbooks, or Brain paths that are not in git, use `fbrain` to
  open, sync, and search that content before assuming it is missing. Typical
  read flow: `fbrain doctor`, `fbrain brain list`, `fbrain open ...`,
  `fbrain sync now --summary`, `fbrain conflicts --json`. For non-read-only
  Brain work, follow
  `finite-skills/skills/software-development/finitebrain/SKILL.md`.

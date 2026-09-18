# Finite Mono Agent Guide

All first-party work lands here. `docs/monorepo-doctrine.md` is the constitution.
Old component repositories are provenance; never sync changes back.

## Always

- **Don't Break Chat or onboarding.** Preserve availability, durable history,
  and the enrollment → admission → launch → identity → usable-chat promise.
- **Never commit secrets.** Document names and locations only; rotate first if
  a value leaks. See `infra/README.md`.
- **Preserve recoverability.** Compute teardown never implies user-data purge.
  A TEE and a Provider Durable Volume are not backups.
- **Production mutation requires explicit user authorization.** Reproduce and
  prove repairs on synthetic state first; identify backup and rollback bounds.
  Selection or ordering never authorizes rewriting durable user state.
- **Use Nix-managed dependencies.** Prefer root `just` recipes; use
  `scripts/with-dev-env` for direct commands outside `IN_NIX_SHELL`. Do not
  install repo dependencies on the host. One root Cargo workspace/lockfile.
- Prune stale material once no current caller, contract, or migration gate
  depends on it. Git history is the archive.

## Load only the guidance relevant to the task

- Persisted state, protocols, Device identity, Agent Runtime lifecycle,
  onboarding, public HTTP routes/edge routing, deployment topology, or repair: read
  [.agents/skills/finite-safety/SKILL.md](.agents/skills/finite-safety/SKILL.md).
- Development setup, checks, workspace/import/release work, or documentation
  routing: read [.agents/skills/finite-repo/SKILL.md](.agents/skills/finite-repo/SKILL.md).
- Component changes: follow the component's `AGENTS.md`; consult retained
  contracts as directed by `docs/agents/domain.md`.
- Product plans and outstanding work: follow `docs/agents/issue-tracker.md`;
  triage labels are in `docs/agents/triage-labels.md`.
- Organization Brain or knowledge outside git: use the `orgbrain` skill and
  `fbrain` to open/sync/search before assuming it is missing. Writes follow
  `finite-skills/skills/software-development/finitebrain/SKILL.md`.

Keep these instructions short. Move conditional detail into project skills.
`just source-structure-check` enforces guide and cleaned Core source limits;
see `docs/agents/source-structure.md`.

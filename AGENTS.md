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

## Load only the guidance relevant to the task

- Persisted state, protocols, Device identity, Agent Runtime lifecycle,
  onboarding, public HTTP routes/edge routing, deployment topology, or repair: read
  [.agents/skills/finite-safety/SKILL.md](.agents/skills/finite-safety/SKILL.md).
- Development setup, checks, workspace/import/release work, or documentation
  routing: read [.agents/skills/finite-repo/SKILL.md](.agents/skills/finite-repo/SKILL.md).
- Component changes: follow the component's `AGENTS.md` and consult retained
  contracts for affected compatibility, security and recovery boundaries.
- Organization Brain or knowledge outside git: use the `orgbrain` skill and
  `fbrain` to open/sync/search before assuming it is missing. Writes follow
  `finite-skills/skills/software-development/finitebrain/SKILL.md`.

Keep these instructions short. Move conditional detail into project skills.
`just source-structure-check` enforces guide and cleaned Core source limits;
see `docs/agents/source-structure.md`.

## GitHub and Linear

- **GitHub owns implementation:** code, tests, executable configuration, PRs and
  releases. Keep engineering principles in `AGENTS.md`; retain only documentation
  needed to develop, operate or safely change the implementation. Explain rationale
  and constraints that code cannot express; avoid catalogs of facts code already owns.
- **[Linear](https://linear.app/finitecomputer) owns planned work:** problems, outcomes, acceptance criteria,
  priorities, decisions and unresolved questions. Keep shared context, terminology
  and context maps there when needed; link to code and PR evidence instead of copies.
- Start with the relevant issue, comments and owning code. Update an existing issue
  when it covers the work. Create a ticket for a meaningful outcome, blocker or
  follow-up that needs tracking; keep implementation steps in the PR or working session.
- Keep tickets concise: problem, outcome, constraints, evidence of completion and
  only the dependencies or decisions needed to proceed. Add a document only when
  multiple issues need the same explanation. Reuse existing team statuses and labels.
- Humans own priorities, scope and acceptance. Agents investigate, propose, implement
  and keep the issue current within the requested scope. Distinguish proposals from
  accepted decisions; ask about unresolved choices that change the outcome.
- PRs explain changes and validation; link the Linear issue when one exists. Read
  historical GitHub issues when referenced. Resolve contract conflicts in Linear.
- Prune stale or duplicate docs; Git history is the archive. Preserve live contracts
  and human-authored guidance until consolidated or a linked replacement is verified.

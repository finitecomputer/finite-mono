# Finite Mono Agent Guide

All first-party work lands here. [Monorepo doctrine](docs/monorepo-doctrine.md) is
the constitution; old component repositories are provenance, never sync targets.

## Engineering principles

- **Don't Break Chat or onboarding.** Preserve availability, durable history,
  and enrollment → admission → launch → identity → usable chat. For persisted
  state, protocols, identity, runtime lifecycle or topology changes, name every
  writer and reader and prove existing-state and mixed-version compatibility.
  Prefer fewer authoritative paths and production-faithful proofs.
- **Preserve recoverability.** Follow the [recovery invariant](docs/adr/0001-recoverability-precedes-operator-blindness.md).
  Compute teardown retains recovery material; data purge needs separate authority.
  A TEE or provider volume is not a backup. Prove restoration onto an empty target.
- **Production mutation requires explicit user authorization.** Gather read-only
  evidence, reproduce and prove repairs on synthetic state, and name backup and
  rollback bounds. Selection or ordering never authorizes rewriting durable state;
  ambiguity fails closed. Use `scripts/finite-status` before and after rollouts;
  add missing probes there. Inspect snapshot SQLite through `scripts/snapshot-sqlite`
  or a scratch copy. Deployment definitions and runbooks live in `infra/`.
- **Never commit secrets.** Document names and locations only; rotate first if
  a value leaks. See `infra/README.md`.
- **Services own public routes in code.** Expose one public router on a dedicated
  listener; the edge proxies it verbatim rather than maintaining a route allowlist.

## Working in the repository

- Follow the owning component's `AGENTS.md`. For setup and checks, read
  [CONTRIBUTING.md](CONTRIBUTING.md); CI commands live in `.github/workflows/ci.yml`.
- Use Nix-managed dependencies, root `just` recipes and `scripts/with-dev-env`
  for direct commands outside `IN_NIX_SHELL`. Keep one root Cargo workspace and
  lockfile; depend on sibling crates instead of copying them.
- Put component rules in that component's `AGENTS.md`; consult existing contracts
  for compatibility, security or recovery. Run `just source-structure-check`; see [its scope](docs/agents/source-structure.md).

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
- For org Brain knowledge, use `fbrain` to open/sync/search; writes follow
  `finite-skills/skills/software-development/finitebrain/SKILL.md`.

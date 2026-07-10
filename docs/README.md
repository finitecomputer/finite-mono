# Finite Monorepo Docs

This folder is the root documentation entry point for `finite-mono`.

Docs here are split into two groups:

- Current monorepo docs: maintained as part of the migration and local
  developer loop.
- Imported orientation docs: copied from `finite-eng-docs` for continuity, but
  not fully revalidated after the monorepo import.

When in doubt, prefer the current monorepo docs and verify imported operational
commands against the owning source folder before relying on them.

## Current Monorepo Docs

- [Monorepo plan](monorepo-plan.md): phased construction checklist.
- [Migration log](monorepo-migration-log.md): source snapshots, validation
  notes, and migration decisions.
- [Local integration harness](local-integration-harness.md): `devfinity`,
  `process-compose`, and `just dev` usage.
- [Devfinity architecture plan](devfinity-architecture-plan.md): plan for
  evolving `devfinity` into a typed integration harness.
- [Fedimint monorepo structure analysis](fedimint-monorepo-structure-analysis.md):
  reference analysis used to calibrate Finite's Rust, Nix, command, docs,
  harness, CI, and quality-gate choices.
- [Recoverability precedes operator-blindness](adr/0001-recoverability-precedes-operator-blindness.md):
  system security decision governing recovery, privacy claims, TEEs, and
  Break-Glass Recovery.
- [Managed skills are hot-swappable product revisions](adr/0002-managed-skills-are-hot-swappable-product-revisions.md):
  one editable skills source, immutable promotion, first-turn availability,
  event-driven activation, and rollback without a Runtime reboot.
- [`finite-agentd` is the agent-owned platform boundary](adr/0003-agentd-is-the-agent-owned-platform-boundary.md):
  typed agent-local commands and supervision over Finite Chat without widening
  Runner or the outbound-only Runtime Management Pipe.

## Imported Orientation Docs

These were copied from `finite-eng-docs` during Phase 7. They are useful as a
starting point, but they still contain pre-monorepo assumptions and references
to repos that are not yet imported into `finite-mono`.

- [Architecture overview](architecture-overview.md)
- [System flow and trust boundaries](system-flow-and-trust-boundaries.md)
- [Navigation plan](navigation-plan.md)
- [Local development matrix](local-dev-matrix.md)
- [Slop audit](slop-audit.md)

## Repo-Local Docs

Docs copied with each source repo remain inside their owning folders for now:

- [`finitecomputer-v2/docs`](../finitecomputer-v2/docs)
- [`finitechat/docs`](../finitechat/docs)
- [`finite-sites/docs`](../finite-sites/docs)
- [`finite-nostr/docs`](../finite-nostr/docs)
- [`finite-brain/docs`](../finite-brain/docs)
- [`finite-search/docs`](../finite-search/docs)
- [`finite-skills/skills`](../finite-skills/skills)
- [`finite-skills/docs`](../finite-skills/docs)
- [`finite-specialization/docs`](../finite-specialization/docs)

Some imported repos also have root-level source repo docs:

- [`finite-identity/README.md`](../finite-identity/README.md)
- [`finite-identity/SPEC.md`](../finite-identity/SPEC.md)
- [`finite-identity/CLI-CONVENTIONS.md`](../finite-identity/CLI-CONVENTIONS.md)
- [`finite-nostr/README.md`](../finite-nostr/README.md)
- [`finite-brain/README.md`](../finite-brain/README.md)
- [`finite-brain/development.md`](../finite-brain/development.md)
- [`finite-search/README.md`](../finite-search/README.md)
- [`finite-skills/README.md`](../finite-skills/README.md)
- [`finite-specialization/README.md`](../finite-specialization/README.md)

Treat repo-local docs as owner-scoped background until the Phase 13 stale-docs
audit promotes, rewrites, or deletes them.

## Docs Rules

- Keep durable monorepo orientation in this folder.
- Keep implementation details with the owning source folder until they are
  stable enough to promote.
- Mark imported or unreviewed docs before linking them as canonical.
- Update `monorepo-migration-log.md` when migration phases change docs layout
  or authority.

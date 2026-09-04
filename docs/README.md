# Finite Monorepo Docs

This folder is the root documentation entry point for `finite-mono`.

Docs here are current monorepo guidance, durable decisions, run records,
postmortems, and audits. Imported orientation docs from `finite-eng-docs` were
removed during the 2026-08-29 cleanup so agents start from current root docs,
component `CONTEXT.md`, ADRs, and runbooks instead of stale navigation layers.

## Current Monorepo Docs

- [Local integration harness](local-integration-harness.md): `devfinity`,
  `process-compose`, and `just dev` usage.
- [LAT logs and host metrics plan](lat-logs-and-host-metrics-plan.md): narrow
  Grafana/Loki/Alloy plan for centralized LAT service logs and basic host
  performance metrics.
- [Recoverability precedes operator-blindness](adr/0001-recoverability-precedes-operator-blindness.md):
  system security decision governing recovery, privacy claims, TEEs, and
  Break-Glass Recovery.
- [Managed skills are hot-swappable product revisions](adr/0002-managed-skills-are-hot-swappable-product-revisions.md):
  one editable skills source, immutable promotion, first-turn availability,
  event-driven activation, and rollback without a Runtime reboot.
- [`finite-agentd` is the agent-owned platform boundary](adr/0003-agentd-is-the-agent-owned-platform-boundary.md):
  typed agent-local commands and supervision over Finite Chat without widening
  Runner or the outbound-only Runtime Management Pipe.
- [Products own bounded identity adapters](adr/0004-products-own-bounded-identity-adapters.md):
  product-specific identity intents over shared key primitives without a
  generic signer authority.
- [finite-lat host roles and safe initial placement](adr/0005-finite-lat-host-roles-and-placement.md):
  lat1 control/existing Agents, lat2 excluded from Agent capacity, lat3 initial
  new-Agent capacity, and fail-closed provider-neutral placement. Docker/image
  CI and the staged lat1 NixOS closure build path use Depot-backed CI.
- [Current deployed infrastructure](../infra/README.md): exact observed fleet
  roles and the boundary between executable configuration and dated captures.
- [finite-lat capacity, redundancy, and admission](runs/finite-lat-capacity-and-redundancy.md):
  the one proposed next candidate, evidence gates, and explicit non-goals.
- [Production baseline — Sites and Agent Runtime rollout](runs/production-baseline-2026-07-15.md):
  the first-cohort known-good production checkpoint, accepted deploy/rollout
  behavior, regression gates, and the separately proposed recovery run.
- [Boss Hosted Chat recovery post-mortem](postmortems/boss-hosted-chat-recovery-2026-07-16.md):
  the legacy binding compatibility failure, ineffective first hotfix,
  misleading test fixtures, and dashboard deploy improvements.
- [Agent Runtime upgrade and rollout post-mortem](postmortems/agent-runtime-upgrade-rollout-2026-07-16.md):
  why upgrades and deploys risked stranding Agents, which guardrails now exist,
  and the prioritized build, rollout, and recovery work still required.
- [Artifact identity and manual drift audit](audits/artifact-identity-and-drift-2026-08-02.md):
  automatic package fingerprints, confirmed compatibility-record drift, and
  the boundary between intentional pins and redundant release bookkeeping.
- [Script surface audit](audits/script-surface-audit-2026-08-29.md):
  inventory of scripts, command facades, workflows, safety patterns, drift
  risks, and recommended script hardening passes.

## Repo-Local Docs

Docs copied with each source repo remain inside their owning folders for now:

- [`finitecomputer-v2/docs`](../finitecomputer-v2/docs)
- [`finitechat/docs`](../finitechat/docs)
- [`finite-sites/docs`](../finite-sites/docs)
- [`finite-nostr/docs`](../finite-nostr/docs)
- [`finite-brain/docs`](../finite-brain/docs)
- [`finite-skills/skills`](../finite-skills/skills)
- [`finite-skills/docs`](../finite-skills/docs)

Some imported repos also have root-level source repo docs:

- [`finite-identity/README.md`](../finite-identity/README.md)
- [`finite-identity/SPEC.md`](../finite-identity/SPEC.md)
- [`finite-identity/CLI-CONVENTIONS.md`](../finite-identity/CLI-CONVENTIONS.md)
- [`finite-nostr/README.md`](../finite-nostr/README.md)
- [`finite-brain/README.md`](../finite-brain/README.md)
- [`finite-brain/development.md`](../finite-brain/development.md)
- [`finite-skills/README.md`](../finite-skills/README.md)

Treat repo-local docs as owner-scoped background. Prefer current root docs,
component `CONTEXT.md`, ADRs, and runbooks over historical plans or imported
orientation.

## Docs Rules

- Keep durable monorepo orientation in this folder.
- Keep implementation details with the owning source folder until they are
  stable enough to promote.
- Mark imported or unreviewed docs before linking them as canonical.
- Delete stale caches instead of preserving extra navigation layers; git
  history is the archive.

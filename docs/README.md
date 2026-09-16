# Finite Monorepo Docs

This folder is the root documentation entry point for `finite-mono`.

Product planning and decisions live in [Linear](https://linear.app/finitecomputer).
Code, tests, and executable configuration establish implemented behavior. This
folder retains operational guidance, contracts, and historical material pending
review; its presence does not establish current product scope.

## Retained Monorepo Docs

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

Use retained docs for relevant context and operational contracts. Reconcile
conflicts with the Linear issue and implementation explicitly; dated plans and
status reports are not current deployment evidence.

## Docs Rules

- Follow the [root agent guide](../AGENTS.md) for product authority and the
  [retained-documentation guidance](agents/domain.md) when changing a component.
- Keep operational instructions, executable documentation inputs, and recovery
  contracts until their replacements are verified.
- Delete stale caches instead of preserving extra navigation layers; git
  history is the archive.

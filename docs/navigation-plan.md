# Navigation Plan

> Status: imported from `finite-eng-docs` during Phase 7 on 2026-07-06. This
> document has not been fully revalidated after the monorepo import. Treat it as
> orientation background, not an authoritative current runbook.

This doc is the entry plan for finding the right repo and the right level of
detail. It should stay short enough to scan before making a change.

## If You Are New

1. Read `README.md` in this folder.
2. Read [Architecture overview](architecture-overview.md).
3. Read [System flow and trust boundaries](system-flow-and-trust-boundaries.md)
   when you need the end-to-end user flow, data classification, key custody,
   or encryption/security boundary model.
4. Read [Local development matrix](local-dev-matrix.md) to choose the right
   setup and validation loop for your task.
5. Read [Slop audit](slop-audit.md) for a candid first-pass risk map.
6. Read the relevant repo docs for the layer you are touching.
7. Before changing behavior across component boundaries, write down the
   affected folders and the expected local or deployment validation path.

## By Task

| Task | Start here | Why |
| --- | --- | --- |
| Understand the end-to-end user flow, front ends, security boundaries, encryption boundaries, and key custody | `system-flow-and-trust-boundaries.md` | Conversation-level map of dashboard, native chat, runtime, Core, runner, Finite Chat, FiniteBrain, Sites, Finite Private, search, TEEs, and encrypted storage |
| Choose a local development setup | `local-dev-matrix.md` | Monorepo component map of prerequisites, first commands, checks, friction, and unification direction |
| Understand the self-serve hosted-agent product | `../finitecomputer-v2/README.md`, `../finitecomputer-v2/CONTEXT.md`, and `../finitecomputer-v2/docs/finite-stack-deployment.md` | v2 is the hard-cut SaaS product: WorkOS, Projects, Core, Runner, Finite Private, native Finite Chat invite, and runtime launch |
| Change v2 dashboard/Core UI | `../finitecomputer-v2/apps/dashboard/README.md` and `../finitecomputer-v2/docs/carry-over-manifest.md` | The v2 dashboard lives in `finitecomputer-v2`; dashboard chat and dashboard-managed publishing are out of scope |
| Prove v2 Agent Runtime behavior | `../finitecomputer-v2/docs/hermes-runtime-test-matrix.md` | Runtime proof climbs local real-Hermes, local Docker, remote Docker, Phala, then dashboard-controlled SaaS launch |
| Change v2 deploy boundaries or hosted Finite Chat deploy mechanics | `../finitecomputer-v2/docs/finite-stack-deployment.md` and `../finitecomputer-v2/docs/service-dependencies.md` | v2 owns SaaS deploy coordination while service source remains in the owning repos |
| Work on legacy dashboard relay/chat or box1/TRF operations | `../../finitecomputer/docs/README.md` and `../../finitecomputer/docs/chat-local-dev.md` | Legacy `finitecomputer` remains the shipped whiteglove product until users migrate |
| Work on encrypted chat protocol or native clients | `../finitechat/docs/architecture.md` and `../finitechat/README.md` | `finitechat` owns protocol, server, iOS, and Rust client state |
| Work on Hermes chat bridge behavior | `../finitechat/integrations/hermes/README.md` | Bridge code lives with `finitechat`; SaaS deployment handoff crosses into `finitecomputer-v2` |
| Work on encrypted knowledge, Vault Working Trees, or the Product Client | `../../finite-brain/README.md`, `../../finite-brain/development.md`, and `../../finite-brain/docs/specs/finitebrain-portability-spec.md` | `finite-brain` owns Vaults, Folders, `fbrain`, Product Client policy, access, sync, and asset/source-note behavior |
| Add or change shared Hermes skills | `../../finite-skills/README.md` and `../../finite-skills/skills/AGENTS.md` | Shared managed skill source of truth |
| Work on web search/extract tools | `../../finite-search/README.md` | Search/extract ops and smokes live there |
| Use reusable Nostr primitives | `../../finite-nostr/README.md` | Product-neutral Nostr helpers only |
| Generate or inspect platform reports | `../../reporting/README.md` | Reporting outputs and notes live there |
| Assess repo quality and shipping risk | `slop-audit.md` | First-pass map of jank, good choices, placeholders, and cleanup priorities |

## By Environment

| Environment | Use for | Avoid using for |
| --- | --- | --- |
| `finitecomputer-v2` dashboard/Core dev | WorkOS/SaaS dashboard surfaces, Project/Core data UI, Finite Private admin/status pages | Dashboard chat, legacy machine operations, proof that a real hosted runtime works |
| `finitecomputer-v2` Hermes runtime matrix | Real runtime image proof, local/remote Docker promotion, Phala canary, dashboard-controlled SaaS launch readiness | Fast UI-only iteration |
| `finitecomputer` chat-local harness | Legacy dashboard chat iteration, relay behavior, runtime connector checks, local Hermes/plugin smoke | v2 product acceptance, full production host behavior, Google OAuth, k3s/TLS/DNS assumptions |
| `finitechat` local server/simulator | Protocol, server, iOS, Rust client, native chat behavior | Hosted dashboard runtime provisioning |
| `finitechat` phone/Docker canaries | Promotion evidence for real Hermes chat behavior | Fast inner-loop UI development |
| `finite-brain` local server/Product Client | Vault, Folder, access, sync, Product Client, Smoke UI, and `fbrain` CLI work | Product-neutral Nostr helpers, hosted SaaS runtime provisioning |
| `finite-search` SSH tunnel smokes | Proving deployed search/extract service behavior | Local product UI iteration |
| Hosted production/staging boxes | Deployment, OAuth, k3s, backups, real runtime rollouts | First-pass code iteration |

## Cross-Component Change Checklist

Before making a change that crosses component boundaries, answer:

- Which folder or component owns the behavior?
- Which folder or component deploys or consumes it?
- What is the lowest-cost local validation path?
- What production or canary validation is required before users depend on it?
- Which doc should be updated after the behavior changes?

## TODO Docs

Create these only when the relevant question becomes active. The broad
trust-boundary view now lives in
[System flow and trust boundaries](system-flow-and-trust-boundaries.md), so
the TODOs below should be deeper drill-downs, not replacements for that map.

- `runtime-and-state.md`: detailed durable map of git-owned, host-owned,
  runtime-owned, and reporting-owned state.
- `chat-stack-v2-and-legacy.md`: v2 native Finite Chat invite path versus
  legacy dashboard relay chat, with product defaults and bridge delete
  conditions.
- `deployment-and-environments.md`: boxes, local harnesses, canaries, and
  promotion gates.
- `cross-component-change-flows.md`: examples of changes that touch multiple
  components.

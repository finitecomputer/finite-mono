# Navigation Plan

> Status: imported from `finite-eng-docs` during Phase 7 on 2026-07-06. Local
> onboarding pointers were revalidated on 2026-07-11; the remaining document is
> orientation background, not an authoritative current runbook.

This doc is the entry plan for finding the right repo and the right level of
detail. It should stay short enough to scan before making a change.

## If You Are New

1. Read the root [`CONTRIBUTING.md`](../CONTRIBUTING.md) and choose the web-design
   or complete local SaaS loop.
2. Read `README.md` in this folder.
3. Read [Architecture overview](architecture-overview.md).
4. Read [System flow and trust boundaries](system-flow-and-trust-boundaries.md)
   when you need the end-to-end user flow, data classification, key custody,
   or encryption/security boundary model.
5. Read [Local development matrix](local-dev-matrix.md) to choose the right
   setup and validation loop for your task.
6. Read [Slop audit](slop-audit.md) for a candid first-pass risk map.
7. Read the relevant repo docs for the layer you are touching.
8. Before changing behavior across component boundaries, write down the
   affected folders and the expected local or deployment validation path.

## By Task

| Task | Start here | Why |
| --- | --- | --- |
| Understand the end-to-end user flow, front ends, security boundaries, encryption boundaries, and key custody | `system-flow-and-trust-boundaries.md` | Conversation-level map of dashboard, native chat, runtime, Core, runner, Finite Chat, FiniteBrain, Sites, Finite Private, search, TEEs, and encrypted storage |
| Choose a local development setup | root `CONTRIBUTING.md`, then `local-dev-matrix.md` | Current copy-paste web/full-stack entrypoints, followed by the broader component map |
| Understand the self-serve hosted-agent product | `../finitecomputer-v2/README.md`, `../finitecomputer-v2/CONTEXT.md`, and `../finitecomputer-v2/docs/finite-stack-deployment.md` | v2 is the hard-cut SaaS product: WorkOS, Projects, Core, generic Runner contract, Hosted Web Device chat, narrow runtime management, and durable runtime launch |
| Change v2 dashboard/Core UI | `../finitecomputer-v2/apps/dashboard/README.md` and `../finitecomputer-v2/docs/carry-over-manifest.md` | The v2 dashboard owns signup/launch, the canonical BoxOne-derived web chat, typed connections, Site/Brain views, and lifecycle UX; product data paths stay in their owning services |
| Prove v2 Agent Runtime behavior | `../finitecomputer-v2/docs/hermes-runtime-test-matrix.md` | Runtime proof climbs the resident streaming Hermes sidecar, local full-product Docker, Kata, Phala, then full SaaS product acceptance |
| Change v2 deploy boundaries or hosted Finite Chat deploy mechanics | `../finitecomputer-v2/docs/finite-stack-deployment.md` and `../finitecomputer-v2/docs/service-dependencies.md` | v2 owns SaaS deploy coordination while service source remains in the owning repos |
| Work on legacy dashboard relay/chat or box1/TRF operations | `../../finitecomputer/docs/README.md` and `../../finitecomputer/docs/chat-local-dev.md` | Legacy `finitecomputer` remains the shipped whiteglove product until users migrate |
| Work on encrypted chat protocol or native clients | `../finitechat/docs/architecture.md` and `../finitechat/README.md` | `finitechat` owns protocol, server, iOS, and Rust client state |
| Work on Hermes chat bridge behavior | `../finitechat/integrations/hermes/README.md` | Bridge code lives with `finitechat`; SaaS deployment handoff crosses into `finitecomputer-v2` |
| Work on encrypted knowledge, Brain Working Trees, or the Product Client | `../../finite-brain/README.md`, `../../finite-brain/development.md`, and `../../finite-brain/docs/specs/finitebrain-portability-spec.md` | `finite-brain` owns Brains, Folders, `fbrain`, Product Client policy, access, sync, and asset/source-note behavior |
| Add or change shared Hermes skills | `../finite-skills/README.md`, `../finite-skills/skills/AGENTS.md`, and `../finite-skills/docs/runtime-delivery-contract.md` | One editable source, a fresh-agent bundled baseline, and the explicit `finite skills sync` compatibility contract |
| Work on web search/extract tools | `../../finite-search/README.md` | Search/extract ops and smokes live there |
| Use reusable Nostr primitives | `../../finite-nostr/README.md` | Product-neutral Nostr helpers only |
| Generate or inspect platform reports | `../../reporting/README.md` | Reporting outputs and notes live there |
| Assess repo quality and shipping risk | `slop-audit.md` | First-pass map of jank, good choices, placeholders, and cleanup priorities |

## By Environment

| Environment | Use for | Avoid using for |
| --- | --- | --- |
| `finitecomputer-v2` dashboard/Core dev | WorkOS/SaaS dashboard surfaces, canonical web chat UI, Project/Core data UI, typed Connections, and Finite Private status | Legacy machine operations or proof that a real hosted runtime works |
| `finitecomputer-v2` Hermes runtime matrix | Real runtime image proof, local Docker, Kata promotion, Phala canary, destructive recovery, and dashboard-controlled SaaS launch readiness | Fast UI-only iteration |
| `finitecomputer` chat-local harness | Legacy dashboard chat iteration, relay behavior, runtime connector checks, local Hermes/plugin smoke | v2 product acceptance, full production host behavior, Google OAuth, k3s/TLS/DNS assumptions |
| `finitechat` local server/simulator | Protocol, server, iOS, Rust client, native chat behavior | Hosted dashboard runtime provisioning |
| `finitechat` phone/Docker canaries | Promotion evidence for real Hermes chat behavior | Fast inner-loop UI development |
| `finite-brain` local server/Product Client | Brain, Folder, access, sync, Product Client, Smoke UI, and `fbrain` CLI work | Product-neutral Nostr helpers, hosted SaaS runtime provisioning |
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

# Finite Computer v2 Agent Guide

This repo is the hard-cut self-serve SaaS codebase for Finite Computer.

It is not the legacy whiteglove product. Do not preserve box1/TRF compatibility
inside v2 unless a migration document explicitly asks for a temporary bridge.

## Product Boundary

v2 owns:

- WorkOS/account-auth dashboard for self-serve users
- BoxOne-parity dashboard web chat through a Finite Chat Hosted Web Device
- product-owned connection UX through focused services, stable APIs, and skills;
  never through Runtime Management Pipe feature commands
- Finite Sites publishing/list/preview and Finite Brain dashboard integration
- separate, revocable product-scoped Email Access Delegations for Sites and
  Brain; never a global human-agent Principal Link
- Core user, organization, Project, entitlement, and runtime-launch state
- runner integration for SaaS Agent Runtimes
- Finite Private grants and runtime-scoped Finite Private keys
- provider-neutral Runner contract with Kata first and Phala confidential fast
  follow
- deployment coordination for finite-lat-1 and finite-lat-2 product services
- the narrow runtime-local `finite` utility for explicit agent-owned workflows
  such as `finite skills sync`, never a control-plane client

v2 depends on separate product repos:

- `finite-sites` for Project Repositories and publishing through `fsite`
- `finitechat` for Finite Chat server, protocol, native clients,
  CLI/core, and the Hermes plugin
- `finite-skills` for Finite-specific agent skills
- `finite-brain` for knowledgebase sharing when it enters the product

## Hard Cuts

Do not add or preserve these as first-class v2 surfaces:

- OpenCode
- a dashboard-only chat transport separate from Finite Chat
- dashboard-to-runtime shell, filesystem, Kubernetes, or provider APIs
- legacy dashboard-managed Published Apps in place of Finite Sites
- `finitec publish`
- `finitec repo`
- `finitec gateway`
- `finitec hermes`
- `finitec finitechat`
- old machine provision/deprovision/upload/site-auth flows
- k3s/Traefik host mutation as a product API
- OpenRouter fallback when Finite Private is required
- product feature commands, feature-specific status, or skills control over the
  Runtime Management Pipe

Existing users stay on legacy `finitecomputer` until migrated. Migration code
must be explicit bridge code with a delete condition.

Runtime Management Pipe v1 is outbound-only Agent Runtime telemetry for generic
health and Product Release identity. It has no inbound command path. Product
features remain in their owning services, UI, stable APIs, local CLIs, or
skills; Core must not grow feature schemas, runtime file editing, or
orchestrator access to support them.

The only legacy Core continuity requirement is Finite Private limiter state:
grants, API-key hashes/tokens, reservations, usage/audit records, and the
operator path needed to keep issued Finite Private keys valid. Do not preserve
old deploy lanes, runtime records, machine control-plane state, or dashboard
surfaces for compatibility unless a migration doc creates a temporary bridge
with a delete condition.

## Working Rules

- Prefer copying proven code over rewriting from scratch, but delete legacy
  compatibility as soon as the v2 path has replacement tests.
- Components stay separate trees inside finite-mono. Do not copy
  `finite-sites`, `finitechat`, or `finite-skills` code into this directory —
  depend on their sibling workspace crates/paths (see
  `../docs/monorepo-doctrine.md`).
- Add docs before adding new compatibility bridges.
- Keep secrets out of the repo. Use `.env.example` files with comments.
- Build and promote one canonical Agent Runtime image through the mono-owned
  workflow. Local Docker, Kata, and Phala prove the same image digest; do not
  create provider- or feature-specific image lanes.
- Keep Hermes pinned to 0.18.2 across image, smoke, and release defaults. The
  production Finite Chat bridge is the resident Rust sidecar with one held
  inbound stream; strict mode reconnects with bounded backoff and never falls
  back to Python polling or CLI-per-message subprocesses.
- A fresh agent receives the image's Finite Skills baseline once. Restarts and
  image replacement do not overwrite it. Existing agents update only when they
  explicitly invoke `finite skills sync`; Core, Runner, and Runtime
  Management Pipe never select, poll, push, or activate a skills revision.
- Treat user data availability as the first security invariant. A Provider
  Durable Volume is not a backup; do not claim recovery until the full Recovery
  Set has restored onto an empty target.
- Do not couple compute teardown to user-data deletion. Runtime Retirement must
  retain recovery material; Purge User Data requires its own explicit,
  retention-gated authorization.
- Describe the first slice honestly as O1 operator-minimized with audited
  Finite-assisted recovery. A TEE alone does not justify an operator-blind
  claim.
- For dashboard code, read `apps/dashboard/AGENTS.md` before editing.
- For Rust changes, keep workspace tests and formatting passing.

## Prompting Contract

When a prompt is not a simple question or very small ask, guide the user toward:

1. A self-contained problem statement
2. Acceptance criteria
3. Constraints: musts, must-nots, preferences, escalation points
4. Decomposition into clean phases
5. Evaluation design for tests and checks

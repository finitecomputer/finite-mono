# Finite Computer v2 Agent Guide

This repo is the hard-cut self-serve SaaS codebase for Finite Computer.

It is not the legacy whiteglove product. Do not preserve box1/TRF compatibility
inside v2 unless a migration document explicitly asks for a temporary bridge.

## Product Boundary

v2 owns:

- WorkOS/account-auth dashboard for self-serve users
- Core user, organization, Project, entitlement, and runtime-launch state
- runner integration for SaaS Agent Runtimes
- Finite Private grants and runtime-scoped Finite Private keys
- Phala/confidential-runner launch path
- deployment coordination for finite-lat-1 and finite-lat-2 product services
- the minimal runtime-side `finitec` contract once it is extracted

v2 depends on separate product repos:

- `finite-sites` for Project Repositories and publishing through `fsite`
- `finitechat` for Finite Chat server, protocol, native clients,
  CLI/core, and the Hermes plugin
- `finite-skills` for Finite-specific agent skills
- `finite-brain` for knowledgebase sharing when it enters the product

## Hard Cuts

Do not add or preserve these as first-class v2 surfaces:

- dashboard chat
- OpenCode
- dashboard-managed Published Apps
- `finitec publish`
- `finitec repo`
- `finitec gateway`
- `finitec hermes`
- `finitec finitechat`
- old machine provision/deprovision/upload/site-auth flows
- k3s/Traefik host mutation as a product API
- OpenRouter fallback when Finite Private is required

Existing users stay on legacy `finitecomputer` until migrated. Migration code
must be explicit bridge code with a delete condition.

The only legacy Core continuity requirement is Finite Private limiter state:
grants, API-key hashes/tokens, reservations, usage/audit records, and the
operator path needed to keep issued Finite Private keys valid. Do not preserve
old deploy lanes, runtime records, machine control-plane state, or dashboard
surfaces for compatibility unless a migration doc creates a temporary bridge
with a delete condition.

## Working Rules

- Prefer copying proven code over rewriting from scratch, but delete legacy
  compatibility as soon as the v2 path has replacement tests.
- Keep service repos separate. Do not vendor `finite-sites`, `finitechat`, or
  `finite-skills` here.
- Add docs before adding new compatibility bridges.
- Keep secrets out of the repo. Use `.env.example` files with comments.
- For dashboard code, read `apps/dashboard/AGENTS.md` before editing.
- For Rust changes, keep workspace tests and formatting passing.

## Prompting Contract

When a prompt is not a simple question or very small ask, guide the user toward:

1. A self-contained problem statement
2. Acceptance criteria
3. Constraints: musts, must-nots, preferences, escalation points
4. Decomposition into clean phases
5. Evaluation design for tests and checks

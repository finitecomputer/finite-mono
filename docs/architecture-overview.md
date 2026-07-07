# Architecture Overview

> Status: imported from `finite-eng-docs` during Phase 7 on 2026-07-06. This
> document has not been fully revalidated after the monorepo import. Treat it as
> orientation background, not an authoritative current runbook.

This is the current high-level map. It is meant to help a new engineer decide
where to look first, not to specify every runtime or protocol flow.

## Product Shape

Finite is a hosted-agent environment. A user gets an account, lands in a
dashboard, and can interact with a provisioned Agent Runtime that already has
Hermes, the Finite Chat plugin, tools such as `fsite` and `fbrain`, skills,
workspace state, Finite Private access, and publishing paths configured.

The current product split is explicit:

- Self-serve SaaS: `finitecomputer-v2` is the product being built now. It owns
  WorkOS auth, the dashboard, Core, Projects, runner launch state, runtime image
  promotion, Finite Private grants, and hosted Finite Chat deploy coordination.
- Legacy whiteglove product: `finitecomputer` remains the product already
  shipped to box1/TRF/smoke while those users are unmigrated. It owns the
  existing dashboard relay loop, broad `finitec`/`finited` operations, host
  runbooks, and migration bridge code.
- Product chat for v2: `finitechat` owns the encrypted protocol, server,
  native clients, CLI/core, and Hermes `finitechat` plugin. v2 displays a
  Finite Chat invite with no PIN; dashboard chat is intentionally not part of
  the v2 launch path.

## Layer Map

```mermaid
flowchart TB
  subgraph Product["Product Surfaces"]
    V2Dashboard["finitecomputer-v2 dashboard"]
    NativeChat["Finite Chat native clients"]
    LegacyDashboard["legacy finitecomputer dashboard"]
  end

  subgraph SaaS["Self-Serve SaaS Control"]
    Core["v2 Core"]
    Runner["Runner"]
    PrivateLimiter["Finite Private limiter"]
  end

  subgraph Legacy["Legacy Whiteglove Platform"]
    LegacyFiniteComputer["finitecomputer"]
    Finited["finited relay/control APIs"]
    Finitec["broad finitec"]
  end

  subgraph Runtime["Agent Runtime"]
    Hermes["Hermes"]
    UserHome["durable runtime state and workspace"]
    SkillBaseline["Finite managed skills"]
    FSite["fsite / Finite Sites"]
    FBrainCli["fbrain CLI"]
  end

  subgraph Supporting["Supporting Services"]
    ChatServer["finitechat server"]
    Brain["finite-brain knowledge system"]
    Search["finite-search: SearXNG / Firecrawl"]
    Nostr["finite-nostr primitives"]
    Reporting["reporting snapshots"]
  end

  V2Dashboard --> Core
  Core --> Runner
  Core --> PrivateLimiter
  Runner --> Runtime
  V2Dashboard --> NativeChat
  NativeChat --> ChatServer
  ChatServer --> Hermes
  Runtime --> Brain
  LegacyDashboard --> Finited
  Finited --> Finitec
  Finitec --> Runtime
  LegacyFiniteComputer --> Finited
  LegacyFiniteComputer --> Runtime
  SkillBaseline --> Hermes
  Runtime --> FSite
  FBrainCli --> Brain
  Hermes --> Search
  NativeChat --> Nostr
  Reporting -. "reads/summarizes state" .-> LegacyFiniteComputer
  Reporting -. "future SaaS reporting input" .-> Core
```

## Ownership Boundaries

`finitecomputer-v2` owns the new self-serve SaaS boundary. If the question
involves WorkOS signup, Projects, Core state, runner launch records, Finite
Private grants, runtime image promotion, hosted Finite Chat deploy mechanics,
or the v2 dashboard, start there.

`finitecomputer` owns the legacy whiteglove platform boundary. If the question
involves box1/TRF/smoke users, the dashboard relay path, broad `finitec` or
`finited` commands, host state, k3s, backups, or migration bridge behavior,
start there.

`finitechat` owns the encrypted chat boundary. If the question involves room
state, OpenMLS, shared Finite identity use in CLI/agent flows, iOS chat, native
client behavior, chat server contracts, or the Hermes chat bridge, start there.
For v2 releases, the deploy coordination handoff crosses through
`finitecomputer-v2`.

`finite-skills` owns the shared skill baseline. It should contain platform-owned
skills that are shipped into managed Hermes runtimes. User-local or machine
specific skills should not be developed there first.

`finite-search` owns the self-hosted search and extraction services consumed by
agent tools. It is an ops/integration repo, not a product app.

`finite-brain` owns the encrypted Vault/Folder knowledge system, trusted
Product Client, `fbrain` CLI, Vault Working Tree sync, and FiniteBrain-specific
policy. Reusable Nostr primitives still belong in `finite-nostr`; FiniteBrain
Vault, Folder, access, sync, and Product Client policy stays in `finite-brain`.

`finite-nostr` owns reusable Nostr helpers. Product-specific policy should stay
out of it.

`reporting` owns generated reporting outputs and notes that summarize platform
state across time.

## State Boundaries

High-level state buckets:

- Git-owned state: source code, runtime baseline definitions, skill baseline,
  runbooks, deployment definitions.
- SaaS-owned state: v2 Core database records for accounts, Projects, runtime
  launches, entitlements, and Finite Private grants.
- Host-owned state: legacy control plane databases, secrets, rendered
  manifests, deployment state, backups.
- Runtime-owned state: user home, Hermes state, workspace files, shared Finite
  identity files for trusted CLI/agent flows, runtime-scoped Finite Private
  credentials, machine/project-specific skill overrides.
- Reporting-owned state: generated snapshots and evidence logs.

Do not assume one repo owns all copies of a concept. For example, a Hermes
integration change may touch `finitechat`, be deployed through
`finitecomputer-v2` for SaaS, still have legacy exposure through
`finitecomputer`, and depend on skills from `finite-skills`.

## Current Local Development Anchor

Pick the local loop by product lane:

- v2 dashboard/Core UI work starts in `finitecomputer-v2/apps/dashboard`.
  The documented lightweight loop is `npm ci`, then `npm run dev`; v2 product
  acceptance still climbs the Hermes runtime test matrix in
  `../finitecomputer-v2/docs/hermes-runtime-test-matrix.md`.
- v2 runtime proof starts with the same runtime contract under local Docker,
  then remote Docker, then Phala before dashboard-controlled SaaS launch. The
  current runtime image path packages `finitechat`, the Hermes `finitechat`
  plugin, `fsite`, and `fbrain`.
- Legacy dashboard relay/chat work still uses the old `finitecomputer`
  MicroSandbox harness:

```bash
# From /Users/alex/Projects/finite
cd finitecomputer
nix develop
just chat-local-bootstrap smoke-finite
just chat-local-up
```

That starts the legacy dashboard, a local relay, and a MicroSandbox agent
runtime. Use `../../finitecomputer/docs/chat-local-dev.md` as the canonical
guide for that lane.
Use [Local development matrix](local-dev-matrix.md) when choosing between repo
specific loops or when onboarding an external contributor.

Other repos have local loops, but they are scoped:

- `finitechat`: local server, iOS simulator, Hermes gateway/canary scripts.
- `finite-brain`: Cargo workspace checks, local `finite-brain-app` server,
  Product Client at `/client`, Smoke UI at `/smoke/ui`, and the `fbrain` CLI.
- `finite-search`: static checks plus remote-service smokes through SSH tunnels.
- `finite-nostr`: Rust library checks.
- `finite-skills`: content validation through managed runtime usage.
- `reporting`: local snapshot/site generation, with optional live probes.

## Open Questions

- What is the first one-command v2 local proof for Core plus runtime launch
  once the Docker/Phala ladder is stable?
- Which legacy users and docs remain blocked on `finitecomputer` before the
  migration bridge can be deleted?
- Which docs are canonical versus historical in older root-level markdown files?
- Which cross-component changes require synchronized version pins or deployment
  gates?

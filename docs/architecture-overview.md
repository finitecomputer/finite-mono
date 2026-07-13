# Architecture Overview

> Status: imported from `finite-eng-docs` during Phase 7 on 2026-07-06. This
> document has not been fully revalidated after the monorepo import. The chat
> layering section was revalidated on 2026-07-13; treat the remainder as
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
  native clients, CLI/core, Hosted Web Device behavior, and Hermes `finitechat`
  plugin. The canonical BoxOne-derived dashboard chat runs through a trusted,
  revocable Hosted Web Device; Electron and native clients later join as
  separate local Devices using the same product account and Rooms.

The security ordering is recoverability first, then progressively stronger
operator privacy. The trusted first cohort targets O1: normal product paths
minimize Finite access, while an explicit audited Finite-assisted recovery path
remains available. Kata is isolated but host-operator-trusted; Phala/TEE can
raise the privacy level only after the same Recovery Set, key-release path, and
empty-target restore are proven. See
[ADR 0001](adr/0001-recoverability-precedes-operator-blindness.md).

## Layer Map

```mermaid
flowchart TB
  subgraph Product["Product Surfaces"]
    V2Dashboard["finitecomputer-v2 dashboard"]
    HostedWeb["Hosted Web Device"]
    NativeChat["Finite Chat native clients"]
    LegacyDashboard["legacy finitecomputer dashboard"]
  end

  subgraph SaaS["Self-Serve SaaS Control"]
    Core["v2 Core"]
    Runner["Runner"]
    PrivateLimiter["Finite Private limiter"]
  end

  subgraph Release["Product Release Inputs"]
    SkillSource["finite-skills source"]
    SkillRevision["immutable Finite Skills Revision"]
    SkillSource --> SkillRevision
  end

  subgraph Legacy["Legacy Whiteglove Platform"]
    LegacyFiniteComputer["finitecomputer"]
    Finited["finited relay/control APIs"]
    Finitec["broad finitec"]
  end

  subgraph Runtime["Agent Runtime"]
    AgentChatDevice["agent Finite Chat Device and Rust sidecar"]
    HermesAdapter["thin Hermes platform adapter"]
    Hermes["Hermes"]
    Agentd["finite-agentd command and authorization ledger"]
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
  V2Dashboard --> HostedWeb
  HostedWeb <--> ChatServer
  NativeChat <--> ChatServer
  ChatServer <--> AgentChatDevice
  AgentChatDevice <--> HermesAdapter
  HermesAdapter <--> Hermes
  AgentChatDevice <--> Agentd
  Runtime --> Brain
  LegacyDashboard --> Finited
  Finited --> Finitec
  Finitec --> Runtime
  LegacyFiniteComputer --> Finited
  LegacyFiniteComputer --> Runtime
  SkillRevision --> SkillBaseline
  Core -. "desired revision digest" .-> SkillBaseline
  SkillBaseline --> Hermes
  Runtime --> FSite
  FBrainCli --> Brain
  Hermes --> Search
  NativeChat --> Nostr
  Reporting -. "reads/summarizes state" .-> LegacyFiniteComputer
  Reporting -. "future SaaS reporting input" .-> Core
```

## Chat protocol versus product layers

The Finite Chat server and Devices are the chat system. The Hosted Web API,
Project binding, and Hermes adapter are product layers using that system; they
must not acquire protocol authority by convenience.

| Layer | Owns | Must not own |
| --- | --- | --- |
| Finite Chat server | Ordered ciphertext log, MLS commits and Welcomes, KeyPackage leases, encrypted blobs, membership intervals, idempotency records, and SSE wake hints | Device-owned applied cursors, WorkOS users, Finite Computer Projects, selected UI state, canonical-Room policy, Hermes turns |
| Finite Chat Device | One Device credential/key, local MLS groups and epoch state, joined Rooms, applied cursors, and encrypt/decrypt/validation | UI projection, model execution, or authority to infer a Project-to-Room binding |
| Finite Chat Rust app runtime | A serialized actor around one Device: durable sync, outbox, and the Rooms/Topics/Chats/messages projection shared by clients | Finite Computer Project identity or Hermes model state |
| Hosted Web Device | A real Finite Chat Device operated for one Account Auth user plus a narrow dashboard HTTP adapter | A second chat protocol, Agent Runtime state, authority to reinterpret ambiguous Rooms |
| Project-to-Room binding | An immutable authenticated product bookmark telling the dashboard which already-authorized Room one Project opens and where it creates Chats | Room membership, message delivery, agent subscriptions, automatic reconciliation, or recovery by identifier or selection order |
| Agent Rust sidecar | The Agent Principal's normal Finite Chat Device plus separate local decrypted APIs for Hermes messages and typed `finite-agentd` commands | Model execution or product binding selection |
| Hermes platform adapter | Thin translation between sidecar records and Hermes callbacks | MLS, Room storage, polling a second chat transport, binding migration |
| Hermes | Model turns, memory, tools, and replies | Finite Chat identity, encryption, Room authority |
| `finite-agentd` | Agent-runtime authorization and an idempotent typed command/result ledger, including owner claim | Model chat, Room membership, or Project-to-Room selection |

The agent Rust Device syncs every Room it has joined. The Hermes adapter also
serves every joined Room by default. Only an explicit `extra.room_id` or
`FINITECHAT_ROOM_ID` configuration narrows what the adapter hands to Hermes;
that optional filter still does not change Device membership or protocol sync.
Hermes's home-channel setting is an outbound routing preference, not a Room
subscription.

On restart, each Finite Chat Device reopens its same durable store and resumes
sync for Rooms it has already joined, provided that store is mounted and
intact. A restart does not create a new server Room, invent membership,
reclassify a Room, or write a Project binding. It may activate a Welcome that
was already authorized before restart; processing that Welcome or later
messages is protocol convergence, not a product migration. Store loss or
corruption is a separate recovery failure, not normal restart behavior.

The product additions should remain narrow: a hosted human Device for browser
access, discovery of the Agent Principal, an explicit Project-to-Room bookmark,
the thin Hermes translation, and a separate typed Agent Platform Channel into
`finite-agentd`. Hermes is not on the management-command path. Once written,
the Project binding is immutable under ordinary product flows: opening it
validates and uses it but does not reconcile, replace, or rewrite it.

The Project-creation workflow writes a sealed one-time bootstrap authorization
before ordinary chat is opened; only that authorization may initialize the
first Room. Bootstrap then creates a sealed staged journal. Before any server
mutation, the journal records the exact Room create request, including the
intended Room id and MLS group id. It next records the claimed Agent KeyPackage
before using that claim to create or add membership. The Device sends only the
journaled Room request; if the server accepted it but the matching local MLS
group was not saved, restart replays that exact request and group id. Finally,
the journal records the exact prepared add-member commit before submit. An
interrupted attempt therefore resumes only those journaled artifacts. After a
claim is journaled, restart does not claim again, scan Rooms, generate a
different group, or adopt another candidate.

Ordinary load, restart, deploy, upgrade, and recovery cannot mint the
authorization. If Core committed creation but the dashboard lost the
authorization response, ordinary chat load only reports that setup is
unfinished. The user-visible `Finish chat setup` action may replay the omitted
handoff only after a fresh Core read proves the exact Account-owned Project and
exactly one durable creation request for it in `requested`, `launching`, or
`running` state; it does not inspect or select a Room. There is no automatic
Room reconciliation or legacy binding migration. Missing authorization, a
retained unbound candidate, or any other ambiguity fails without choosing from
selection, display order, timestamps, or opaque identifiers.

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

`finite-skills` owns the only editable source for the Managed Skills Baseline.
CI publishes immutable Finite Skills Revisions; each Runtime image embeds one
offline revision and compatible promoted revisions activate between turns
through a narrow Runtime Capability. A Finite Sites repository is a read-only
distribution mirror, and neither it nor the dashboard nor an old GitHub repo is
an authoring source. User-local skills remain runtime-owned data.

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

- Git-owned state: source code, runtime baseline definitions, the editable
  skill baseline, runbooks, and deployment definitions.
- Release-owned state: immutable skill artifacts, manifests, digests,
  compatibility evidence, and Product Release pins.
- SaaS-owned state: v2 Core database records for accounts, Projects, runtime
  launches, entitlements, Finite Private grants, and desired/observed Finite
  Skills Revision ids.
- Host-owned state: legacy control plane databases, secrets, rendered
  manifests, deployment state, backups.
- Runtime-owned state: agent home, Hermes state, workspace files, that agent's
  Finite Home identity shared only by its local Finite tools, runtime-scoped
  Finite Private credentials, managed-revision caches, and user-owned skill
  overrides. Managed caches are reproducible; user skill content is not.
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
- v2 runtime proof starts with the resident streaming Hermes sidecar, then the
  same runtime contract under local full-product Docker, Kata, and Phala before
  dashboard-controlled SaaS launch. The target Runtime image packages
  `finitechat`, the Hermes `finitechat` plugin, `fsite`, `fbrain`, and one baked
  Finite Skills Revision in lockstep; the current image still omits the skills
  revision and its activation client.
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
  once the Docker/Kata/Phala ladder is stable?
- Which legacy users and docs remain blocked on `finitecomputer` before the
  migration bridge can be deleted?
- Which docs are canonical versus historical in older root-level markdown files?
- Which cross-component changes require synchronized version pins or deployment
  gates?
- Should stable Finite Skills Revisions automatically activate fleet-wide after
  a canary cohort, or require per-Project opt-in?

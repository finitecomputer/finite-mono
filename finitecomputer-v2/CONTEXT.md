# Finite Computer v2

Finite Computer v2 is the hard-cut self-serve SaaS product for deploying and
managing hosted Hermes Agent Runtimes through Finite Chat, Finite Sites, and
Finite Private.

## Language

**Finite Computer v2**:
The self-serve SaaS product that lets a user sign in, deploy an Agent Runtime,
and manage it through the dashboard and Finite Chat.
_Avoid_: Legacy finitecomputer, box1 product

**Legacy Finite Computer**:
The existing whiteglove product deployed to box1, TRF, smoke, and similar hosts.
_Avoid_: v1 when the distinction is infrastructure-specific

**Project**:
The user-facing container for one hosted agent experience.
_Avoid_: Machine, pod, Phala instance

**Agent Runtime**:
The isolated environment where Hermes and runtime tools execute for one Project.
_Avoid_: Machine, dashboard server, Core worker

**Desired Runtime State**:
The Core-owned target lifecycle state for an Agent Runtime.
_Avoid_: Runner memory, provider config dump

**Runner**:
The substrate that starts, stops, and hosts Agent Runtimes.
_Avoid_: Phala when discussing the product contract

**Runtime Operation**:
A user or operator requested lifecycle change for an Agent Runtime.
_Avoid_: Shell command, provider task

**Provider Runtime Handle**:
The provider-specific identity used to find an Agent Runtime after a Runner restarts.
_Avoid_: Local container name, Phala-only app id

**Confidential Runner**:
A Runner that provides stronger operator-privacy guarantees through
confidential-computing infrastructure.
_Avoid_: TEE product, Phala-only project type

**Core**:
The v2 service that owns account-linked Project state, runtime launch state,
entitlements, and Finite Private grants.
_Avoid_: Legacy control plane, finited

**Runtime-Scoped Finite Private Key**:
A Finite Private credential issued for one Project or Agent Runtime.
_Avoid_: User API key, shared provider fallback

**Account Auth**:
The dashboard login and billing identity used to create and administer Projects.
In v2 today this is WorkOS.
_Avoid_: Nostr identity, Agent Runtime key

**User Primary Key**:
The Nostr public key that represents the user cryptographically. In the
`finite-auth` target model this may be a Frostr group public key.
_Avoid_: WorkOS user id, NIP-05 name, device key

**Agent Chat Identity**:
The Nostr identity an Agent Runtime uses as a participant in Finite Chat.
_Avoid_: User Primary Key, WorkOS user id

**Agent Signing Session**:
A bounded authorization that lets an Agent Runtime request signatures as a
user's User Primary Key for a specific scope and time.
_Avoid_: Separate default agent account, permanent delegated nsec

**Agent Root Secret**:
The runtime-generated secret material used to restore an Agent Runtime's own
cryptographic identity and derive or unwrap runtime-owned service keys.
_Avoid_: Finite Private API key, WorkOS session, user primary key

**User Backup Key**:
The recovery material delivered to the user after first successful pairing so
the user can recover an Agent Runtime if provider durable state is lost.
_Avoid_: Routine restart unlock, dashboard password, operator-held escrow key

**Hosted Pairing**:
The launch-time flow that lets the user add a newly deployed Agent Runtime to
Finite Chat.
_Avoid_: PIN, machine claim token

**Finite Chat Invite**:
The user-facing invite link or code displayed after an Agent Runtime is ready.
_Avoid_: PIN, runtime token

**Finite Sites Project Repository**:
The Git-backed source repository and publishing path owned by Finite Sites.
_Avoid_: `finitec repo`, dashboard published app

**Minimal finitec**:
The runtime-side v2 adapter for heartbeat, status, and narrow runtime command
work.
_Avoid_: Product CLI, publish tool, chat bridge

**Runtime Management Pipe**:
The authenticated Core API surface used by an Agent Runtime to report health and capabilities.
_Avoid_: Dashboard chat, arbitrary command relay, Finite Chat

## Relationships

- **Finite Computer v2** owns **Core**, Account Auth, dashboard, runner launch,
  and Finite Private grant orchestration.
- **Legacy Finite Computer** continues serving existing whiteglove users until
  they are migrated.
- A **Project** has one primary **Agent Runtime** at launch.
- **Core** stores the **Desired Runtime State** for an **Agent Runtime**.
- A **Runtime Operation** moves an **Agent Runtime** toward **Desired Runtime State**.
- A **Runner** hosts one or more **Agent Runtimes**.
- **Core** is the source of truth for desired **Agent Runtime** lifecycle state.
- A **Runner** reattaches to an **Agent Runtime** by its **Provider Runtime Handle**.
- **Phala** is a **Confidential Runner** implementation, not the product model.
- **Account Auth** owns dashboard access and billing; **User Primary Key** owns
  cryptographic user identity.
- **Agent Chat Identity** is distinct from an **Agent Signing Session** acting
  as the user's **User Primary Key**.
- **Agent Root Secret** must never be visible to Core, dashboard, or operators.
- **User Backup Key** is for disaster recovery, not normal restart behavior.
- **Finite Chat Invite** is produced by **Hosted Pairing** and does not use a
  PIN in v2.
- Website and repo workflows use **Finite Sites Project Repositories**, not
  `finitec repo` or `finitec publish`.
- User chat reaches Hermes through Finite Chat and the Finite Chat Hermes
  plugin, not through `finitec`.
- **Minimal finitec** is a client for the **Runtime Management Pipe**.
- **Minimal finitec** does not support legacy machine/operator commands,
  dashboard chat, or arbitrary command execution.

## Flagged Ambiguities

- "finitecomputer" now means either **Legacy Finite Computer** or
  **Finite Computer v2**. Say which one when discussing deploys or code moves.
- "repo" in v2 means a **Finite Sites Project Repository**, not `finitec repo`.
- "publish" in v2 means `fsite`/Finite Sites, not dashboard Published Apps.

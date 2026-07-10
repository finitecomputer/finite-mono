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

**Hosted Web Device**:
A Finite-operated Finite Chat device bound to one Account Auth identity whose server-held device state powers dashboard web chat.
_Avoid_: Browser E2EE, dashboard relay, Agent Principal Key

**User Nostr Identity**:
The human-controlled Nostr identity used by the user's Finite Chat account and eligible to be generated or imported by the user.
_Avoid_: Account Auth, Agent Principal Key, Device key

**Agent Principal Key**:
The Nostr key owned by one Agent Runtime and shared by its Finite Chat, Finite Sites, and Finite Brain tools through that runtime's Finite Home.
_Avoid_: User Nostr Identity, Account Auth, shared fleet key

**Email Access Delegation**:
A revocable product-owned authorization connecting one verified email Principal to one Agent Principal inside exactly one Finite product.
_Avoid_: Principal Link, Google account sharing, agent impersonation

**User Recovery Key**:
A user-controlled Recovery Authority intended to unlock Recovery Snapshots without relying on Finite operator custody.
_Avoid_: Routine restart unlock, dashboard password, Finite recovery key

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

**Runtime Management Pipe**:
The outbound-only authenticated channel through which an Agent Runtime reports
generic health and Product Release telemetry to Core.
_Avoid_: Command channel, product feature API, chat transport, credential handoff

**Finite Product Release**:
A tested compatibility set of hosted services, user-facing binaries, and one Agent Runtime image delivered as one Finite Computer product.
_Avoid_: Component latest, untested version mix

**Managed Skills Baseline**:
The platform-owned set of Finite skills bundled into the Runtime image and
copied once when a new Agent Runtime initializes.
_Avoid_: User skills, Hermes built-ins, fleet desired state

**Finite Skills Revision**:
One tested version of the Managed Skills Baseline bundled in a Runtime image or
installed later by an explicit local sync.
_Avoid_: Fleet desired state, automatic update target, mutable checkout

**Skills Sync**:
The explicit, agent-local `finite skills sync` operation that lets an existing
agent adopt the tested baseline in its current Runtime image at its own pace.
_Avoid_: Core rollout, polling updater, Runtime Management Pipe request, Runner operation

**User Skill Override**:
A user-owned skill that deliberately takes precedence over a baseline skill without modifying it.
_Avoid_: Managed edit, forked Product Release

**Recoverability Contract**:
The tested promise connecting a Finite Product Release to its Recovery Set, covered failures, restoration paths, and recovery objectives.
_Avoid_: Backup exists, probably recoverable, disaster plan

**Recovery Set**:
The exact user data and key material covered by one Recoverability Contract.
_Avoid_: Data directory, everything, encrypted state

**Recovery Snapshot**:
A versioned, integrity-checked, provider-independent copy of one Recovery Set.
_Avoid_: Durable volume, live mount, tarball

**Recovery Readiness**:
The evidence that a current Recovery Snapshot has successfully restored its Recovery Set onto an empty replacement target.
_Avoid_: Backup completed, snapshot exists, green timer

**Provider Durable Volume**:
The primary live storage attached to an Agent Runtime, which may survive lifecycle operations but is never itself a backup.
_Avoid_: Recovery Snapshot, backup volume

**User Data Availability Invariant**:
The release rule that no single failure covered by the Recoverability Contract may make user data permanently inaccessible.
_Avoid_: Best-effort backup, eventual recovery

**Operator-Privacy Level**:
The evidence-backed statement of what Finite and infrastructure providers can access during normal operation and Break-Glass Recovery for one Finite Product Release.
_Avoid_: Private, zero-access, TEE-secure

**Break-Glass Recovery**:
An explicit, audited privacy downgrade used to rescue user data after normal recovery paths fail.
_Avoid_: Debug mode, operator shell, silent rescue

**Recovery Authority**:
A user, device, service, or protected key capable of unlocking or reassigning recovery material for a declared Recoverability Contract.
_Avoid_: Backup owner, admin access, master key

**Finite-Assisted Recovery Authority**:
The narrowly controlled Finite capability to unlock a Recovery Snapshot during an authorized and audited Break-Glass Recovery.
_Avoid_: Master key, operator access, invisible escrow

**Purge User Data**:
The separately authorized irreversible deletion of a Recovery Set and every retained Recovery Snapshot after its retention and export gates pass.
_Avoid_: Destroy runtime, stop, cancel subscription

**Runtime Retirement**:
The deprovisioning of Agent Runtime compute and public endpoints while preserving recovery material through the declared retention period.
_Avoid_: Purge User Data, subscription cancellation, provider destroy

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
- **Account Auth** owns dashboard access and billing; **User Nostr Identity**
  owns the human's cryptographic chat identity.
- **Account Auth** authorizes access to one **Hosted Web Device**, but that
  device has its own revocable Finite Chat key and durable store.
- A **Hosted Web Device** is a user device alongside Electron or native
  devices; it is not an **Agent Principal Key** or room authority.
- A **User Nostr Identity** and **Agent Principal Key** are always distinct by
  default; the agent signs agent operations as itself.
- **Email Access Delegations** for Finite Sites and Finite Brain are separate;
  Brain additionally requires Folder Key Grants to the **Agent Principal Key**.
- An agent exercising an **Email Access Delegation** continues signing as
  itself, and revocation never changes the human or agent identity.
- Finite Chat, Finite Sites, and Finite Brain inside one **Agent Runtime** use
  the same **Agent Principal Key** without copying it into product-specific
  config stores.
- **User Recovery Key** is a target Recovery Authority, not a shipped
  capability until its empty-target restore has passed.
- **Finite Chat Invite** is produced by **Hosted Pairing** and does not use a
  PIN in v2.
- Website and repo workflows use **Finite Sites Project Repositories**, not
  `finitec repo` or `finitec publish`.
- User chat reaches Hermes through Finite Chat and the Finite Chat Hermes
  plugin, not through `finitec`.
- The **Runtime Management Pipe** v1 flows only from **Agent Runtime** to
  **Core** and carries generic health and Product Release telemetry.
- Lifecycle requests flow from **Core** to a **Runner** through the Runner work
  contract; they are not inbound Runtime Management Pipe messages.
- Product features, credentials, chat state, skills commands, shell access,
  filesystem access, and provider APIs never travel over the **Runtime
  Management Pipe**.
- A **Project** chooses a **Runner** class at launch without exposing its
  **Provider Runtime Handle** as the product model.
- A **Finite Product Release** pins the compatible Core/dashboard, Agent
  Runtime, Hermes, Finite Chat, Finite Sites, Finite Brain, hosted-service
  versions, and the **Finite Skills Revision** bundled for newly created
  agents.
- A new **Agent Runtime** receives its bundled **Managed Skills Baseline** once
  at initialization; restart and image replacement do not overwrite that
  installed baseline.
- Existing agents update at their own pace through the explicit
  **Skills Sync**. Core stores no desired skills revision, and Core, Runner,
  and the **Runtime Management Pipe** do not poll, push, or activate skills.
- The **Managed Skills Baseline** never rewrites user skills, and a **User Skill
  Override** remains intact across explicit sync, restart, and recovery.
- A **Finite Product Release** declares one **Recoverability Contract** and one
  **Operator-Privacy Level**; neither may claim behavior that has not passed its
  recovery and access tests.
- The **User Data Availability Invariant** takes precedence over increasing the
  **Operator-Privacy Level**.
- A **Confidential Runner** can improve the **Operator-Privacy Level**, but it
  cannot replace backup, export, identity recovery, or **Break-Glass Recovery**.
- A **Recovery Authority** may be retired only after another recovery path for
  the same **Recovery Set** has been successfully exercised.
- A **Provider Durable Volume** holds primary runtime state; a **Recovery
  Snapshot** is an independent copy of that state and its required key material.
- **Purge User Data** is never implied by stop, runtime retirement,
  subscription cancellation, or compute deprovisioning.
- **Runtime Retirement** preserves a restorable **Recovery Snapshot**;
  **Purge User Data** is a later, separately authorized transition.

## Example Dialogue

> **Dev:** "Can we ship this dashboard feature by adding a Runtime Management Pipe command?"
> **Domain expert:** "No. Product features belong in their owning service, UI, stable CLI, or skill. The pipe only receives generic health and release telemetry."

> **Dev:** "Does opening Electron replace the Hosted Web Device?"
> **Domain expert:** "No. Account Auth enrolls another Finite Chat Device, and each Device heals independently from the canonical Room log."

> **Dev:** "Does fsite sign as the human because it runs inside their agent?"
> **Domain expert:** "No. It signs with that Agent Runtime's Agent Principal Key; human access is granted separately."

> **Dev:** "Does allowing my agent to use Sites shared to my email also open my Brain?"
> **Domain expert:** "No. Email Access Delegations are product-scoped, and Brain separately grants Folder Keys to the agent npub."

> **Dev:** "Can we remove Finite's recovery access as soon as the runtime moves into a TEE?"
> **Domain expert:** "No. First prove the user-controlled recovery path for the same Recovery Set; a TEE changes operator access, not whether lost keys can strand data."

> **Dev:** "Does Core tell every existing agent when to update Finite Skills?"
> **Domain expert:** "No. New agents get the image's baseline once. Existing agents will update explicitly at their own pace with `finite skills sync`."

> **Dev:** "Can a baseline update replace a skill the user customized?"
> **Domain expert:** "No. The local copy is a User Skill Override; an explicit baseline sync must leave user-owned skills alone."

## Flagged Ambiguities

- "finitecomputer" now means either **Legacy Finite Computer** or
  **Finite Computer v2**. Say which one when discussing deploys or code moves.
- "repo" in v2 means a **Finite Sites Project Repository**, not `finitec repo`.
- "publish" in v2 means `fsite`/Finite Sites, not dashboard Published Apps.
- "shared Finite identity" means a shared `finite-identity` contract within one
  Finite Home, not a shared secret between a human and an agent.
- The retired **User Primary Key** / **Agent Signing Session** language came
  from the removed `finite-auth` experiment; use **User Nostr Identity** and
  **Agent Principal Key** instead.
- "Finite cannot see user data" previously mixed normal-operation access with
  cryptographic impossibility. Resolve it through the release's explicit
  **Operator-Privacy Level**, including its **Break-Glass Recovery** posture.
- "skills installed" previously meant baked into an image, present on disk,
  copied into a new Agent Runtime, visible to Hermes, or explicitly synced.
  Name the bundled or locally synced **Finite Skills Revision** without
  implying Core desired state or automatic fleet rollout.

# Auth And Key Custody Brief

Status: discussion brief for Paul, Austin, and Alex.

Date: 2026-07-02.

Related repo: `finitecomputer/finite-auth`.

## Purpose

Finite Computer v2 needs one coherent answer for account login, chat identity,
agent identity, runtime secrets, user recovery, and future Nostr/Frostr auth.

This document is intentionally a decision brief, not an implementation spec.
It records what seems decided, what should align with Austin's `finite-auth`
work, and which questions need Alex/Austin review before we lock the recovery
and custody model.

## Source Alignment

`finite-auth` is a WIP private repo described as "Nostr-first authentication
policy for Finite Rust services." Its current docs establish these domain
rules:

- The stable Finite cryptographic principal is a Nostr public key.
- NIP-05 is identification, not the account key.
- A User Primary Key may be a Frostr group public key.
- The target Frostr setup is client-orchestrated 2-of-3:
  server share, user-client share, and native secure-storage share.
- The native secure-storage share is a Cold Backup Share for recovery or
  rotation, not routine signing.
- Agents should use bounded Agent Signing Sessions when acting as the user's
  User Primary Key.
- Agent accountability belongs in Finite authorization/session/audit records,
  not separate default Nostr identities.

Those rules should steer v2 key language, but they do not automatically replace
the v2 SaaS dashboard login or the Agent Runtime's chat identity.

## Current v2 Product Shape

Account Auth:
WorkOS remains the immediate dashboard login and billing identity. It answers:
"Who can create, view, pay for, restart, stop, recover, or destroy this
Project?"

Core:
Core owns Projects, runtime launch state, Finite Private grants, runtime-scoped
Finite Private API keys, lifecycle operations, and plaintext-safe runtime facts.
Core must not store agent private keys, user data, chat contents, raw recovery
secrets, or provider fallback model keys.

Agent Runtime:
The runtime runs Hermes, the Finite Chat Hermes plugin, `finitechat`, `fsite`,
Finite-managed skills, and Finite Private configuration. Durable provider state
is mounted at `/data`.

Finite Chat:
The native Finite Chat app is the user-facing chat UI for this release. The
dashboard shows lifecycle and plaintext-safe ops, not chat messages or
connection state. Hosted pairing uses no PIN.

Provider:
Phala is the current default Confidential Runner because it provides durable
mounts. Docker remains the local and remote preflight backend. Tinfoil/fstore
is parked while durable mounts are available elsewhere.

## Key Concepts

### Account Auth

Dashboard account identity. Today this is WorkOS.

It should authorize SaaS product operations, billing, Project visibility, and
dashboard lifecycle actions.

It should not be treated as the user's Nostr identity.

### User Primary Key

The user's cryptographic Nostr identity. In the `finite-auth` target model this
can be a Frostr group public key.

Long term, this is the identity a user uses across Finite Chat, FiniteBrain,
and other Nostr-native surfaces.

### Agent Chat Identity

The Agent Runtime's own Finite Chat participant identity.

This identity is useful because humans talk to an agent as a distinct actor in
chat. It should not be confused with the agent acting as the user for external
Nostr/Frostr signing.

### Agent Signing Session

A bounded authorization for an Agent Runtime to request signatures as the
user's User Primary Key for a specific scope and time.

This is the `finite-auth` target for "agent acts for user" operations. It is
separate from the Agent Chat Identity.

### Agent Root Secret

The runtime-owned secret material that restores the Agent Runtime's own
cryptographic identity and can derive or unwrap runtime-owned keys for
Finite Chat, Finite Sites, Finite Brain, local encryption, and profile signing.

The Agent Root Secret should be generated inside the confidential runtime on
first boot or first setup. Core, dashboard, runner, and operators should never
see it.

### User Backup Key

Recovery material delivered to the user after first successful pairing. The
user stores it in iOS Keychain and may also export/write it down depending on
product policy.

This is for disaster recovery when provider durable state is lost or moved. It
is not the normal restart unlock path.

## Decisions That Seem Settled

### 1. WorkOS stays for initial SaaS account login

Decision:
Use WorkOS for immediate dashboard login, account linking, billing, and Project
administration.

Reason:
It lets v2 ship self-serve SaaS without blocking on the full Nostr/Frostr auth
stack.

Constraint:
Do not let WorkOS become the cryptographic user identity. It is an account and
billing identity.

### 2. Core must never custody Agent Root Secrets

Decision:
Core stores only plaintext-safe runtime facts and token hashes. Core does not
store raw agent keys, raw recovery keys, raw Finite Private keys after issue,
chat contents, user data, or decrypted runtime state.

Reason:
The product promise is that operators cannot inspect the user's agent data or
agent private keys.

Implication:
Lifecycle controls must be coarse provider/runtime actions: restart, recover,
stop, destroy. They cannot be "edit the user's home directory from dashboard."

### 3. Normal Phala restart should not require a user unlock

Decision:
With Phala durable mounts, normal restart should preserve the Agent Runtime's
state and key material. The user should not need perfect timing or manual chat
unlock for ordinary restarts.

Reason:
Routine restart unlock over chat creates a bootstrapping problem: if the agent
cannot decrypt chat until unlocked, chat cannot be the only unlock pipe.

Escalation:
If a provider restart loses or cannot mount durable state, that is disaster
recovery, not normal lifecycle.

### 4. User Backup Key is disaster recovery material

Decision:
On first successful chat pairing, the agent should deliver a recovery package
to the user so they can restore the Agent Runtime if provider durable state is
lost.

Reason:
This preserves non-custodial recovery without making every normal restart a
manual unlock flow.

Open detail:
The exact package format, UX wording, storage policy, and rotation story need
Austin/Alex review.

### 5. Distinguish Agent Chat Identity from Agent Signing Session

Decision:
The agent can have its own chat identity while also later receiving scoped
Agent Signing Sessions to act as the user's User Primary Key.

Reason:
Finite Chat needs the agent to appear as an agent participant. FiniteBrain and
future Nostr-native actions may need the agent to act as the user
cryptographically. Those are different concepts.

Implication:
Do not collapse "agent npub in chat" into "agent acts as user's Nostr key."

### 6. No PIN or legacy claim token in v2 hosted pairing

Decision:
Hosted pairing displays a Finite Chat invite with no PIN. The old dashboard
claim-token and connection flows are not part of v2.

Reason:
The PIN flow was poor UX and fragile for hosted agents. It also confused setup
with durable key custody.

## Recommended Near-Term Architecture

### First boot

1. Runner launches the runtime from the promoted OCI image.
2. Provider mounts durable state at `/data`.
3. Runtime boot checks whether `/data` already contains an Agent Root Secret or
   wrapped equivalent.
4. If none exists, runtime creates initial Agent Root Secret inside the
   confidential runtime.
5. Runtime derives or unwraps Agent Chat Identity and service identities.
6. Runtime publishes a no-PIN Finite Chat invite endpoint.
7. Core records plaintext-safe facts: runtime handle, artifact id, state schema,
   runtime status, invite/status URL, Hermes availability, and active inference
   profile.

### First user chat

1. User joins from the native app.
2. Agent verifies the room/pairing policy for a fresh hosted Project.
3. Agent sends a recovery package to the user.
4. The iOS app stores it in Keychain and clearly marks backup status.
5. Optional later UX allows explicit export/write-down.

### Normal restart

1. Dashboard/Core request `restart`.
2. Runner restarts the provider runtime with the same durable mount.
3. Runtime reads existing state from `/data`.
4. Runner/Core wait for heartbeat/readiness.
5. User keeps chatting without an unlock ceremony.

### Disaster recovery

1. Provider state is lost, inaccessible, or intentionally migrated.
2. New runtime starts without the Agent Root Secret.
3. User provides the User Backup Key through a recovery flow.
4. Runtime restores Agent Root Secret or rotates into a new one.
5. Core records plaintext-safe recovery status, not the key.

This flow is intentionally separate from normal restart.

## Hard Questions For Austin And Alex

### Q1. What exactly is inside the User Backup Key?

Recommended answer:
A versioned recovery package that can restore or rotate the Agent Root Secret,
not a raw nsec shown directly to the user.

The package should include:

- schema version;
- Project or agent id binding;
- public identity material;
- encrypted secret material or secret-share package;
- creation timestamp;
- rotation generation;
- checksum/authenticated metadata.

Need review:
Should the user ever see a human-readable phrase, or should the app store and
export an encrypted package/QR/file only?

### Q2. Is the Agent Root Secret one key or a seed for derived keys?

Recommended answer:
Treat it as a root seed or wrapping secret and derive separate keys with domain
separation.

Example domains:

- `finitechat.agent_identity`;
- `fsite.agent_identity`;
- `fbrain.agent_identity`;
- `local_state.encryption`;
- `profile.signing`.

Need review:
Which services truly need the same Nostr identity, and which should get
separate identities derived from one root?

### Q3. Should the Agent Chat Identity be agent-owned or user-owned?

Recommended answer:
Agent-owned for chat participation. User-owned signing should happen through
future Agent Signing Sessions.

Reason:
The chat UX needs "this agent is speaking." External user-authorized actions
need "this agent acted as the user under scope X."

Need review:
How does Finite Chat represent the relationship between the user's identity and
the agent's chat identity?

### Q4. How does finite-auth link to WorkOS users?

Recommended answer:
Core links WorkOS user id/email to a future User Primary Key after the user
proves control of that key. WorkOS remains account/billing auth; finite-auth
becomes cryptographic auth.

Need review:
Can one WorkOS account link multiple User Primary Keys? How do team accounts,
org billing, and account recovery work?

### Q5. Can Phala durable state be trusted as the normal key persistence layer?

Recommended answer:
For v2 launch, yes for normal restart behavior, but not as the only disaster
recovery mechanism.

Need review:
What are Phala's exact guarantees for durable volume encryption, migration,
snapshotting, deletion, and operator/provider visibility? If those guarantees
are insufficient, the Agent Root Secret should wrap runtime state so raw mount
contents are not enough to decrypt user data.

### Q6. Does the Agent Root Secret need at-rest encryption on the durable mount?

Recommended answer:
Yes, if we can do it without reintroducing a routine unlock problem. The root
secret should preferably be sealed to runtime/provider attestation or wrapped by
a key only available inside the confidential runtime.

Need review:
What sealing primitive does Phala give us today? If none, do we accept durable
mount custody risk for launch while relying on provider confidentiality, or do
we need app-level encryption immediately?

### Q7. What does "recover-known-good chat runtime" do in a key-aware world?

Recommended answer:
It should never replace cryptographic identity. It can reset generated Hermes
chat config and restart services, but must preserve Agent Root Secret, chat
identity, room membership, message state, and workspace data.

Need review:
Should recovery be an image-owned boot mode flag, a finitechat-core API, or a
dedicated runtime management API exposed by minimal `finitec`?

### Q8. How does the user rotate or revoke a compromised agent?

Recommended answer:
Core can stop/destroy compute, but cryptographic rotation must happen through
the user and agent runtime. A future "rotate agent root" flow should produce a
new backup package and invalidate old runtime-owned service keys.

Need review:
What can be revoked server-side if the old agent key has already signed Nostr
events or published service state?

### Q9. How do multiple user devices get recovery material?

Recommended answer:
The first device stores the User Backup Key in native secure storage. Additional
devices should receive recovery access only through an explicit user-approved
device-add flow, not silent server sync.

Need review:
Should iCloud Keychain count as acceptable platform sync? Should users be able
to disable sync and rely on manual export?

### Q10. What does finite-auth own for v2?

Recommended answer:
Not WorkOS replacement for launch. For v2, finite-auth should own:

- User Primary Key proof and sessions;
- future Frostr keyset metadata;
- Agent Signing Sessions;
- audit facts for acting as user;
- native secure storage guidance and package references.

It should not own:

- Core Project lifecycle;
- runtime provider handles;
- Finite Private limiter grants;
- Agent Runtime boot policy;
- native iOS Keychain implementation details.

Need review:
Do we need a small finite-auth integration in v2 now, or should v2 only prepare
the vocabulary and defer implementation until FiniteBrain/Finite Chat need it?

## Product Requirements This Implies

- The dashboard should show backup status, not backup secret material.
- The iOS app should explicitly say whether the recovery package is safely
  stored.
- Core should expose plaintext-safe runtime facts for support:
  runtime status, provider handle, image/artifact, state schema, last heartbeat,
  invite endpoint, and operation history.
- Core should not expose chat messages, user files, agent key material, or raw
  recovery package bytes.
- Agent runtime deploy should publish a profile for the Agent Chat Identity so
  users see a real name/avatar.
- Recovery UX should be tested in the same ladder as runtime deploy:
  local Docker, remote Docker, Phala.

## Proposed Next Decisions

1. Alex/Austin review this vocabulary:
   Account Auth, User Primary Key, Agent Chat Identity, Agent Signing Session,
   Agent Root Secret, User Backup Key.
2. Decide the User Backup Key package shape.
3. Decide whether Agent Root Secret is a root seed with service-specific
   derivations.
4. Decide what Phala sealing/durable volume guarantees are acceptable for v2.
5. Write an ADR only after those decisions are resolved.

## Current Recommendation

Ship v2 with WorkOS account auth, Phala durable mounts, Core lifecycle, and a
runtime-generated Agent Root Secret that Core never sees.

Add first-use User Backup Key delivery through Finite Chat, stored by the iOS
app in Keychain, as disaster recovery rather than routine restart unlock.

Prepare for `finite-auth` by using its vocabulary and keeping Agent Chat
Identity distinct from future Agent Signing Sessions. Do not block the SaaS
create-agent flow on Frostr implementation.

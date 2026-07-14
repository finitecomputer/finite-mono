# Finite Identity

Finite Identity owns the shared identity language for Finite tools and products. It defines how Finite-controlled email names, Nostr keys, NIP-05 names, and product principals relate to each other.

## Language

**Finite VIP Email**:
A Finite-controlled email address on `finite.vip` that is also the canonical user-facing identity string. Its full form is `localpart@finite.vip`.
_Avoid_: finite-vip email, account email, VIP address

**Finite VIP Domain**:
The `finite.vip` domain that hosts Finite VIP Emails and their matching NIP-05 names.
_Avoid_: finite-vip, VIP host

**NIP-05 Name**:
The public Nostr name for a Finite identity. In v1 it is always identical to the user's Finite VIP Email, such as `localpart@finite.vip`.
_Avoid_: handle, username, nostr email

**Third-Party NIP-05 Name**:
A NIP-05 identifier on a domain not owned by Finite. Third-Party NIP-05 Names are future work and are not trusted as product grantees in v1.
_Avoid_: external handle, external nostr address

**NIP-05 Endpoint**:
The public `.well-known/nostr.json` HTTP endpoint for the Finite VIP Domain. In v1, the Identity Authority owns the response for this endpoint.
_Avoid_: static nostr file, nostr profile endpoint

**Identity Recovery**:
The explicit process for restoring control of a Native Principal or moving its product authority to a replacement key without orphaning user data.
_Avoid_: reset, relink, silent reassignment

**Disabled Binding**:
A Finite VIP Email or NIP-05 Name binding that the Identity Authority keeps for audit history but no longer serves or resolves. Disabling a binding is an operator safety action, not Identity Recovery or reassignment.
_Avoid_: deleted binding, reset binding, transferred binding

**Principal**:
The identity subject that Finite products attach permissions to. A Principal is either a Native Principal or an Email-Only Principal.
_Avoid_: account, user, member

**Native Principal**:
A Principal backed by a Nostr public key controlled by a human or agent Finite identity keypair.
_Avoid_: native account, npub user

**User Nostr Identity**:
The human-controlled Nostr keypair used across that user's Hosted Web, Electron,
and iOS surfaces. Hosted Web keeps it behind a server-side signer adapter while
native surfaces keep it in protected local storage; the custody difference does
not create another Principal.
_Avoid_: WorkOS identity, hosted-device identity, agent key

**Agent Principal Key**:
The distinct Nostr keypair owned by one Agent Runtime and used for that agent's
operations across Finite products. It is never the user's User Nostr Identity
and never uses the user's hosted product adapter to act as that user.
_Avoid_: user key, shared signer, Account Auth

**Finite Home**:
The filesystem root that scopes one Local Identity Key and the Finite tool state belonging to that identity owner.
_Avoid_: User home, shared fleet home, product config directory

**Email-Only Principal**:
A Principal backed by verified control of an email address before the person has linked that email to a Native Principal.
_Avoid_: guest user, invited user, external account

**Invited Email**:
Any email address that a Finite product can grant access to before the recipient has a Native Principal. An Invited Email can become an Email-Only Principal, but only an address on the Finite VIP Domain can become a Finite VIP Email and NIP-05 Name in v1.
_Avoid_: external email, collaborator email

**Principal Link**:
A verified identity-equivalence relationship from an email address to a Native Principal. Products can use Principal Links during authorization without immediately rewriting their product-owned permission records. A Principal Link says the email and npub are the same Principal; it is never an authorization for a distinct agent to act for that Principal.
_Avoid_: alias, migration, account merge

**Email Access Delegation**:
A revocable product-owned authorization allowing a distinct Agent Principal to exercise a verified email Principal's grants inside exactly one Finite product.
_Avoid_: email link, account link, agent identity binding

**Principal Resolution**:
The Finite Identity answer to "who does this email, NIP-05 name, npub, or caller prove as right now?" Principal Resolution lets products attach permissions to stable product concepts while delegating identity proof and email-to-native links to Finite Identity.
_Avoid_: user lookup, account lookup, auth mapping

**Product Grant**:
A product-owned permission record that names a Principal or Invited Email exactly as the product user granted it. Finite Identity does not own Product Grants; it only resolves whether a caller satisfies them.
_Avoid_: identity grant, membership row, access mapping

**Identity Authority**:
The deployed Finite Identity service and its identity-owned storage. It is the source of truth for Principal Resolution, Finite VIP Email bindings, and NIP-05 Names.
_Avoid_: auth server, account service

**Identity Contract**:
The product-facing HTTP contract exposed by the Identity Authority. Finite products consume identity through this contract rather than by owning or directly mutating identity storage.
_Avoid_: internal API, shared database, crate API

**Identity Client Flow**:
A reusable client-side identity workflow implemented by Finite Identity and exposed through product CLIs. A standalone identity CLI may expose the same flows, but product users should not need to leave the product workflow for routine identity setup.
_Avoid_: fsite auth flow, fbrain auth flow

**Product Signer Adapter**:
A product-owned adapter that uses Finite Identity's key-storage and lifecycle
primitives to perform that product's validated identity operations without
handing raw key material to product client code. Each product owns its own
adapter and bounded provider contract; Finite Identity does not own a universal
product adapter, product grants, content crypto, or authorization policy.
For Hosted Web, Finite Chat's Hosted Device is the initial user-key setup and
custody flow. The product's adapter acts as the same User Nostr Identity used
by Electron and iOS, not as a separate product identity. Account Auth may
authorize its session, but the product must still grant the User Nostr Identity
access explicitly. It does not make the User Nostr Identity and an Agent
Principal Key the same identity.
_Avoid_: shared signer, generic signer API, product key store

**Resolution Cache**:
A short-lived product-held cache of Principal Resolution answers. A Resolution Cache is never the source of truth and must fail closed when an answer is missing, expired, or uncertain.
_Avoid_: identity replica, local identity store

**Local Identity Key**:
The human- or agent-owned Nostr keypair generated, imported, and stored under one Finite Home by the Finite Identity client contract.
_Avoid_: server key, account key, hosted key

**Binding Proof**:
The combined proof required to bind a Finite VIP Email to a Native Principal in v1: a valid email challenge token for the Finite VIP Email and a NIP-98-authenticated request signed by the target Local Identity Key.
_Avoid_: signup proof, verification proof, login proof

**Email Challenge**:
A short-lived, single-use proof request sent to an email address. The challenge token is opaque random secret material, stored only as a hash by the Identity Authority.
_Avoid_: magic token, signed token, email login

**Mailer Adapter**:
The deployment-specific implementation that delivers Email Challenges. Finite Identity owns the challenge flow, while a Mailer Adapter performs delivery through dev outbox, Resend, Postmark, or another provider.
_Avoid_: email service, notification service

## Relationships

- One **Finite Home** contains exactly one **Local Identity Key**.
- Each hosted agent has its own **Finite Home** and **Local Identity Key**;
  `finitechat`, `fsite`, and `fbrain` inside that agent use the same key.
- A human's Finite Chat identity lives separately from every agent **Finite
  Home** and may be generated or imported by the human.
- **Account Auth** is outside Finite Identity; proving a dashboard session does
  not reveal, replace, or silently mint a **Local Identity Key**.
- The **Identity Authority** stores public resolution/binding state and never a
  **Local Identity Key** secret.
- A **Product Grant** may name an email or Native Principal, but an agent does
  not satisfy a human email grant merely because the agent belongs to that
  human's Project.
- A **Principal Link** proves identity equivalence. An **Email Access
  Delegation** authorizes a different Principal; the two are never inferred
  from each other.
- One **Email Access Delegation** connects one verified email Principal, one
  Agent Principal, and one Finite product; revocation in that product grants no
  authority in another product.
- An Agent Principal exercising an **Email Access Delegation** still signs as
  itself, and product audit records both the agent and delegation.
- Finite Identity proves Principal relationships; each product owns issuance,
  enforcement, revocation, and resource-specific consequences of its **Email
  Access Delegations**.
- A Finite Product Release does not satisfy its recoverability promise unless
  **Identity Recovery** and the affected product-owned grants or encrypted key
  access are restored together.

## Example Dialogue

> **Dev:** "Do a user's Finite Chat app and hosted agent load the same identity file?"
> **Domain expert:** "No. Each has its own Finite Home and Local Identity Key; only the agent's tools share the agent key."

> **Dev:** "Does WorkOS become the agent's signing identity?"
> **Domain expert:** "No. Account Auth gates the dashboard; the agent's Local Identity Key signs agent operations."

> **Dev:** "If I let my agent use Sites shared to my email, does Brain inherit that access?"
> **Domain expert:** "No. The Sites Email Access Delegation is product-scoped; Brain needs its own delegation and Folder Key Grants to the agent npub."

## Flagged Ambiguities

- "Shared identity" was used to mean both shared code/path conventions and a
  shared human-agent signer. Resolved: Finite tools in one **Finite Home** share
  one **Local Identity Key**; humans and agents do not share that secret.
- The Identity Authority v1 contract deliberately omitted key-loss recovery;
  that omission is now a launch gap, not an acceptable permanent product state.
- "Link my email to my agent" previously mixed identity equivalence with
  authorization. Resolved: same-identity proof creates a **Principal Link**;
  cross-identity access creates a product-scoped **Email Access Delegation**.

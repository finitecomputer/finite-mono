# Finite Identity

Finite Identity owns the shared identity language for Finite tools and products. It defines how Finite-controlled email names, Nostr keys, and NIP-05 names relate to each other. Its deployed service is the NIP-05 name Directory; products answer every authorization question against their own tables.

## Language

**Mailbox Address**:
A deliverable address whose control can be proved with an Email Challenge. A
Mailbox Address may also be spelled like a NIP-05 Name, but the two types are
never interchangeable at a CLI or delivery boundary.
_Avoid_: email-shaped identifier, login email, Nostr email

**Finite VIP Mailbox Address**:
A Mailbox Address on the Finite-controlled `finite.vip` domain. Its full form
is `localpart@finite.vip`.
_Avoid_: finite-vip email, account email, VIP address

**Finite VIP Domain**:
The `finite.vip` domain that hosts Finite VIP Mailbox Addresses and NIP-05 Names.
_Avoid_: finite-vip, VIP host

**NIP-05 Name**:
The public resolution name for a Nostr key. Its `localpart@domain` spelling
does not prove that mail can be delivered to it.
_Avoid_: mailbox, login email, Nostr email

**Third-Party NIP-05 Name**:
A NIP-05 identifier on a domain not owned by Finite. Third-Party NIP-05 Names are future work and are not trusted as product grantees in v1.
_Avoid_: external handle, external nostr address

**NIP-05 Endpoint**:
The public `.well-known/nostr.json` HTTP endpoint for the Finite VIP Domain. In v1, the Identity Directory owns the response for this endpoint.
_Avoid_: static nostr file, nostr profile endpoint

**Identity Recovery**:
The explicit process for restoring control of a Native Principal or moving its product authority to a replacement key without orphaning user data.
_Avoid_: reset, relink, silent reassignment

**Disabled Binding**:
A Finite VIP Mailbox Address or NIP-05 Name binding that the Identity Directory keeps for audit history but no longer serves or resolves. Disabling a binding is an operator safety action, not Identity Recovery or reassignment.
_Avoid_: deleted binding, reset binding, transferred binding

**Principal**:
The identity subject that Finite products attach permissions to, identified
by its Nostr public key.
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

**Managed Agent NIP-05**:
The canonical NIP-05 Name assigned to one hosted agent and immutably bound to
that agent's Agent Principal Key by the trusted provisioning path. It is not a
Mailbox Address and must never be passed to an email-delivery flag. Products
show this readable name to people and resolve it through Finite Identity; the
underlying `npub` remains the authorization subject and an advanced diagnostic.
_Avoid_: agent email, agent account, shared user email

**Finite Home**:
The filesystem root that scopes one Local Identity Key and the Finite tool state belonging to that identity owner.
_Avoid_: User home, shared fleet home, product config directory

**Invited Email**:
Legacy name for a Mailbox Address that a Finite product granted access to
before the recipient had a Native Principal. Retired with the email-shaped
grant machinery: products grant npubs or capability links, and an email
address is delivery, never an identity.
_Avoid_: external email, collaborator email

**Email Access Delegation**:
A revocable product-owned authorization allowing a distinct Agent Principal to exercise a verified email Principal's grants inside exactly one Finite product.
_Avoid_: email link, account link, agent identity binding

**Product Grant**:
A product-owned permission record that names a Principal (npub) or a
capability token exactly as the product user granted it. Products resolve
Product Grants against their own tables; Finite Identity is never asked
whether a caller satisfies a grant.
_Avoid_: identity grant, membership row, access mapping

**Sites Email Principal**:
A Sites-owned durable access subject established by verified control of one
Mailbox Address. It can own an Authorized Sites Key set without becoming the
authorization model for Chat or the encryption subject for Brain.
_Avoid_: Sites account, global email identity

**Authorized Sites Key**:
A revocable human or agent `npub` authorized by a Sites Email Principal.
Possession is proved by signature; membership is added or revoked only through
the Sites mailbox-authority flow.
_Avoid_: linked identity, email key, shared signer

**Originating Publisher**:
The native `npub` that performed a Sites publish operation. It is durable audit
provenance even when access was exercised through a Sites Email Principal's
Authorized Sites Key set.
_Avoid_: owner email, publishing account

**Identity Directory**:
The deployed Finite Identity service and its identity-owned storage — the
shrunken Identity Authority; both names name the same deployment. It is the
source of truth for Finite VIP Mailbox bindings and NIP-05 Names: name lookup
and name claiming, plus operator audit/disable. It resolves nothing else.
_Avoid_: auth server, account service

**Identity Contract**:
The product-facing HTTP contract exposed by the Identity Directory. Finite
products use it for NIP-05 lookup and name claiming rather than by owning or
directly mutating identity storage.
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

**Local Identity Key**:
The human- or agent-owned Nostr keypair generated, imported, and stored under one Finite Home by the Finite Identity client contract.
_Avoid_: server key, account key, hosted key

**Binding Proof**:
The combined proof required to bind a Finite VIP Mailbox Address to a Native Principal in v1: a valid Email Challenge token for that mailbox and a NIP-98-authenticated request signed by the target Local Identity Key.
_Avoid_: signup proof, verification proof, login proof

**Email Challenge**:
A short-lived, single-use proof request sent to an email address. The challenge token is opaque random secret material, stored only as a hash by the Identity Directory. Its only remaining use is proving control of a Finite VIP Mailbox Address when claiming its NIP-05 Name.
_Avoid_: magic token, signed token, email login

**Mailer Adapter**:
The deployment-specific implementation that delivers Email Challenges. Finite Identity owns the challenge flow, while a Mailer Adapter performs delivery through dev outbox, the shared `finite-mail` Resend transport, or another provider.
_Avoid_: email service, notification service

## Relationships

- One **Finite Home** contains exactly one **Local Identity Key**.
- Each hosted agent has its own **Finite Home** and **Local Identity Key**;
  `finitechat`, `fsite`, and `fbrain` inside that agent use the same key.
- Each newly provisioned hosted agent has one **Managed Agent NIP-05**. The
  trusted runner registers the runtime's public Agent Principal Key; it never
  receives authority to issue product grants.
- Core remains the source of truth for which WorkOS account owns a hosted
  agent. Finite Identity binds that agent's Managed Agent NIP-05 to its Agent
  Principal Key, while each product owns any access role granted because of the
  account-agent association.
- A human's Finite Chat identity lives separately from every agent **Finite
  Home** and may be generated or imported by the human.
- **Account Auth** is outside Finite Identity; proving a dashboard session does
  not reveal, replace, or silently mint a **Local Identity Key**.
- The **Identity Directory** stores public binding state and never a
  **Local Identity Key** secret.
- A **Product Grant** names an npub or a capability token, never an email;
  an agent does not satisfy a grant merely because it belongs to a human's
  Project.
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
  authorization. Resolved by deletion: email-shaped identity claims died with
  the directory shrink; cross-identity access is a product-scoped **Email
  Access Delegation** or a capability link.

# Keep Personal Vaults User-Owned And Grant Agents Folder-Scoped Access

Status: superseded by ADR-0023 and ADR-0024 on 2026-07-16.

Historical implementation note: this ADR described the former limited-member,
Chat setup-ticket, Agent Workspace, and Folder-by-Folder delegation design.
ADR-0023 and ADR-0024 replace that design with one fully trusted Personal Agent
and standing account-bound bootstrap authority. This document is retained only
as decision history and is not current implementation guidance.

## Context

FiniteBrain's Personal Vault is each user's single starting place. The current
Finite identity boundary gives the user and every Agent Runtime different Nostr
keys: the user has a User Nostr Identity, while the runtime has its own Agent
Principal Key. The agent must be able to work for the user from Chat or from a
later Brain session without becoming a second Personal Vault owner, an
Organization Vault admin, or a holder of the user's secret.

The portable specification already permits limited Personal Vault members for
Folder-scoped sharing, but the current server/store only allow the owner to
view a Personal Vault and reject Personal Vault member mutation. The existing
product-scoped Email Access Delegation is also a documented contract rather
than a deployed Brain feature. This ADR resolves the intended product behavior;
implementation follows separately.

## Decision

- A user's first successful Brain initialization creates that user's one
  Personal Vault; no path creates a second. Its `ownerUserId` is the user's
  User Nostr Identity, and that identity remains the sole owner.
- An Agent Principal remains a distinct Member Identity. It never becomes a
  Personal Vault owner or admin, never uses the user's Brain Identity Provider,
  and never receives the user's identity secret.
- An explicit Chat request creates a short-lived, single-use **Personal Vault
  Bootstrap Authorization** bound to that user, that Agent Principal, and the
  initial Agent Workspace Folder. It is an internal Brain mechanism, not a
  second user-facing approval. It cannot create a second Vault or broaden
  access after setup.
- Brain owns a durable, revocable, audited Email Access Delegation for the
  user-agent pairing. The delegation is product-scoped and establishes neither
  identity equivalence nor content access by itself.
- With that explicit delegation, Brain may add the Agent Principal as a limited
  Personal Vault Member and issue Folder Access plus encrypted Folder Key Grants
  to the agent's own npub. The default initial scope is one dedicated restricted
  **Agent Workspace Folder**. The agent can read and write that Folder only.
- The owner may later grant another restricted Folder deliberately. Project,
  Account Auth, dashboard, and shared Sites state never grant the agent Brain
  access implicitly.
- A user-first bootstrap creates or ensures the user-owned Personal Vault and
  then establishes the same delegated Agent Workspace access. An agent-first
  bootstrap, initiated by an explicit user request in Chat, must consume the
  valid Bootstrap Authorization and atomically create the user-owned Personal
  Vault, durable delegation, limited agent access, and all required Folder Key
  Grants. Both paths converge on the same state.
- The explicit user-approved agent-first bootstrap is the only non-owner
  execution path that may create initial Agent Workspace access. Its
  authorization is consumed with the successful transaction and cannot be
  replayed. Once the Vault exists, only the Personal Vault owner can create,
  broaden, or revoke agent access. Revocation removes the durable delegation
  and access, then rotates every Folder Key in the revoked scope. It cannot
  promise retroactive erasure of plaintext or keys already retained by the
  agent.

## Consequences

- Personal Vaults stay personal while providing a safe, useful starting place
  for agents.
- Server authorization and metadata filtering must allow a limited Personal
  Vault Member to see only the Personal Vault metadata and Folders in its
  explicit scope.
- Personal Vault bootstrap, member/access mutation, grant-recipient validation,
  audit, one-use authorization consumption, and complete-scope revocation are
  enforced by the phase-one implementation and public conformance tests. Brain
  continues to fail closed rather than treating an agent's Project, email
  session, or user key as authority.
- Finite Sites needs its own future product delegation and policy. It does not
  inherit this Brain access.

## Rejected shapes

- Creating an Organization Vault merely to give the user's agent access.
- Making the agent a Personal Vault co-owner or admin.
- Sharing the user's Nostr secret with the agent or reviving `finite-auth`.
- Giving an agent every Personal Vault Folder by default.
- Treating the Email Access Delegation as a Folder Key Grant or a cross-product
  authorization.

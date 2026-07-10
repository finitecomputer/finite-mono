# Identity Boundary v1

Status: active product contract, accepted 2026-07-09. This replaces the removed
`finite-auth`/shared-user-agent-signer experiment and its auth/key-custody
brief.

## Problem Statement

Finite needs email-authenticated SaaS access and Nostr-authenticated agentic
operations without treating a WorkOS account, human chat key, agent key, or
device key as the same identity. Shared identity means shared Finite Identity
code and one key per Finite Home—not a secret shared between a human and agent.

## Canonical Identities

| Identity | Owner/custody | Purpose |
| --- | --- | --- |
| Account Auth | WorkOS session linked to Core | Gates personal dashboard, billing, Hosted Web Device access, and user-facing SaaS operations |
| User Nostr Identity | Human-controlled Finite Chat account; generated or imported by the human | Human participation in Finite Chat and Nostr-native user operations |
| Hosted Web Device | Finite-operated, separately revocable Device under the user's chat account | Trusted-server dashboard web chat; not browser E2EE |
| Electron/native Device | Local device key and durable store | Local-custody access to the same Finite Chat account and Rooms |
| Agent Principal Key | One key under the Agent Runtime's Finite Home on durable `/data` | Signs agent operations across Finite Chat, Finite Sites, and Finite Brain |
| Runtime Credential | Core-issued, runtime-scoped secret injected by Runner | Authenticates the narrow Runtime Management Pipe only |

## Invariants

- Every Agent Runtime owns its Agent Principal Key; product APIs, Core,
  dashboard, Runner processes, user Devices, and other agents never receive its
  nsec. A Kata host operator remains inside the physical trust boundary while
  the key is stored on an unsealed mounted volume; operator-blind custody needs
  an attested/encrypted runner and is not claimed for Kata v1.
- `FINITE_HOME=/data/agent` makes `finitechat`, `fsite`, and `fbrain` inside one
  runtime converge on that agent key through `finite-identity`.
- A user's Finite Chat nsec is separate from every Agent Principal Key and may
  be brought by the user.
- WorkOS gates SaaS access but is not a Nostr signer and does not authenticate
  agentic product mutations.
- Dashboard user requests carry the standard WorkOS AuthKit access token to
  Core. Core validates the JWT signature against the WorkOS JWKS and checks
  issuer, client id, expiry, and subject on every user-scoped request; caller-
  supplied identity headers are not authentication.
- The standard AuthKit access token does not carry email. After validating its
  `sub`, Core resolves that exact WorkOS user through the read-only User
  Management API and requires its email to be verified. Core does not require a
  custom JWT template or trust an email forwarded by Dashboard.
- Core consumes standard WorkOS session claims instead of minting a parallel
  Finite session token. Account Auth remains an adapter that can be replaced
  without changing Project, Agent Runtime, Principal, or product-grant
  semantics.
- Finite operators belong to one internal WorkOS operator organization. Core
  requires that exact validated `org_id` for every administrator request. The
  internal canary has one operator predicate across admin routes; named
  permissions and split duties are deferred until customer admission needs
  them.
- The WorkOS operator organization and Core Customer Organizations are
  different objects. Customer ownership, billing, Projects, and entitlements
  remain Core concepts, and ordinary customer/canary accounts do not become
  operator-organization members.
- Runner and other services use separate, route-scoped service credentials.
  A Runner credential cannot call user or administrator routes, assert a
  WorkOS subject, or elevate a user. Core reads the Runner capability from
  `FC_CORE_RUNNER_API_TOKEN`; the non-Runner service and Finite Private usage
  capabilities remain separate and all three configured values must differ.
- The Hosted Web Device is authorized by Account Auth but participates as a
  Finite Chat Device; it is not the agent and not room authority.
- Agent operations are attributable to the agent npub by default. Acting as the
  human requires a future explicit grant; there is no default shared signer or
  Agent Signing Session.
- Finite Identity owns local key conventions, NIP-98 helpers, email challenges,
  email/NIP-05 bindings, and Principal Resolution. Products own their grants.

## Email Sharing Boundary

Finite Sites and Finite Brain may store a Product Grant exactly as entered,
including an email address. A future trusted Account Auth adapter may let a
WorkOS-authenticated human satisfy a matching email grant in a user-facing web
flow by validating the WorkOS email claim server-side. The current Finite
Identity Principal Resolution API is pubkey-only and does not implement that
adapter. WorkOS is not treated as a Nostr signer. An Agent Principal does not
automatically satisfy that human email grant merely because its Project belongs
to the human.

The current Finite Identity contract allows multiple email-only pubkeys for a
general invited email, but a Finite VIP Email/NIP-05 binding resolves to one
pubkey. Therefore "let my agent access resources shared to my email" uses an
explicit, product-scoped, revocable Email Access Delegation rather than silently
binding the human's email to the agent or treating Google OAuth possession as
the human identity. A Principal Link may express that an email and npub are the
same Principal; it is never this delegation.

Finite Sites and Finite Brain own separate delegations. The agent always signs
as its Agent Principal Key and audits identify both the agent and delegation.
Brain authorization alone cannot decrypt content: Brain must also issue Folder
Key Grants to the agent npub for every readable Folder in scope. Revoking a
Sites delegation has no effect on Brain and vice versa.
This boundary is recorded in
[Finite Identity ADR 0016](../../finite-identity/docs/adr/0016-email-access-is-product-scoped-delegation.md).

Current implementation gap: no Sites or Brain service currently stores,
issues, resolves, audits, or revokes this first-class delegation. Finite
Identity Principal Resolution handles identity equivalence, not delegated
authority. Until each product implements its own delegation contract, agents
must fail closed instead of using `--link-native`, a human email session, or
Google OAuth possession as a substitute.

## Recovery Boundary

WorkOS can re-authenticate the human account but cannot reconstruct a User
Nostr Identity, Agent Principal Key, Device store, MLS history, or Finite Brain
Folder Key. The Finite Product Release Recoverability Contract must cover those
keys together with the product data they unlock.

The O1 first slice may use a separately protected Finite-Assisted Recovery
Authority under explicit, audited Break-Glass Recovery. That authority is not
ordinary Core state and does not make WorkOS a signer. Later O2/O3 releases may
remove unilateral Finite recovery only after a User Recovery Key or other
user-controlled path restores the same Recovery Set onto an empty target.

## Removed `finite-auth` Experiment

`finite-auth-core`, `finite-auth-store`, their documentation, and the obsolete
auth/key-custody brief were removed from the monorepo on 2026-07-09 after
confirming that no product crate consumed them. Git history and the monorepo
migration log retain the provenance. They are not part of the Finite Product
Release or target architecture and must not be reintroduced as a shared
human-agent signer.

## Evaluation

- Fresh Agent Runtime boot creates one durable agent identity and all three
  Finite tools report the same agent npub.
- A second Agent Runtime for the same WorkOS user creates a different npub.
- Importing a human nsec into Finite Chat never changes an agent identity.
- Hosted Web and Electron Devices join the same user account while retaining
  independent device stores.
- WorkOS logout blocks dashboard/Hosted Web access without revoking agent keys
  or stopping Nostr-authenticated agent operations.
- Core rejects an expired, wrong-issuer, wrong-client, or invalidly signed
  WorkOS access token and derives the acting user only from validated claims.
- Core rejects caller-supplied identity headers without a valid matching
  WorkOS access token, and a Runner credential is rejected on every user and
  administrator route.
- Core accepts every administrator route only when the validated token has the
  configured operator `org_id`; a missing or different organization fails
  closed.
- The internal WorkOS operator organization id is never persisted as or
  inferred to be a Core Customer Organization id.
- Core/Runner logs, facts, release evidence, and Runtime Management messages
  contain no user or agent nsec.
- A Sites Email Access Delegation lets the agent satisfy matching Sites email
  grants but no Brain grant; a separately issued Brain delegation without
  Folder Key Grants still cannot decrypt Folder content.
- Revocation blocks new delegated authorization without changing the human's
  Principal Link, NIP-05 identity, or the agent's npub.
- Deleting one human Device store, the Hosted Web Device store, or an Agent
  Provider Durable Volume exercises the declared Recovery Authority and either
  restores the same identity plus usable data or produces an explicit readable
  export/migration result named by the Recoverability Contract.
